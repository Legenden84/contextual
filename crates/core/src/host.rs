//! The one interface every project-file operation goes through (plan §6.3).
//!
//! [`LocalHost`] runs the core in-process; the app's `RemoteHost` sends the same `protocol`
//! types to `contextual-agent` (which wraps a `LocalHost`). Both must pass the same contract
//! tests, so they can't drift.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use contextual_protocol::commands::{PrepareCommand, PreparedCommand};
use contextual_protocol::fs::{
    Apply, ApplyResult, DirListing, ListDirs, ReadFiles, ReadFilesResult, ReadOptions, Scan,
    ScanResult, Undo, UndoResult,
};
use contextual_protocol::git::{
    BranchList, CommitResult, Comparison, FileDiff, GitCall, GitReply, GitStatus, PrDiff, PrResult,
    PushResult,
};

use crate::commands::LaunchPermit;
use crate::commands::execution::{
    ExecutionEngine, ExecutionHandle, ExecutionJournal, ManagedCommandRunner,
};
use crate::workspace::git::Git;
use crate::workspace::{browse, files, paths, scan};
use crate::{Error, Result};

#[async_trait]
pub trait WorkspaceHost: Send + Sync {
    /// Lets later calls use `root` (and paths inside it). Called when the user opens a project.
    async fn allow_root(&self, root: &str) -> Result<()>;
    async fn list_dirs(&self, req: ListDirs) -> Result<DirListing>;
    async fn scan(&self, req: Scan) -> Result<ScanResult>;
    async fn read(&self, req: ReadFiles) -> Result<ReadFilesResult>;
    async fn apply(&self, req: Apply) -> Result<ApplyResult>;
    async fn undo(&self, req: Undo) -> Result<UndoResult>;
    /// In-process execution admission, offered only by native local hosts.
    /// Remote hosts deliberately return None: supplying a local project label must not enable
    /// a local execution fallback for a project served by an agent.
    fn local_command_admission(
        &self,
    ) -> Option<Arc<crate::commands::admission::ExecutionAdmission>> {
        None
    }
    /// Inspects an invocation on this host without executing it or granting approval.
    /// Hosts without command preparation fail closed rather than falling back to the app.
    async fn prepare_command(&self, _req: PrepareCommand) -> Result<PreparedCommand> {
        Err(Error::tool(
            "This workspace host does not support command preparation.",
        ))
    }
    /// Consumes one approved attempt on this project's host, without queuing or retrying.
    ///
    /// The caller must obtain the permit through ApprovalGate::begin after explicit review.
    /// Admission and launch refusal are synchronous. On error, no worker was started and the
    /// caller must finish the gate attempt as FailedToStart, never reuse its approval.
    /// Dropping the handle requests Stop; lifecycle owners must cancel and await workers.
    ///
    /// Default refusal is intentional. In particular, agent hosts must not fall back to an
    /// app-side runner merely because their remote execution transport is not implemented.
    fn start_command(
        &self,
        _permit: LaunchPermit,
        _scope: tokio_util::sync::CancellationToken,
    ) -> Result<ExecutionHandle> {
        Err(Error::tool(
            "Command execution is unavailable on this workspace host.",
        ))
    }
    /// Source Control and pull requests. Use the typed helpers on `dyn WorkspaceHost` (`git_status`, `pr_compare`…).
    async fn git(&self, call: GitCall) -> Result<GitReply>;
}

fn unexpected(reply: GitReply) -> Error {
    Error::internal(format!(
        "Unexpected reply from the workspace host: {reply:?}"
    ))
}

