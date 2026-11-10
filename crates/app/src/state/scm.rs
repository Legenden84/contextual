//! Source Control for the open project (VS Code-style): git status, stage / unstage, commit
//! (smart commit when nothing is staged), push or publish, a model-drafted commit message, and
//! branches. Every git call goes through the workspace host.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use contextual_core::host::WorkspaceHost;
use contextual_core::llm::{FoundryClient, FoundryConfig};
use contextual_protocol::fs::Repository;
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
    pub repositories: Vec<Repository>,
    active_root: Option<String>,
    pub repository_statuses: HashMap<String, GitStatus>,
    pub repository_errors: HashMap<String, String>,
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
    refreshing: bool,
    refresh_pending: bool,
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
            repositories: Vec::new(),
            active_root: None,
            repository_statuses: HashMap::new(),
            repository_errors: HashMap::new(),
            busy: None,
            error: None,
            done: None,
            drafted: None,
            branches: None,
            branch_picker_open: false,
            branch_error: None,
            seen_rescans,
            refreshing: false,
            refresh_pending: false,
            _sub: sub,
        };
        this.refresh(cx);
        this
    }

    fn root(&self, _cx: &Context<Self>) -> String {
        self.active_root.clone().unwrap_or_default()
    }

    pub fn repository_root(&self) -> Option<&str> {
        self.active_root.as_deref()
    }

    pub fn active_repository(&self) -> Option<&Repository> {
        self.repositories.iter().find(|r| Some(&r.root) == self.active_root.as_ref())
    }

    pub fn repository_name(&self, cx: &gpui_kit::App) -> String {
        self.active_repository()
            .filter(|r| !r.path.is_empty())
            .map(|r| r.path.clone())
            .unwrap_or_else(|| self.project.read(cx).name.clone())
    }

    fn reset_scope(&mut self) {
        self.status = None;
        self.branches = None;
        self.branch_picker_open = false;
        self.branch_error = None;
        self.error = None;
        self.done = None;
        self.drafted = None;
    }

    /// Selects the scope of Git actions, not a workspace mode.
    pub fn select_repository(&mut self, root: &str, cx: &mut Context<Self>) {
        if self.busy.is_some()
            || self.branch_picker_open
            || self.repository_root() == Some(root)
            || !self.repositories.iter().any(|r| r.root == root)
        {
            return;
        }
        self.reset_scope();
        self.active_root = Some(root.to_string());
        self.status = self.repository_statuses.get(root).cloned();
        self.refresh(cx);
        cx.notify();
    }

    fn sync_repositories(&mut self, cx: &Context<Self>) {
        self.repositories = self.project.read(cx).repositories.clone();
        if !self.repositories.iter().any(|r| Some(&r.root) == self.active_root.as_ref()) {
            self.reset_scope();
            self.active_root = self.repositories.first().map(|r| r.root.clone());
        }
        self.not_repo = self.repositories.is_empty();
        self.repository_statuses.retain(|root, _| self.repositories.iter().any(|r| &r.root == root));
        self.repository_errors.retain(|root, _| self.repositories.iter().any(|r| &r.root == root));
        self.status = self.active_root.as_ref().and_then(|root| self.repository_statuses.get(root)).cloned();
    }

    /// Files with changes across all repositories, for the sidebar badge.
    pub fn change_count(&self) -> usize {
        self.repository_statuses.values().map(status_change_count).sum()
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            self.refresh_pending = true;
            return;
        }
        if self.refreshing {
            // A fresh read must follow the current one: it may predate an external commit.
            self.refresh_pending = true;
            return;
        }
        self.refresh_pending = false;
        self.sync_repositories(cx);
        self.refreshing = true;
        let host = self.host.clone();
        let repositories = self.repositories.clone();
        let job = runtime::spawn(cx, async move {
            let mut results = Vec::with_capacity(repositories.len());
            for repository in repositories {
                let result = host.git_status(RootRequest { root: repository.root.clone() }).await;
                results.push((repository.root, result));
            }
            Ok(results)
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.refreshing = false;
                if this.busy.is_some() {
                    this.refresh_pending = true;
                    return;
                }
                if std::mem::take(&mut this.refresh_pending) {
                    // Don't publish a snapshot that another refresh has already superseded.
                    this.refresh(cx);
                    return;
                }
                match result {
                    Ok(results) => {
                        this.repository_statuses.clear();
                        this.repository_errors.clear();
                        for (root, result) in results {
                            match result {
                                Ok(status) => {
                                    this.repository_statuses.insert(root, status);
                                }
                                Err(error) => {
                                    this.repository_errors.insert(root, error.message);
                                }
                            }
                        }
                        this.status = this.active_root.as_ref()
                            .and_then(|root| this.repository_statuses.get(root)).cloned();
                    }
                    Err(error) => {
                        this.status = None;
                        this.error = Some(error.message);
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
        if self.busy.is_some() || self.active_root.is_none() {
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

    /// Stages rows in their own repository without changing the commit/push selection.
    pub fn stage_in(&mut self, root: &str, paths: Vec<String>, cx: &mut Context<Self>) {
        if !self.repositories.iter().any(|repository| repository.root == root)
            || !self.repository_statuses.contains_key(root)
        {
            return;
        }
        let (host, root) = (self.host.clone(), root.to_string());
        let scope = root.clone();
        self.act(
            "Stage",
            async move {
                host.git_stage(PathsRequest { root, paths })
                    .await
                    .map_err(|mut error| {
                        error.message = format!("{scope}: {}", error.message);
                        error
                    })
            },
            |_, _| {},
            cx,
        );
    }

    /// Unstages only the named repository's rows.
    pub fn unstage_in(&mut self, root: &str, paths: Vec<String>, cx: &mut Context<Self>) {
        if !self.repositories.iter().any(|repository| repository.root == root)
            || !self.repository_statuses.contains_key(root)
        {
            return;
        }
        let (host, root) = (self.host.clone(), root.to_string());
        let scope = root.clone();
        self.act(
            "Unstage",
            async move {
                host.git_unstage(PathsRequest { root, paths })
                    .await
                    .map_err(|mut error| {
                        error.message = format!("{scope}: {}", error.message);
                        error
                    })
            },
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
        if self.busy.is_some() || self.active_root.is_none() || self.status.is_none() {
            return;
        }
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
        let requested_root = root.clone();
        let job = runtime::spawn(cx, async move {
            host.git_branches(BranchesRequest { root, fetch }).await
        });
        cx.spawn(async move |this, cx| {
            if let Ok(list) = job.await {
                this.update(cx, |this, cx| {
                    if this.repository_root() != Some(requested_root.as_str()) {
                        return;
                    }
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
        if self.busy.is_some() || self.active_root.is_none() || self.status.is_none() {
            return;
        }
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
                this.refresh(cx);
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

pub fn status_change_count(status: &GitStatus) -> usize {
    status.staged.iter()
        .chain(&status.changes)
        .chain(&status.conflicts)
        .map(|entry| entry.path.as_str())
        .collect::<HashSet<_>>()
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextual_protocol::git::{ChangeLetter, StatusEntry};

    #[test]
    fn badges_deduplicate_within_each_repository_not_across_repositories() {
        let entry = StatusEntry {
            path: "README.md".into(),
            from: None,
            letter: ChangeLetter::M,
        };
        let frontend = GitStatus {
            staged: vec![entry.clone()],
            changes: vec![entry.clone()],
            ..GitStatus::default()
        };
        let backend = GitStatus {
            changes: vec![entry],
            ..GitStatus::default()
        };
        assert_eq!(status_change_count(&frontend), 1);
        assert_eq!([frontend, backend].iter().map(status_change_count).sum::<usize>(), 2);
    }
}
