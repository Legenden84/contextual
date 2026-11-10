//! A conversation as the app keeps it, and the pure steps of a turn (ported from `send` in
//! `AppProvider.tsx`). The JSON shapes match what the old app saved, so saved chats carry over.
//!
//! The async parts (reading files through the host, streaming from Foundry, repairing edits)
//! are orchestrated by the app; everything decidable without I/O lives here and is tested.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::llm::types::{ApiKind, ChatMessage as LlmMessage, ChatRole, MessageContent};
use crate::logic::changes::{ChangeContext, parse_changes};
use crate::logic::format::fmt_k;
use crate::logic::hash::hash_text;
use crate::logic::markdown::{Block, parse_markdown};
use crate::logic::prompt::{ApiContent, ContextFile, ContextImage, TurnInput, build_turn};
use crate::logic::types::{Change, ChangeStatus, Deployment};
use contextual_protocol::fs::FileContent;

/// One turn as sent to / received from the model (`ApiMessage` in the old app).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiMessage {
    pub role: ChatRole,
    pub content: ApiContent,
    /// User turns: the system prompt sent with this turn, recorded only when it differs from
    /// the one before it (`None`: unchanged, or a chat saved before this was recorded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// User turns: written by the app (answering a read request), not typed by the user.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub followup: bool,
    /// Assistant turns: the tokens the reply used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl ApiMessage {
    pub fn new(role: ChatRole, content: ApiContent) -> Self {
        Self {
            role,
            content,
            system: None,
            followup: false,
            usage: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestRef {
    pub url: String,
    pub number: Option<u64>,
    pub branch: String,
}

/// The conversation as the model has seen it, so each turn sends only what's new.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadState {
    pub id: String,
    /// Azure's id for the last reply (Responses API); `None` when the transcript must be sent.
    pub response_id: Option<String>,
    pub api: Option<ApiKind>,
    /// Turns exactly as sent and received (a stable prefix for prompt caching).
    pub transcript: Vec<ApiMessage>,
    /// Version (hash) of each file the model has, by path.
    pub file_versions: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Files saved from this chat's replies and not yet in a pull request.
    #[serde(default)]
    pub applied: Vec<String>,
    #[serde(default)]
    pub pull_requests: Vec<PullRequestRef>,
    /// Hash of the project tree the model was sent (when it picks its files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_version: Option<String>,
    /// Skills and skill files the model loaded: kept in context like ticked files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loaded: Vec<String>,
    /// Cumulative unsaved proposals. `None` means an older chat: recover from its messages.
    /// An explicit empty list must survive saving so historical changes don't reappear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_changes: Option<Vec<Change>>,
}

impl ThreadState {
    pub fn fresh() -> Self {
        Self {
            id: format!("t{}", uuid::Uuid::new_v4().simple()),
            response_id: None,
            api: None,
            transcript: Vec::new(),
            file_versions: BTreeMap::new(),
            title: None,
            applied: Vec::new(),
            pull_requests: Vec::new(),
            tree_version: None,
            loaded: Vec::new(),
            pending_changes: Some(Vec::new()),
        }
    }

    /// The system prompt recorded last (what later turns were sent unless they record another).
    pub fn last_system(&self) -> Option<&str> {
        self.transcript
            .iter()
            .rev()
            .find_map(|m| m.system.as_deref())
    }

    /// Adds a completed round to the transcript: the input as sent (with its system prompt when
    /// it changed) and the reply as received.
    pub fn record_round(
        &mut self,
        input: ApiContent,
        system: &str,
        followup: bool,
        reply: String,
        usage: Option<Usage>,
    ) {
        let system = (self.last_system() != Some(system)).then(|| system.to_string());
        self.transcript.push(ApiMessage {
            system,
            followup,
            ..ApiMessage::new(ChatRole::User, input)
        });
        self.transcript.push(ApiMessage {
            usage,
            ..ApiMessage::new(ChatRole::Assistant, ApiContent::Text(reply))
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Warn,
    Error,
    Ok,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageImage {
    pub name: String,
    pub url: String,
}

/// A message as the chat shows it (`ChatMessage` in the old app).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiMessage {
    pub id: String,
    pub role: Role,
    pub text: String,
    pub time: String,
    /// User: what was sent with it. Assistant: usage and cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<Change>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub streaming: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<ApiContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<MessageImage>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<ApiKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// Assistant: the context the model asked for in this reply, and what it got.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<ReadLog>,
}

/// What a read request fetched (shown as one row under the reply).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadLog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
    /// "path (reason)".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<String>,
    /// Over the per-message limit: waiting for the user, or declined.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held_back: Vec<String>,
    pub tokens: u64,
    /// The requested context was fetched successfully. Older logs leave this false rather
    /// than presenting an unverified success indicator.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub completed: bool,
    /// The reply changed files it hadn't been sent: they were sent and the changes asked again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unseen_edits: bool,
}

impl ReadLog {
    pub fn from_resolved(r: &crate::logic::reads::Resolved) -> Self {
        let skills: Vec<String> = r
            .skill_files
            .iter()
            .filter(|p| p.ends_with("SKILL.md"))
            .cloned()
            .collect();
        Self {
            files: r
                .files
                .iter()
                .chain(r.skill_files.iter().filter(|p| !p.ends_with("SKILL.md")))
                .cloned()
                .collect(),
            skills,
            folders: r.listings.iter().map(|(p, _)| p.clone()).collect(),
            refused: r
                .refused
                .iter()
                .map(|x| format!("{} ({})", x.path, x.reason))
                .collect(),
            held_back: Vec::new(),
            tokens: r.tokens,
            completed: false,
            unseen_edits: false,
        }
    }

    /// "Loaded skill frontend-design · read 4 files · listed 1 folder · ~12k tokens"
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.skills.is_empty() {
            let names: Vec<String> = self
                .skills
                .iter()
                .map(|p| {
                    crate::logic::reads::skill_dir(p)
                        .rsplit('/')
                        .next()
                        .unwrap_or(p)
                        .to_string()
                })
                .collect();
            parts.push(format!(
                "Loaded {} {}",
                if names.len() == 1 { "skill" } else { "skills" },
                names.join(", ")
            ));
        }
        let n = self.files.len();
        if n > 0 {
            parts.push(format!(
                "read {n} {}",
                if n == 1 { "file" } else { "files" }
            ));
        }
        let f = self.folders.len();
        if f > 0 {
            parts.push(format!(
                "listed {f} {}",
                if f == 1 { "folder" } else { "folders" }
            ));
        }
        if !self.refused.is_empty() {
            parts.push(format!("{} not available", self.refused.len()));
        }
        if !self.held_back.is_empty() {
            parts.push(format!("{} held back", self.held_back.len()));
        }
        if self.tokens > 0 {
            parts.push(format!("~{} tokens", fmt_k(self.tokens)));
        }
        let mut s = parts.join(" · ");
        if let Some(first) = s.get(..1) {
            s = first.to_uppercase() + &s[1..];
        }
        if self.unseen_edits {
            s = format!("Changed files it hadn't seen: sent them, asked again · {s}");
        }
        if s.is_empty() {
            "Asked for context".into()
        } else {
            s
        }
    }
}

impl UiMessage {
    fn new(prefix: &str, role: Role, text: String, time: &str) -> Self {
        Self {
            id: new_id(prefix),
            role,
            text,
            time: time.to_string(),
            meta: None,
            model: None,
            changes: None,
            streaming: false,
            sent: None,
            images: None,
            tone: None,
            api: None,
            usage: None,
            cost: None,
            reads: None,
        }
    }

    /// A sent message: `meta` says what went with it, `sent` is the content as sent.
    pub fn user(text: &str, turn: &PreparedTurn, time: &str) -> Self {
        Self {
            meta: Some(turn.meta.clone()),
            sent: Some(turn.input.clone()),
            images: (!turn.images.is_empty()).then(|| turn.images.clone()),
            ..Self::new("u", Role::User, text.to_string(), time)
        }
    }

    /// An empty reply that is about to stream in.
    pub fn assistant(model: &str, time: &str) -> Self {
        Self {
            model: Some(model.to_string()),
            streaming: true,
            ..Self::new("a", Role::Assistant, String::new(), time)
        }
    }

    pub fn note(text: impl Into<String>, tone: Tone, time: &str) -> Self {
        Self {
            tone: Some(tone),
            ..Self::new("n", Role::Note, text.into(), time)
        }
    }
}

pub fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

/// What `threads.rs` stores per conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedThread {
    pub version: u32,
    #[serde(flatten)]
    pub thread: ThreadState,
    pub messages: Vec<UiMessage>,
}

impl SavedThread {
    pub fn new(thread: ThreadState, messages: Vec<UiMessage>) -> Self {
        Self {
            version: 1,
            thread,
            messages,
        }
    }

    /// Title for the Chats list: the user's, else the first message (80 chars).
    pub fn title(&self) -> String {
        self.thread.title.clone().unwrap_or_else(|| {
            self.messages
                .iter()
                .find(|m| m.role == Role::User)
                .map(|m| m.text.chars().take(80).collect())
                .unwrap_or_else(|| "Conversation".into())
        })
    }

    pub fn turns(&self) -> usize {
        self.thread.transcript.len() / 2
    }

    /// Legacy chats stored only per-reply proposals. Keep the latest version of every path,
    /// in first-seen order. Their `old` contents already refer to disk, not another proposal.
    pub fn last_changes(messages: &[UiMessage]) -> Vec<Change> {
        let mut changes: Vec<Change> = Vec::new();
        for change in messages.iter().filter_map(|m| m.changes.as_ref()).flatten() {
            if let Some(previous) = changes.iter_mut().find(|c| c.path == change.path) {
                *previous = change.clone();
            } else {
                changes.push(change.clone());
            }
        }
        changes
    }

    pub fn pending_changes(&self) -> Vec<Change> {
        self.thread
            .pending_changes
            .clone()
            .unwrap_or_else(|| Self::last_changes(&self.messages))
    }
}

/// How a turn is built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnOptions {
    /// "Resend full context": attach every file, changed or not.
    pub force_all: bool,
    /// The project tree to send first.
    pub tree: Option<String>,
    /// An app-written follow-up (answering a read request).
    pub followup: bool,
}

