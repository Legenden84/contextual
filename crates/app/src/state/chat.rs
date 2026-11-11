//! The conversation of the open project: messages, the thread as the model has seen it, the
//! proposed changes and saving them. Pure steps come from `contextual_core::chat`; this model
//! runs the async ones (read files through the host, stream from Foundry, repair, save, undo).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use contextual_core::chat::{
    self, MessageImage, PreparedTurn, ReadLog, STOPPED, SavedThread, ThreadState, Tone,
    TurnOptions, UiMessage, Usage,
};
use contextual_core::commands::execution::{ExecutionProgress, ExecutionResult};
use contextual_core::commands::history::{RecordStatus, ShareChoice, SharedResult};
use contextual_core::commands::{CallId, CommandRequest, LaunchPermit, ReviewPlan};
use contextual_core::host::WorkspaceHost;
use contextual_core::inspect::LiveRound;
use contextual_core::llm::pacing::display_seconds;
use contextual_core::llm::types::{ApiKind, ChatEvent, ConversationRequest, RepairInput};
use contextual_core::llm::{FoundryClient, FoundryConfig};
use contextual_core::llm::{ToolCall, WaitReason};
use contextual_core::location::ProjectRoot;
use contextual_core::logic::changes::ChangeContext;
use contextual_core::logic::context::LiveContextTokens;
use contextual_core::logic::edits::format_edits;
use contextual_core::logic::format::estimate_tokens;
use contextual_core::logic::images::Attachment;
use contextual_core::logic::markdown::Block;
use contextual_core::logic::prompt::ApiContent;
use contextual_core::logic::prompt::{system_prompt, with_commands, with_reads};
use contextual_core::logic::reads::{self, LIMITS, ReadRequest, Resolved};
use contextual_core::logic::types::{Change, ChangeStatus, Deployment, Rule};
use contextual_core::threads::{ThreadStore, ThreadSummary};
use contextual_protocol::commands::PrepareCommand;
use contextual_protocol::fs::{Apply, ApplyChange, ChangeStatus as WireStatus, ReadFiles, Undo};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use tokio_util::sync::{CancellationToken, DropGuard};

use crate::runtime;
use crate::state::{Project, Tab};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CenterView {
    Chat,
    Diff,
    /// Every round as sent and received (the Inspect view).
    Inspect,
}

/// Observable work, not an inference about the model's private reasoning.
/// This is transient UI state and is never stored in the conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatActivity {
    PreparingContext,
    WaitingForResponse,
    WritingResponse,
    ReadingFiles,
    CheckingEdits,
    SavingChanges,
    UndoingChanges,
    Stopping,
}

impl ChatActivity {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreparingContext => "Preparing context",
            Self::WaitingForResponse => "Waiting for response",
            Self::WritingResponse => "Writing response",
            Self::ReadingFiles => "Reading files",
            Self::CheckingEdits => "Checking edits",
            Self::SavingChanges => "Saving changes",
            Self::UndoingChanges => "Undoing changes",
            Self::Stopping => "Stopping",
        }
    }
}

/// A request held back for Azure capacity: sent by itself when the wait is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacityWait {
    pub until: std::time::Instant,
    pub reason: WaitReason,
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
    /// Offer the command-proposal tool in this turn (development switch).
    pub commands: bool,
}

/// Called when a command may have changed files.
type FilesChanged = std::rc::Rc<dyn Fn(&mut App)>;

/// What a command card needs beyond what is recorded: transient, never saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardRuntime {
    /// The host is preparing the proposal.
    pub preparing: bool,
    /// Run was pressed and the launch is being set up.
    pub starting: bool,
    pub error: Option<String>,
}

/// The one command that runs in this chat.
struct RunningCommand {
    call: CallId,
    stop: CancellationToken,
    /// The latest bounded output, shown while it runs. In memory only.
    progress: ExecutionProgress,
}

