//! Git operations for a VS Code-style Source Control view: status, stage, unstage, file diff,
//! commit (incl. smart commit), push, branches (list, switch, create with recorded parent).
//! Pull requests are in [`super::pr`].
//!
//! Commands run through a [`CommandRunner`] with fixed argument vectors (no shell) in the
//! project folder and never prompt for credentials. Messages match the old `git.ts`.

use crate::process::{Cmd, CommandRunner, RunOutput, SystemRunner};
use crate::workspace::paths::{normalize_input_path, path_string, resolve_inside};
use crate::{Error, Result};
use contextual_protocol::git::{
    BranchInfo, BranchList, ChangeLetter, CommitResult, FileDiff, GitStatus, PushResult,
    StatusEntry,
};
use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Larger files are listed but not shown as diffs (in UTF-16 units, like the old server).
pub(crate) const MAX_DIFF_FILE: usize = 400 * 1024;
pub(crate) const PUSH_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Git and GitHub CLI operations, run through a pluggable [`CommandRunner`].
#[derive(Clone)]
pub struct Git {
    runner: Arc<dyn CommandRunner>,
}

impl Default for Git {
    fn default() -> Self {
        Self::new(Arc::new(SystemRunner::default()))
    }
}

impl std::fmt::Debug for Git {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Git").finish_non_exhaustive()
    }
}

/// `git config` key holding the branch a Contextual-created branch came from.
pub(crate) fn parent_key(branch: &str) -> String {
    format!("branch.{branch}.contextualParent")
}