/// The next turn, ready to send.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTurn {
    pub input: ApiContent,
    /// `file_versions` after this turn (committed only if the turn completes).
    pub versions: BTreeMap<String, String>,
    pub images: Vec<MessageImage>,
    /// "3 of 5 files sent (2 unchanged) · 1 rules"
    pub meta: String,
    /// Written by the app (answering a read request).
    pub followup: bool,
}

/// Builds the turn from freshly read files: only files the model doesn't have yet (or that
/// changed since) are attached; dropped files are listed as removed.
pub fn prepare_turn(
    thread: &ThreadState,
    read: &[FileContent],
    selected: &HashSet<String>,
    text: &str,
    attachments: &[MessageImage],
    rules_on: usize,
    opts: TurnOptions,
) -> PreparedTurn {
    // The model works on an in-memory proposal, not an implicit disk write. Always keep
    // pending paths in context, including new files that don't exist in the project tree.
    let mut read = read.to_vec();
    let mut selected = selected.clone();
    let mut pending_paths = Vec::new();
    let mut deleted_paths = Vec::new();
    for change in thread
        .pending_changes
        .iter()
        .flatten()
        .filter(|c| usable_proposal(c))
    {
        selected.insert(change.path.clone());
        read.retain(|f| f.path != change.path);
        if change.status == ChangeStatus::D {
            deleted_paths.push(change.path.clone());
            read.push(FileContent {
                path: change.path.clone(),
                error: Some("pending deletion (not saved)".into()),
                ..Default::default()
            });
        } else {
            pending_paths.push(change.path.clone());
            read.push(FileContent {
                path: change.path.clone(),
                content: Some(change.new.clone()),
                ..Default::default()
            });
        }
    }
    let mut text = text.to_string();
    if !pending_paths.is_empty() || !deleted_paths.is_empty() {
        let mut note = String::from(
            "# Pending changes\n\nThese changes are in memory only, not saved to disk. \
             Continue from the latest proposed contents supplied as context, not the older disk copies. \
             Return only files you change; other pending changes are kept.",
        );
        if !pending_paths.is_empty() {
            note.push_str(&format!(
                "\n\nPending file versions:\n- {}",
                pending_paths.join("\n- ")
            ));
        }
        if !deleted_paths.is_empty() {
            note.push_str(&format!(
                "\n\nPending deletions (use FILE to recreate):\n- {}",
                deleted_paths.join("\n- ")
            ));
        }
        text = format!("{note}\n\n{text}");
    }
    let force_all = opts.force_all;
    let followup = opts.followup;
    let mut versions = thread.file_versions.clone();
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();
    for f in &read {
        let unavailable = format!("unavailable:{}", f.error.as_deref().unwrap_or(""));
        let version = hash_text(
            f.image
                .as_deref()
                .or(f.content.as_deref())
                .unwrap_or(&unavailable),
        );
        if !force_all && versions.get(&f.path) == Some(&version) {
            unchanged.push(f.path.clone());
        } else {
            changed.push(ContextFile {
                path: f.path.clone(),
                content: f.content.clone(),
                image: f.image.clone(),
                condensed: f.condensed_from.is_some(),
            });
            versions.insert(f.path.clone(), version);
        }
    }
    let removed: Vec<String> = versions
        .keys()
        .filter(|p| !selected.contains(*p))
        .cloned()
        .collect();
    for p in &removed {
        versions.remove(p);
    }
    let input = build_turn(&TurnInput {
        text: text.to_string(),
        files: changed.clone(),
        unchanged: unchanged.clone(),
        removed,
        attachments: attachments
            .iter()
            .map(|a| ContextImage {
                name: a.name.clone(),
                url: a.url.clone(),
            })
            .collect(),
        tree: opts.tree,
        followup: opts.followup,
    });
    let total = read.len();
    let files = if total == 0 {
        "no files".to_string()
    } else {
        let mut s = format!(
            "{} of {total} {} sent",
            changed.len(),
            if total == 1 { "file" } else { "files" }
        );
        if !unchanged.is_empty() {
            s.push_str(&format!(" ({} unchanged)", unchanged.len()));
        }
        s
    };
    PreparedTurn {
        input,
        versions,
        images: attachments.to_vec(),
        meta: format!("{files} · {rules_on} rules"),
        followup,
    }
}

