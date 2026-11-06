//! Pull requests, like GitHub's compare view: commits and file diffs of
//! `merge-base(origin/base, head)..head`, base defaulting to head's parent, and `gh pr create`.
//! Messages match the old `git.ts`.

use super::git::{FETCH_TIMEOUT, Git, PUSH_TIMEOUT, lf, parent_key, skip_reason};
use crate::process::DEFAULT_TIMEOUT;
use crate::{Error, Result};
use contextual_protocol::git::{
    ChangeLetter, Commit, Comparison, ComparisonFile, ExistingPr, PrDiff, PrRequest, PrResult,
};
use std::time::Duration;

const GH_CREATE_TIMEOUT: Duration = Duration::from_secs(60);
/// Candidate branches looked at when guessing a parent.
const MAX_PARENT_CANDIDATES: usize = 40;

struct DefaultBranch {
    name: Option<String>,
    github: bool,
    problem: Option<String>,
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

impl Git {
    /// A local branch if it exists, else its origin copy.
    async fn ref_for(&self, root: &str, branch: &str) -> Option<String> {
        if self
            .git_ok(
                &[
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("refs/heads/{branch}"),
                ],
                root,
            )
            .await
        {
            return Some(branch.to_string());
        }
        if self
            .git_ok(
                &[
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("refs/remotes/origin/{branch}"),
                ],
                root,
            )
            .await
        {
            return Some(format!("origin/{branch}"));
        }
        None
    }

    /// Base for comparing: the origin copy when it exists (it is what the PR merges into), else the local branch.
    async fn base_ref_for(&self, root: &str, branch: &str) -> String {
        if self
            .git_ok(
                &[
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("refs/remotes/origin/{branch}"),
                ],
                root,
            )
            .await
        {
            format!("origin/{branch}")
        } else {
            branch.to_string()
        }
    }

    /// The branch `branch` was created from, like GitHub's suggested base: the parent recorded
    /// when Contextual created it, else the branch it forked from most recently (fewest commits
    /// since the fork point), else the default branch.
    async fn parent_of(
        &self,
        root: &str,
        branch: &str,
        default_branch: Option<&str>,
        names: &[String],
    ) -> Option<String> {
        let recorded = self
            .git(&["config", "--get", &parent_key(branch)], root)
            .await
            .stdout
            .trim()
            .to_string();
        if !recorded.is_empty()
            && recorded != branch
            && self.ref_for(root, &recorded).await.is_some()
        {
            return Some(recorded);
        }
        let Some(head) = self.ref_for(root, branch).await else {
            return default_branch.map(str::to_string);
        };
        let tip = self
            .git(&["rev-parse", &head], root)
            .await
            .stdout
            .trim()
            .to_string();
        let mut best: Option<(String, u64)> = None;
        for name in names
            .iter()
            .filter(|n| *n != branch)
            .take(MAX_PARENT_CANDIDATES)
        {
            let Some(r) = self.ref_for(root, name).await else {
                continue;
            };
            let mb = self
                .git(&["merge-base", &r, &head], root)
                .await
                .stdout
                .trim()
                .to_string();
            // No shared history, or this branch is fully contained in the candidate (a child or a copy).
            if mb.is_empty() || mb == tip {
                continue;
            }
            let count = self
                .git(&["rev-list", "--count", &format!("{mb}..{head}")], root)
                .await;
            let Ok(ahead) = count.stdout.trim().parse::<u64>() else {
                continue;
            };
            let better = match &best {
                None => true,
                Some((_, b)) => {
                    ahead < *b || (ahead == *b && Some(name.as_str()) == default_branch)
                }
            };
            if better {
                best = Some((name.clone(), ahead));
            }
        }
        best.map(|b| b.0)
            .or_else(|| default_branch.map(str::to_string))
    }

