//! Git / PR integration tests on temporary repositories with real `git`; `gh` is faked through
//! the `CommandRunner` (the fake answers like `fixtures/git/gh`).
//!
//! `parity_with_node_server` replays the sequence the old TypeScript server was recorded with
//! (`fixtures/git/expected.json`; see `fixtures/README.md`) and compares the JSON, ignoring
//! hashes, relative dates and the temp dir.
#![cfg(unix)]

use async_trait::async_trait;
use contextual_core::process::{Cmd, CommandRunner, PathPolicy, RunOutput, SystemRunner};
use contextual_core::workspace::git::Git;
use contextual_protocol::git::PrRequest;
use pretty_assertions::assert_eq;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/git")
}

#[derive(Clone, Copy, PartialEq)]
enum GhMode {
    /// Runs `fixtures/git/gh`.
    Script,
    /// `gh` is not installed.
    Missing,
}

/// Real git (isolated from the user's config); `gh` replaced by the fixture script.
struct FakeGh {
    inner: SystemRunner,
    mode: GhMode,
    calls: Mutex<Vec<Vec<String>>>,
}

impl FakeGh {
    fn new(mode: GhMode) -> Arc<Self> {
        let inner = SystemRunner::new(PathPolicy::Inherit)
            .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
            .with_env("GIT_CONFIG_NOSYSTEM", "1");
        Arc::new(Self {
            inner,
            mode,
            calls: Mutex::new(Vec::new()),
        })
    }
    fn gh_calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl CommandRunner for FakeGh {
    async fn run(&self, mut cmd: Cmd) -> RunOutput {
        if cmd.program == "gh" {
            self.calls.lock().unwrap().push(cmd.args.clone());
            if self.mode == GhMode::Missing {
                return RunOutput::failure(127, "gh is not installed");
            }
            cmd.program = fixtures().join("gh").to_string_lossy().into_owned();
        }
        self.inner.run(cmd).await
    }
}

/// Builds the scripted repo; returns (temp dir guard, canonical tmp path, work dir).
fn scripted_repo() -> (tempfile::TempDir, String, String) {
    let dir = tempfile::Builder::new()
        .prefix("ctx-git-")
        .tempdir()
        .unwrap();
    let tmp = std::fs::canonicalize(dir.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let status = std::process::Command::new("sh")
        .arg(fixtures().join("make-repo.sh"))
        .arg(&tmp)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "make-repo.sh failed");
    let work = format!("{tmp}/work");
    (dir, tmp, work)
}

fn git_cmd(root: &str, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Replaces hashes, relative dates and the temp dir so runs can be compared.
fn normalize(v: Value, tmp: &str) -> Value {
    let index = Regex::new(r"index [0-9a-f]+\.\.[0-9a-f]+").unwrap();
    fn walk(v: Value, key: Option<&str>, tmp: &str, index: &Regex) -> Value {
        match v {
            Value::String(s) => match key {
                Some("sha" | "commit" | "when") => Value::String(format!("<{}>", key.unwrap())),
                _ => Value::String(
                    index
                        .replace_all(&s.replace(tmp, "<tmp>"), "index <idx>")
                        .into_owned(),
                ),
            },
            Value::Array(a) => {
                Value::Array(a.into_iter().map(|x| walk(x, None, tmp, index)).collect())
            }
            Value::Object(o) => Value::Object(
                o.into_iter()
                    .map(|(k, x)| (k.clone(), walk(x, Some(&k), tmp, index)))
                    .collect(),
            ),
            other => other,
        }
    }
    walk(v, None, tmp, &index)
}

fn val<T: serde::Serialize>(r: contextual_core::Result<T>) -> Value {
    serde_json::to_value(r.unwrap()).unwrap()
}

fn err<T: std::fmt::Debug>(r: contextual_core::Result<T>) -> Value {
    Value::String(r.unwrap_err().message)
}

#[tokio::test]
async fn parity_with_node_server() {
    let (_guard, tmp, root) = scripted_repo();
    let runner = FakeGh::new(GhMode::Script);
    let git = Git::new(runner.clone());
    let r = &root;
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();

    let mut out = Map::new();
    out.insert("status".into(), val(git.status(r).await));
    out.insert("branches".into(), val(git.list_branches(r, false).await));
    out.insert(
        "compareDefault".into(),
        val(git.compare(r, None, None).await),
    );
    out.insert(
        "compareChild".into(),
        val(git.compare(r, None, Some("feature/child")).await),
    );
    out.insert(
        "compareMainChild".into(),
        val(git.compare(r, Some("main"), Some("feature/child")).await),
    );
    out.insert(
        "compareMissing".into(),
        val(git.compare(r, Some("main"), Some("nope")).await),
    );
    out.insert(
        "diffStaged".into(),
        val(git.file_diff(r, "README.md", true).await),
    );
    out.insert(
        "diffUnstaged".into(),
        val(git.file_diff(r, "src/app.ts", false).await),
    );
    out.insert(
        "diffDeleted".into(),
        val(git.file_diff(r, "crlf.txt", false).await),
    );
    out.insert(
        "diffRenamed".into(),
        val(git.file_diff(r, "renamed-again.txt", true).await),
    );
    out.insert("commitDiff".into(), val(git.commit_diff(r).await));
    out.insert("prDiff".into(), val(git.pr_diff(r, "main", None).await));
    out.insert("errNotRepo".into(), err(git.status(&tmp).await));
    out.insert(
        "errEscape".into(),
        err(git.file_diff(r, "../x", false).await),
    );
    out.insert(
        "errEmptyMessage".into(),
        err(git.commit(r, "  ", false).await),
    );
    out.insert(
        "errBadBranch".into(),
        err(git.create_branch(r, "bad..name", None).await),
    );

    git.stage(r, &s(&["src/app.ts", "crlf.txt", "notes/todo.md"]))
        .await
        .unwrap();
    out.insert("afterStage".into(), val(git.status(r).await));
    git.unstage(r, &s(&["README.md"])).await.unwrap();
    out.insert("afterUnstage".into(), val(git.status(r).await));
    out.insert(
        "commit".into(),
        val(git.commit(r, "  Work in progress  ", false).await),
    );
    out.insert(
        "createBranch".into(),
        val(git.create_branch(r, "feature/new", None).await),
    );
    out.insert("push".into(), val(git.push(r).await));
    out.insert(
        "commitAll".into(),
        val(git.commit(r, "Everything else", true).await),
    );
    out.insert(
        "errNothingToCommit".into(),
        err(git.commit(r, "x", false).await),
    );
    out.insert(
        "switchRemote".into(),
        val(git.switch_branch(r, "origin/feature/login").await),
    );
    out.insert(
        "switchBack".into(),
        val(git.switch_branch(r, "feature/new").await),
    );
    out.insert("compareNew".into(), val(git.compare(r, None, None).await));
    let pr = PrRequest {
        title: " New thing ".into(),
        body: String::new(),
        base: "feature/login".into(),
        draft: true,
        ..PrRequest::default()
    };
    out.insert(
        "createPr".into(),
        val(git.create_pull_request(r, &pr).await),
    );
    let same = PrRequest {
        title: "x".into(),
        base: "feature/new".into(),
        ..PrRequest::default()
    };
    out.insert(
        "errSameBranch".into(),
        err(git.create_pull_request(r, &same).await),
    );

    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures().join("expected.json")).unwrap())
            .unwrap();
    let expected_tmp = expected["tmp"].as_str().unwrap().to_string();
    let expected = normalize(expected["results"].clone(), &expected_tmp);
    let actual = normalize(Value::Object(out), &tmp);
    for (key, want) in expected.as_object().unwrap() {
        assert_eq!(&actual[key], want, "{key}");
    }
    assert_eq!(
        actual.as_object().unwrap().len(),
        expected.as_object().unwrap().len()
    );