/// What the background launch reports back while it runs.
enum Launch {
    Started,
    Progress(ExecutionProgress),
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
    /// The turn only returns command results: the user typed nothing.
    results_only: bool,
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
    /// Most recently prepared reply: message id and blocks matching that message's text.
    /// Transient, shared with the render cache, and never persisted. Only one snapshot is held.
    pub reply_preview: Option<(String, Arc<Vec<Block>>)>,
    /// Changes when context accounting inputs in the stored thread change, not on deltas.
    pub context_revision: u64,
    /// Changes on every thread replacement, including reopening the same saved thread.
    pub thread_generation: u64,
    pub attachments: Vec<Attachment>,
    cancel: Option<CancellationToken>,
    activity: ChatActivity,
    /// Set while the request of the streaming round waits for quota or a rate limit.
    capacity_wait: Option<CapacityWait>,
    /// File contents as sent (diff originals), by path.
    contents: HashMap<String, String>,
    /// Sent only as condensed references: never offered for saving.
    reference_only: HashSet<String>,
    /// Which proposed changes are ticked for saving.
    pub include: HashMap<String, bool>,
    /// Immutable cumulative snapshot for the shared diff cache, separate from historical replies.
    change_snapshot: Option<UiMessage>,
    /// Changes belonging to the currently offered undo snapshot.
    undo_changes: Vec<Change>,
    pub diff_mode: DiffMode,
    pub save_dialog_open: bool,
    pub pr_open: bool,
    pub saving: bool,
    pub save_error: Option<String>,
    pub toast: Option<Toast>,
    pub threads: Vec<ThreadSummary>,
    /// Changes when the saved-history listing is published, never on response deltas.
    pub threads_revision: u64,
    /// A read request waiting for the user.
    pub pending: Option<PendingReads>,
    /// The round streaming now, as sent (the Inspect view shows it before it completes).
    pub live: Option<LiveRound>,
    /// Small, incremental gauge estimate; never persisted or added to stored history.
    pub live_context: Option<LiveContextTokens>,
    /// Approved commands can run for this project's host (development switch).
    pub commands_enabled: bool,
    /// Identifies this opening of the project to the host's command preparation.
    project_epoch: String,
    command_cards: HashMap<CallId, CardRuntime>,
    running: Option<RunningCommand>,
    /// Results the user chose to share when the conversation continues, by provider call
    /// identity. In memory only until they are sent.
    staged_shares: BTreeMap<String, SharedResult>,
    /// Cancels the command being launched or running when this chat goes away with its
    /// project, so that nothing keeps running unobserved.
    command_scope: Option<DropGuard>,
    /// Turns true when the host has finished with the running command, cleanup included.
    command_done: Option<tokio::sync::watch::Receiver<bool>>,
    /// Told when a command may have changed files, so that the project is rescanned.
    files_changed: Option<FilesChanged>,
    _quit_sub: Subscription,
    _navigation_sub: Subscription,
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
        // Project owns the active tab. Notify feature views when navigation changes,
        // including opening previews from the explorer or Source Control.
        let mut active = project.read(cx).active.clone();
        let navigation_sub = cx.observe(&project, move |_, project, cx| {
            let next = &project.read(cx).active;
            if &active != next {
                active = next.clone();
                cx.notify();
            }
        });
        let mut this = Self {
            project,
            host,
            store: ThreadStore::open_default().ok().map(Arc::new),
            thread: ThreadState::fresh(),
            messages: Vec::new(),
            reply_preview: None,
            context_revision: 0,
            thread_generation: 0,
            attachments: Vec::new(),
            cancel: None,
            activity: ChatActivity::PreparingContext,
            capacity_wait: None,
            contents: HashMap::new(),
            reference_only: HashSet::new(),
            include: HashMap::new(),
            change_snapshot: None,
            undo_changes: Vec::new(),
            diff_mode: DiffMode::Unified,
            save_dialog_open: false,
            pr_open: false,
            saving: false,
            save_error: None,
            toast: None,
            threads: Vec::new(),
            threads_revision: 0,
            pending: None,
            live: None,
            live_context: None,
            commands_enabled: false,
            project_epoch: format!(
                "project-{}",
                contextual_core::commands::InvocationId::new().as_str()
            ),
            command_cards: HashMap::new(),
            running: None,
            staged_shares: BTreeMap::new(),
            command_scope: None,
            command_done: None,
            files_changed: None,
            _quit_sub: cx.on_app_quit(Self::stop_command_for_quit),
            _navigation_sub: navigation_sub,
            _tasks: Vec::new(),
        };
        this.refresh_threads(cx);
        this
    }

    pub fn streaming(&self) -> bool {
        self.cancel.is_some()
    }

    /// Gate the last reported phase on the actual operation lifetime. Every existing
    /// completion/error path clears `cancel` or `saving`, so no stale busy label remains.
    /// The wait still ahead of the streaming round's request, if it is being held back.
    pub fn capacity_wait(&self) -> Option<(std::time::Duration, WaitReason)> {
        let wait = self
            .capacity_wait
            .filter(|_| self.activity() == Some(ChatActivity::WaitingForResponse))?;
        let left = wait
            .until
            .saturating_duration_since(std::time::Instant::now());
        (!left.is_zero()).then_some((left, wait.reason))
    }

    /// The activity line: the activity's label, or a countdown while the request waits.
    pub fn activity_text(&self) -> Option<String> {
        let activity = self.activity()?;
        Some(match self.capacity_wait() {
            Some((left, WaitReason::Quota)) => {
                format!("Waiting for token quota · {} s", display_seconds(left))
            }
            Some((left, WaitReason::RateLimited)) => {
                format!(
                    "Rate limited by Azure · retrying in {} s",
                    display_seconds(left)
                )
            }
            None => activity.label().to_string(),
        })
    }

    pub fn activity(&self) -> Option<ChatActivity> {
        if let Some(token) = &self.cancel {
            Some(if token.is_cancelled() {
                ChatActivity::Stopping
            } else {
                self.activity
            })
        } else {
            self.saving.then_some(self.activity)
        }
    }

    /// Feature visibility is derived from the workspace's single active tab.
    pub fn center(&self, cx: &App) -> CenterView {
        match &self.project.read(cx).active {
            Tab::Diff => CenterView::Diff,
            Tab::Inspect => CenterView::Inspect,
            Tab::Chat | Tab::Preview(_) => CenterView::Chat,
        }
    }

    /// Existing feature entry points open or activate a workspace tab.
    pub fn set_center(&mut self, center: CenterView, cx: &mut Context<Self>) {
        let tab = match center {
            CenterView::Chat => Tab::Chat,
            CenterView::Diff => Tab::Diff,
            CenterView::Inspect => Tab::Inspect,
        };
        self.project.update(cx, |p, cx| {
            p.activate(tab);
            cx.notify();
        });
        cx.notify();
    }

    /// The cumulative pending set, with its own immutable identity for background diffs.
    /// Historical messages retain their own changes and cache keys.
    pub fn change_message(&self) -> Option<&UiMessage> {
        self.change_snapshot.as_ref()
    }

    fn set_changes(&mut self, changes: Vec<Change>) {
        self.include
            .retain(|path, _| changes.iter().any(|c| &c.path == path));
        for change in &changes {
            let included = self.include.entry(change.path.clone()).or_insert(true);
            if change.conflict.is_some() {
                *included = false;
            }
        }
        self.thread.pending_changes = Some(changes.clone());
        // Pending versions are part of the next input, even before Save.
        self.context_revision = self.context_revision.wrapping_add(1);
        self.change_snapshot = if changes.is_empty() {
            None
        } else {
            let mut snapshot = UiMessage::assistant("", "");
            snapshot.streaming = false;
            snapshot.changes = Some(changes);
            Some(snapshot)
        };
    }

    /// Borrowed changes for render paths; do not clone file contents just to show counts.
    pub fn changes_ref(&self) -> &[Change] {
        self.change_message()
            .and_then(|m| m.changes.as_deref())
            .unwrap_or_default()
    }

    /// An owned snapshot for operations that need to outlive a model borrow.
    pub fn changes(&self) -> Vec<Change> {
        self.changes_ref().to_vec()
    }

    pub fn included(&self) -> Vec<Change> {
        self.changes_ref()
            .iter()
            .filter(|c| self.include.get(&c.path).copied().unwrap_or(false))
            .cloned()
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
        if self.streaming() || self.saving {
            return;
        }
        if self.command_running() {
            // Continuing would have to answer the running command's call.
            self.note(
                "Not sent: a command is still running. Stop it or wait for it to finish.",
                Tone::Warn,
            );
            cx.notify();
            return;
        }
        self.pending = None;
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());
        self.activity = ChatActivity::PreparingContext;
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
                        force_all: false,
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
                results_only: false,
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
            results_only,
        } = turn;
        let Ok((base, ai_id, staged)) = this.update(cx, |this, cx| {
            let ai = UiMessage::assistant(&env.deployment.name, &now());
            let id = ai.id.clone();
            this.messages.push(ai);
            this.activity = ChatActivity::WaitingForResponse;
            cx.notify();
            (this.thread.clone(), id, this.staged_shares.clone())
        }) else {
            return;
        };
        // Every command call the model made in its last reply is answered with this request:
        // rejected, not run, "ran, result not shared", or exactly the result the user chose
        // to share in the preview. Nothing else about a command is sent.
        let answers = match base.plan_answers(&staged) {
            Ok(answers) => answers,
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    this.messages.retain(|m| m.id != ai_id);
                    this.note(format!("Not sent: {}", error.message), Tone::Error);
                    this.cancel = None;
                    cx.notify();
                });
                return;
            }
        };

        let mut system = system_prompt(&rules_on, &env.output_prompt);
        if env.auto {
            system = with_reads(system, &env.skill_index);
        }
        if env.commands {
            // Only in turns that offer the tool: how to use it, and that proposing runs
            // nothing. Inspect shows it with the rest of the system prompt.
            system = with_commands(system);
        }
        let _ = this.update(cx, |this, cx| {
            this.live_context = Some(LiveContextTokens::new(&system, &prepared.input));
            this.live = Some(LiveRound {
                input: prepared.input.clone(),
                system: system.clone(),
                followup: prepared.followup,
                reply: String::new(),
            });
            cx.notify();
        });
        let system_sent = system.clone();
        let req = ConversationRequest {
            system,
            history: chat::history(&base),
            input: chat::to_message_content(&prepared.input),
            previous_response_id: base.response_id.clone(),
            command_tool: env.commands,
            tool_results: chat::tool_results(&answers),
        };
        let config = env.config.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel(runtime::STREAM_QUEUE_CAPACITY);
        let stream_token = token.clone();
        let streamer = cx.update(|cx| {
            runtime::spawn(cx, async move {
                let client = FoundryClient::new(config)?;
                let events = client.stream_conversation(req, stream_token.clone());
                runtime::forward_chat_events(events, tx, stream_token).await;
                Ok(())
            })
        });

        let mut reply = String::new();
        let mut usage: Option<Usage> = None;
        let mut done: Option<(Option<String>, Option<String>, ApiKind, bool)> = None;
        let mut failure: Option<String> = None;
        // Command calls of the reply. They are only recorded as proposals below; receiving
        // one prepares, approves and starts nothing.
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut tools_unsupported: Option<String> = None;
        // Continue draining after Stop so already accepted text is retained. The bridge
        // cancels upstream independently and flushes its final buffered text before closing.
        while let Some(event) = rx.recv().await {
            match event {
                ChatEvent::Waiting { seconds, reason } => {
                    let alive = this
                        .update(cx, |this, cx| {
                            this.capacity_wait = Some(CapacityWait {
                                until: std::time::Instant::now()
                                    + std::time::Duration::from_secs(seconds),
                                reason,
                            });
                            cx.notify();
                        })
                        .is_ok();
                    if !alive {
                        token.cancel();
                        return;
                    }
                }
                ChatEvent::Delta { text } => {
                    reply.push_str(&text);
                    // Keep the raw response here, but prepare its display on the blocking
                    // pool. Awaiting one job at a time preserves ordering and lets the
                    // existing bounded bridge apply backpressure while GPUI stays free.
                    let source = reply.clone();
                    let auto = env.auto;
                    let job = cx.update(|cx| {
                        runtime::spawn(
                            cx,
                            async move { runtime::prepare_reply(source, auto).await },
                        )
                    });
                    let (current, parsed) = match job.await {
                        Ok((current, parsed)) => (current, Some(parsed)),
                        Err(_) => {
                            // Preserve the previous display path if the background task
                            // fails. No raw text, stream events or proposal data are lost.
                            tracing::warn!(
                                target: "contextual::performance",
                                "Background reply preparation failed; using foreground fallback"
                            );
                            let current = if env.auto {
                                reads::without_read_blocks(&reply)
                            } else {
                                reply.clone()
                            };
                            (current, None)
                        }
                    };
                    let alive = this
                        .update(cx, |this, cx| {
                            if !text.is_empty() {
                                this.activity = ChatActivity::WritingResponse;
                                this.capacity_wait = None;
                            }
                            if let Some(m) = this.messages.iter_mut().find(|m| m.id == ai_id) {
                                m.text = current;
                            }
                            this.reply_preview = parsed.map(|blocks| (ai_id.clone(), blocks));
                            if let Some(l) = this.live.as_mut() {
                                l.reply.push_str(&text);
                            }
                            if let Some(context) = this.live_context.as_mut() {
                                context.push_reply(&text);
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
                ChatEvent::ToolCall {
                    call_id,
                    name,
                    arguments,
                    problem,
                } => calls.push(ToolCall {
                    call_id,
                    name,
                    arguments,
                    problem,
                }),
                ChatEvent::ToolsUnsupported { message } => tools_unsupported = Some(message),
            }
        }
        let _ = this.update(cx, |this, _| this.capacity_wait = None);
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
        // A reply can consist of command proposals alone.
        let proposed_commands = !calls.is_empty();
        let mut completed =
            (!reply.is_empty() || proposed_commands) && done.is_some() && failure.is_none();

        // Commit the turn, then see whether the model asked for context (or edited files it
        // hadn't seen): if so, answer it in another round instead of finishing.
        let Ok(next) = this.update(cx, |this, cx| {
            if completed {
                if answers.is_empty() && calls.is_empty() {
                    this.thread.record_round(
                        prepared.input.clone(),
                        &system_sent,
                        prepared.followup,
                        reply.clone(),
                        usage,
                    );
                } else {
                    let location = this.project.read(cx).location.clone();
                    let answered: Vec<CallId> =
                        answers.iter().map(|planned| planned.call.clone()).collect();
                    let recorded = this.thread.record_command_round(
                        answers,
                        (!results_only).then(|| prepared.input.clone()),
                        &system_sent,
                        prepared.followup,
                        reply.clone(),
                        std::mem::take(&mut calls),
                        usage,
                        &location,
                        &contextual_core::threads::now_iso(),
                    );
                    if recorded.is_ok() {
                        // The model has its answers: what was held for sharing is done with.
                        this.staged_shares.clear();
                        this.project.update(cx, |project, _| {
                            for call in &answered {
                                project.commands.forget_output(call);
                            }
                        });
                    }
                    if let Err(error) = recorded {
                        // Nothing was recorded: the chat is as it was before this turn.
                        failure = Some(format!(
                            "The reply could not be recorded. {}",
                            error.message
                        ));
                        completed = false;
                    }
                }
            }
            if completed && let Some((_, response_id, api, _)) = &done {
                this.thread.response_id = response_id.clone();
                this.thread.api = Some(*api);
                let turn = this
                    .thread
                    .transcript
                    .last()
                    .and_then(|message| message.command_turn.clone());
                if let Some(m) = this.messages.iter_mut().find(|m| m.id == ai_id) {
                    m.command_turn = turn;
                }
                this.thread.file_versions = prepared.versions.clone();
                if let Some(v) = &tree_version {
                    this.thread.tree_version = Some(v.clone());
                }
                this.context_revision = this.context_revision.wrapping_add(1);
            }
            this.live = None;
            this.live_context = None;
            this.activity = ChatActivity::CheckingEdits;
            cx.notify();
            // Automatic read follow-ups pause when the reply proposes commands: the next
            // request must wait for the user's decisions.
            if !(env.auto && completed && !truncated) || proposed_commands {
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
            if completed {
                chat::proposed_changes_on_top(&reply, truncated, &ctx, this.changes_ref())
            } else {
                // An interrupted or failed reply must not supersede reviewed proposals.
                Vec::new()
            }
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
            if reply.is_empty() && !(completed && proposed_commands) {
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
                let merged = chat::merge_pending_changes(this.changes_ref(), &changes);
                this.set_changes(merged);
            }
            if let Some(message) = tools_unsupported {
                this.note(message, Tone::Warn);
            }
            this.cancel = None;
            cx.notify();
            if completed {
                this.save_thread(cx);
                this.prepare_open_commands(cx);
            }
        });
    }

    // ------------------------------------------------------------------ commands

    /// A command started from this chat is running.
    pub fn command_running(&self) -> bool {
        self.running.is_some()
    }

    /// Registers what to do when a command may have changed files: rescan the project,
    /// which also refreshes Source Control.
    pub fn on_files_changed(&mut self, rescan: impl Fn(&mut App) + 'static) {
        self.files_changed = Some(std::rc::Rc::new(rescan));
    }

    /// The app is quitting: stop the command, then give its host a bounded moment to end the
    /// process tree and record what it observed. The framework only waits very briefly for
    /// quit handlers, so the wait happens here, and only when a command is in flight.
    fn stop_command_for_quit(
        &mut self,
        _: &mut Context<Self>,
    ) -> impl Future<Output = ()> + 'static + use<> {
        const QUIT_WAIT: std::time::Duration = std::time::Duration::from_secs(6);
        const STEP: std::time::Duration = std::time::Duration::from_millis(20);
        // Dropping the guard cancels the command's scope, in whatever phase it is.
        drop(self.command_scope.take());
        if let Some(done) = self.command_done.take() {
            let deadline = std::time::Instant::now() + QUIT_WAIT;
            // A closed channel means the launch ended without a running command.
            while !*done.borrow() && done.has_changed().is_ok() {
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(STEP);
            }
        }
        std::future::ready(())
    }

    /// After an attempt that may have run: whatever it changed on disk must not be
    /// silently overwritten, and what the app shows must be current.
    ///
    /// Unsaved proposals and the last save are checked against the files as they are now.
    /// A proposal whose file changed is kept but marked as a conflict, which leaves it out
    /// of Save; an Undo that would overwrite later work is withdrawn. File contents
    /// remembered for diffs are replaced by what was just read, and the project is
    /// rescanned. File versions already sent to the model need no reset: every turn reads
    /// the selected files again and resends those whose content changed.
    fn refresh_after_command(&mut self, cx: &mut Context<Self>) {
        let pending = self.changes();
        let saved = self.undo_changes.clone();
        let mut paths: Vec<String> = pending
            .iter()
            .chain(&saved)
            .map(|change| change.path.clone())
            .collect();
        paths.sort();
        paths.dedup();
        // Originals remembered for other files may be stale now; they are read again
        // when a reply touches them.
        self.contents.clear();
        self.context_revision = self.context_revision.wrapping_add(1);
        let rescan = self.files_changed.clone();
        if paths.is_empty() {
            if let Some(rescan) = rescan {
                cx.defer(move |cx| rescan(cx));
            }
            return;
        }
        let (host, root) = (self.host.clone(), self.root(cx));
        let checked = paths.clone();
        let job = runtime::spawn(cx, async move {
            host.read(ReadFiles {
                root,
                paths: checked,
                condense: false,
            })
            .await
        });
        cx.spawn(async move |this, cx| {
            let read = job.await;
            this.update(cx, |this, cx| {
                // A file that cannot be read now counts as changed.
                let mut disk: BTreeMap<String, Option<String>> =
                    paths.into_iter().map(|path| (path, None)).collect();
                if let Ok(read) = read {
                    for file in read.files {
                        if file.error.is_none() && file.condensed_from.is_none() {
                            disk.insert(file.path, file.content);
                        }
                    }
                }
                for (path, text) in &disk {
                    if let Some(text) = text {
                        this.contents.insert(path.clone(), text.clone());
                    }
                }
                let checked = chat::revalidate_after_command(&pending, &saved, &disk);
                if !checked.conflicts.is_empty() {
                    let n = checked.conflicts.len();
                    this.note(
                        format!(
                            "The command changed {n} {} with unsaved proposals. {} marked as \
                             {} and left out of Save until you review {}.",
                            if n == 1 { "file" } else { "files" },
                            if n == 1 { "It is" } else { "They are" },
                            if n == 1 { "a conflict" } else { "conflicts" },
                            if n == 1 { "it" } else { "them" },
                        ),
                        Tone::Warn,
                    );
                    this.set_changes(checked.changes);
                }
                if checked.undo_stale {
                    this.undo_changes.clear();
                    if this
                        .toast
                        .as_ref()
                        .is_some_and(|toast| toast.backup_id.is_some())
                    {
                        this.toast = None;
                    }
                    this.note(
                        "Undo for the last save is no longer offered: its files changed afterwards.",
                        Tone::Warn,
                    );
                }
                this.save_thread(cx);
                cx.notify();
                if let Some(rescan) = rescan {
                    cx.defer(move |cx| rescan(cx));
                }
            })
            .ok();
        })
        .detach();
    }

    /// The result the user chose to share for this proposal, not yet sent.
    pub fn staged_share(&self, call: &CallId) -> Option<&SharedResult> {
        self.staged_shares.get(&call.call)
    }

    /// Builds exactly what would be shared for a finished attempt whose output is still
    /// held. `None` omits a stream; text that differs from the capture is the user's edit.
    fn shared_result(
        &self,
        call: &CallId,
        stdout: Option<String>,
        stderr: Option<String>,
        cx: &App,
    ) -> contextual_core::Result<SharedResult> {
        let gone = || {
            contextual_core::Error::conflict(
                "This command's output is no longer held, so it cannot be shared.",
            )
        };
        let desk = &self.project.read(cx).commands;
        let (invocation, result) = desk.output(call).ok_or_else(gone)?;
        let attempt = self
            .thread
            .commands
            .get(call)
            .and_then(|record| record.attempts.last())
            .filter(|attempt| &attempt.invocation == invocation)
            .ok_or_else(gone)?;
        let choice = ShareChoice::from_edit(&result.output, stdout, stderr)?;
        Ok(SharedResult::observed(call, attempt, result, &choice))
    }

    /// The text the model would receive for these choices, for the share preview.
    pub fn share_preview(
        &self,
        call: &CallId,
        stdout: Option<String>,
        stderr: Option<String>,
        cx: &App,
    ) -> Result<String, String> {
        let result = self
            .shared_result(call, stdout, stderr, cx)
            .map_err(|error| error.message)?;
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
    }

    /// The user confirmed the preview: this result is sent when the conversation continues.
    /// Nothing is sent by this.
    pub fn stage_share(
        &mut self,
        call: &CallId,
        stdout: Option<String>,
        stderr: Option<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.shared_result(call, stdout, stderr, cx) {
            Ok(result) => {
                self.staged_shares.insert(call.call.clone(), result);
                self.card_mut(call).error = None;
                cx.notify();
                true
            }
            Err(error) => {
                self.fail_card(call, error.message, cx);
                false
            }
        }
    }

    pub fn unstage_share(&mut self, call: &CallId, cx: &mut Context<Self>) {
        self.staged_shares.remove(&call.call);
        cx.notify();
    }

    /// What every open call of the last reply would be told right now.
    pub fn pending_answers(&self) -> Vec<chat::PlannedAnswer> {
        self.thread
            .plan_answers(&self.staged_shares)
            .unwrap_or_default()
    }

    /// The turn of the last reply, while it still has calls the model was not told about.
    pub fn open_command_turn(&self) -> Option<&str> {
        let (turn, calls) = self.thread.unanswered_calls();
        turn.filter(|_| !calls.is_empty())
    }

    /// Continues the conversation with command results only: the user typed nothing. One
    /// request, which answers every open call of the last reply. Any commands in the reply
    /// come back as proposals awaiting review.
    pub fn continue_commands(&mut self, env: SendEnv, rules_on: Vec<Rule>, cx: &mut Context<Self>) {
        if self.streaming()
            || self.saving
            || self.command_running()
            || self.open_command_turn().is_none()
        {
            return;
        }
        self.pending = None;
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());
        self.activity = ChatActivity::WaitingForResponse;
        cx.notify();
        let turn = Turn {
            prepared: PreparedTurn {
                input: ApiContent::Text(String::new()),
                versions: self.thread.file_versions.clone(),
                images: Vec::new(),
                meta: String::new(),
                followup: false,
            },
            tree_version: None,
            rounds: Rounds::default(),
            results_only: true,
        };
        let task = cx.spawn(async move |this, cx| {
            Self::stream(this, turn, env, rules_on, token, cx).await;
        });
        self._tasks.push(task);
    }

    /// Transient state of one command card.
    pub fn card_runtime(&self, call: &CallId) -> CardRuntime {
        self.command_cards.get(call).cloned().unwrap_or_default()
    }

    /// The bounded output of the running command, if `call` is the one running.
    pub fn running_output(&self, call: &CallId) -> Option<&ExecutionProgress> {
        self.running
            .as_ref()
            .filter(|running| &running.call == call)
            .map(|running| &running.progress)
    }

    fn card_mut(&mut self, call: &CallId) -> &mut CardRuntime {
        self.command_cards.entry(call.clone()).or_default()
    }

    fn fail_card(&mut self, call: &CallId, message: impl Into<String>, cx: &mut Context<Self>) {
        let card = self.card_mut(call);
        card.preparing = false;
        card.starting = false;
        card.error = Some(message.into());
        cx.notify();
    }

    /// Asks the host to inspect a request for this project. Inspection only.
    fn preparation(
        &self,
        request: &CommandRequest,
        cx: &Context<Self>,
    ) -> impl Future<Output = contextual_core::Result<ReviewPlan>> + 'static + use<> {
        let project = self.project.read(cx);
        let root = ProjectRoot {
            location: project.location.clone(),
            path: project.root.clone(),
        };
        let prepare = serde_json::to_string(request).map(|command_json| PrepareCommand {
            root: root.path.clone(),
            project_epoch: self.project_epoch.clone(),
            command_json,
        });
        let host = self.host.clone();
        runtime::spawn(cx, async move {
            let prepared = host.prepare_command(prepare?).await?;
            ReviewPlan::from_prepared(root, prepared)
        })
    }

    /// Prepares every proposal that still awaits the user and has no plan on screen: after
    /// a reply, and after a saved chat is opened. This approves nothing.
    pub fn prepare_open_commands(&mut self, cx: &mut Context<Self>) {
        if !self.commands_enabled {
            return;
        }
        let desk = &self.project.read(cx).commands;
        let waiting: Vec<CallId> = self
            .thread
            .commands
            .records()
            .iter()
            .filter(|record| {
                record.answered.is_none()
                    && record.status() == RecordStatus::NeedsReview
                    && desk.plan(&record.call).is_none()
            })
            .map(|record| record.call.clone())
            .collect();
        for call in waiting {
            self.prepare_command(call, None, cx);
        }
    }

    /// Prepares one proposal and puts the plan up for review. `edited` replaces the request
    /// with the user's version; otherwise it is what last ran, or what the model proposed.
    pub fn prepare_command(
        &mut self,
        call: CallId,
        edited: Option<CommandRequest>,
        cx: &mut Context<Self>,
    ) {
        if !self.commands_enabled || self.card_runtime(&call).preparing {
            return;
        }
        let Some(record) = self.thread.commands.get(&call) else {
            return;
        };
        let request = edited
            .or_else(|| {
                record
                    .attempts
                    .last()
                    .map(|attempt| attempt.request.clone())
            })
            .or_else(|| record.request().cloned());
        let Some(request) = request else {
            return;
        };
        let card = self.card_mut(&call);
        card.preparing = true;
        card.error = None;
        cx.notify();
        let job = self.preparation(&request, cx);
        cx.spawn(async move |this, cx| {
            let plan = job.await;
            this.update(cx, |this, cx| {
                let reviewed = plan.and_then(|plan| {
                    this.project
                        .update(cx, |project, _| project.commands.review(&call, plan))
                });
                match reviewed {
                    Ok(()) => {
                        this.card_mut(&call).preparing = false;
                        cx.notify();
                    }
                    Err(error) => this.fail_card(&call, error.message, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// The user opened the editor: approval is void from this moment, whatever follows.
    pub fn edit_command(&mut self, call: &CallId, cx: &mut Context<Self>) {
        let voided = self
            .project
            .update(cx, |project, _| project.commands.editing(call));
        if let Err(error) = voided {
            self.fail_card(call, error.message, cx);
        }
        cx.notify();
    }

    /// The user finished editing: the new text is validated and prepared, and goes back
    /// up for review. Nothing is approved by editing.
    pub fn review_edit(&mut self, call: CallId, text: &str, cx: &mut Context<Self>) -> bool {
        match contextual_core::commands::parse_arguments(text) {
            Ok(request) => {
                self.prepare_command(call, Some(request), cx);
                true
            }
            Err(error) => {
                self.fail_card(&call, error.message, cx);
                false
            }
        }
    }

    pub fn reject_command(&mut self, call: &CallId, cx: &mut Context<Self>) {
        // The desk only knows proposals that were prepared; the record decides.
        let _ = self
            .project
            .update(cx, |project, _| project.commands.reject(call));
        match self
            .thread
            .commands
            .reject(call, &contextual_core::threads::now_iso())
        {
            Ok(()) => {
                self.command_cards.remove(call);
                self.save_thread(cx);
                cx.notify();
            }
            Err(error) => self.fail_card(call, error.message, cx),
        }
    }

    /// The user pressed Run on the plan shown for `call`.
    ///
    /// Approval is taken synchronously, so a second click cannot produce a second one. The
    /// host then prepares the request again; only if that matches what was shown is the
    /// approval consumed and the command started.
    pub fn run_command(&mut self, call: CallId, cx: &mut Context<Self>) {
        if self.streaming() || self.saving || self.command_running() {
            return;
        }
        let Some(shown) = self.project.read(cx).commands.plan(&call).cloned() else {
            return;
        };
        let approval = match self
            .project
            .update(cx, |project, _| project.commands.approve(&call, &shown))
        {
            Ok(approval) => approval,
            Err(error) => return self.fail_card(&call, error.message, cx),
        };
        let card = self.card_mut(&call);
        card.starting = true;
        card.error = None;
        cx.notify();

        let again = self.preparation(&shown.request, cx);
        let thread_id = self.thread.id.clone();
        cx.spawn(async move |this, cx| {
            let current = again.await;
            let Ok(permit) = this.update(cx, |this, cx| {
                let permit = current.and_then(|current| {
                    this.project
                        .update(cx, |project, _| project.commands.begin(approval, &current))
                });
                if let Err(error) = &permit {
                    this.fail_card(&call, error.message.clone(), cx);
                }
                permit
            }) else {
                return;
            };
            let Ok(permit) = permit else { return };
            Self::supervise(this, call, permit, shown.request, thread_id, cx).await;
        })
        .detach();
    }

    /// Starts the permitted command on the host and follows it to its end.
    async fn supervise(
        this: gpui_kit::WeakEntity<Self>,
        call: CallId,
        permit: LaunchPermit,
        request: CommandRequest,
        thread_id: String,
        cx: &mut gpui_kit::AsyncApp,
    ) {
        let invocation = permit.invocation().clone();
        let stop = CancellationToken::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Launch>(16);
        let (done_tx, done_rx) = tokio::sync::watch::channel(false);
        let Ok(job) = this.update(cx, |this, cx| {
            let host = this.host.clone();
            let scope = stop.clone();
            // From here on, losing this chat (its project closes) or quitting stops it.
            this.command_scope = Some(stop.clone().drop_guard());
            this.command_done = Some(done_rx);
            runtime::spawn(cx, async move {
                // Synchronous admission: an error here means nothing was started.
                let handle = host.start_command(permit, scope)?;
                let _ = tx.send(Launch::Started).await;
                let mut progress = handle.progress();
                let forward = tokio::spawn(async move {
                    while progress.changed().await.is_ok() {
                        let latest = progress.borrow_and_update().clone();
                        if tx.send(Launch::Progress(latest)).await.is_err() {
                            break;
                        }
                    }
                });
                let result = handle.wait().await;
                forward.abort();
                // The host is finished with it: process tree ended, completion recorded.
                let _ = done_tx.send(true);
                Ok::<ExecutionResult, contextual_core::Error>(result)
            })
        }) else {
            return;
        };

        let mut started = false;
        while let Some(event) = rx.recv().await {
            let alive = this
                .update(cx, |this, cx| {
                    match event {
                        Launch::Started => {
                            started = true;
                            this.card_mut(&call).starting = false;
                            this.running = Some(RunningCommand {
                                call: call.clone(),
                                stop: stop.clone(),
                                progress: ExecutionProgress::default(),
                            });
                            if this.thread.id == thread_id {
                                let recorded = this.thread.commands.begin(
                                    &call,
                                    invocation.clone(),
                                    request.clone(),
                                    &contextual_core::threads::now_iso(),
                                );
                                if recorded.is_ok() {
                                    this.save_thread(cx);
                                }
                            }
                        }
                        Launch::Progress(progress) => {
                            if let Some(running) = this.running.as_mut() {
                                running.progress = progress;
                            }
                        }
                    }
                    cx.notify();
                })
                .is_ok();
            if !alive {
                // The chat is gone: ask the command to stop. Its host still cleans up.
                stop.cancel();
                return;
            }
        }

        let result = job.await;
        let _ = this.update(cx, |this, cx| {
            this.running = None;
            this.command_done = None;
            if let Some(scope) = this.command_scope.take() {
                // It ended by itself: nothing left to cancel.
                scope.disarm();
            }
            this.card_mut(&call).starting = false;
            // Anything that may have run can have changed files, whatever its outcome.
            let may_have_run = match &result {
                Ok(result) => result.spawned,
                Err(_) => started,
            };
            match result {
                Ok(result) => {
                    if this.thread.id == thread_id {
                        let _ = this.thread.commands.finish(
                            &call,
                            &invocation,
                            &result,
                            &contextual_core::threads::now_iso(),
                        );
                    }
                    if let Some(detail) = result.detail.clone().filter(|_| !result.spawned) {
                        this.card_mut(&call).error = Some(detail);
                    }
                    let held = this.project.update(cx, |project, _| {
                        project.commands.finished(&call, &invocation, result)
                    });
                    if let Err(error) = held {
                        this.card_mut(&call).error = Some(error.message);
                    }
                    this.save_thread(cx);
                }
                Err(error) if !started => {
                    // Refused before anything started. The consumed approval is gone.
                    let _ = this
                        .project
                        .update(cx, |project, _| project.commands.failed_to_start(&call));
                    this.card_mut(&call).error = Some(error.message);
                }
                Err(error) => {
                    // It started and its result was lost: the outcome is unknown, and the
                    // project stays blocked until that is checked.
                    if this.thread.id == thread_id {
                        this.thread
                            .restore_commands(&contextual_core::threads::now_iso());
                        this.save_thread(cx);
                    }
                    this.card_mut(&call).error = Some(error.message);
                }
            }
            if may_have_run {
                this.refresh_after_command(cx);
            }
            cx.notify();
        });
    }

    /// Asks the running command to stop. This does not undo what it already did.
    pub fn stop_command(&mut self, cx: &mut Context<Self>) {
        if let Some(running) = &self.running {
            running.stop.cancel();
        }
        cx.notify();
    }

    /// After an unknown outcome: asks the host whether anything from it still runs. Only if
    /// the host establishes that nothing does can commands be reviewed again; everything
    /// then needs fresh preparation and approval. This starts and stops nothing.
    pub fn reconcile_commands(&mut self, cx: &mut Context<Self>) {
        if !self.commands_enabled || self.command_running() {
            return;
        }
        let (host, root) = (self.host.clone(), self.root(cx));
        let job = runtime::spawn(cx, async move { host.reconcile_commands(&root).await });
        cx.spawn(async move |this, cx| {
            let report = job.await;
            this.update(cx, |this, cx| {
                match report {
                    Ok(report) if !report.blocked() => {
                        let cleared = this
                            .project
                            .update(cx, |project, _| project.commands.reconciled());
                        match cleared {
                            Ok(()) => {
                                this.command_cards.clear();
                                this.note(
                                    "Nothing from the earlier command is still running. \
                                     Commands can be reviewed again.",
                                    Tone::Ok,
                                );
                                this.prepare_open_commands(cx);
                            }
                            Err(error) => this.note(error.message, Tone::Warn),
                        }
                    }
                    Ok(report) => {
                        let detail = report
                            .invocations
                            .iter()
                            .find_map(|invocation| invocation.detail.clone())
                            .unwrap_or_default();
                        this.note(
                            format!(
                                "It could not be established that the earlier command has \
                                 ended, so commands stay blocked. {detail}"
                            ),
                            Tone::Warn,
                        );
                    }
                    Err(error) => this.note(
                        format!(
                            "The earlier command could not be checked. {}",
                            error.message
                        ),
                        Tone::Warn,
                    ),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
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
        self.activity = ChatActivity::ReadingFiles;
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
        let loaded_before = self.thread.loaded.len();
        for f in &r.skill_files {
            if !self.thread.loaded.contains(f) {
                self.thread.loaded.push(f.clone());
            }
        }
        if self.thread.loaded.len() != loaded_before {
            self.context_revision = self.context_revision.wrapping_add(1);
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
                    log.completed = files.iter().all(|file| file.error.is_none());
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
                    results_only: false,
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

    /// A new, empty chat with no files selected or context carried over.
    pub fn start_fresh(&mut self, cx: &mut Context<Self>) {
        if self.streaming() || self.saving || self.refuse_while_command_runs(cx) {
            return;
        }
        self.command_cards.clear();
        self.staged_shares.clear();
        self.contents.clear();
        self.reference_only.clear();
        self.pending = None;
        self.project.update(cx, |p, cx| {
            p.clear_selection();
            p.clear_model_marks();
            p.reset_chat_tabs();
            cx.notify()
        });
        self.thread = ThreadState::fresh();
        self.live = None;
        self.live_context = None;
        self.context_revision = self.context_revision.wrapping_add(1);
        self.thread_generation = self.thread_generation.wrapping_add(1);
        self.messages.clear();
        self.reply_preview = None;
        self.include.clear();
        self.change_snapshot = None;
        self.undo_changes.clear();
        self.toast = None;
        self.save_dialog_open = false;
        self.attachments.clear();
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
                Ok(()) => this.refresh_threads(cx),
                Err(_) => {
                    this.toast = Some(Toast {
                        title: "Could not save the conversation".into(),
                        detail:
                            "It stays open, but won’t be available in chat history after a restart."
                                .into(),
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

    /// Reloads chat history without replacing the current conversation.
    /// Saved conversations are loaded only by an explicit `open_thread` action.
    pub fn refresh_threads(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let root = self.thread_key(cx);
        let job = runtime::spawn(cx, async move { store.list(&root).await });
        cx.spawn(async move |this, cx| {
            let list = job.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                this.threads = list;
                this.threads_revision = this.threads_revision.wrapping_add(1);
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
        if self.streaming() || self.saving || self.refuse_while_command_runs(cx) {
            return;
        }
        let generation = self.thread_generation;
        let root = self.thread_key(cx);
        let job = runtime::spawn(cx, async move { store.load(&root, &id).await });
        cx.spawn(async move |this, cx| {
            let Ok(value) = job.await else { return };
            let Ok(saved) = serde_json::from_value::<SavedThread>(value) else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.streaming() || this.saving || this.thread_generation != generation {
                    return;
                }
                let changes = saved.pending_changes();
                // Re-tick the files the model has from this chat.
                let paths: Vec<String> = saved.thread.file_versions.keys().cloned().collect();
                let loaded = saved.thread.loaded.clone();
                this.project.update(cx, |p, cx| {
                    p.select_paths(&paths);
                    p.clear_model_marks();
                    p.model_loaded
                        .extend(loaded.into_iter().filter(|f| f.ends_with("SKILL.md")));
                    p.reset_chat_tabs();
                    cx.notify()
                });
                this.pending = None;
                this.include.clear();
                this.contents.clear();
                this.reference_only.clear();
                this.undo_changes.clear();
                this.toast = None;
                this.save_dialog_open = false;
                for c in &changes {
                    if c.status != ChangeStatus::A {
                        this.contents.insert(c.path.clone(), c.old.clone());
                    }
                }
                this.attachments.clear();
                this.thread = saved.thread;
                this.command_cards.clear();
                this.staged_shares.clear();
                // A command that was running when the app stopped has an unknown outcome.
                // Nothing is approved or started by opening a chat.
                let interrupted = this
                    .thread
                    .restore_commands(&contextual_core::threads::now_iso());
                this.live = None;
                this.live_context = None;
                this.set_changes(changes);
                this.context_revision = this.context_revision.wrapping_add(1);
                this.thread_generation = this.thread_generation.wrapping_add(1);
                this.reply_preview = None;
                this.messages = saved
                    .messages
                    .into_iter()
                    .map(|mut m| {
                        m.streaming = false;
                        m
                    })
                    .collect();
                if interrupted > 0 {
                    this.note(
                        "A command was running when the app stopped. Its outcome is unknown \
                         and it was not run again.",
                        Tone::Warn,
                    );
                    this.save_thread(cx);
                }
                this.prepare_open_commands(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Another chat cannot be opened or started while a command from this one runs: its
    /// result belongs to this chat. Says so and returns true when that is the case.
    fn refuse_while_command_runs(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.command_running() {
            return false;
        }
        self.toast = Some(Toast {
            title: "A command is running".into(),
            detail: "Stop it or wait for it to finish before switching chats.".into(),
            error: true,
            backup_id: None,
        });
        cx.notify();
        true
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
            this.update(cx, |this, cx| this.refresh_threads(cx)).ok();
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
            this.update(cx, |this, cx| this.refresh_threads(cx)).ok();
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
            this.update(cx, |this, cx| this.refresh_threads(cx)).ok();
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
        if picked.is_empty() || self.saving || self.streaming() {
            return;
        }
        self.saving = true;
        self.activity = ChatActivity::SavingChanges;
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
                        let remaining = this
                            .changes_ref()
                            .iter()
                            .filter(|c| !picked.iter().any(|saved| saved.path == c.path))
                            .cloned()
                            .collect();
                        this.set_changes(remaining);
                        this.undo_changes = picked.clone();
                        this.context_revision = this.context_revision.wrapping_add(1);
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
        if self.streaming() || self.saving {
            return;
        }
        let Some(backup_id) = self.toast.as_ref().and_then(|t| t.backup_id.clone()) else {
            return;
        };
        self.saving = true;
        self.activity = ChatActivity::UndoingChanges;
        cx.notify();
        let req = Undo {
            root: self.root(cx),
            backup_id,
        };
        let host = self.host.clone();
        let job = runtime::spawn(cx, async move { host.undo(req).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                this.toast = Some(match result {
                    Ok(r) => {
                        let restored =
                            chat::merge_pending_changes(&this.undo_changes, this.changes_ref());
                        for change in &this.undo_changes {
                            this.thread.file_versions.remove(&change.path);
                        }
                        this.contents.clear();
                        for change in &restored {
                            if change.status != ChangeStatus::A {
                                this.contents
                                    .insert(change.path.clone(), change.old.clone());
                            }
                        }
                        this.set_changes(restored);
                        this.undo_changes.clear();
                        this.context_revision = this.context_revision.wrapping_add(1);
                        this.save_thread(cx);
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