    async fn default_branch_of(&self, root: &str) -> DefaultBranch {
        let view = self
            .run_with(
                "gh",
                &[
                    "repo",
                    "view",
                    "--json",
                    "defaultBranchRef",
                    "-q",
                    ".defaultBranchRef.name",
                ],
                root,
                DEFAULT_TIMEOUT,
            )
            .await;
        if view.ok {
            let name = view.stdout.trim();
            return DefaultBranch {
                name: (!name.is_empty()).then(|| name.to_string()),
                github: true,
                problem: None,
            };
        }
        let head = self
            .git(
                &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
                root,
            )
            .await
            .stdout;
        let head = head.trim();
        let head = head.strip_prefix("origin/").unwrap_or(head);
        let problem = if view.stderr.contains("not installed") {
            "The GitHub CLI (gh) is not installed. Install it and run \"gh auth login\"."
                .to_string()
        } else {
            let first = view
                .stderr
                .trim()
                .split('\n')
                .next()
                .unwrap_or("")
                .to_string();
            format!("The GitHub CLI can't access this repository: {first}")
        };
        DefaultBranch {
            name: (!head.is_empty()).then(|| head.to_string()),
            github: false,
            problem: Some(problem),
        }
    }

    /// What a PR from `head` (default: the checked-out branch) into `base` (default: head's
    /// parent) would contain, like GitHub's compare view. Fetches origin quietly first.
    pub async fn compare(
        &self,
        root_input: &str,
        base: Option<&str>,
        head: Option<&str>,
    ) -> Result<Comparison> {
        let root = self.repo_root(root_input).await?;
        self.compare_at(&root, base, head).await
    }

