//! Source Control for the open project (VS Code-style): git status, stage / unstage, commit
//! (smart commit when nothing is staged), push or publish, a model-drafted commit message, and
//! branches. Every git call goes through the workspace host.

use std::sync::Arc;

use contextual_core::host::WorkspaceHost;
use contextual_core::llm::{FoundryClient, FoundryConfig};
use contextual_protocol::git::{
    BranchList, BranchesRequest, CommitRequest, CreateBranchRequest, GitStatus, PathsRequest,
    RootRequest, SwitchRequest,
};
use gpui_kit::{Context, Entity, EventEmitter, Subscription};
use tokio_util::sync::CancellationToken;

use crate::runtime;
use crate::state::Project;

/// Open the branch comparison from Source Control without coupling it to the chat view.
pub struct OpenPullRequest;

pub struct ScmModel {
    project: Entity<Project>,
    host: Arc<dyn WorkspaceHost>,
    /// `None` until loaded, or when the folder isn't a git repository (`not_repo`).
    pub status: Option<GitStatus>,
    pub not_repo: bool,
    /// What's running ("Commit", "Push", "Drafting"…), for button labels.
    pub busy: Option<&'static str>,
    pub error: Option<String>,
    pub done: Option<String>,
    /// A drafted commit message waiting to go into the message box.
    pub drafted: Option<String>,
    pub branches: Option<BranchList>,
    pub branch_picker_open: bool,
    pub branch_error: Option<String>,
    seen_rescans: u64,
    _sub: Subscription,
}

impl EventEmitter<OpenPullRequest> for ScmModel {}

impl ScmModel {
    pub fn new(
        project: Entity<Project>,
        host: Arc<dyn WorkspaceHost>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Refresh after each rescan (saves, undo, the rescan button).
        let sub = cx.observe(&project, |this: &mut Self, project, cx| {
            let n = project.read(cx).rescans;
            if n != this.seen_rescans {
                this.seen_rescans = n;
                this.refresh(cx);
            }
        });
        let seen_rescans = project.read(cx).rescans;
        let mut this = Self {
            project,
            host,
            status: None,
            not_repo: false,
            busy: None,
            error: None,
            done: None,
            drafted: None,
            branches: None,
            branch_picker_open: false,
            branch_error: None,
            seen_rescans,
            _sub: sub,
        };
        this.refresh(cx);
        this
    }

    fn root(&self, cx: &Context<Self>) -> String {
        self.project.read(cx).root.clone()
    }