/// Paths whose disk estimates must be replaced, and the estimated context for their pending
/// versions. Reuse turn preparation so deletions, conflicts and already-sent hashes obey the
/// same rules as Send. Call only when stored context changes, not on streaming deltas.
pub fn pending_context_tokens(
    thread: &ThreadState,
    selected: &HashSet<String>,
) -> (HashSet<String>, u64) {
    let paths: HashSet<String> = thread
        .pending_changes
        .iter()
        .flatten()
        .filter(|change| usable_proposal(change))
        .map(|change| change.path.clone())
        .collect();
    if paths.is_empty() {
        return (paths, 0);
    }
    let prepared = prepare_turn(
        thread,
        &[],
        selected,
        "",
        &[],
        0,
        TurnOptions::default(),
    );
    (
        paths,
        crate::logic::context::content_tokens(&prepared.input),
    )
}

/// The transcript as Foundry request messages.
pub fn history(thread: &ThreadState) -> Vec<LlmMessage> {
    thread
        .transcript
        .iter()
        .map(|m| LlmMessage {
            role: m.role,
            content: to_message_content(&m.content),
        })
        .collect()
}

/// `ApiContent` (prompt layer) → `MessageContent` (Foundry layer): the same JSON shape.
pub fn to_message_content(c: &ApiContent) -> MessageContent {
    serde_json::from_value(serde_json::to_value(c).expect("content serialises"))
        .expect("same wire shape")
}

