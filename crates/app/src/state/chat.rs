//! The conversation of the open project: messages, the thread as the model has seen it, the
//! proposed changes and saving them. Pure steps come from `contextual_core::chat`; this model
//! runs the async ones (read files through the host, stream from Foundry, repair, save, undo).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use contextual_core::chat::{
    self, MessageImage, PreparedTurn, ReadLog, STOPPED, SavedThread, ThreadState, Tone,
    TurnOptions, UiMessage, Usage,
};
use contextual_core::host::WorkspaceHost;
use contextual_core::llm::types::{ApiKind, ChatEvent, ConversationRequest, RepairInput};
use contextual_core::llm::{FoundryClient, FoundryConfig};
use contextual_core::logic::changes::ChangeContext;
use contextual_core::logic::edits::format_edits;
use contextual_core::logic::format::estimate_tokens;
use contextual_core::logic::images::Attachment;
use contextual_core::logic::prompt::{system_prompt, with_reads};
use contextual_core::logic::reads::{self, LIMITS, ReadRequest, Resolved};
use contextual_core::logic::types::{Change, ChangeStatus, Deployment, Rule};
use contextual_core::threads::{ThreadStore, ThreadSummary};
use contextual_protocol::fs::{Apply, ApplyChange, ChangeStatus as WireStatus, ReadFiles, Undo};
use futures::StreamExt as _;
use gpui_kit::{Context, Entity, Task};
use tokio_util::sync::CancellationToken;

use crate::runtime;
use crate::state::Project;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterView {
    Chat,
    Diff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DiffMode {
    #[default]
    Unified,
    Split,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub title: String,
    pub detail: String,
    pub error: bool,
    /// Undo snapshot to restore.
    pub backup_id: Option<String>,
}

/// What the app needs from the environment for a turn (resolved at send time).
#[derive(Clone)]
pub struct SendEnv {
    pub config: FoundryConfig,
    pub deployment: Deployment,
    pub output_prompt: String,
    /// The model picks its files (project tree, read requests, skill index).
    pub auto: bool,
    /// The skills the model may load (system prompt), when `auto`.
    pub skill_index: String,
}

/// Read rounds of one user message.
#[derive(Clone, Copy, Debug, Default)]
struct Rounds {
    /// Rounds answered so far.
    n: usize,
    /// Files and tokens read automatically so far.
    files: usize,
    tokens: u64,
}

/// A turn to stream.
struct Turn {
    prepared: PreparedTurn,
    /// The project tree version this turn sends (recorded when it completes).
    tree_version: Option<String>,
    rounds: Rounds,
}

/// The reads to answer a request with.
struct ReadStep {
    /// The reply that asked.
    ai_id: String,
    resolved: Resolved,
    /// Over the limit and declined by the user (told to the model).
    held_back: Option<Resolved>,
    /// The reply changed files it hadn't been sent.
    unseen: bool,
    rounds: Rounds,
}

enum NextRound {
    Continue(Box<ReadStep>),
    /// Waiting for the user (over the limit).
    Wait,
}

/// A read request over the per-message limit, waiting for the user.
pub struct PendingReads {
    step: ReadStep,
    held: Resolved,
    env: SendEnv,
}

impl PendingReads {
    /// The reply it belongs to.
    pub fn ai_id(&self) -> &str {
        &self.step.ai_id
    }

    /// (files, estimated tokens) waiting.
    pub fn size(&self) -> (usize, u64) {
        (self.held.reads(), self.held.tokens)
    }
}

pub struct ChatModel {
    project: Entity<Project>,
    host: Arc<dyn WorkspaceHost>,
    store: Option<Arc<ThreadStore>>,
    pub thread: ThreadState,
    pub messages: Vec<UiMessage>,
    pub attachments: Vec<Attachment>,
    /// "Resend full context" for the next message.
    pub resend_all: bool,
    cancel: Option<CancellationToken>,
    /// File contents as sent (diff originals), by path.
    contents: HashMap<String, String>,
    /// Sent only as condensed references: never offered for saving.
    reference_only: HashSet<String>,
    /// Which proposed changes are ticked for saving.
    pub include: HashMap<String, bool>,
    pub center: CenterView,
    pub diff_mode: DiffMode,
    pub save_dialog_open: bool,
    pub pr_open: bool,
    pub saving: bool,
    pub save_error: Option<String>,
    pub toast: Option<Toast>,
    pub threads: Vec<ThreadSummary>,
    /// A read request waiting for the user.
    pub pending: Option<PendingReads>,
    _tasks: Vec<Task<()>>,
}

fn now() -> String {
    let t = runtime::now_local();
    format!("{:02}:{:02}", t.hour(), t.minute())
}

struct Ctx<'a> {
    project: &'a Project,
    contents: &'a HashMap<String, String>,
    reference_only: &'a HashSet<String>,
}