pub(crate) fn lf(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Length in UTF-16 code units (what JavaScript's `.length` reports).
pub(crate) fn js_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The first `n` UTF-16 code units of `s` (never splitting a character).
pub(crate) fn js_truncate(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

pub(crate) fn skip_reason(texts: &[Option<&str>]) -> Option<String> {
    if texts.iter().flatten().any(|t| js_len(t) > MAX_DIFF_FILE) {
        Some("too large to show".into())
    } else if texts.iter().flatten().any(|t| t.contains('\0')) {
        Some("binary".into())
    } else {
        None
    }
}

fn letter_of(c: char) -> ChangeLetter {
    match c {
        'A' => ChangeLetter::A,
        'D' => ChangeLetter::D,
        'R' | 'C' => ChangeLetter::R,
        _ => ChangeLetter::M,
    }
}

/// ICU root collation order of ASCII punctuation and symbols (what Node's `localeCompare` uses).
const PUNCT_ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// Primary weight of a character: whitespace/punctuation/symbols, then digits, then letters
/// (case-insensitive), then everything else by code point.
fn primary(c: char) -> u32 {
    if let Some(i) = PUNCT_ORDER.find(c) {
        return i as u32;
    }
    if c.is_whitespace() {
        return 0;
    }
    if c.is_ascii_digit() {
        return 100 + (c as u32 - '0' as u32);
    }
    if c.is_ascii_alphabetic() {
        return 200 + (c.to_ascii_lowercase() as u32 - 'a' as u32);
    }
    let lower = c.to_lowercase().next().unwrap_or(c);
    1000 + lower as u32
}

/// Approximates `a.localeCompare(b)` (ICU root collation) for file paths: punctuation before
/// digits before letters, case-insensitive first, then lowercase before uppercase.
pub(crate) fn locale_cmp(a: &str, b: &str) -> Ordering {
    let pa = a.chars().map(primary);
    let pb = b.chars().map(primary);
    pa.cmp(pb)
        .then_with(|| {
            let ta = a.chars().map(|c| c.is_uppercase() as u8);
            let tb = b.chars().map(|c| c.is_uppercase() as u8);
            ta.cmp(tb)
        })
        .then_with(|| a.cmp(b))
}

pub(crate) fn parse_status(out: &str) -> GitStatus {
    let mut st = GitStatus::default();
    let parts: Vec<&str> = out.split('\0').collect();
    let mut i = 0;
    while i < parts.len() {
        let line = parts[i];
        i += 1;
        if line.is_empty() {
            continue;
        }
        if let Some(head) = line.strip_prefix("# branch.head ") {
            st.branch = (head != "(detached)").then(|| head.to_string());
        } else if let Some(up) = line.strip_prefix("# branch.upstream ") {
            st.upstream = Some(up.to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            let mut it = ab.split(' ');
            let num = |s: Option<&str>| {
                s.and_then(|s| s.parse::<i64>().ok())
                    .map(i64::unsigned_abs)
                    .unwrap_or(0)
            };
            st.ahead = num(it.next());
            st.behind = num(it.next());
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            let f: Vec<&str> = line.split(' ').collect();
            let xy: Vec<char> = f.get(1).copied().unwrap_or("..").chars().collect();
            let renamed = line.starts_with("2 ");
            let skip = if renamed { 9 } else { 8 };
            let path = f.get(skip..).map(|p| p.join(" ")).unwrap_or_default();
            let from = if renamed {
                let from = parts.get(i).map(|s| s.to_string());
                i += 1;
                from
            } else {
                None
            };
            let (x, y) = (
                xy.first().copied().unwrap_or('.'),
                xy.get(1).copied().unwrap_or('.'),
            );
            if x != '.' {
                st.staged.push(StatusEntry {
                    path: path.clone(),
                    from,
                    letter: letter_of(x),
                });
            }
            if y != '.' {
                st.changes.push(StatusEntry {
                    path,
                    from: None,
                    letter: letter_of(y),
                });
            }
        } else if let Some(path) = line.strip_prefix("? ") {
            st.changes.push(StatusEntry {
                path: path.to_string(),
                from: None,
                letter: ChangeLetter::U,
            });
        } else if line.starts_with("u ") {
            let path = line.split(' ').skip(10).collect::<Vec<_>>().join(" ");
            st.conflicts.push(StatusEntry {
                path,
                from: None,
                letter: ChangeLetter::C,
            });
        }
    }
    st.staged.sort_by(|a, b| locale_cmp(&a.path, &b.path));
    st.changes.sort_by(|a, b| locale_cmp(&a.path, &b.path));
    st
}

pub(crate) fn parse_branches(out: &str, current: Option<String>) -> BranchList {
    let f: Vec<&str> = out.split('\0').collect();
    let mut list = BranchList {
        current,
        local: Vec::new(),
        remote: Vec::new(),
    };
    let mut i = 0;
    while i + 4 < f.len() {
        let r = f[i].strip_prefix('\n').unwrap_or(f[i]);
        let upstream = (!f[i + 1].is_empty()).then(|| f[i + 1].to_string());
        let info = |name: &str| BranchInfo {
            name: name.to_string(),
            upstream: upstream.clone(),
            sha: f[i + 2].to_string(),
            when: f[i + 3].to_string(),
            subject: f[i + 4].to_string(),
        };
        if let Some(name) = r.strip_prefix("refs/heads/") {
            list.local.push(info(name));
        } else if let Some(name) = r.strip_prefix("refs/remotes/") {
            if !r.ends_with("/HEAD") {
                list.remote.push(info(name));
            }
        }
        i += 5;
    }
    list
}

impl Git {
    pub fn new(runner: Arc<dyn CommandRunner>) -> Self {
        Self { runner }
    }

    pub fn runner(&self) -> &Arc<dyn CommandRunner> {
        &self.runner
    }

    pub(crate) async fn run_with(
        &self,
        program: &str,
        args: &[&str],
        cwd: &str,
        timeout: Duration,
    ) -> RunOutput {
        self.runner
            .run(
                Cmd::new(program, args.iter().copied())
                    .cwd(cwd)
                    .timeout(timeout),
            )
            .await
    }

    pub(crate) async fn git(&self, args: &[&str], cwd: &str) -> RunOutput {
        self.run_with("git", args, cwd, crate::process::DEFAULT_TIMEOUT)
            .await
    }

    pub(crate) async fn git_ok(&self, args: &[&str], cwd: &str) -> bool {
        self.git(args, cwd).await.ok
    }

    /// Runs a command that must succeed; returns its trimmed stdout.
    pub(crate) async fn must_with(
        &self,
        program: &str,
        args: &[&str],
        cwd: &str,
        what: &str,
        timeout: Duration,
    ) -> Result<String> {
        let r = self.run_with(program, args, cwd, timeout).await;
        if !r.ok {
            let reason = r.stderr.trim();
            let reason = if reason.is_empty() {
                "unknown error"
            } else {
                reason
            };
            return Err(Error::tool(format!("{what} failed: {reason}")));
        }
        Ok(r.stdout.trim().to_string())
    }

    pub(crate) async fn must(&self, args: &[&str], cwd: &str, what: &str) -> Result<String> {
        self.must_with("git", args, cwd, what, crate::process::DEFAULT_TIMEOUT)
            .await
    }

    /// Normalises the root and checks that it is inside a git work tree.
    pub(crate) async fn repo_root(&self, root_input: &str) -> Result<String> {
        let root = path_string(&normalize_input_path(Some(root_input)));
        if !self
            .git_ok(&["rev-parse", "--is-inside-work-tree"], &root)
            .await
        {
            return Err(Error::conflict("This folder is not a git repository."));
        }
        // Git otherwise walks upwards and can commit or switch a repository outside the
        // opened folder. Every caller must name an actual discovered repository root.
        let prefix = self
            .must(
                &["rev-parse", "--show-prefix"],
                &root,
                "Reading repository root",
            )
            .await?;
        if !prefix.is_empty() {
            return Err(Error::conflict(
                "This folder is inside a git repository. Open its repository root to use Source Control.",
            ));
        }
        Ok(root)
    }

    pub(crate) async fn show(&self, root: &str, spec: &str) -> Option<String> {
        let r = self.git(&["show", spec], root).await;
        r.ok.then_some(r.stdout)
    }

    /// Unified diff text for drafting messages, capped for the model.
    pub(crate) async fn diff_text(&self, root: &str, args: &[&str]) -> String {
        const MAX_CHARS: usize = 60_000;
        let mut full = vec!["diff", "--no-color"];
        full.extend_from_slice(args);
        let d = self.git(&full, root).await.stdout;
        if js_len(&d) > MAX_CHARS {
            format!("{}\n… (diff truncated)", js_truncate(&d, MAX_CHARS))
        } else {
            d
        }
    }

    pub(crate) async fn status_at(&self, root: &str) -> Result<GitStatus> {
        let out = self
            .must(
                &[
                    "status",
                    "--porcelain=v2",
                    "--branch",
                    "-z",
                    "--untracked-files=all",
                ],
                root,
                "git status",
            )
            .await?;
        Ok(parse_status(&out))
    }

    // ------------------------------------------------------------------ status

    /// Branch, upstream, ahead/behind and staged / changes / conflicts with VS Code letters.
    pub async fn status(&self, root_input: &str) -> Result<GitStatus> {
        let root = self.repo_root(root_input).await?;
        self.status_at(&root).await
    }

    /// Stages edits, new files and deletions of these project-relative paths.
    pub async fn stage(&self, root_input: &str, paths: &[String]) -> Result<()> {
        let root = self.repo_root(root_input).await?;
        for p in paths {
            resolve_inside(std::path::Path::new(&root), p)?;
        }
        // `git add -A` can't name a path that is gone from both the worktree and the index, so
        // those are skipped (already staged as deleted).
        let mut args = vec![
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
        ];
        args.extend(paths.iter().map(String::as_str));
        let known = self.git(&args, &root).await;
        let present: std::collections::HashSet<&str> =
            known.stdout.split('\0').filter(|s| !s.is_empty()).collect();
        let addable: Vec<&str> = paths
            .iter()
            .map(String::as_str)
            .filter(|p| present.contains(p))
            .collect();
        if !addable.is_empty() {
            let mut args = vec!["add", "-A", "--"];
            args.extend(addable);
            self.must(&args, &root, "Staging").await?;
        }
        Ok(())
    }

    pub async fn unstage(&self, root_input: &str, paths: &[String]) -> Result<()> {
        let root = self.repo_root(root_input).await?;
        for p in paths {
            resolve_inside(std::path::Path::new(&root), p)?;
        }
        let has_head = self
            .git_ok(&["rev-parse", "--verify", "-q", "HEAD"], &root)
            .await;
        let mut args: Vec<&str> = if has_head {
            vec!["restore", "--staged", "--"]
        } else {
            vec!["rm", "-q", "--cached", "-r", "--ignore-unmatch", "--"]
        };
        args.extend(paths.iter().map(String::as_str));
        self.must(&args, &root, "Unstaging").await?;
        Ok(())
    }

    /// Staged: HEAD → index. Unstaged: index → working tree (like VS Code's two diff views).
    pub async fn file_diff(&self, root_input: &str, path: &str, staged: bool) -> Result<FileDiff> {
        let root = self.repo_root(root_input).await?;
        let abs = resolve_inside(std::path::Path::new(&root), path)?;
        let (before, after) = if staged {
            (
                self.show(&root, &format!("HEAD:{path}")).await,
                self.show(&root, &format!(":{path}")).await,
            )
        } else {
            let before = match self.show(&root, &format!(":{path}")).await {
                Some(b) => Some(b),
                None => self.show(&root, &format!("HEAD:{path}")).await,
            };
            let after = tokio::fs::read(&abs)
                .await
                .ok()
                .map(|b| String::from_utf8_lossy(&b).into_owned());
            (before, after)
        };
        let skipped = skip_reason(&[before.as_deref(), after.as_deref()]);
        let text = |t: Option<String>| {
            if skipped.is_some() {
                String::new()
            } else {
                lf(&t.unwrap_or_default())
            }
        };
        Ok(FileDiff {
            path: path.to_string(),
            old: text(before),
            new: text(after),
            skipped,
        })
    }

    /// The staged diff (or every change, when nothing is staged), for a model-written commit message.
    pub async fn commit_diff(&self, root_input: &str) -> Result<String> {
        let root = self.repo_root(root_input).await?;
        let staged = self.diff_text(&root, &["--cached"]).await;
        if !staged.trim().is_empty() {
            return Ok(staged);
        }
        let st = self.status_at(&root).await?;
        let mut parts = vec![self.diff_text(&root, &["HEAD"]).await];
        parts.extend(
            st.changes
                .iter()
                .filter(|c| c.letter == ChangeLetter::U)
                .map(|c| format!("new file: {}", c.path)),
        );
        Ok(parts.join("\n"))
    }

    /// Commits the staged changes; with `all`, stages everything first (VS Code's smart commit).
    pub async fn commit(&self, root_input: &str, message: &str, all: bool) -> Result<CommitResult> {
        let root = self.repo_root(root_input).await?;
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::invalid("Write a commit message first."));
        }
        if all {
            self.must(&["add", "-A"], &root, "Staging all changes")
                .await?;
        }
        let names = self
            .git(&["diff", "--cached", "--name-only", "-z"], &root)
            .await
            .stdout;
        let files = names.split('\0').filter(|s| !s.is_empty()).count() as u64;
        if files == 0 {
            return Err(Error::conflict("There are no staged changes to commit."));
        }
        self.must(&["commit", "-m", message], &root, "Commit")
            .await?;
        let commit = self
            .must(
                &["rev-parse", "--short", "HEAD"],
                &root,
                "Reading the commit",
            )
            .await?;
        Ok(CommitResult { commit, files })
    }

    /// Pushes; publishes the branch with `-u origin HEAD` when it has no upstream.
    pub async fn push(&self, root_input: &str) -> Result<PushResult> {
        let root = self.repo_root(root_input).await?;
        let st = self.status_at(&root).await?;
        let Some(branch) = st.branch else {
            return Err(Error::conflict("Not on a branch (detached HEAD)."));
        };
        if st.upstream.is_some() {
            self.must_with("git", &["push"], &root, "Push", PUSH_TIMEOUT)
                .await?;
        } else {
            self.must_with(
                "git",
                &["push", "-u", "origin", "HEAD"],
                &root,
                "Publishing the branch",
                PUSH_TIMEOUT,
            )
            .await?;
        }
        let after = self.status_at(&root).await?;
        let upstream = after.upstream.unwrap_or_else(|| format!("origin/{branch}"));
        Ok(PushResult { branch, upstream })
    }

    // ------------------------------------------------------------------ branches

    pub(crate) async fn branches_at(&self, root: &str, fetch: bool) -> Result<BranchList> {
        if fetch {
            self.run_with(
                "git",
                &["fetch", "--prune", "--quiet", "origin"],
                root,
                FETCH_TIMEOUT,
            )
            .await;
        }
        let out = self
            .must(
                &[
                    "for-each-ref",
                    "--sort=-committerdate",
                    "--format=%(refname)%00%(upstream:short)%00%(objectname:short)%00%(committerdate:relative)%00%(contents:subject)%00",
                    "refs/heads",
                    "refs/remotes/origin",
                ],
                root,
                "Listing branches",
            )
            .await?;
        let current = self.status_at(root).await?.branch;
        Ok(parse_branches(&out, current))
    }

    /// Local and origin branches, most recently committed first. `fetch` refreshes the remote ones first.
    pub async fn list_branches(&self, root_input: &str, fetch: bool) -> Result<BranchList> {
        let root = self.repo_root(root_input).await?;
        self.branches_at(&root, fetch).await
    }

    /// Deletes an exact local branch name using Git's merged-branch safety check.
    ///
    /// Never deletes remote refs, forces deletion, or switches the working tree.
    /// Git also refuses branches checked out in this or another worktree.
    pub async fn delete_branch(&self, root_input: &str, name: &str) -> Result<()> {
        let root = self.repo_root(root_input).await?;
        if name.is_empty() || name.starts_with('-') {
            return Err(Error::invalid("Choose a valid local branch name."));
        }
        // Validate a literal ref, not --branch shorthand such as @{-1}.
        let reference = format!("refs/heads/{name}");
        self.must(
            &["check-ref-format", &reference],
            &root,
            "Branch name check",
        )
        .await?;
        self.must(
            &["show-ref", "--verify", "--", &reference],
            &root,
            "Finding the local branch",
        )
        .await?;
        if self.status_at(&root).await?.branch.as_deref() == Some(name) {
            return Err(Error::conflict(
                "Switch to another branch before deleting the current branch.",
            ));
        }
        self.must(
            &["branch", "-d", "--", name],
            &root,
            &format!("Deleting local branch {name}"),
        )
        .await?;
        Ok(())
    }

    /// Switches to a branch. A remote branch without a local copy is checked out as a new tracking branch.
    pub async fn switch_branch(&self, root_input: &str, name: &str) -> Result<GitStatus> {
        let root = self.repo_root(root_input).await?;
        if let Some(local) = name.strip_prefix("origin/") {
            let exists = self
                .git_ok(
                    &[
                        "rev-parse",
                        "--verify",
                        "-q",
                        &format!("refs/heads/{local}"),
                    ],
                    &root,
                )
                .await;
            if exists {
                self.must(&["switch", local], &root, &format!("Switching to {local}"))
                    .await?;
            } else {
                self.must(
                    &["switch", "--track", name],
                    &root,
                    &format!("Checking out {name}"),
                )
                .await?;
            }
        } else {
            self.must(
                &["check-ref-format", "--branch", name],
                &root,
                "Branch name check",
            )
            .await?;
            self.must(&["switch", name], &root, &format!("Switching to {name}"))
                .await?;
        }
        self.status_at(&root).await
    }

    /// Creates a branch from `from` (default: the current branch), switches to it and records its parent.
    pub async fn create_branch(
        &self,
        root_input: &str,
        name: &str,
        from: Option<&str>,
    ) -> Result<GitStatus> {
        let root = self.repo_root(root_input).await?;
        self.must(
            &["check-ref-format", "--branch", name],
            &root,
            "Branch name check",
        )
        .await?;
        let from = from.filter(|f| !f.is_empty());
        let parent = match from {
            Some(f) => Some(f.to_string()),
            None => self.status_at(&root).await?.branch,
        };
        let mut args = vec!["switch", "-c", name];
        args.extend(from);
        self.must(&args, &root, &format!("Creating {name}")).await?;
        if let Some(parent) = parent {
            let parent = parent.strip_prefix("origin/").unwrap_or(&parent);
            self.git(&["config", &parent_key(name), parent], &root)
                .await;
        }
        self.status_at(&root).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_order_matches_node() {
        let mut t = vec![
            "a.txt", "A.txt", "a_b", "a-b", "ab", "a b", "B", "a1", "a10", "a2", "e", "f", "Zed",
            "zed", "src/x", "src-x", "src_x", "src.x",
        ];
        t.sort_by(|a, b| locale_cmp(a, b));
        // From `node -e` with localeCompare (ICU), minus non-ASCII entries.
        assert_eq!(
            t,
            vec![
                "a b", "a_b", "a-b", "a.txt", "A.txt", "a1", "a10", "a2", "ab", "B", "e", "f",
                "src_x", "src-x", "src.x", "src/x", "zed", "Zed"
            ]
        );
    }

    #[test]
    fn parses_porcelain_v2() {
        let out = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 M. N... 100644 100644 100644 a b src/a b.txt\0\
2 R. N... 100644 100644 100644 a b R100 new.txt\0old.txt\0\
1 .D N... 100644 100644 000000 a b gone.txt\0\
? new dir/x.txt\0\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\0";
        let st = parse_status(out);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!((st.ahead, st.behind), (2, 1));
        assert_eq!(st.staged.len(), 2);
        assert_eq!(st.staged[0].path, "new.txt");
        assert_eq!(st.staged[0].from.as_deref(), Some("old.txt"));
        assert_eq!(st.staged[0].letter, ChangeLetter::R);
        assert_eq!(st.staged[1].path, "src/a b.txt");
        assert_eq!(st.changes[0].path, "gone.txt");
        assert_eq!(st.changes[0].letter, ChangeLetter::D);
        assert_eq!(st.changes[1].letter, ChangeLetter::U);
        assert_eq!(st.conflicts[0].path, "conflict.txt");
    }

    #[test]
    fn truncates_like_js() {
        assert_eq!(js_truncate("a😀b", 2), "a");
        assert_eq!(js_truncate("a😀b", 3), "a😀");
        assert_eq!(js_len("😀"), 2);
    }
}
