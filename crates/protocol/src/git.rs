//! Source Control and pull request types.
//!
//! Wire format matches the old `/api/git/*` and `/api/pr/*` JSON: camelCase keys; fields that
//! were `T | null` serialise as `null`, fields that were optional (`?:`) are omitted when `None`.

use serde::{Deserialize, Serialize};

/// VS Code-style letters: M modified, A added, D deleted, R renamed, U untracked, C conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeLetter {
    M,
    A,
    D,
    R,
    U,
    C,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub path: String,
    /// Original path of a rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    pub letter: ChangeLetter,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub staged: Vec<StatusEntry>,
    pub changes: Vec<StatusEntry>,
    pub conflicts: Vec<StatusEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub old: String,
    pub new: String,
    /// Not shown: binary or too large.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    /// Short hash of the new commit.
    pub commit: String,
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub branch: String,
    pub upstream: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    /// Local: "feature/x"; remote: "origin/feature/x".
    pub name: String,
    pub upstream: Option<String>,
    pub sha: String,
    pub when: String,
    pub subject: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchList {
    pub current: Option<String>,
    pub local: Vec<BranchInfo>,
    pub remote: Vec<BranchInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub when: String,
}

/// A file of a comparison: [`FileDiff`] plus its letter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonFile {
    pub path: String,
    pub letter: ChangeLetter,
    pub old: String,
    pub new: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingPr {
    pub url: String,
    pub number: u64,
}

/// What a PR from `branch` into `base` would contain, like GitHub's compare view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// Source branch of the PR (compare).
    pub branch: Option<String>,
    /// The checked-out branch.
    pub current: Option<String>,
    pub default_branch: Option<String>,
    /// Target branch of the PR (base).
    pub base: String,
    pub remote: Option<String>,
    /// gh can reach the repository.
    pub github: bool,
    /// Why a PR can't be created here, if it can't.
    pub problem: Option<String>,
    pub commits: Vec<Commit>,
    pub files: Vec<ComparisonFile>,
    /// Uncommitted changes (only counted when the source is the checked-out branch).
    pub uncommitted: u64,
    /// Commits of the source branch not on origin yet (pushed when the PR is created).
    pub unpushed: u64,
    pub existing_pr: Option<ExistingPr>,
}

/// Diff and commit list for a model-written PR description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDiff {
    pub diff: String,
    pub commits: String,
    pub branch: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrRequest {
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub base: String,
    /// Source branch; default: the checked-out branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default)]
    pub draft: bool,
    /// Create this branch at HEAD first (when the source is the checked-out default branch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrResult {
    pub url: String,
    pub number: Option<u64>,
    pub branch: String,
    pub updated_existing: bool,
}

/// Model-written PR title and description.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDraft {
    pub title: String,
    pub body: String,
}

/// Model-written commit message (`/api/git/draft-commit`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDraft {
    pub message: String,
}

// ------------------------------------------------------------------ requests

/// `{ root }`: status, push, draft-commit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootRequest {
    pub root: String,
}

/// `{ root, paths }`: stage, unstage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathsRequest {
    pub root: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRequest {
    pub root: String,
    pub path: String,
    #[serde(default)]
    pub staged: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitRequest {
    pub root: String,
    pub message: String,
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchesRequest {
    pub root: String,
    #[serde(default)]
    pub fetch: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchRequest {
    pub root: String,
    pub name: String,
}

/// Deletes only the named local branch, without forcing unmerged deletion.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBranchRequest {
    pub root: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBranchRequest {
    pub root: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareRequest {
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

/// `{ root, base, head?, notes }`: model-drafted PR description.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDraftRequest {
    pub root: String,
    pub base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default)]
    pub notes: String,
}

/// `{ root, title, body, base, head?, draft, newBranch? }`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePrRequest {
    pub root: String,
    #[serde(flatten)]
    pub pr: PrRequest,
}

/// `{ root, base, head? }` for the diff a PR description is drafted from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDiffRequest {
    pub root: String,
    pub base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
}

/// One Source Control / PR operation, sent to the workspace host as `workspace/git`.
/// On the wire: `{ "op": "status", "root": "…" }`, `{ "op": "commit", "root": "…", "message": "…" }`…
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum GitCall {
    Status(RootRequest),
    /// Replies with the new status, like the old `/api/git/stage`.
    Stage(PathsRequest),
    Unstage(PathsRequest),
    Diff(DiffRequest),
    /// The staged (or else all) changes as one diff, for a drafted commit message.
    CommitDiff(RootRequest),
    Commit(CommitRequest),
    Push(RootRequest),
    Branches(BranchesRequest),
    Switch(SwitchRequest),
    CreateBranch(CreateBranchRequest),
    DeleteBranch(DeleteBranchRequest),
    Compare(CompareRequest),
    PrDiff(PrDiffRequest),
    CreatePr(CreatePrRequest),
}