/// Paths of the files a reply proposes to write, edit or delete.
pub fn touched_paths(reply: &str) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(blocks: &[Block], out: &mut Vec<String>) {
        for b in blocks {
            match b {
                Block::Code { path: Some(p), .. }
                | Block::Edit { path: p, .. }
                | Block::Delete { path: p } => {
                    if !out.contains(p) {
                        out.push(p.clone())
                    }
                }
                Block::Quote { blocks } => walk(blocks, out),
                _ => {}
            }
        }
    }
    walk(&parse_markdown(reply), &mut out);
    out
}

/// Parses a finished reply into proposed changes (none when the reply was cut off).
pub fn proposed_changes(reply: &str, truncated: bool, ctx: &dyn ChangeContext) -> Vec<Change> {
    if truncated {
        Vec::new()
    } else {
        parse_changes(reply, ctx)
    }
}

fn usable_proposal(change: &Change) -> bool {
    change.conflict.is_none() && change.unplaced.as_ref().is_none_or(Vec::is_empty)
}

/// Resolve SEARCH/REPLACE against the latest valid proposal. The returned changes are
/// relative to that version, including the `old` used by edit repair.
pub fn proposed_changes_on_top(
    reply: &str,
    truncated: bool,
    ctx: &dyn ChangeContext,
    pending: &[Change],
) -> Vec<Change> {
    struct Proposed<'a> {
        disk: &'a dyn ChangeContext,
        pending: &'a [Change],
    }

    impl Proposed<'_> {
        fn get(&self, path: &str) -> Option<&Change> {
            self.pending
                .iter()
                .find(|c| c.path == path && usable_proposal(c))
        }
    }

    impl ChangeContext for Proposed<'_> {
        fn exists(&self, path: &str) -> bool {
            self.get(path)
                .map_or_else(|| self.disk.exists(path), |c| c.status != ChangeStatus::D)
        }

        fn original(&self, path: &str) -> Option<String> {
            match self.get(path) {
                Some(c) => (c.status != ChangeStatus::D).then(|| c.new.clone()),
                None => self.disk.original(path),
            }
        }

        fn changed_on_disk(&self, path: &str) -> bool {
            self.disk.changed_on_disk(path)
        }

        fn read_only(&self, path: &str) -> bool {
            self.disk.read_only(path)
        }
    }

    proposed_changes(reply, truncated, &Proposed { disk: ctx, pending })
}

/// Compose successive proposals into one disk-relative change per path. Newer contents win,
/// but the original disk baseline remains the Save precondition until that file is saved.
pub fn merge_pending_changes(pending: &[Change], updates: &[Change]) -> Vec<Change> {
    let mut merged = pending.to_vec();
    for update in updates {
        let Some(index) = merged.iter().position(|c| c.path == update.path) else {
            merged.push(update.clone());
            continue;
        };
        let previous = &merged[index];
        if previous.status == ChangeStatus::A
            && update.status == ChangeStatus::D
            && update.conflict.is_none()
        {
            // Creating and then deleting an unsaved file cancels the creation.
            merged.remove(index);
            continue;
        }
        let mut next = update.clone();
        next.old = previous.old.clone();
        next.status = if previous.status == ChangeStatus::A {
            ChangeStatus::A
        } else if update.status == ChangeStatus::D {
            ChangeStatus::D
        } else {
            ChangeStatus::M
        };
        // A follow-up must not silently remove an existing safety warning.
        if next.conflict.is_none() {
            next.conflict = previous.conflict.clone();
        }
        merged[index] = next;
    }
    visible_changes(merged)
}

