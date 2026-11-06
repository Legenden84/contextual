//! Shared data types from `src/web/data/types.ts`: explorer nodes, rules, deployments and
//! proposed changes. They serialise like the TS objects (camelCase, absent optional fields).

use serde::{Deserialize, Serialize};

use super::edits::EditBlock;

/// Added, modified or deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeStatus {
    A,
    M,
    D,
}

impl ChangeStatus {
    pub fn letter(self) -> char {
        match self {
            Self::A => 'A',
            Self::M => 'M',
            Self::D => 'D',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Dir,
    File,
}

/// A file or folder from the project scan (`/api/project/scan` JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeNode>>,
    /// Exclusion reason (`ignored: node_modules`, `lock file`, `binary`, `.gitignore`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluded: Option<String>,
    /// Changed on disk since it was loaded into context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_on_disk: Option<bool>,
    /// Last modified time from the scan, for real folders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime_ms: Option<f64>,
    /// Sent to the model as a picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<bool>,
    /// Large HTML/SVG sent condensed: `tokens` is the condensed size, this the original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condensed_from: Option<u64>,
}

impl TreeNode {
    /// Has a (non-empty) exclusion reason. An empty reason counts as not excluded, as in JS.
    pub fn is_excluded(&self) -> bool {
        self.excluded.as_deref().is_some_and(|r| !r.is_empty())
    }

    pub fn is_file(&self) -> bool {
        self.kind == NodeKind::File
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuleType {
    Rule,
    Skill,
}

/// A project rule or skill file, sent in the system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: RuleType,
    pub tokens: u64,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub name: String,
    pub limit: u64,
    /// USD per 1M input tokens.
    pub in_price: f64,
    /// USD per 1M output tokens.
    pub out_price: f64,
}

/// A proposed change to one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub status: ChangeStatus,
    pub old: String,
    pub new: String,
    /// Set when the file changed on disk after it was sent to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<String>,
    /// The edit blocks this change was built from (existing files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<Vec<EditBlock>>,
    /// Edit blocks that couldn't be placed; resolved before the change is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unplaced: Option<Vec<EditBlock>>,
}
