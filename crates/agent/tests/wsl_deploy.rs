//! WSL deployment end to end on Linux: a fake `wsl.exe` runs the command it is given (as
//! `wsl.exe -d <distro> -e …` would inside the distro), with `$HOME` in a temp folder.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use contextual_core::host::WorkspaceHost;
use contextual_core::remote::{AgentLauncher, SupervisedHost};
use contextual_core::wsl::{WslLauncher, agent_version};
use contextual_protocol::fs::{Scan, ScanOptions};

fn fake_wsl(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("wsl.exe");
    std::fs::write(
        &path,
        "#!/bin/sh\n[ \"$1\" = -d ] && shift 2\n[ \"$1\" = -e ] && shift\nexec \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn arch() -> String {
    String::from_utf8(
        std::process::Command::new("uname")
            .arg("-m")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string()
}

fn launcher(home: &std::path::Path, tools: &std::path::Path) -> WslLauncher {
    WslLauncher {
        distro: "Test".into(),
        app_version: "0.1.0".into(),
        wsl: fake_wsl(tools),
        binaries: vec![(arch(), env!("CARGO_BIN_EXE_contextual-agent").into())],
        env: vec![
            ("HOME".into(), home.to_string_lossy().into_owned()),
            (
                "XDG_CACHE_HOME".into(),
                home.join(".cache").to_string_lossy().into_owned(),
            ),
        ],
        commands: false,
    }
}

#[tokio::test]
async fn deploys_once_prunes_old_versions_and_serves() {
    let (home, tools, project) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    std::fs::write(project.path().join("a.txt"), "hi\n").unwrap();
    let stale = home.path().join(".cache/contextual/agent/0.0.1-old");
    std::fs::create_dir_all(&stale).unwrap();

    let host = SupervisedHost::new(Arc::new(launcher(home.path(), tools.path())));
    let root = project.path().to_string_lossy().into_owned();
    host.allow_root(&root).await.unwrap();
    let scan = host
        .scan(Scan {
            root: root.clone(),
            options: ScanOptions::default(),
        })
        .await
        .unwrap();
    assert_eq!(scan.files, 1);

    let binary = std::fs::read(env!("CARGO_BIN_EXE_contextual-agent")).unwrap();
    let deployed = home.path().join(format!(
        ".cache/contextual/agent/{}/contextual-agent",
        agent_version("0.1.0", &binary)
    ));
    assert!(deployed.is_file(), "agent copied into the distro");
    assert!(!stale.exists(), "old versions pruned");

    // A second launch finds it installed and doesn't copy again.
    let before = std::fs::metadata(&deployed).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let again = launcher(home.path(), tools.path()).launch().await.unwrap();
    assert!(again.is_alive());
    assert_eq!(
        std::fs::metadata(&deployed).unwrap().modified().unwrap(),
        before
    );
}

#[tokio::test]
async fn missing_architecture_is_a_clear_error() {
    let (home, tools) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut l = launcher(home.path(), tools.path());
    l.binaries = vec![(
        "riscv64".into(),
        env!("CARGO_BIN_EXE_contextual-agent").into(),
    )];
    let err = l.launch().await.unwrap_err();
    assert!(err.message.contains("No agent for Test"), "{}", err.message);
}
