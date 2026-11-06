//! Host contract suite (plan §11.3): the same scenarios against `LocalHost` (in-process) and
//! `RemoteHost` (the real `contextual-agent` binary over stdio), so the two can't drift.

use std::path::Path;
use std::process::Command as StdCommand;
use std::sync::Arc;

use contextual_core::error::ErrorCode;
use contextual_core::host::{LocalHost, WorkspaceHost};
use contextual_core::remote::RemoteHost;
use contextual_protocol::fs::{
    Apply, ApplyChange, ChangeStatus, ListDirs, ReadFiles, Scan, ScanOptions, Undo,
};
use contextual_protocol::git::{CommitRequest, GitStatus, PathsRequest, RootRequest};
use tempfile::TempDir;

struct Fixture {
    project: TempDir,
    undo: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let project = tempfile::tempdir().unwrap();
        let p = project.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/a.txt"), "one\r\ntwo\r\n").unwrap();
        std::fs::write(p.join("README.md"), "\u{feff}# Hi\n").unwrap();
        std::fs::write(p.join("AGENTS.md"), "Be nice.\n").unwrap();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["add", "-A"]);
        git(
            p,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
            ],
        );
        Self {
            project,
            undo: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> String {
        self.project.path().to_string_lossy().into_owned()
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = StdCommand::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

async fn local(f: &Fixture) -> Arc<dyn WorkspaceHost> {
    let host = LocalHost::new().with_undo_dir(f.undo.path());
    host.allow_root(&f.root()).await.unwrap();
    Arc::new(host)
}

async fn remote(f: &Fixture) -> Arc<dyn WorkspaceHost> {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_contextual-agent"));
    // The agent keeps undo snapshots under the XDG cache dir.
    cmd.env("XDG_CACHE_HOME", f.undo.path())
        .env("RUST_LOG", "debug");
    Arc::new(
        RemoteHost::spawn(cmd, "test", vec![f.root()])
            .await
            .unwrap(),
    )
}

macro_rules! contract {
    ($($name:ident),* $(,)?) => {
        mod local_host {
            $(#[tokio::test] async fn $name() { super::$name(super::local).await })*
        }
        mod remote_host {
            $(#[tokio::test] async fn $name() { super::$name(super::remote).await })*
        }
    };
}

contract!(
    refuses_other_roots,
    scan_and_read,
    apply_then_undo,
    conflict_round_trips_exactly,
    git_stage_commit,
    list_dirs
);

async fn refuses_other_roots<F: AsyncFnOnce(&Fixture) -> Arc<dyn WorkspaceHost>>(make: F) {
    let f = Fixture::new();
    let host = make(&f).await;
    let other = tempfile::tempdir().unwrap();
    let err = host
        .scan(Scan {
            root: other.path().to_string_lossy().into(),
            options: ScanOptions::default(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
    let err = host
        .git_status(RootRequest { root: "/".into() })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
}

async fn scan_and_read<F: AsyncFnOnce(&Fixture) -> Arc<dyn WorkspaceHost>>(make: F) {
    let f = Fixture::new();
    let host = make(&f).await;
    let scan = host
        .scan(Scan {
            root: f.root(),
            options: ScanOptions::default(),
        })
        .await
        .unwrap();
    assert_eq!(scan.branch.as_deref(), Some("main"));
    assert_eq!(scan.files, 3);
    assert!(scan.rules.iter().any(|r| r.path == "AGENTS.md"));

    let read = host
        .read(ReadFiles {
            root: f.root(),
            paths: vec!["src/a.txt".into(), "README.md".into(), "../x".into()],
            condense: false,
        })
        .await
        .unwrap();
    assert_eq!(read.files[0].content.as_deref(), Some("one\ntwo\n"));
    assert_eq!(read.files[1].content.as_deref(), Some("# Hi\n"));
    assert!(read.files[2].error.is_some());
}

async fn apply_then_undo<F: AsyncFnOnce(&Fixture) -> Arc<dyn WorkspaceHost>>(make: F) {
    let f = Fixture::new();
    let host = make(&f).await;
    let changes = vec![
        ApplyChange {
            path: "src/a.txt".into(),
            status: ChangeStatus::M,
            content: Some("one\nTWO\n".into()),
            expected: Some("one\ntwo\n".into()),
        },
        ApplyChange {
            path: "src/new/b.txt".into(),
            status: ChangeStatus::A,
            content: Some("b\n".into()),
            expected: None,
        },
    ];
    let res = host
        .apply(Apply {
            root: f.root(),
            changes,
            backup: true,
        })
        .await
        .unwrap();
    assert_eq!((res.created, res.overwritten, res.deleted), (1, 1, 0));
    let p = f.project.path();
    assert_eq!(
        std::fs::read(p.join("src/a.txt")).unwrap(),
        b"one\r\nTWO\r\n"
    );

    let undo = host
        .undo(Undo {
            root: f.root(),
            backup_id: res.backup_id.unwrap(),
        })
        .await
        .unwrap();
    assert_eq!((undo.restored, undo.removed), (1, 1));
    assert_eq!(
        std::fs::read(p.join("src/a.txt")).unwrap(),
        b"one\r\ntwo\r\n"
    );
    assert!(!p.join("src/new").exists());
}

async fn conflict_round_trips_exactly<F: AsyncFnOnce(&Fixture) -> Arc<dyn WorkspaceHost>>(make: F) {
    let f = Fixture::new();
    let host = make(&f).await;
    let change = ApplyChange {
        path: "src/a.txt".into(),
        status: ChangeStatus::M,
        content: Some("x\n".into()),
        expected: Some("not what is on disk\n".into()),
    };
    let err = host
        .apply(Apply {
            root: f.root(),
            changes: vec![change],
            backup: true,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict, "{err:?}");
    assert!(
        err.message.starts_with("Nothing was saved"),
        "{}",
        err.message
    );
}

async fn git_stage_commit<F: AsyncFnOnce(&Fixture) -> Arc<dyn WorkspaceHost>>(make: F) {
    let f = Fixture::new();
    let host = make(&f).await;
    std::fs::write(f.project.path().join("c.txt"), "c\n").unwrap();
    let st: GitStatus = host
        .git_stage(PathsRequest {
            root: f.root(),
            paths: vec!["c.txt".into()],
        })
        .await
        .unwrap();
    assert_eq!(st.staged.len(), 1);
    // The commit uses the repo's identity; set one for the test repo.
    git(f.project.path(), &["config", "user.name", "t"]);
    git(f.project.path(), &["config", "user.email", "t@t"]);
    let c = host
        .git_commit(CommitRequest {
            root: f.root(),
            message: "add c".into(),
            all: false,
        })
        .await
        .unwrap();
    assert_eq!(c.files, 1);
    let st = host
        .git_status(RootRequest { root: f.root() })
        .await
        .unwrap();
    assert!(st.staged.is_empty() && st.changes.is_empty());
}

async fn list_dirs<F: AsyncFnOnce(&Fixture) -> Arc<dyn WorkspaceHost>>(make: F) {
    let f = Fixture::new();
    let host = make(&f).await;
    let listing = host
        .list_dirs(ListDirs {
            path: Some(f.root()),
            hidden: false,
        })
        .await
        .unwrap();
    assert!(listing.dirs.iter().any(|e| e.name == "src"));
    assert!(!listing.dirs.iter().any(|e| e.name == ".git"));
}

#[tokio::test]
async fn remote_handshake_and_shutdown() {
    let f = Fixture::new();
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_contextual-agent"));
    cmd.env("XDG_CACHE_HOME", f.undo.path());
    let host = RemoteHost::spawn(cmd, "test", vec![f.root()])
        .await
        .unwrap();
    assert_eq!(
        host.info().protocol_version,
        contextual_protocol::rpc::PROTOCOL_VERSION
    );
    assert!(host.info().capabilities.iter().any(|c| c == "git"));
    let err = host
        .request::<_, serde_json::Value>("nope", serde_json::Value::Null)
        .await
        .unwrap_err();
    assert!(err.message.contains("Unknown method"), "{}", err.message);
    host.shutdown().await.unwrap();
    let err = host
        .request::<_, serde_json::Value>("nope", serde_json::Value::Null)
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Io);
}