impl ChangeContext for Ctx<'_> {
    fn exists(&self, path: &str) -> bool {
        self.project.node(path).is_some()
    }
    fn original(&self, path: &str) -> Option<String> {
        self.contents.get(path).cloned()
    }
    fn changed_on_disk(&self, path: &str) -> bool {
        self.project
            .node(path)
            .is_some_and(|n| n.changed_on_disk == Some(true))
    }
    fn read_only(&self, path: &str) -> bool {
        self.reference_only.contains(path)
    }
}

impl ChatModel {
    pub fn new(
        project: Entity<Project>,
        host: Arc<dyn WorkspaceHost>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            project,
            host,
            store: ThreadStore::open_default().ok().map(Arc::new),
            thread: ThreadState::fresh(),
            messages: Vec::new(),
            attachments: Vec::new(),
            resend_all: false,
            cancel: None,
            contents: HashMap::new(),
            reference_only: HashSet::new(),
            include: HashMap::new(),
            center: CenterView::Chat,
            diff_mode: DiffMode::Unified,
            save_dialog_open: false,
            pr_open: false,
            saving: false,
            save_error: None,
            toast: None,
            threads: Vec::new(),
            pending: None,
            _tasks: Vec::new(),
        };
        this.refresh_threads(true, cx);
        this
    }

    pub fn streaming(&self) -> bool {
        self.cancel.is_some()
    }

    /// The changes of the latest reply that proposed any.
    pub fn changes(&self) -> Vec<Change> {
        SavedThread::last_changes(&self.messages)
    }

    pub fn included(&self) -> Vec<Change> {
        self.changes()
            .into_iter()
            .filter(|c| self.include.get(&c.path).copied().unwrap_or(false))
            .collect()
    }

    /// Saved chats are kept per project location (`Project::key`).
    fn thread_key(&self, cx: &Context<Self>) -> String {
        self.project.read(cx).key()
    }

    fn root(&self, cx: &Context<Self>) -> String {
        self.project.read(cx).root.clone()
    }

    fn note(&mut self, text: impl Into<String>, tone: Tone) {
        self.messages.push(UiMessage::note(text, tone, &now()));
    }

    // ------------------------------------------------------------------ send

    pub fn send(
        &mut self,
        text: String,
        env: SendEnv,
        rules_on: Vec<Rule>,
        cx: &mut Context<Self>,
    ) {
        if self.streaming() {
            return;
        }
        self.pending = None;
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());
        cx.notify();

        let p = self.project.read(cx);
        let root = p.root.clone();
        // Ticked files, plus the skills and skill files the model loaded in this chat.
        let mut selected = p.selected.clone();
        selected.extend(self.thread.loaded.iter().cloned());
        let mut paths: Vec<String> = selected.iter().cloned().collect();
        paths.sort();
        let tree = (env.auto && self.thread.tree_version.as_deref() != Some(&p.model_tree_version))
            .then(|| (p.model_tree.clone(), p.model_tree_version.clone()));
        let host = self.host.clone();
        let read_req = ReadFiles {
            root: root.clone(),
            paths,
            condense: true,
        };
        let read = runtime::spawn(cx, async move { host.read(read_req).await });

        let task = cx.spawn(async move |this, cx| {
            let read = read.await;
            let Ok(prepared) = this.update(cx, |this, cx| match read {
                Ok(r) => {
                    this.record_read(&r.files);
                    let opts = TurnOptions {
                        force_all: this.resend_all,
                        tree: tree.as_ref().map(|(t, _)| t.clone()),
                        followup: false,
                    };
                    Some(this.prepare(r.files, &selected, &text, rules_on.len(), opts, cx))
                }
                Err(e) => {
                    this.cancel = None;
                    this.note(
                        format!("Not sent: could not read the selected files. {}", e.message),
                        Tone::Error,
                    );
                    cx.notify();
                    None
                }
            }) else {
                return;
            };
            let Some(prepared) = prepared else { return };
            let turn = Turn {
                prepared,
                tree_version: tree.map(|(_, v)| v),
                rounds: Rounds::default(),
            };
            Self::stream(this, turn, env, rules_on, token, cx).await;
        });
        self._tasks.push(task);
    }

    /// Remembers file contents as sent (diff originals) and which were condensed references.
    fn record_read(&mut self, files: &[contextual_protocol::fs::FileContent]) {
        for f in files {
            if f.condensed_from.is_some() {
                self.reference_only.insert(f.path.clone());
            } else {
                self.reference_only.remove(&f.path);
                if let Some(c) = &f.content {
                    self.contents.insert(f.path.clone(), c.clone());
                }
            }
        }
    }

    /// Builds the user's turn and adds the user message.
    fn prepare(
        &mut self,
        files: Vec<contextual_protocol::fs::FileContent>,
        selected: &HashSet<String>,
        text: &str,
        rules: usize,
        opts: TurnOptions,
        cx: &mut Context<Self>,
    ) -> PreparedTurn {
        let images: Vec<MessageImage> = self
            .attachments
            .drain(..)
            .map(|a| MessageImage {
                name: a.name,
                url: a.url,
            })
            .collect();
        let tree_sent = opts.tree.is_some();
        let mut prepared =
            chat::prepare_turn(&self.thread, &files, selected, text, &images, rules, opts);
        if tree_sent {
            prepared.meta.push_str(" · project tree");
        }
        self.resend_all = false;
        self.messages.push(UiMessage::user(text, &prepared, &now()));
        cx.notify();
        prepared
    }

    async fn stream(
        this: gpui_kit::WeakEntity<Self>,
        turn: Turn,
        env: SendEnv,
        rules_on: Vec<Rule>,
        token: CancellationToken,
        cx: &mut gpui_kit::AsyncApp,
    ) {
        let Turn {
            prepared,
            tree_version,
            rounds,
        } = turn;
        let Ok((base, ai_id)) = this.update(cx, |this, cx| {
            let ai = UiMessage::assistant(&env.deployment.name, &now());
            let id = ai.id.clone();
            this.messages.push(ai);
            cx.notify();
            (this.thread.clone(), id)
        }) else {
            return;
        };

        let mut system = system_prompt(&rules_on, &env.output_prompt);
        if env.auto {
            system = with_reads(system, &env.skill_index);
        }
        let req = ConversationRequest {
            system,
            history: chat::history(&base),
            input: chat::to_message_content(&prepared.input),
            previous_response_id: base.response_id.clone(),
        };
        let config = env.config.clone();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<ChatEvent>();
        let stream_token = token.clone();
        let streamer = cx.update(|cx| {
            runtime::spawn(cx, async move {
                let client = FoundryClient::new(config)?;
                let mut events = client.stream_conversation(req, stream_token);
                while let Some(e) = events.next().await {
                    if tx.unbounded_send(e).is_err() {
                        break;
                    }
                }
                Ok(())
            })
        });

        let mut reply = String::new();
        let mut usage: Option<Usage> = None;
        let mut done: Option<(Option<String>, Option<String>, ApiKind, bool)> = None;
        let mut failure: Option<String> = None;
        while let Some(event) = rx.next().await {
            match event {
                ChatEvent::Delta { text } => {
                    reply.push_str(&text);
                    // A read request is shown as a row, not as a code block.
                    let current = if env.auto {
                        reads::without_read_blocks(&reply)
                    } else {
                        reply.clone()
                    };
                    let alive = this
                        .update(cx, |this, cx| {
                            if let Some(m) = this.messages.iter_mut().find(|m| m.id == ai_id) {
                                m.text = current;
                            }
                            cx.notify();
                        })
                        .is_ok();
                    if !alive {
                        token.cancel();
                        return;
                    }
                }
                ChatEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cached_tokens,
                } => {
                    usage = Some(Usage {
                        input_tokens,
                        output_tokens,
                        cached_tokens,
                    })
                }
                ChatEvent::Done {
                    finish_reason,
                    response_id,
                    api,
                    chain_reset,
                } => done = Some((finish_reason, response_id, api, chain_reset)),
                ChatEvent::Error { message } => failure = Some(message),
            }
        }
        if let Err(e) = streamer.await
            && failure.is_none()
        {
            failure = Some(e.message);
        }
        if token.is_cancelled() {
            failure = Some(STOPPED.into());
        }

        let truncated = done
            .as_ref()
            .is_some_and(|d| d.0.as_deref() == Some("length"));
        let completed = !reply.is_empty() && done.is_some() && failure.is_none();

        // Commit the turn, then see whether the model asked for context (or edited files it
        // hadn't seen): if so, answer it in another round instead of finishing.
        let Ok(next) = this.update(cx, |this, cx| {
            if completed && let Some((_, response_id, api, _)) = &done {
                this.thread.response_id = response_id.clone();
                this.thread.api = Some(*api);
                this.thread.transcript.push(chat::ApiMessage {
                    role: contextual_core::llm::types::ChatRole::User,
                    content: prepared.input.clone(),
                });
                this.thread.transcript.push(chat::ApiMessage {
                    role: contextual_core::llm::types::ChatRole::Assistant,
                    content: contextual_core::logic::prompt::ApiContent::Text(reply.clone()),
                });
                this.thread.file_versions = prepared.versions.clone();
                if let Some(v) = &tree_version {
                    this.thread.tree_version = Some(v.clone());
                }
            }
            if !(env.auto && completed && !truncated) {
                return None;
            }
            this.next_round(&reply, &ai_id, &rounds, usage, &env, cx)
        }) else {
            return;
        };
        if let Some(step) = next {
            match step {
                NextRound::Continue(next) => {
                    let _ = this.update(cx, |this, cx| {
                        this.continue_reads(*next, env, rules_on, cx);
                    });
                }
                NextRound::Wait => {}
            }
            return;
        }

        // Originals for every touched file the model wasn't sent, so diffs compare against disk.
        let Ok((root, missing, host)) = this.update(cx, |this, cx| {
            let p = this.project.read(cx);
            let missing: Vec<String> = chat::touched_paths(&reply)
                .into_iter()
                .filter(|path| p.node(path).is_some() && !this.contents.contains_key(path))
                .collect();
            (this.root(cx), missing, this.host.clone())
        }) else {
            return;
        };
        if !missing.is_empty() {
            let req = ReadFiles {
                root,
                paths: missing,
                condense: false,
            };
            let job = cx.update(|cx| runtime::spawn(cx, async move { host.read(req).await }));
            if let Ok(r) = job.await {
                let _ = this.update(cx, |this, _| {
                    for f in r.files {
                        if let Some(c) = f.content {
                            this.contents.insert(f.path, c);
                        }
                    }
                });
            }
        }

        // Parse changes; quietly ask for files whose edit blocks couldn't be placed.
        let Ok(mut proposed) = this.update(cx, |this, cx| {
            let p = this.project.read(cx);
            let ctx = Ctx {
                project: p,
                contents: &this.contents,
                reference_only: &this.reference_only,
            };
            chat::proposed_changes(&reply, truncated, &ctx)
        }) else {
            return;
        };
        // Each reply shows its own usage; earlier rounds show theirs.
        let mut usage_total = usage;
        for change in proposed.iter_mut() {
            let Some(unplaced) = change.unplaced.clone().filter(|u| !u.is_empty()) else {
                continue;
            };
            let input = RepairInput {
                path: change.path.clone(),
                original: change.old.clone(),
                edits: format_edits(change.edits.as_deref().unwrap_or(&unplaced)),
            };
            let config = env.config.clone();
            let job = cx.update(|cx| {
                runtime::spawn(cx, async move {
                    FoundryClient::new(config)?
                        .repair_file(&input, &CancellationToken::new())
                        .await
                })
            });
            match job.await {
                Ok(r) => {
                    change.new = r.content;
                    change.unplaced = None;
                    if let Some(u) = usage_total.as_mut() {
                        u.input_tokens += r.input_tokens;
                        u.output_tokens += r.output_tokens;
                    }
                }
                Err(_) => {
                    change.conflict.get_or_insert_with(|| {
                        "The proposed changes could not be matched to the current file. Ask again to get them.".into()
                    });
                }
            }
        }
        let changes = chat::visible_changes(proposed);

        let _ = this.update(cx, |this, cx| {
            let (meta, cost) = match &usage_total {
                Some(u) => {
                    let (m, c) = chat::usage_meta(u, &env.deployment);
                    (Some(m), c)
                }
                None => (None, None),
            };
            let api = done.as_ref().map(|d| d.2);
            if reply.is_empty() {
                this.messages.retain(|m| m.id != ai_id);
            } else if let Some(m) = this.messages.iter_mut().find(|m| m.id == ai_id) {
                m.streaming = false;
                m.meta = meta;
                m.changes = Some(changes.clone());
                m.api = api;
                m.usage = usage_total;
                m.cost = cost;
            }
            let chain_reset = done.as_ref().is_some_and(|d| d.3);
            for (t, tone) in
                chat::turn_notes(failure.as_deref(), chain_reset, api, base.api, truncated)
            {
                this.note(t, tone);
            }
            if env.auto && completed && rounds.n > LIMITS.rounds {
                this.note(
                    "The model kept asking for files after the read limit; its last reply is shown as is.",
                    Tone::Warn,
                );
            }
            if !changes.is_empty() {
                this.include = changes
                    .iter()
                    .map(|c| (c.path.clone(), c.conflict.is_none()))
                    .collect();
            }
            this.cancel = None;
            cx.notify();
            if completed {
                this.save_thread(cx);
            }
        });
    }

    /// After a completed reply (model picks files): the next round if the model asked for
    /// context or changed files it hadn't been sent; `None` to finish the reply as usual.
    fn next_round(
        &mut self,
        reply: &str,
        ai_id: &str,
        rounds: &Rounds,
        usage: Option<Usage>,
        env: &SendEnv,
        cx: &mut Context<Self>,
    ) -> Option<NextRound> {
        if rounds.n > LIMITS.rounds {
            return None;
        }
        let p = self.project.read(cx);
        let sent: BTreeSet<String> = self.thread.file_versions.keys().cloned().collect();
        let (request, unseen) = match reads::parse_read_request(reply) {
            Some(req) => (req, false),
            None => {
                // Changes to existing files the model was never sent: send them, ask again.
                let unseen: Vec<String> = chat::touched_paths(reply)
                    .into_iter()
                    .filter(|path| {
                        p.node(path).is_some()
                            && !sent.contains(path)
                            && !self.reference_only.contains(path)
                    })
                    .collect();
                if unseen.is_empty() || rounds.n == LIMITS.rounds {
                    return None;
                }
                (
                    ReadRequest {
                        paths: unseen,
                        folders: Vec::new(),
                    },
                    true,
                )
            }
        };
        let resolved = reads::resolve(&request, p.tree(), &p.rules, &p.skills_never, &sent);
        let (now_ok, held) = reads::within_limits(resolved, rounds.files, rounds.tokens, LIMITS);
        let mut log = ReadLog::from_resolved(&now_ok);
        log.unseen_edits = unseen;
        let deployment = env.deployment.clone();
        if let Some(m) = self.messages.iter_mut().find(|m| m.id == ai_id) {
            m.text = reads::without_read_blocks(reply);
            m.streaming = false;
            m.changes = None;
            if let Some(u) = usage {
                let (meta, cost) = chat::usage_meta(&u, &deployment);
                m.meta = Some(meta);
                m.usage = Some(u);
                m.cost = cost;
            }
            if let Some(h) = &held {
                log.held_back = h.files.iter().chain(&h.skill_files).cloned().collect();
            }
            m.reads = Some(log);
        }
        let next = Rounds {
            n: rounds.n + 1,
            files: rounds.files,
            tokens: rounds.tokens,
        };
        let step = ReadStep {
            ai_id: ai_id.to_string(),
            resolved: now_ok,
            held_back: None,
            unseen,
            rounds: next,
        };
        if let Some(held) = held {
            // Past the limit for one message: the user decides about the rest.
            self.pending = Some(PendingReads {
                step,
                held,
                env: env.clone(),
            });
            self.cancel = None;
            self.save_thread(cx);
            cx.notify();
            return Some(NextRound::Wait);
        }
        Some(NextRound::Continue(Box::new(step)))
    }

    /// Reads what the model asked for, ticks it, and sends it as the next round.
    fn continue_reads(
        &mut self,
        step: ReadStep,
        env: SendEnv,
        rules_on: Vec<Rule>,
        cx: &mut Context<Self>,
    ) {
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());
        let r = &step.resolved;
        self.project.update(cx, |p, cx| {
            p.add_model_picks(&r.files);
            p.model_loaded.extend(
                r.skill_files
                    .iter()
                    .filter(|f| f.ends_with("SKILL.md"))
                    .cloned(),
            );
            cx.notify();
        });
        for f in &r.skill_files {
            if !self.thread.loaded.contains(f) {
                self.thread.loaded.push(f.clone());
            }
        }
        let paths: Vec<String> = r.files.iter().chain(&r.skill_files).cloned().collect();
        let mut selected = self.project.read(cx).selected.clone();
        selected.extend(self.thread.loaded.iter().cloned());
        let host = self.host.clone();
        let read_req = ReadFiles {
            root: self.root(cx),
            paths,
            condense: true,
        };
        let read = (!read_req.paths.is_empty())
            .then(|| runtime::spawn(cx, async move { host.read(read_req).await }));
        cx.notify();
        let task = cx.spawn(async move |this, cx| {
            let files = match read {
                None => Vec::new(),
                Some(job) => match job.await {
                    Ok(r) => r.files,
                    Err(e) => {
                        let _ = this.update(cx, |this, cx| {
                            this.cancel = None;
                            this.note(
                                format!(
                                    "Could not read the files the model asked for. {}",
                                    e.message
                                ),
                                Tone::Error,
                            );
                            cx.notify();
                        });
                        return;
                    }
                },
            };
            if token.is_cancelled() {
                let _ = this.update(cx, |this, cx| {
                    this.cancel = None;
                    this.note(STOPPED, Tone::Warn);
                    cx.notify();
                });
                return;
            }
            let Ok(turn) = this.update(cx, |this, _| {
                this.record_read(&files);
                let read_tokens: u64 = files
                    .iter()
                    .map(|f| f.content.as_deref().map_or(0, estimate_tokens))
                    .sum();
                if let Some(m) = this.messages.iter_mut().find(|m| m.id == step.ai_id)
                    && let Some(log) = m.reads.as_mut()
                {
                    log.tokens = read_tokens;
                }
                let last = step.rounds.n == LIMITS.rounds;
                let text = if step.unseen {
                    reads::unseen_edit_text(&step.resolved.files)
                } else {
                    reads::followup_text(&step.resolved, step.held_back.as_ref(), last)
                };
                let prepared = chat::prepare_turn(
                    &this.thread,
                    &files,
                    &selected,
                    &text,
                    &[],
                    rules_on.len(),
                    TurnOptions {
                        followup: true,
                        ..TurnOptions::default()
                    },
                );
                let mut rounds = step.rounds;
                rounds.files += step.resolved.reads();
                rounds.tokens += read_tokens;
                Turn {
                    prepared,
                    tree_version: None,
                    rounds,
                }
            }) else {
                return;
            };
            Self::stream(this, turn, env, rules_on, token, cx).await;
        });
        self._tasks.push(task);
    }

    /// The model asked for more than one message may read: send it all (`true`) or tell the
    /// model the rest was declined.
    pub fn answer_pending(&mut self, send: bool, rules_on: Vec<Rule>, cx: &mut Context<Self>) {
        let Some(PendingReads {
            mut step,
            held,
            env,
        }) = self.pending.take()
        else {
            return;
        };
        if send {
            step.resolved.files.extend(held.files);
            step.resolved.skill_files.extend(held.skill_files);
            step.resolved.tokens += held.tokens;
            if let Some(log) = self
                .messages
                .iter_mut()
                .find(|m| m.id == step.ai_id)
                .and_then(|m| m.reads.as_mut())
            {
                let back = std::mem::take(&mut log.held_back);
                log.files.extend(back);
            }
        } else {
            step.held_back = Some(held);
        }
        self.continue_reads(step, env, rules_on, cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = &self.cancel {
            t.cancel();
        }
        self.pending = None;
        cx.notify();
    }

    /// A new, empty chat (the files stay ticked).
    pub fn start_fresh(&mut self, cx: &mut Context<Self>) {
        if self.streaming() {
            return;
        }
        self.reference_only.clear();
        self.pending = None;
        self.project.update(cx, |p, cx| {
            p.clear_model_marks();
            cx.notify()
        });
        self.thread = ThreadState::fresh();
        self.messages.clear();
        self.include.clear();
        self.attachments.clear();
        self.center = CenterView::Chat;
        cx.notify();
    }

    // ------------------------------------------------------------------ threads

    fn save_thread(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let saved = SavedThread::new(
            self.thread.clone(),
            self.messages
                .iter()
                .cloned()
                .map(|mut m| {
                    m.streaming = false;
                    m
                })
                .collect(),
        );
        let (root, id, title, turns) = (
            self.thread_key(cx),
            self.thread.id.clone(),
            saved.title(),
            saved.turns() as u64,
        );
        let job = runtime::spawn(cx, async move {
            let value = serde_json::to_value(&saved)?;
            store.save(&root, &id, Some(&title), turns, &value).await
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.refresh_threads(false, cx),
                Err(_) => {
                    this.toast = Some(Toast {
                        title: "Could not save the conversation".into(),
                        detail: "It stays open, but won’t be restored after a restart.".into(),
                        error: true,
                        backup_id: None,
                    });
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Reloads the Chats list; with `restore`, opens the newest chat (as on opening a folder).
    pub fn refresh_threads(&mut self, restore: bool, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let root = self.thread_key(cx);
        let job = runtime::spawn(cx, async move { store.list(&root).await });
        cx.spawn(async move |this, cx| {
            let list = job.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                let newest = list.first().map(|t| t.id.clone());
                this.threads = list;
                if restore && let Some(id) = newest {
                    this.open_thread(id, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_thread(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.streaming() {
            return;
        }
        let root = self.thread_key(cx);
        let job = runtime::spawn(cx, async move { store.load(&root, &id).await });
        cx.spawn(async move |this, cx| {
            let Ok(value) = job.await else { return };
            let Ok(saved) = serde_json::from_value::<SavedThread>(value) else {
                return;
            };
            this.update(cx, |this, cx| {
                // Re-tick the files the model has from this chat.
                let paths: Vec<String> = saved.thread.file_versions.keys().cloned().collect();
                let loaded = saved.thread.loaded.clone();
                this.project.update(cx, |p, cx| {
                    p.select_paths(&paths);
                    p.clear_model_marks();
                    p.model_loaded
                        .extend(loaded.into_iter().filter(|f| f.ends_with("SKILL.md")));
                    cx.notify()
                });
                this.pending = None;
                this.include = SavedThread::last_changes(&saved.messages)
                    .iter()
                    .map(|c| (c.path.clone(), c.conflict.is_none()))
                    .collect();
                for c in SavedThread::last_changes(&saved.messages) {
                    if c.status != ChangeStatus::A {
                        this.contents.entry(c.path.clone()).or_insert(c.old.clone());
                    }
                }
                this.thread = saved.thread;
                this.messages = saved
                    .messages
                    .into_iter()
                    .map(|mut m| {
                        m.streaming = false;
                        m
                    })
                    .collect();
                this.center = CenterView::Chat;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn rename_thread(&mut self, id: String, title: String, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let root = self.thread_key(cx);
        if id == self.thread.id {
            self.thread.title = Some(title.clone());
        }
        let job = runtime::spawn(cx, async move { store.rename(&root, &id, &title).await });
        cx.spawn(async move |this, cx| {
            let _ = job.await;
            this.update(cx, |this, cx| this.refresh_threads(false, cx))
                .ok();
        })
        .detach();
    }

    pub fn delete_thread(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let root = self.thread_key(cx);
        if id == self.thread.id {
            self.start_fresh(cx);
        }
        let job = runtime::spawn(cx, async move { store.delete(&root, &id).await });
        cx.spawn(async move |this, cx| {
            let _ = job.await;
            this.update(cx, |this, cx| this.refresh_threads(false, cx))
                .ok();
        })
        .detach();
    }

    pub fn delete_all_threads(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let root = self.thread_key(cx);
        self.start_fresh(cx);
        let job = runtime::spawn(cx, async move { store.delete_all(&root).await });
        cx.spawn(async move |this, cx| {
            let _ = job.await;
            this.update(cx, |this, cx| this.refresh_threads(false, cx))
                .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------------ save / undo

    pub fn toggle_include(&mut self, path: &str, cx: &mut Context<Self>) {
        let on = self.include.get(path).copied().unwrap_or(false);
        self.include.insert(path.to_string(), !on);
        cx.notify();
    }

    /// Saves the ticked changes in place (with an undo snapshot); `on_saved` rescans.
    pub fn save(
        &mut self,
        on_saved: impl FnOnce(&mut gpui_kit::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let picked = self.included();
        if picked.is_empty() || self.saving {
            return;
        }
        self.saving = true;
        self.save_error = None;
        cx.notify();
        let root = self.root(cx);
        let branch = self.project.read(cx).branch.clone();
        let req = Apply {
            root,
            backup: true,
            changes: picked
                .iter()
                .map(|c| ApplyChange {
                    path: c.path.clone(),
                    status: match c.status {
                        ChangeStatus::A => WireStatus::A,
                        ChangeStatus::M => WireStatus::M,
                        ChangeStatus::D => WireStatus::D,
                    },
                    content: (c.status != ChangeStatus::D).then(|| c.new.clone()),
                    expected: (c.status != ChangeStatus::A).then(|| c.old.clone()),
                })
                .collect(),
        };
        let host = self.host.clone();
        let job = runtime::spawn(cx, async move { host.apply(req).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(r) => {
                        for c in &picked {
                            if c.status == ChangeStatus::D {
                                this.contents.remove(&c.path);
                            } else {
                                this.contents.insert(c.path.clone(), c.new.clone());
                            }
                        }
                        chat::record_saved(&mut this.thread, &picked);
                        let (title, detail) = chat::saved_toast(&picked, branch.as_deref());
                        this.toast = Some(Toast {
                            title,
                            detail,
                            error: false,
                            backup_id: r.backup_id,
                        });
                        this.save_dialog_open = false;
                        this.save_thread(cx);
                        on_saved(cx);
                    }
                    Err(e) => this.save_error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn undo(
        &mut self,
        on_undone: impl FnOnce(&mut gpui_kit::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(backup_id) = self.toast.as_ref().and_then(|t| t.backup_id.clone()) else {
            return;
        };
        let req = Undo {
            root: self.root(cx),
            backup_id,
        };
        let host = self.host.clone();
        let job = runtime::spawn(cx, async move { host.undo(req).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.toast = Some(match result {
                    Ok(r) => {
                        this.contents.clear();
                        Toast {
                            title: "Changes undone".into(),
                            detail: format!("Restored {}, removed {}", r.restored, r.removed),
                            error: false,
                            backup_id: None,
                        }
                    }
                    Err(e) => Toast {
                        title: "Undo failed".into(),
                        detail: e.message,
                        error: true,
                        backup_id: None,
                    },
                });
                on_undone(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Notes a created (or updated) pull request in the chat and the thread.
    pub fn record_pr(
        &mut self,
        pr: contextual_protocol::git::PrResult,
        text: String,
        cx: &mut Context<Self>,
    ) {
        self.thread.pull_requests.push(chat::PullRequestRef {
            url: pr.url,
            number: pr.number,
            branch: pr.branch,
        });
        self.note(text, Tone::Ok);
        self.save_thread(cx);
        cx.notify();
    }

    pub fn dismiss_toast(&mut self, cx: &mut Context<Self>) {
        self.toast = None;
        cx.notify();
    }

    pub fn add_attachment(&mut self, a: Attachment, cx: &mut Context<Self>) {
        self.attachments.push(a);
        cx.notify();
    }
}
