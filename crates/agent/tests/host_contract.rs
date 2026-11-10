//! Host contract suite (plan §11.3): the same scenarios against `LocalHost` (in-process) and
//! `RemoteHost` (the real `contextual-agent` binary over stdio), so the two can't drift.

use std::path::Path;
use std::process::Command as StdCommand;
use std::sync::Arc;

use contextual_core::error::ErrorCode;
use contextual_core::host::{LocalHost, WorkspaceHost};
use contextual_core::remote::{LocalAgentLauncher, RemoteHost, SupervisedHost};
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

fn launcher(f: &Fixture) -> LocalAgentLauncher {
    LocalAgentLauncher {
        binary: env!("CARGO_BIN_EXE_contextual-agent").into(),
        app_version: "test".into(),
        env: vec![(
            "XDG_CACHE_HOME".into(),
            f.undo.path().to_string_lossy().into_owned(),
        )],
    }
}

/// The agent behind the supervisor (started lazily, restarted when it dies).
async fn supervised(f: &Fixture) -> Arc<dyn WorkspaceHost> {
    let host = SupervisedHost::new(Arc::new(launcher(f)));
    host.allow_root(&f.root()).await.unwrap();
    Arc::new(host)
}

macro_rules! contract {
    ($($name:ident),* $(,)?) => {
        mod local_host {
            $(#[tokio::test] async fn $name() { super::$name(super::local).await })*
        }
        mod remote_host {
            $(#[tokio::test] async fn $name() { super::$name(super::remote).await })*
        }
        mod supervised_host {
            $(#[tokio::test] async fn $name() { super::$name(super::supervised).await })*
        }
    };
}

contract!(
    refuses_other_roots,
    scan_and_read,
    apply_then_undo,
    conflict_round_trips_exactly,
    git_stage_commit,
    sibling_repositories,
    list_dirs,
    command_preparation
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

async fn sibling_repositories<F: AsyncFnOnce(&Fixture) -> Arc<dyn WorkspaceHost>>(make: F) {
    let f = Fixture::new();
    let workspace = f.project.path();
    std::fs::remove_dir_all(workspace.join(".git")).unwrap();
    for name in ["frontend", "backend"] {
        let dir = workspace.join(name);
        std::fs::create_dir(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.name", "t"]);
        git(&dir, &["config", "user.email", "t@t"]);
        std::fs::write(dir.join("README.md"), format!("{name}\n")).unwrap();
        std::fs::write(dir.join(".gitignore"), "/generated/\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "init"]);
        std::fs::create_dir(dir.join("generated")).unwrap();
        std::fs::write(dir.join("generated/output.txt"), "ignored\n").unwrap();
    }
    let host = make(&f).await;
    let scan = host
        .scan(Scan {
            root: f.root(),
            options: ScanOptions::default(),
        })
        .await
        .unwrap();
    assert_eq!(scan.branch, None);
    assert_eq!(
        scan.repositories
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        vec!["backend", "frontend"],
    );
    assert!(
        host.git_status(RootRequest { root: f.root() })
            .await
            .is_err()
    );
    let backend = scan.repositories[0].root.clone();
    let frontend = scan.repositories[1].root.clone();
    let read = host
        .read(ReadFiles {
            root: f.root(),
            paths: vec!["frontend/README.md".into(), "backend/README.md".into()],
            condense: false,
        })
        .await
        .unwrap();
    assert_eq!(read.files[0].content.as_deref(), Some("frontend\n"));
    assert_eq!(read.files[1].content.as_deref(), Some("backend\n"));

    let saved = host
        .apply(Apply {
            root: f.root(),
            changes: vec![
                ApplyChange {
                    path: "frontend/README.md".into(),
                    status: ChangeStatus::M,
                    content: Some("new frontend\n".into()),
                    expected: Some("frontend\n".into()),
                },
                ApplyChange {
                    path: "backend/README.md".into(),
                    status: ChangeStatus::M,
                    content: Some("new backend\n".into()),
                    expected: Some("backend\n".into()),
                },
            ],
            backup: true,
        })
        .await
        .unwrap();
    assert_eq!(saved.overwritten, 2);
    let status = host
        .git_stage(PathsRequest {
            root: frontend.clone(),
            paths: vec!["README.md".into()],
        })
        .await
        .unwrap();
    assert_eq!(status.staged.len(), 1);
    let status = host
        .git_status(RootRequest {
            root: backend.clone(),
        })
        .await
        .unwrap();
    assert!(status.staged.is_empty());
    assert_eq!(status.changes.len(), 1, "generated files must stay ignored");

    let diff = host
        .git_diff(contextual_protocol::git::DiffRequest {
            root: frontend.clone(),
            path: "README.md".into(),
            staged: true,
        })
        .await
        .unwrap();
    assert_eq!(diff.old, "frontend\n");
    assert_eq!(diff.new, "new frontend\n");

    host.git_unstage(PathsRequest {
        root: frontend.clone(),
        paths: vec!["README.md".into()],
    })
    .await
    .unwrap();
    let undone = host
        .undo(Undo {
            root: f.root(),
            backup_id: saved.backup_id.unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(undone.restored, 2);
    assert_eq!(
        std::fs::read_to_string(workspace.join("frontend/README.md")).unwrap(),
        "frontend\n"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("backend/README.md")).unwrap(),
        "backend\n"
    );

    std::fs::write(workspace.join("frontend/README.md"), "commit frontend\n").unwrap();
    std::fs::write(workspace.join("backend/README.md"), "leave backend\n").unwrap();
    host.git_commit(CommitRequest {
        root: frontend.clone(),
        message: "Frontend only".into(),
        all: true,
    })
    .await
    .unwrap();
    assert!(
        host.git_status(RootRequest {
            root: frontend.clone()
        })
        .await
        .unwrap()
        .changes
        .is_empty()
    );
    assert_eq!(
        host.git_status(RootRequest {
            root: backend.clone()
        })
        .await
        .unwrap()
        .changes
        .len(),
        1
    );
    host.git_create_branch(contextual_protocol::git::CreateBranchRequest {
        root: frontend,
        name: "frontend-change".into(),
        from: None,
    })
    .await
    .unwrap();
    assert_eq!(
        host.git_status(RootRequest { root: backend })
            .await
            .unwrap()
            .branch
            .as_deref(),
        Some("main"),
    );
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

fn command_request(root: String) -> contextual_protocol::commands::PrepareCommand {
    contextual_protocol::commands::PrepareCommand {
        root,
        project_epoch: "test-project-instance".into(),
        command_json: serde_json::json!({
            "version": 1,
            "purpose": "Prepare this test executable without running it.",
            "command": {
                "kind": "exec",
                "program": std::env::current_exe().unwrap().to_str().unwrap(),
                "args": ["a b", "$HOME", "&"]
            },
            "cwd": ".",
            "timeoutSeconds": 30,
            "environment": [{"name": "CI", "value": "true"}]
        })
        .to_string(),
    }
}

async fn command_preparation<F: AsyncFnOnce(&Fixture) -> Arc<dyn WorkspaceHost>>(make: F) {
    let f = Fixture::new();
    let host = make(&f).await;
    let request = command_request(f.root());
    let first = host.prepare_command(request.clone()).await.unwrap();
    let second = host.prepare_command(request.clone()).await.unwrap();

    assert_eq!(first.request, request);
    assert_eq!(first.host_epoch, second.host_epoch);
    assert!(!first.host_epoch.is_empty());
    assert_eq!(first.args, ["a b", "$HOME", "&"]);
    assert_eq!(first.cwd, first.root);
    assert_eq!(
        first.environment.get("CI").map(String::as_str),
        Some("true")
    );
    assert!(!first.environment.contains_key("AZURE_OPENAI_API_KEY"));
    assert!(!first.environment.contains_key("SSH_AUTH_SOCK"));
    assert!(!first.environment.contains_key("GIT_DIR"));

    let other = tempfile::tempdir().unwrap();
    let error = host
        .prepare_command(command_request(other.path().to_str().unwrap().into()))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Forbidden);

    let mut invalid = command_request(f.root());
    let mut arguments: serde_json::Value = serde_json::from_str(&invalid.command_json).unwrap();
    arguments["cwd"] = serde_json::json!("../outside");
    invalid.command_json = arguments.to_string();
    assert_eq!(
        host.prepare_command(invalid).await.unwrap_err().code,
        ErrorCode::InvalidInput,
    );
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

#[tokio::test]
async fn supervisor_restarts_a_dead_agent_and_keeps_roots() {
    let f = Fixture::new();
    let host = SupervisedHost::new(Arc::new(launcher(&f)));
    host.allow_root(&f.root()).await.unwrap();
    let first = host.connected().await.unwrap();
    let first_preparation = host
        .prepare_command(command_request(f.root()))
        .await
        .unwrap();
    assert!(
        host.scan(Scan {
            root: f.root(),
            options: ScanOptions::default()
        })
        .await
        .is_ok()
    );

    // The agent goes away (crash, distro shut down…).
    first.shutdown().await.unwrap();
    for _ in 0..50 {
        if !first.is_alive() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(!first.is_alive());

    // The next call starts a new agent, which already knows the project root.
    let scan = host
        .scan(Scan {
            root: f.root(),
            options: ScanOptions::default(),
        })
        .await
        .unwrap();
    assert_eq!(scan.files, 3);
    let second = host.connected().await.unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    let second_preparation = host
        .prepare_command(command_request(f.root()))
        .await
        .unwrap();
    assert_ne!(first_preparation.host_epoch, second_preparation.host_epoch);
    // Roots stay enforced on the new agent.
    let other = tempfile::tempdir().unwrap();
    let err = host
        .scan(Scan {
            root: other.path().to_string_lossy().into(),
            options: ScanOptions::default(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Forbidden);
}