    /// Files with changes (staged, unstaged or conflicted), for the sidebar badge.
    pub fn change_count(&self) -> usize {
        let Some(s) = &self.status else { return 0 };
        let mut paths: Vec<&str> = s
            .staged
            .iter()
            .chain(&s.changes)
            .chain(&s.conflicts)
            .map(|e| e.path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let req = RootRequest {
            root: self.root(cx),
        };
        let job = runtime::spawn(cx, async move { host.git_status(req).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(s) => {
                        this.status = Some(s);
                        this.not_repo = false;
                    }
                    Err(_) => {
                        this.status = None;
                        this.not_repo = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs one git action, then refreshes; `done` is shown on success.
    fn act<T: Send + 'static>(
        &mut self,
        what: &'static str,
        job: impl std::future::Future<Output = contextual_core::Result<T>> + Send + 'static,
        on_ok: impl FnOnce(&mut Self, T) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(what);
        self.error = None;
        self.done = None;
        cx.notify();
        let job = runtime::spawn(cx, job);
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(v) => on_ok(this, v),
                    Err(e) => this.error = Some(e.message),
                }
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn stage(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let (host, root) = (self.host.clone(), self.root(cx));
        self.act(
            "Stage",
            async move { host.git_stage(PathsRequest { root, paths }).await },
            |_, _| {},
            cx,
        );
    }

    pub fn unstage(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let (host, root) = (self.host.clone(), self.root(cx));
        self.act(
            "Unstage",
            async move { host.git_unstage(PathsRequest { root, paths }).await },
            |_, _| {},
            cx,
        );
    }

    /// Commits the staged changes, or everything when nothing is staged (smart commit).
    pub fn commit(&mut self, message: String, cx: &mut Context<Self>) {
        let all = self
            .status
            .as_ref()
            .is_some_and(|s| s.staged.is_empty() && !s.changes.is_empty());
        let (host, root) = (self.host.clone(), self.root(cx));
        self.act(
            "Commit",
            async move { host.git_commit(CommitRequest { root, message, all }).await },
            |this, r| {
                this.done = Some(format!(
                    "Committed {} {} as {}.",
                    r.files,
                    if r.files == 1 { "file" } else { "files" },
                    r.commit
                ))
            },
            cx,
        );
    }

    pub fn push(&mut self, cx: &mut Context<Self>) {
        let (host, root) = (self.host.clone(), self.root(cx));
        self.act(
            "Push",
            async move { host.git_push(RootRequest { root }).await },
            |this, r| this.done = Some(format!("Pushed {} to {}.", r.branch, r.upstream)),
            cx,
        );
    }

    /// Asks the model for a message for the staged (or all) changes.
    pub fn draft_message(&mut self, config: FoundryConfig, cx: &mut Context<Self>) {
        let (host, root) = (self.host.clone(), self.root(cx));
        self.act(
            "Drafting",
            async move {
                let diff = host.git_commit_diff(RootRequest { root }).await?;
                FoundryClient::new(config)?
                    .draft_commit_message(&diff, &CancellationToken::new())
                    .await
            },
            |this, message| this.drafted = Some(message),
            cx,
        );
    }

    /// Review committed changes before explicitly pushing or creating a pull request.
    pub fn open_pull_request(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() || !self.status.as_ref().is_some_and(|s| s.branch.is_some()) {
            return;
        }
        cx.emit(OpenPullRequest);
    }

    // ------------------------------------------------------------------ branches

    pub fn open_branch_picker(&mut self, cx: &mut Context<Self>) {
        self.branch_picker_open = true;
        self.branch_error = None;
        // Local list first (fast), then again after a quiet fetch.
        self.load_branches(false, cx);
        self.load_branches(true, cx);
        cx.notify();
    }

    pub fn close_branch_picker(&mut self, cx: &mut Context<Self>) {
        self.branch_picker_open = false;
        cx.notify();
    }

    fn load_branches(&mut self, fetch: bool, cx: &mut Context<Self>) {
        let (host, root) = (self.host.clone(), self.root(cx));
        let job = runtime::spawn(cx, async move {
            host.git_branches(BranchesRequest { root, fetch }).await
        });
        cx.spawn(async move |this, cx| {
            if let Ok(list) = job.await {
                this.update(cx, |this, cx| {
                    this.branches = Some(list);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn branch_action(
        &mut self,
        job: impl std::future::Future<Output = contextual_core::Result<GitStatus>> + Send + 'static,
        on_switched: impl FnOnce(&mut gpui_kit::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.busy = Some("Branch");
        self.branch_error = None;
        cx.notify();
        let job = runtime::spawn(cx, job);
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match result {
                    Ok(s) => {
                        this.status = Some(s);
                        this.branch_picker_open = false;
                        on_switched(cx);
                    }
                    Err(e) => this.branch_error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Switches branch; `on_switched` rescans (the files changed under us).
    pub fn switch_branch(
        &mut self,
        name: String,
        on_switched: impl FnOnce(&mut gpui_kit::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let (host, root) = (self.host.clone(), self.root(cx));
        self.branch_action(
            async move { host.git_switch(SwitchRequest { root, name }).await },
            on_switched,
            cx,
        );
    }

    /// Creates a branch from the current one (recording it as the parent) and switches to it.
    pub fn create_branch(
        &mut self,
        name: String,
        on_switched: impl FnOnce(&mut gpui_kit::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let (host, root) = (self.host.clone(), self.root(cx));
        self.branch_action(
            async move {
                host.git_create_branch(CreateBranchRequest {
                    root,
                    name,
                    from: None,
                })
                .await
            },
            on_switched,
            cx,
        );
    }
}