    async fn compare_at(
        &self,
        root: &str,
        base_input: Option<&str>,
        head_input: Option<&str>,
    ) -> Result<Comparison> {
        let st = self.status_at(root).await?;
        let remote = self
            .git(&["remote", "get-url", "origin"], root)
            .await
            .stdout
            .trim()
            .to_string();
        let remote = (!remote.is_empty()).then_some(remote);
        if remote.is_some() {
            self.run_with("git", &["fetch", "--quiet", "origin"], root, FETCH_TIMEOUT)
                .await;
        }
        let def = if remote.is_some() {
            self.default_branch_of(root).await
        } else {
            DefaultBranch {
                name: None,
                github: false,
                problem: Some("The repository has no \"origin\" remote.".into()),
            }
        };

        let head = non_empty(head_input)
            .map(str::to_string)
            .or_else(|| st.branch.clone());
        let is_current = head.is_some() && head == st.branch;
        let head_ref = match &head {
            None => Some("HEAD".to_string()),
            Some(_) if is_current => Some("HEAD".to_string()),
            Some(h) => self.ref_for(root, h).await,
        };
        let branches = self.branches_at(root, false).await?;
        let mut names: Vec<String> = Vec::new();
        for n in branches
            .local
            .iter()
            .map(|b| b.name.clone())
            .chain(branches.remote.iter().map(|b| {
                b.name
                    .strip_prefix("origin/")
                    .unwrap_or(&b.name)
                    .to_string()
            }))
        {
            if !names.contains(&n) {
                names.push(n);
            }
        }
        let base = match non_empty(base_input) {
            Some(b) => b.to_string(),
            None => {
                let guessed = match &head {
                    Some(h) => self.parent_of(root, h, def.name.as_deref(), &names).await,
                    None => def.name.clone(),
                };
                guessed
                    .or_else(|| def.name.clone())
                    .unwrap_or_else(|| "main".to_string())
            }
        };

        let mut result = Comparison {
            branch: head.clone(),
            current: st.branch.clone(),
            default_branch: def.name.clone(),
            base: base.clone(),
            remote,
            github: def.github,
            problem: def.problem,
            commits: Vec::new(),
            files: Vec::new(),
            uncommitted: if is_current {
                (st.staged.len() + st.changes.len()) as u64
            } else {
                0
            },
            unpushed: 0,
            existing_pr: None,
        };
        let Some(head_ref) = head_ref else {
            let h = head.as_deref().unwrap_or("");
            result
                .problem
                .get_or_insert_with(|| format!("Branch {h} doesn't exist."));
            return Ok(result);
        };
        let base_ref = self.base_ref_for(root, &base).await;
        let merge_base = self
            .git(&["merge-base", &base_ref, &head_ref], root)
            .await
            .stdout
            .trim()
            .to_string();
        if merge_base.is_empty() {
            let h = head.as_deref().unwrap_or("null");
            result.problem.get_or_insert_with(|| {
                format!("Can't compare {h} with {base}: the branches have no common history.")
            });
            return Ok(result);
        }

        let log = self
            .git(
                &[
                    "log",
                    "--format=%h%x00%s%x00%an%x00%ar%x00",
                    &format!("{merge_base}..{head_ref}"),
                ],
                root,
            )
            .await;
        let fields: Vec<&str> = log.stdout.split('\0').collect();
        let mut i = 0;
        while i + 3 < fields.len() {
            result.commits.push(Commit {
                sha: fields[i]
                    .strip_prefix('\n')
                    .unwrap_or(fields[i])
                    .trim()
                    .to_string(),
                subject: fields[i + 1].to_string(),
                author: fields[i + 2].to_string(),
                when: fields[i + 3].to_string(),
            });
            i += 4;
        }

        if let Some(h) = &head {
            let on_origin = self
                .git_ok(
                    &[
                        "rev-parse",
                        "--verify",
                        "-q",
                        &format!("refs/remotes/origin/{h}"),
                    ],
                    root,
                )
                .await;
            result.unpushed = if on_origin {
                let count = self
                    .git(
                        &["rev-list", "--count", &format!("origin/{h}..{head_ref}")],
                        root,
                    )
                    .await;
                count.stdout.trim().parse().unwrap_or(0)
            } else {
                result.commits.len() as u64
            };
        }

        let out = self
            .git(
                &["diff", "--name-status", "-z", "-M", &merge_base, &head_ref],
                root,
            )
            .await
            .stdout;
        let names2: Vec<&str> = out.split('\0').collect();
        let mut i = 0;
        while i < names2.len() {
            let code = names2[i];
            i += 1;
            if code.is_empty() {
                continue;
            }
            let c0 = code.chars().next().unwrap_or('M');
            let letter = match c0 {
                'R' => ChangeLetter::R,
                'A' => ChangeLetter::A,
                'D' => ChangeLetter::D,
                _ => ChangeLetter::M,
            };
            let from = if c0 == 'R' || c0 == 'C' {
                i += 1;
                names2.get(i - 1).map(|s| s.to_string())
            } else {
                None
            };
            let path = names2.get(i).map(|s| s.to_string()).unwrap_or_default();
            i += 1;
            let before = if letter == ChangeLetter::A {
                Some(String::new())
            } else {
                self.show(
                    root,
                    &format!("{merge_base}:{}", from.as_deref().unwrap_or(&path)),
                )
                .await
            };
            let after = if letter == ChangeLetter::D {
                Some(String::new())
            } else {
                self.show(root, &format!("{head_ref}:{path}")).await
            };
            let skipped = skip_reason(&[before.as_deref(), after.as_deref()]);
            let text = |t: Option<String>| {
                if skipped.is_some() {
                    String::new()
                } else {
                    lf(&t.unwrap_or_default())
                }
            };
            result.files.push(ComparisonFile {
                path,
                letter,
                old: text(before),
                new: text(after),
                skipped,
            });
        }

        if let Some(h) = head
            .as_deref()
            .filter(|h| def.github && Some(*h) != def.name.as_deref())
        {
            let pr = self
                .run_with(
                    "gh",
                    &["pr", "view", h, "--json", "url,number,state,baseRefName"],
                    root,
                    DEFAULT_TIMEOUT,
                )
                .await;
            if pr.ok {
                if let Ok(j) = serde_json::from_str::<serde_json::Value>(&pr.stdout) {
                    if j["state"] == "OPEN" {
                        result.existing_pr = Some(ExistingPr {
                            url: j["url"].as_str().unwrap_or_default().to_string(),
                            number: j["number"].as_u64().unwrap_or_default(),
                        });
                    }
                }
            }
        }
        Ok(result)
    }

    /// Diff of base...head plus the commit list, for a model-written PR description.
    pub async fn pr_diff(
        &self,
        root_input: &str,
        base: &str,
        head: Option<&str>,
    ) -> Result<PrDiff> {
        let root = self.repo_root(root_input).await?;
        let c = self.compare_at(&root, Some(base), head).await?;
        let head_ref = match &c.branch {
            Some(b) if c.current.as_ref() != Some(b) => self
                .ref_for(&root, b)
                .await
                .unwrap_or_else(|| "HEAD".to_string()),
            _ => "HEAD".to_string(),
        };
        let base_ref = self.base_ref_for(&root, &c.base).await;
        let diff = self
            .diff_text(&root, &[&format!("{base_ref}...{head_ref}")])
            .await;
        let commits = c
            .commits
            .iter()
            .map(|x| format!("- {}", x.subject))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(PrDiff {
            diff,
            commits,
            branch: c.branch.unwrap_or_default(),
        })
    }