/// Drops no-op modifications (same content, no conflict).
pub fn visible_changes(changes: Vec<Change>) -> Vec<Change> {
    changes
        .into_iter()
        .filter(|c| !(c.status == ChangeStatus::M && c.old == c.new && c.conflict.is_none()))
        .collect()
}

/// "12k in (8k cached) / 1.5k out · $0.031" and the cost, when prices are set.
pub fn usage_meta(usage: &Usage, dep: &Deployment) -> (String, Option<f64>) {
    let cost = dep
        .priced()
        .then(|| dep.cost(usage.input_tokens, usage.cached_tokens, usage.output_tokens));
    let mut meta = format!("{} in", fmt_k(usage.input_tokens));
    if usage.cached_tokens > 0 {
        meta.push_str(&format!(" ({} cached)", fmt_k(usage.cached_tokens)));
    }
    meta.push_str(&format!(" / {} out", fmt_k(usage.output_tokens)));
    if let Some(c) = cost {
        meta.push_str(&format!(" · ${c:.3}"));
    }
    (meta, cost)
}

/// Notices shown after a turn (as the old app did).
pub fn turn_notes(
    failure: Option<&str>,
    chain_reset: bool,
    api: Option<ApiKind>,
    previous_api: Option<ApiKind>,
    truncated: bool,
) -> Vec<(String, Tone)> {
    let mut out = Vec::new();
    if let Some(f) = failure {
        out.push((
            f.to_string(),
            if f == STOPPED {
                Tone::Warn
            } else {
                Tone::Error
            },
        ));
    }
    if chain_reset && api == Some(ApiKind::Responses) {
        out.push((
            "Azure no longer had this conversation stored, so the full conversation was sent once and a new chain started.".into(),
            Tone::Warn,
        ));
    }
    if api == Some(ApiKind::Chat) && previous_api == Some(ApiKind::Responses) {
        out.push((
            "The Responses API is not available for this deployment; the full conversation is now sent with each message.".into(),
            Tone::Warn,
        ));
    }
    if truncated {
        out.push((
            "The reply hit the completion token limit and was cut off, so its file changes were not offered for saving. Raise AZURE_OPENAI_MAX_COMPLETION_TOKENS or ask for fewer files.".into(),
            Tone::Warn,
        ));
    }
    out
}

/// Failure text when the user pressed Stop.
pub const STOPPED: &str = "Stopped.";

/// After saving: the model wrote these versions itself, so they needn't be sent back.
pub fn record_saved(thread: &mut ThreadState, saved: &[Change]) {
    for c in saved {
        if c.status == ChangeStatus::D {
            thread.file_versions.remove(&c.path);
        } else {
            thread
                .file_versions
                .insert(c.path.clone(), hash_text(&c.new));
        }
        if !thread.applied.contains(&c.path) {
            thread.applied.push(c.path.clone());
        }
    }
}

