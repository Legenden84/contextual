//! The one interface every project-file operation goes through (plan §6.3).
//!
//! [`LocalHost`] runs the core in-process; the app's `RemoteHost` sends the same `protocol`
//! types to `contextual-agent` (which wraps a `LocalHost`). Both must pass the same contract
//! tests, so they can't drift.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use contextual_protocol::fs::{
    Apply, ApplyResult, DirListing, ListDirs, ReadFiles, ReadFilesResult, ReadOptions, Scan, ScanResult, Undo, UndoResult,
};

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
        self.roots.write().unwrap_or_else(|e| e.into_inner()).insert(root);
    }

    /// The normalised root if it is, or is inside, an allowed root.
    pub fn check(&self, root: &str) -> Result<PathBuf> {
        let root = paths::normalize_input_path(Some(root));
        let roots = self.roots.read().unwrap_or_else(|e| e.into_inner());
        if roots.iter().any(|allowed| paths::is_inside(allowed, &root)) {
            Ok(root)
        } else {
            Err(Error::forbidden(format!("Not an open project: {}", paths::path_string(&root))))
        }
    }
}

/// Runs the core in this process. The core's file functions are synchronous, so each call goes
/// to tokio's blocking pool.
#[derive(Debug, Default, Clone)]
pub struct LocalHost {
    allowlist: RootAllowlist,
}

impl LocalHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allowlist(&self) -> &RootAllowlist {
        &self.allowlist
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| Error::internal(format!("worker failed: {e}")))?
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
            let files = files::read_files(&req.root, &req.paths, ReadOptions { condense: req.condense });
            Ok(ReadFilesResult { files })
        })
        .await
    }

    async fn apply(&self, req: Apply) -> Result<ApplyResult> {
        self.allowlist.check(&req.root)?;
        blocking(move || files::apply_changes(&req.root, &req.changes, req.backup)).await
    }

    async fn undo(&self, req: Undo) -> Result<UndoResult> {
        self.allowlist.check(&req.root)?;
        blocking(move || files::undo_apply(&req.root, &req.backup_id)).await
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