macro_rules! typed {
    ($(#[$doc:meta])* $name:ident, $variant:ident, $req:ty => $reply:ident($out:ty)) => {
        $(#[$doc])*
        pub async fn $name(&self, req: $req) -> Result<$out> {
            match self.git(GitCall::$variant(req)).await? {
                GitReply::$reply(v) => Ok(v),
                other => Err(unexpected(other)),
            }
        }
    };
}

/// Typed wrappers around [`WorkspaceHost::git`].
impl dyn WorkspaceHost {
    typed!(git_status, Status, contextual_protocol::git::RootRequest => Status(GitStatus));
    typed!(git_stage, Stage, contextual_protocol::git::PathsRequest => Status(GitStatus));
    typed!(git_unstage, Unstage, contextual_protocol::git::PathsRequest => Status(GitStatus));
    typed!(git_diff, Diff, contextual_protocol::git::DiffRequest => Diff(FileDiff));
    typed!(git_commit_diff, CommitDiff, contextual_protocol::git::RootRequest => CommitDiff(String));
    typed!(git_commit, Commit, contextual_protocol::git::CommitRequest => Commit(CommitResult));
    typed!(git_push, Push, contextual_protocol::git::RootRequest => Push(PushResult));
    typed!(git_branches, Branches, contextual_protocol::git::BranchesRequest => Branches(BranchList));
    typed!(git_switch, Switch, contextual_protocol::git::SwitchRequest => Status(GitStatus));
    typed!(git_create_branch, CreateBranch, contextual_protocol::git::CreateBranchRequest => Status(GitStatus));
    typed!(pr_compare, Compare, contextual_protocol::git::CompareRequest => Compare(Comparison));
    typed!(pr_diff, PrDiff, contextual_protocol::git::PrDiffRequest => PrDiff(PrDiff));
    typed!(pr_create, CreatePr, contextual_protocol::git::CreatePrRequest => CreatePr(PrResult));
}

/// Runs one [`GitCall`] with the core's [`Git`] (shared by [`LocalHost`] and the agent).
pub async fn run_git(git: &Git, call: GitCall) -> Result<GitReply> {
    Ok(match call {
        GitCall::Status(r) => GitReply::Status(git.status(&r.root).await?),
        GitCall::Stage(r) => {
            git.stage(&r.root, &r.paths).await?;
            GitReply::Status(git.status(&r.root).await?)
        }
        GitCall::Unstage(r) => {
            git.unstage(&r.root, &r.paths).await?;
            GitReply::Status(git.status(&r.root).await?)
        }
        GitCall::Diff(r) => GitReply::Diff(git.file_diff(&r.root, &r.path, r.staged).await?),
        GitCall::CommitDiff(r) => GitReply::CommitDiff(git.commit_diff(&r.root).await?),
        GitCall::Commit(r) => GitReply::Commit(git.commit(&r.root, &r.message, r.all).await?),
        GitCall::Push(r) => GitReply::Push(git.push(&r.root).await?),
        GitCall::Branches(r) => GitReply::Branches(git.list_branches(&r.root, r.fetch).await?),
        GitCall::Switch(r) => GitReply::Status(git.switch_branch(&r.root, &r.name).await?),
        GitCall::CreateBranch(r) => GitReply::Status(
            git.create_branch(&r.root, &r.name, r.from.as_deref())
                .await?,
        ),
        GitCall::Compare(r) => GitReply::Compare(
            git.compare(&r.root, r.base.as_deref(), r.head.as_deref())
                .await?,
        ),
        GitCall::PrDiff(r) => {
            GitReply::PrDiff(git.pr_diff(&r.root, &r.base, r.head.as_deref()).await?)
        }
        GitCall::CreatePr(r) => GitReply::CreatePr(git.create_pull_request(&r.root, &r.pr).await?),
    })
}

/// Project roots the user opened (dialog, recents, CLI). Workspace calls outside them are refused,
/// so a bug or a malicious message can't make the host read or write elsewhere.
#[derive(Debug, Default, Clone)]
pub struct RootAllowlist {
    roots: Arc<RwLock<BTreeSet<PathBuf>>>,
}

impl RootAllowlist {
    pub fn allow(&self, root: &str) {
        let root = paths::normalize_input_path(Some(root));
        self.roots
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(root);
    }

    /// Command roots use native absolute paths without trimming, expansion or WSL translation.
    /// Canonical containment also rejects a symlink inside an allowed root that leads outside.
    pub fn check_command_root(&self, root: &str) -> Result<PathBuf> {
        let requested = std::path::Path::new(root);
        if root.contains('\0') || !requested.is_absolute() {
            return Err(Error::invalid(
                "Command project root must be a native absolute path.",
            ));
        }
        let real = dunce::canonicalize(requested)?;
        if !real.is_dir() {
            return Err(Error::invalid(
                "Command project root must be an existing directory.",
            ));
        }
        let roots = self.roots.read().unwrap_or_else(|e| e.into_inner());
        if roots.iter().any(|allowed| {
            dunce::canonicalize(allowed).is_ok_and(|allowed| real.starts_with(allowed))
        }) {
            Ok(real)
        } else {
            Err(Error::forbidden(
                "Command project root is not inside an open project.",
            ))
        }
    }

    /// The normalised root if it is, or is inside, an allowed root.
    pub fn check(&self, root: &str) -> Result<PathBuf> {
        let root = paths::normalize_input_path(Some(root));
        let roots = self.roots.read().unwrap_or_else(|e| e.into_inner());
        if roots.iter().any(|allowed| paths::is_inside(allowed, &root)) {
            Ok(root)
        } else {
            Err(Error::forbidden(format!(
                "Not an open project: {}",
                paths::path_string(&root)
            )))
        }
    }
}

struct CommandBackend {
    runner: Arc<dyn ManagedCommandRunner>,
    journal: Arc<dyn ExecutionJournal>,
}

/// Runs the core in this process. The core's file functions are synchronous, so each call goes
/// to tokio's blocking pool.
#[derive(Clone, Default)]
pub struct LocalHost {
    allowlist: RootAllowlist,
    git: Arc<Git>,
    /// Shared across clones; replaced when a new host/agent instance is created.
    command_preparer: Arc<crate::commands::prepare::CommandPreparer>,
    /// Explicit integration seam. Ordinary hosts have no execution backend.
    command_backend: Option<Arc<CommandBackend>>,
    /// Undo snapshots; `None` means the OS cache dir (`files::default_undo_dir`).
    undo_dir: Option<PathBuf>,
}

impl std::fmt::Debug for LocalHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalHost")
            .field("allowlist", &self.allowlist)
            .finish_non_exhaustive()
    }
}