    // The PR was created with the exact gh arguments.
    let create = runner
        .gh_calls()
        .into_iter()
        .find(|c| c[..2] == ["pr", "create"])
        .unwrap();
    assert_eq!(
        create,
        s(&[
            "pr",
            "create",
            "--base",
            "feature/login",
            "--head",
            "feature/new",
            "--title",
            "New thing",
            "--body",
            " ",
            "--draft"
        ])
    );
    // The parent of the new branch was recorded.
    assert_eq!(
        git_cmd(
            r,
            &["config", "--get", "branch.feature/new.contextualParent"]
        ),
        "feature/login"
    );
}

#[tokio::test]
async fn new_branch_from_default_branch_moves_default_back() {
    let (_guard, _tmp, root) = scripted_repo();
    let runner = FakeGh::new(GhMode::Script);
    let git = Git::new(runner.clone());
    git_cmd(&root, &["stash", "-u", "-q"]);
    git_cmd(&root, &["switch", "-q", "main"]);
    std::fs::write(format!("{root}/local.txt"), "local\n").unwrap();
    git_cmd(&root, &["add", "local.txt"]);
    git_cmd(&root, &["commit", "-q", "-m", "Local work on main"]);
    let origin_main = git_cmd(&root, &["rev-parse", "origin/main"]);
    let local_tip = git_cmd(&root, &["rev-parse", "HEAD"]);

    // Without a new branch the default branch is refused.
    let req = PrRequest {
        title: "T".into(),
        base: "feature/login".into(),
        ..PrRequest::default()
    };
    let e = git.create_pull_request(&root, &req).await.unwrap_err();
    assert_eq!(
        e.message,
        "main is the default branch. Create a new branch for the pull request."
    );

    let req = PrRequest {
        new_branch: Some("feature/from-main".into()),
        ..req
    };
    let res = git.create_pull_request(&root, &req).await.unwrap();
    assert_eq!(res.branch, "feature/from-main");
    assert_eq!(res.number, Some(8));
    assert!(!res.updated_existing);
    assert_eq!(
        git_cmd(&root, &["branch", "--show-current"]),
        "feature/from-main"
    );
    assert_eq!(git_cmd(&root, &["rev-parse", "HEAD"]), local_tip);
    // main is back at origin/main: the commit only lives on the new branch now.
    assert_eq!(git_cmd(&root, &["rev-parse", "main"]), origin_main);
    assert_eq!(
        git_cmd(&root, &["rev-parse", "origin/feature/from-main"]),
        local_tip
    );
    assert_eq!(
        git_cmd(
            &root,
            &[
                "config",
                "--get",
                "branch.feature/from-main.contextualParent"
            ]
        ),
        "feature/login"
    );
    let create = runner
        .gh_calls()
        .into_iter()
        .find(|c| c[..2] == ["pr", "create"])
        .unwrap();
    assert_eq!(
        create[3..6],
        ["feature/login", "--head", "feature/from-main"]
    );
}

