//! Native-path allowlist checks for non-executing command preparation.

use contextual_core::host::RootAllowlist;

#[test]
fn command_roots_must_be_open_existing_native_directories() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    let allowlist = RootAllowlist::default();
    allowlist.allow(project.path().to_str().unwrap());

    assert!(
        allowlist
            .check_command_root(project.path().to_str().unwrap())
            .is_ok()
    );
    assert!(
        allowlist
            .check_command_root(project.path().join("src").to_str().unwrap(),)
            .is_ok()
    );
    assert!(
        allowlist
            .check_command_root(outside.path().to_str().unwrap())
            .is_err()
    );
    assert!(allowlist.check_command_root("relative").is_err());
    assert!(allowlist.check_command_root("~").is_err());
}

#[cfg(unix)]
#[test]
fn a_symlink_cannot_turn_an_allowed_child_into_an_outside_command_root() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), project.path().join("escape")).unwrap();
    let allowlist = RootAllowlist::default();
    allowlist.allow(project.path().to_str().unwrap());

    assert!(
        allowlist
            .check_command_root(project.path().join("escape").to_str().unwrap(),)
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn windows_paths_are_not_translated_by_command_preparation() {
    let allowlist = RootAllowlist::default();
    allowlist.allow("/mnt/c");
    assert!(allowlist.check_command_root(r"C:\project").is_err());
    assert!(
        allowlist
            .check_command_root(r"\\wsl$\Ubuntu\home\project")
            .is_err()
    );
}