/// "3 files saved" and "1 deleted, 1 created, 1 overwritten · not committed on main".
pub fn saved_toast(saved: &[Change], branch: Option<&str>) -> (String, String) {
    let n = saved.len();
    let count = |s: ChangeStatus| saved.iter().filter(|c| c.status == s).count();
    let parts: Vec<String> = [
        (ChangeStatus::D, "deleted"),
        (ChangeStatus::A, "created"),
        (ChangeStatus::M, "overwritten"),
    ]
    .into_iter()
    .filter(|(s, _)| count(*s) > 0)
    .map(|(s, w)| format!("{} {w}", count(s)))
    .collect();
    let title = format!("{n} {} saved", if n == 1 { "file" } else { "files" });
    let mut detail = format!("{} · not committed", parts.join(", "));
    if let Some(b) = branch {
        detail.push_str(&format!(" on {b}"));
    }
    (title, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, content: &str) -> FileContent {
        FileContent {
            path: path.into(),
            content: Some(content.into()),
            ..Default::default()
        }
    }

    #[test]
    fn only_new_or_changed_files_are_attached() {
        let mut thread = ThreadState::fresh();
        let selected: HashSet<String> = ["a.rs".into(), "b.rs".into()].into();
        let read = [file("a.rs", "fn a() {}\n"), file("b.rs", "fn b() {}\n")];
        let t1 = prepare_turn(
            &thread,
            &read,
            &selected,
            "hi",
            &[],
            1,
            TurnOptions::default(),
        );
        assert_eq!(t1.meta, "2 of 2 files sent · 1 rules");
        thread.file_versions = t1.versions;

        let read2 = [
            file("a.rs", "fn a() {}\n"),
            file("b.rs", "fn b() { changed }\n"),
        ];
        let t2 = prepare_turn(
            &thread,
            &read2,
            &selected,
            "again",
            &[],
            1,
            TurnOptions::default(),
        );
        assert_eq!(t2.meta, "1 of 2 files sent (1 unchanged) · 1 rules");
        let text = crate::logic::prompt::content_text(&t2.input);
        assert!(text.contains("b.rs") && text.contains("changed"));
        assert!(text.contains("Unchanged files"));

        // Deselected files are dropped from the versions and reported as removed.
        let only_a: HashSet<String> = ["a.rs".into()].into();
        let t3 = prepare_turn(
            &thread,
            &read2[..1],
            &only_a,
            "x",
            &[],
            0,
            TurnOptions::default(),
        );
        assert!(!t3.versions.contains_key("b.rs"));
        // Force: everything is attached again.
        let t4 = prepare_turn(
            &thread,
            &read2,
            &selected,
            "x",
            &[],
            0,
            TurnOptions {
                force_all: true,
                ..TurnOptions::default()
            },
        );
        assert_eq!(t4.meta, "2 of 2 files sent · 0 rules");
    }

    #[test]
    fn image_only_turn_survives_saved_thread_roundtrip() {
        use crate::llm::types::ContentPart;

        let mut thread = ThreadState::fresh();
        let images = vec![MessageImage {
            name: "screenshot.png".into(),
            url: "data:image/png;base64,AA==".into(),
        }];
        let turn = prepare_turn(
            &thread,
            &[],
            &HashSet::new(),
            "",
            &images,
            0,
            TurnOptions::default(),
        );
        let MessageContent::Parts(parts) = to_message_content(&turn.input) else {
            panic!("An image attachment must produce multipart content");
        };
        assert!(parts.iter().any(|part| matches!(
            part,
            ContentPart::ImageUrl { image_url } if image_url.url == images[0].url
        )));
        let user = UiMessage::user("", &turn, "10:00");
        assert_eq!(user.images.as_ref(), Some(&images));
        thread.record_round(turn.input, "system", false, "I see the image.".into(), None);
        let saved = SavedThread::new(thread, vec![user]);
        let restored: SavedThread =
            serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert_eq!(restored, saved);
        assert_eq!(
            history(&restored.thread)[0].content,
            MessageContent::Parts(parts)
        );
    }

    #[test]
    fn saved_thread_reads_the_old_format() {
        let old = serde_json::json!({
            "version": 1, "id": "t1", "responseId": "resp_1", "api": "responses",
            "transcript": [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "hello"}],
            "fileVersions": {"a.rs": "abc"}, "applied": [], "pullRequests": [],
            "messages": [
                {"id": "u1", "role": "user", "text": "hi", "time": "10:00", "meta": "no files · 0 rules"},
                {"id": "a1", "role": "assistant", "text": "hello", "time": "10:00", "model": "gpt", "api": "responses",
                 "usage": {"inputTokens": 10, "outputTokens": 2, "cachedTokens": 0}, "changes": []}
            ]
        });
        let saved: SavedThread = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(saved.title(), "hi");
        assert_eq!(saved.turns(), 1);
        assert_eq!(saved.thread.response_id.as_deref(), Some("resp_1"));
        let again = serde_json::to_value(&saved).unwrap();
        assert_eq!(again["fileVersions"], old["fileVersions"]);
        assert_eq!(again["messages"][1]["usage"], old["messages"][1]["usage"]);
    }

    #[test]
    fn touched_paths_and_toasts() {
        let reply = "Sure.\n\n### FILE: src/a.rs\n```rust\nfn a() {}\n```\n\n### DELETE: old.rs\n";
        assert_eq!(
            touched_paths(reply),
            vec!["src/a.rs".to_string(), "old.rs".to_string()]
        );
        let c = |p: &str, s| Change {
            path: p.into(),
            status: s,
            old: String::new(),
            new: "x".into(),
            conflict: None,
            edits: None,
            unplaced: None,
        };
        let saved = [
            c("a", ChangeStatus::A),
            c("b", ChangeStatus::M),
            c("d", ChangeStatus::D),
        ];
        let (title, detail) = saved_toast(&saved, Some("main"));
        assert_eq!(title, "3 files saved");
        assert_eq!(
            detail,
            "1 deleted, 1 created, 1 overwritten · not committed on main"
        );
        let mut t = ThreadState::fresh();
        t.file_versions.insert("d".into(), "v".into());
        record_saved(&mut t, &saved);
        assert!(t.file_versions.contains_key("a") && !t.file_versions.contains_key("d"));
        assert_eq!(t.applied.len(), 3);
    }

    fn proposal(path: &str, status: ChangeStatus, old: &str, new: &str) -> Change {
        Change {
            path: path.into(),
            status,
            old: old.into(),
            new: new.into(),
            conflict: None,
            edits: None,
            unplaced: None,
        }
    }

    struct ProposalDisk;

    impl ChangeContext for ProposalDisk {
        fn exists(&self, path: &str) -> bool {
            path == "a.txt"
        }

        fn original(&self, path: &str) -> Option<String> {
            (path == "a.txt").then(|| "one\ntwo\n".into())
        }

        fn changed_on_disk(&self, _: &str) -> bool {
            false
        }
    }

    #[test]
    fn consecutive_edits_keep_other_files_and_the_disk_baseline() {
        let first = vec![
            proposal("a.txt", ChangeStatus::M, "one\ntwo\n", "ONE\ntwo\n"),
            proposal("b.txt", ChangeStatus::A, "", "keep"),
        ];
        let next = proposed_changes_on_top(
            "### EDIT: a.txt\n<<<<<<< SEARCH\nONE\n=======\nFIRST\n>>>>>>> REPLACE",
            false,
            &ProposalDisk,
            &first,
        );
        assert_eq!(next[0].old, "ONE\ntwo\n");
        assert_eq!(next[0].new, "FIRST\ntwo\n");
        assert!(next[0].unplaced.as_ref().unwrap().is_empty());
        let merged = merge_pending_changes(&first, &next);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].old, "one\ntwo\n");
        assert_eq!(merged[0].new, "FIRST\ntwo\n");
        assert_eq!(merged[1], first[1]);
        assert_eq!(merge_pending_changes(&merged, &[]), merged);

        let cut_off = proposed_changes_on_top("### DELETE: b.txt", true, &ProposalDisk, &merged);
        assert_eq!(merge_pending_changes(&merged, &cut_off), merged);
    }

    #[test]
    fn pending_creation_can_be_edited_or_cancelled() {
        let first = vec![proposal("new.txt", ChangeStatus::A, "", "first")];
        let next = proposed_changes_on_top(
            "### EDIT: new.txt\n<<<<<<< SEARCH\nfirst\n=======\nsecond\n>>>>>>> REPLACE",
            false,
            &ProposalDisk,
            &first,
        );
        let merged = merge_pending_changes(&first, &next);
        assert_eq!(
            merged,
            vec![proposal("new.txt", ChangeStatus::A, "", "second")]
                .into_iter()
                .map(|mut c| {
                    c.edits = next[0].edits.clone();
                    c.unplaced = next[0].unplaced.clone();
                    c
                })
                .collect::<Vec<_>>()
        );
        let deleted = proposed_changes_on_top("### DELETE: new.txt", false, &ProposalDisk, &merged);
        assert!(merge_pending_changes(&merged, &deleted).is_empty());
    }

    #[test]
    fn delete_recreate_and_revert_compose_against_disk() {
        let deleted = vec![proposal("a.txt", ChangeStatus::D, "one\ntwo\n", "")];
        let recreated = proposed_changes_on_top(
            "### FILE: a.txt\n```\nreplacement\n```",
            false,
            &ProposalDisk,
            &deleted,
        );
        let merged = merge_pending_changes(&deleted, &recreated);
        assert_eq!(merged[0].status, ChangeStatus::M);
        assert_eq!(merged[0].old, "one\ntwo\n");
        assert_eq!(merged[0].new, "replacement");

        let reverted = proposal("a.txt", ChangeStatus::M, "replacement", "one\ntwo\n");
        assert!(merge_pending_changes(&merged, &[reverted]).is_empty());
    }

    #[test]
    fn followup_context_uses_unsaved_versions_including_new_files() {
        let mut thread = ThreadState::fresh();
        thread.pending_changes = Some(vec![
            proposal("a.txt", ChangeStatus::M, "disk", "pending"),
            proposal("new.txt", ChangeStatus::A, "", "created"),
            proposal("gone.txt", ChangeStatus::D, "old", ""),
        ]);
        let turn = prepare_turn(
            &thread,
            &[file("a.txt", "disk"), file("gone.txt", "old")],
            &HashSet::new(),
            "continue",
            &[],
            0,
            TurnOptions::default(),
        );
        let text = crate::logic::prompt::content_text(&turn.input);
        assert!(text.contains("pending"));
        assert!(text.contains("created"));
        assert!(text.contains("Pending deletions"));
        assert_eq!(turn.versions["a.txt"], hash_text("pending"));
        assert_eq!(turn.versions["new.txt"], hash_text("created"));
        assert!(!text.contains("# Removed from context"));
    }

    #[test]
    fn pending_context_counts_new_files_before_they_exist_on_disk() {
        let mut thread = ThreadState::fresh();
        let content = "new contents\n".repeat(1_000);
        thread.pending_changes = Some(vec![proposal("new.txt", ChangeStatus::A, "", &content)]);
        let (paths, tokens) = pending_context_tokens(&thread, &HashSet::new());
        assert_eq!(paths, HashSet::from(["new.txt".into()]));
        assert!(tokens >= crate::logic::format::estimate_tokens(&content));

        // After that version has been sent, only the pending-state note is needed.
        thread
            .file_versions
            .insert("new.txt".into(), hash_text(&content));
        let (_, unchanged_tokens) = pending_context_tokens(&thread, &HashSet::new());
        assert!(unchanged_tokens < tokens);
    }

    #[test]
    fn conflicted_proposals_do_not_replace_disk_token_estimates() {
        let mut thread = ThreadState::fresh();
        let mut change = proposal("a.txt", ChangeStatus::M, "disk", "proposal");
        change.conflict = Some("Changed on disk".into());
        thread.pending_changes = Some(vec![change]);
        assert_eq!(
            pending_context_tokens(&thread, &HashSet::new()),
            (HashSet::new(), 0)
        );
    }

    #[test]
    fn context_estimates_survive_reopening_a_saved_thread() {
        use crate::logic::context::content_tokens;

        let mut thread = ThreadState::fresh();
        thread.record_round(
            ApiContent::Text("Implement the change".into()),
            "system",
            false,
            "Proposed changes".repeat(100),
            None,
        );
        thread.pending_changes = Some(vec![proposal(
            "new.txt",
            ChangeStatus::A,
            "",
            &"new contents".repeat(100),
        )]);
        let selected = HashSet::new();
        let before_pending = pending_context_tokens(&thread, &selected);
        let before_history: u64 = thread
            .transcript
            .iter()
            .map(|message| content_tokens(&message.content))
            .sum();
        let saved = SavedThread::new(thread, Vec::new());
        let restored: SavedThread =
            serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert_eq!(
            pending_context_tokens(&restored.thread, &selected),
            before_pending
        );
        assert_eq!(
            restored
                .thread
                .transcript
                .iter()
                .map(|message| content_tokens(&message.content))
                .sum::<u64>(),
            before_history
        );
    }

    #[test]
    fn pending_state_roundtrips_and_legacy_replies_accumulate() {
        let a = proposal("a.txt", ChangeStatus::M, "old", "first");
        let b = proposal("b.txt", ChangeStatus::A, "", "keep");
        let newer = proposal("a.txt", ChangeStatus::M, "old", "second");
        let mut first = UiMessage::assistant("model", "10:00");
        first.changes = Some(vec![a, b.clone()]);
        let mut second = UiMessage::assistant("model", "10:01");
        second.changes = Some(vec![newer.clone()]);
        let mut thread = ThreadState::fresh();
        thread.pending_changes = None;
        let mut saved = SavedThread::new(thread, vec![first, second]);
        assert_eq!(saved.pending_changes(), vec![newer.clone(), b.clone()]);

        // After a partial Save, only the unsaved path returns on reopening.
        saved.thread.pending_changes = Some(vec![b.clone()]);
        let restored: SavedThread =
            serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert_eq!(restored.pending_changes(), vec![b]);

        // After saving everything, historical replies must not resurrect changes.
        saved.thread.pending_changes = Some(Vec::new());
        let restored: SavedThread =
            serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert!(restored.pending_changes().is_empty());
    }

    #[test]
    fn undo_restores_saved_baselines_without_losing_followup_work() {
        let saved = vec![proposal("a.txt", ChangeStatus::M, "disk", "saved")];
        let pending = vec![
            proposal("a.txt", ChangeStatus::M, "saved", "latest"),
            proposal("b.txt", ChangeStatus::A, "", "keep"),
        ];
        let restored = merge_pending_changes(&saved, &pending);
        assert_eq!(restored[0].old, "disk");
        assert_eq!(restored[0].new, "latest");
        assert_eq!(restored[1], pending[1]);
    }

    #[test]
    fn usage_and_notes() {
        let dep = Deployment {
            name: "d".into(),
            limit: 1000,
            in_price: 1.0,
            out_price: 2.0,
            ..Deployment::default()
        };
        let (meta, cost) = usage_meta(
            &Usage {
                input_tokens: 12_000,
                output_tokens: 1_500,
                cached_tokens: 8_000,
            },
            &dep,
        );
        assert_eq!(meta, "12k in (8k cached) / 1.5k out · $0.015");
        assert!((cost.unwrap() - 0.015).abs() < 1e-9);
        let notes = turn_notes(
            Some(STOPPED),
            false,
            Some(ApiKind::Chat),
            Some(ApiKind::Responses),
            false,
        );
        assert_eq!(notes[0].1, Tone::Warn);
        assert_eq!(notes.len(), 2);
    }
}