impl GitCall {
    /// The project root the call operates on (checked against the host's allowlist).
    pub fn root(&self) -> &str {
        match self {
            GitCall::Status(r) | GitCall::CommitDiff(r) | GitCall::Push(r) => &r.root,
            GitCall::Stage(r) | GitCall::Unstage(r) => &r.root,
            GitCall::Diff(r) => &r.root,
            GitCall::Commit(r) => &r.root,
            GitCall::Branches(r) => &r.root,
            GitCall::Switch(r) => &r.root,
            GitCall::CreateBranch(r) => &r.root,
            GitCall::DeleteBranch(r) => &r.root,
            GitCall::Compare(r) => &r.root,
            GitCall::PrDiff(r) => &r.root,
            GitCall::CreatePr(r) => &r.root,
        }
    }
}

/// The result of a [`GitCall`]; the variant matches the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", content = "result", rename_all = "camelCase")]
pub enum GitReply {
    Status(GitStatus),
    Diff(FileDiff),
    CommitDiff(String),
    Commit(CommitResult),
    Push(PushResult),
    Branches(BranchList),
    DeleteBranch(()),
    Compare(Comparison),
    PrDiff(PrDiff),
    CreatePr(PrResult),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_shape() {
        let st = GitStatus {
            branch: Some("main".into()),
            staged: vec![StatusEntry {
                path: "b".into(),
                from: Some("a".into()),
                letter: ChangeLetter::R,
            }],
            changes: vec![StatusEntry {
                path: "c".into(),
                from: None,
                letter: ChangeLetter::U,
            }],
            ..GitStatus::default()
        };
        assert_eq!(
            serde_json::to_value(&st).unwrap(),
            json!({"branch":"main","upstream":null,"ahead":0,"behind":0,
                   "staged":[{"path":"b","from":"a","letter":"R"}],
                   "changes":[{"path":"c","letter":"U"}],"conflicts":[]})
        );
        let pr = PrResult {
            url: "u".into(),
            number: None,
            branch: "b".into(),
            updated_existing: false,
        };
        assert_eq!(
            serde_json::to_value(&pr).unwrap(),
            json!({"url":"u","number":null,"branch":"b","updatedExisting":false})
        );
        let req: CreatePrRequest =
            serde_json::from_value(json!({"root":"/r","title":"t","base":"main","newBranch":"x"}))
                .unwrap();
        assert_eq!(req.pr.new_branch.as_deref(), Some("x"));
        assert!(!req.pr.draft);
    }

    #[test]
    fn delete_branch_wire_shape() {
        let call = GitCall::DeleteBranch(DeleteBranchRequest {
            root: "/r".into(),
            name: "feature/finished".into(),
        });
        let value = serde_json::to_value(&call).unwrap();
        assert_eq!(
            value,
            json!({"op": "deleteBranch", "root": "/r", "name": "feature/finished"})
        );
        assert_eq!(call.root(), "/r");
        assert_eq!(serde_json::from_value::<GitCall>(value).unwrap(), call);

        let reply = GitReply::DeleteBranch(());
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value, json!({"op": "deleteBranch", "result": null}));
        assert_eq!(serde_json::from_value::<GitReply>(value).unwrap(), reply);
    }

    #[test]
    fn git_call_wire_shape() {
        let call = GitCall::Commit(CommitRequest {
            root: "/r".into(),
            message: "m".into(),
            all: true,
        });
        let v = serde_json::to_value(&call).unwrap();
        assert_eq!(
            v,
            json!({"op": "commit", "root": "/r", "message": "m", "all": true})
        );
        assert_eq!(serde_json::from_value::<GitCall>(v).unwrap(), call);
        let pr = json!({"op": "createPr", "root": "/r", "title": "t", "base": "main"});
        let call: GitCall = serde_json::from_value(pr).unwrap();
        assert_eq!(call.root(), "/r");
        let reply = GitReply::CommitDiff("d".into());
        let v = serde_json::to_value(&reply).unwrap();
        assert_eq!(v, json!({"op": "commitDiff", "result": "d"}));
        assert_eq!(serde_json::from_value::<GitReply>(v).unwrap(), reply);
    }
}