    /// Pushes the source branch (or a new one from HEAD) and opens a PR into `base` with `gh`,
    /// or only pushes when the branch already has an open PR.
    pub async fn create_pull_request(&self, root_input: &str, req: &PrRequest) -> Result<PrResult> {
        let root = self.repo_root(root_input).await?;
        let title = req.title.trim();
        if title.is_empty() {
            return Err(Error::invalid("The pull request needs a title."));
        }
        let c = self
            .compare_at(&root, Some(&req.base), req.head.as_deref())
            .await?;
        if !c.github {
            return Err(Error::tool(c.problem.clone().unwrap_or_else(|| {
                "GitHub is not available for this repository.".into()
            })));
        }
        let Some(mut branch) = c.branch.clone() else {
            return Err(Error::conflict("Not on a branch (detached HEAD)."));
        };
        if branch == c.base {
            return Err(Error::conflict("Choose different branches to compare."));
        }
        if c.commits.is_empty() {
            return Err(Error::conflict(format!(
                "There are no commits to compare between {} and {}.",
                c.base, branch
            )));
        }

        let is_current = Some(&branch) == c.current.as_ref();
        let new_branch = non_empty(req.new_branch.as_deref());
        if let Some(nb) = new_branch.filter(|_| is_current) {
            self.must(
                &["check-ref-format", "--branch", nb],
                &root,
                "Branch name check",
            )
            .await?;
            let from = branch.clone();
            self.must(
                &["switch", "-c", nb],
                &root,
                &format!("Creating branch {nb}"),
            )
            .await?;
            self.git(&["config", &parent_key(nb), &c.base], &root).await;
            branch = nb.to_string();
            // The commits now live on the new branch; put the default branch back where the remote
            // has it, as GitHub Desktop does, but only when that loses nothing (purely ahead of origin).
            if Some(&from) == c.default_branch.as_ref() {
                let behind = self
                    .git(
                        &["rev-list", "--count", &format!("{from}..origin/{from}")],
                        &root,
                    )
                    .await;
                if behind.stdout.trim() == "0" {
                    self.git(&["branch", "-f", &from, &format!("origin/{from}")], &root)
                        .await;
                }
            }
        } else if Some(&branch) == c.default_branch.as_ref() {
            return Err(Error::conflict(format!(
                "{branch} is the default branch. Create a new branch for the pull request."
            )));
        }

        // Push the source branch (a local branch that isn't checked out is pushed by name).
        let has_local = self
            .git_ok(
                &[
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("refs/heads/{branch}"),
                ],
                &root,
            )
            .await;
        if has_local {
            let target = if is_current || new_branch.is_some() {
                "HEAD"
            } else {
                branch.as_str()
            };
            self.must_with(
                "git",
                &["push", "-u", "origin", target],
                &root,
                "Push",
                PUSH_TIMEOUT,
            )
            .await?;
        }
        if let (Some(pr), None) = (&c.existing_pr, new_branch) {
            return Ok(PrResult {
                url: pr.url.clone(),
                number: Some(pr.number),
                branch,
                updated_existing: true,
            });
        }

        let body = req.body.trim();
        let body = if body.is_empty() { " " } else { body };
        let mut args = vec![
            "pr", "create", "--base", &c.base, "--head", &branch, "--title", title, "--body", body,
        ];
        if req.draft {
            args.push("--draft");
        }
        let out = self
            .must_with(
                "gh",
                &args,
                &root,
                "Creating the pull request",
                GH_CREATE_TIMEOUT,
            )
            .await?;
        let url = out.split('\n').next_back().unwrap_or("").trim().to_string();
        let number = regex::Regex::new(r"/pull/(\d+)")
            .ok()
            .and_then(|re| re.captures(&url).and_then(|m| m[1].parse::<u64>().ok()))
            .filter(|n| *n != 0);
        Ok(PrResult {
            url,
            number,
            branch,
            updated_existing: false,
        })
    }
}
