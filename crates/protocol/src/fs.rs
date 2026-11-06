//! Browse, scan, read, apply/undo types.
//!
//! These mirror the JSON of the old `/api/fs/dirs` and `/api/project/*` routes field by field:
//! camelCase names, TypeScript-optional fields (`foo?: T`) skipped when `None`, and nullable
//! fields (`foo: T | null`) always present (serialised as `null`). Paths are plain `String`s
//! (absolute paths as the workspace host spells them, project paths with forward slashes).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------------------------
// Browse (Open folder dialog)
// ---------------------------------------------------------------------------------------------

/// `GET /api/fs/dirs?path=&hidden=1`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDirs {
    /// What the user typed; `None`/empty means the home folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Show hidden folders.
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    /// Looks like a project root (has .git, a solution, package.json, pyproject…).
    pub project: bool,
}

/// A shortcut in the folder browser (Home, `/`, drives, volumes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    pub path: String,
    /// `null` at the file-system root.
    pub parent: Option<String>,
    pub dirs: Vec<DirEntry>,
    pub places: Vec<Place>,
    /// More sub-directories exist than were listed.
    pub truncated: bool,
}

// ---------------------------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------------------------

/// Scan options. Missing fields take the old server's defaults (`scanOptions()` in `api.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOptions {
    #[serde(default)]
    pub exclusions: Vec<String>,
    /// KB; a JS number, so fractions are allowed. Values `<= 0` mean the default (256).
    #[serde(default = "default_max_file_size_kb")]
    pub max_file_size_kb: f64,
    #[serde(default = "default_true")]
    pub respect_gitignore: bool,
    #[serde(default = "default_true")]
    pub skip_binary: bool,
    #[serde(default)]
    pub include_hidden: bool,
}

fn default_max_file_size_kb() -> f64 {
    256.0
}

fn default_true() -> bool {
    true
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            exclusions: Vec::new(),
            max_file_size_kb: default_max_file_size_kb(),
            respect_gitignore: true,
            skip_binary: true,
            include_hidden: false,
        }
    }
}

/// `POST /api/project/scan`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub root: String,
    #[serde(default)]
    pub options: ScanOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Dir,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanNode {
    pub name: String,
    pub path: String,
    pub kind: NodeKind,
    /// Files only (absent for symlinks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    /// Exclusion reason: `ignored: <dir>`, `ignored: <glob>`, `lock file`, `.gitignore`,
    /// `secrets`, `hidden`, `binary`, `too large (N KB)`, `symlink`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluded: Option<String>,
    /// Directories only (always present for them, possibly empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<ScanNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime_ms: Option<i64>,
    /// Sent to the model as a picture, not as text (only ever `Some(true)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<bool>,
    /// Large HTML/SVG sent condensed: `tokens` is the condensed size; this is the original
    /// token estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condensed_from: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleType {
    Rule,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleFile {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: RuleType,
    pub tokens: u64,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub name: String,
    pub root: String,
    pub branch: Option<String>,
    pub tree: Vec<ScanNode>,
    pub rules: Vec<RuleFile>,
    pub files: u64,
    /// The scan stopped at the entry cap.
    pub truncated: bool,
}

// ---------------------------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadOptions {
    /// Condense large HTML/SVG (for sending to the model; previews want the real file).
    #[serde(default)]
    pub condense: bool,
}

/// `POST /api/project/read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFiles {
    pub root: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub condense: bool,
}

/// Response of `POST /api/project/read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFilesResult {
    pub files: Vec<FileContent>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub path: String,
    /// Text with LF line endings and no BOM; saving restores the file's own format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// For image files: a data URL to send as a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The content is a condensed copy (original size in bytes); send it as a read-only reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condensed_from: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Apply / undo
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeStatus {
    /// Added: the file must not exist yet.
    A,
    /// Modified.
    M,
    /// Deleted.
    D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyChange {
    pub path: String,
    pub status: ChangeStatus,
    /// New contents for A / M (LF line endings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Contents the model saw, for M / D. Saving is refused if the file changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
}

/// `POST /api/project/apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Apply {
    pub root: String,
    pub changes: Vec<ApplyChange>,
    /// Keep an undo snapshot (default `true`, like `body.backup !== false`).
    #[serde(default = "default_true")]
    pub backup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub created: u64,
    pub overwritten: u64,
    pub deleted: u64,
    /// Undo snapshot id, when one was kept (`null` otherwise).
    pub backup_id: Option<String>,
}

/// `POST /api/project/undo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Undo {
    pub root: String,
    pub backup_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoResult {
    pub restored: u64,
    pub removed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn optional_and_nullable_fields() {
        let f = FileContent {
            path: "a".into(),
            error: Some("binary".into()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&f).unwrap(),
            json!({ "path": "a", "error": "binary" })
        );
        let r = ApplyResult {
            created: 1,
            overwritten: 0,
            deleted: 0,
            backup_id: None,
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({ "created": 1, "overwritten": 0, "deleted": 0, "backupId": null })
        );
        let rule = RuleFile {
            path: "AGENTS.md".into(),
            kind: RuleType::Rule,
            tokens: 1,
            content: "x".into(),
        };
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            json!({ "path": "AGENTS.md", "type": "Rule", "tokens": 1, "content": "x" })
        );
    }

    #[test]
    fn scan_option_defaults() {
        let o: ScanOptions = serde_json::from_value(json!({})).unwrap();
        assert_eq!(o, ScanOptions::default());
        let a: Apply = serde_json::from_value(json!({ "root": "/x", "changes": [] })).unwrap();
        assert!(a.backup);
        let c: ApplyChange = serde_json::from_value(json!({ "path": "a", "status": "D" })).unwrap();
        assert_eq!(c.status, ChangeStatus::D);
    }
}