impl LocalHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// With a custom [`Git`] (e.g. a fake `gh` runner in tests).
    pub fn with_git(git: Git) -> Self {
        Self {
            git: Arc::new(git),
            ..Self::default()
        }
    }

    /// Creates a fresh host with an explicitly supplied managed backend and durable journal.
    ///
    /// This is a host-service integration seam, not an app setting or a native backend
    /// selector. The runner must meet ManagedCommandRunner's ownership contract. No native
    /// implementation is supplied yet; fake processes exercise this path in tests.
    /// Construction gives the host a fresh preparation epoch, invalidating earlier snapshots.
    pub fn with_command_backend(
        runner: Arc<dyn ManagedCommandRunner>,
        journal: Arc<dyn ExecutionJournal>,
    ) -> Self {
        Self {
            command_backend: Some(Arc::new(CommandBackend { runner, journal })),
            ..Self::default()
        }
    }

    /// Keeps undo snapshots in `dir` instead of the OS cache dir.
    pub fn with_undo_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.undo_dir = Some(dir.into());
        self
    }

    fn undo_dir(&self) -> PathBuf {
        self.undo_dir
            .clone()
            .unwrap_or_else(files::default_undo_dir)
    }

    pub fn allowlist(&self) -> &RootAllowlist {
        &self.allowlist
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::internal(format!("worker failed: {e}")))?
}

#[async_trait]
impl WorkspaceHost for LocalHost {
    async fn allow_root(&self, root: &str) -> Result<()> {
        self.allowlist.allow(root);
        Ok(())
    }

    async fn list_dirs(&self, req: ListDirs) -> Result<DirListing> {
        // Browsing isn't project-bound: it's how the user picks a project.
        blocking(move || browse::list_dirs(req.path.as_deref(), req.hidden)).await
    }

    async fn scan(&self, req: Scan) -> Result<ScanResult> {
        self.allowlist.check(&req.root)?;
        blocking(move || scan::scan_project(&req.root, &req.options)).await
    }

    async fn read(&self, req: ReadFiles) -> Result<ReadFilesResult> {
        self.allowlist.check(&req.root)?;
        blocking(move || {
            let files = files::read_files(
                &req.root,
                &req.paths,
                ReadOptions {
                    condense: req.condense,
                },
            );
            Ok(ReadFilesResult { files })
        })
        .await
    }

    async fn apply(&self, req: Apply) -> Result<ApplyResult> {
        self.allowlist.check(&req.root)?;
        let dir = self.undo_dir();
        blocking(move || files::apply_changes_in(&dir, &req.root, &req.changes, req.backup)).await
    }

    async fn undo(&self, req: Undo) -> Result<UndoResult> {
        self.allowlist.check(&req.root)?;
        let dir = self.undo_dir();
        blocking(move || files::undo_apply_in(&dir, &req.root, &req.backup_id)).await
    }

    fn local_command_admission(
        &self,
    ) -> Option<Arc<crate::commands::admission::ExecutionAdmission>> {
        Some(crate::commands::admission::local_registry())
    }

    async fn prepare_command(&self, req: PrepareCommand) -> Result<PreparedCommand> {
        let allowlist = self.allowlist.clone();
        let preparer = self.command_preparer.clone();
        blocking(move || {
            let root = allowlist.check_command_root(&req.root)?;
            preparer.prepare(&root, req)
        })
        .await
    }

    fn start_command(
        &self,
        permit: LaunchPermit,
        scope: tokio_util::sync::CancellationToken,
    ) -> Result<ExecutionHandle> {
        let backend = self.command_backend.as_ref().ok_or_else(|| {
            Error::tool("Command execution is unavailable on this workspace host.")
        })?;
        // The engine receives this exact host instance, including its preparation epoch and
        // allowlist. It cannot accidentally pair an agent host with an app-side process runner.
        ExecutionEngine::new(
            Arc::new(self.clone()),
            backend.runner.clone(),
            backend.journal.clone(),
        )
        .start(permit, scope)
    }

    async fn git(&self, call: GitCall) -> Result<GitReply> {
        self.allowlist.check(call.root())?;
        run_git(&self.git, call).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        let list = RootAllowlist::default();
        assert!(list.check("/tmp/project").is_err());
        list.allow("/tmp/project");
        assert!(list.check("/tmp/project").is_ok());
        assert!(list.check("/tmp/project/sub").is_ok());
        assert!(list.check("/tmp/project-other").is_err());
        assert!(list.check("/tmp/project/../x").is_err());
    }
}