#[tokio::test]
async fn existing_pr_is_only_pushed() {
    let (_guard, _tmp, root) = scripted_repo();
    let runner = FakeGh::new(GhMode::Script);
    let git = Git::new(runner.clone());
    let req = PrRequest {
        title: "T".into(),
        base: "main".into(),
        ..PrRequest::default()
    };
    let res = git.create_pull_request(&root, &req).await.unwrap();
    assert_eq!(
        serde_json::to_value(&res).unwrap(),
        json!({"url":"https://github.com/acme/demo/pull/7","number":7,"branch":"feature/login","updatedExisting":true})
    );
    assert!(runner.gh_calls().iter().all(|c| c[..2] != ["pr", "create"]));
    // The unpushed commit went to origin.
    assert_eq!(
        git_cmd(&root, &["rev-parse", "origin/feature/login"]),
        git_cmd(&root, &["rev-parse", "HEAD"])
    );
}

#[tokio::test]
async fn gh_missing_is_reported() {
    let (_guard, _tmp, root) = scripted_repo();
    let git = Git::new(FakeGh::new(GhMode::Missing));
    let c = git.compare(&root, None, None).await.unwrap();
    assert!(!c.github);
    assert_eq!(c.default_branch.as_deref(), Some("main"));
    assert_eq!(
        c.problem.as_deref(),
        Some("The GitHub CLI (gh) is not installed. Install it and run \"gh auth login\".")
    );
    assert_eq!(c.existing_pr, None);
    let req = PrRequest {
        title: "T".into(),
        base: "main".into(),
        ..PrRequest::default()
    };
    let e = git.create_pull_request(&root, &req).await.unwrap_err();
    assert_eq!(e.message, c.problem.unwrap());
    let e = git
        .create_pull_request(
            &root,
            &PrRequest {
                title: " ".into(),
                ..req
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.message, "The pull request needs a title.");
}

#[tokio::test]
async fn repo_without_origin_or_commits() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    git_cmd(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(format!("{root}/a.txt"), "a\n").unwrap();
    let git = Git::new(FakeGh::new(GhMode::Script));
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();

    // Stage and unstage before the first commit (`git rm --cached`).
    git.stage(&root, &s(&["a.txt", "gone.txt"])).await.unwrap();
    let st = git.status(&root).await.unwrap();
    assert_eq!(
        serde_json::to_value(&st.staged).unwrap(),
        json!([{"path":"a.txt","letter":"A"}])
    );
    git.unstage(&root, &s(&["a.txt"])).await.unwrap();
    let st = git.status(&root).await.unwrap();
    assert!(st.staged.is_empty());
    assert_eq!(st.branch.as_deref(), Some("main"));
    assert_eq!(st.upstream, None);

    let c = git.compare(&root, None, None).await.unwrap();
    assert_eq!(c.remote, None);
    assert_eq!(
        c.problem.as_deref(),
        Some("The repository has no \"origin\" remote.")
    );
    assert_eq!(c.base, "main");

    let e = git.push(&root).await.unwrap_err();
    assert!(
        e.message.starts_with("Publishing the branch failed: "),
        "{}",
        e.message
    );
}

#[tokio::test]
async fn git_actions_do_not_fall_back_to_an_ancestor_repository() {
    let (_guard, _tmp, root) = scripted_repo();
    let git = Git::new(FakeGh::new(GhMode::Script));
    let before = git_cmd(&root, &["rev-parse", "HEAD"]);
    let error = git
        .commit(&format!("{root}/src"), "Must not commit the parent", true)
        .await
        .unwrap_err();
    assert!(error.message.contains("Open its repository root"));
    assert_eq!(git_cmd(&root, &["rev-parse", "HEAD"]), before);
}

#[tokio::test]
async fn stage_rejects_paths_outside_the_project() {
    let (_guard, _tmp, root) = scripted_repo();
    let git = Git::new(FakeGh::new(GhMode::Script));
    let e = git
        .stage(&root, &["../outside".to_string()])
        .await
        .unwrap_err();
    assert_eq!(e.message, "Path escapes the project: ../outside");
    let e = git
        .unstage(&root, &["/etc/passwd".to_string()])
        .await
        .unwrap_err();
    assert_eq!(e.message, "Invalid project path: /etc/passwd");
}
