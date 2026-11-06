//! Turns an assistant response into proposed file changes (`lib/changes.ts`): whole files
//! (`### FILE:`), edit blocks applied to the local copy (`### EDIT:`, see [`super::edits`]) and
//! deletions (`### DELETE:`).

use std::collections::HashMap;

use super::edits::{EditOutcome, apply_edits};
use super::markdown::{Block, parse_markdown};
use super::types::{Change, ChangeStatus};

/// What the parser needs to know about the project.
pub trait ChangeContext {
    /// Does the path exist in the project?
    fn exists(&self, path: &str) -> bool;
    /// Contents as they were sent to the model, when known.
    fn original(&self, path: &str) -> Option<String>;
    /// Has the file changed on disk since it was loaded into context?
    fn changed_on_disk(&self, path: &str) -> bool;
    /// Sent only as a condensed reference: never offered for apply.
    fn read_only(&self, _path: &str) -> bool {
        false
    }
}

pub const CONFLICT: &str =
    "Changed on disk after it was sent. The proposal was made against the older version.";

/// Changes keyed by path, in first-seen order (like a JS `Map`).
#[derive(Default)]
struct Ordered {
    list: Vec<Change>,
    index: HashMap<String, usize>,
}

impl Ordered {
    fn get(&self, path: &str) -> Option<&Change> {
        self.index.get(path).map(|&i| &self.list[i])
    }

    fn set(&mut self, change: Change) {
        match self.index.get(&change.path) {
            Some(&i) => self.list[i] = change,
            None => {
                self.index.insert(change.path.clone(), self.list.len());
                self.list.push(change);
            }
        }
    }
}

pub fn parse_changes(response: &str, ctx: &dyn ChangeContext) -> Vec<Change> {
    let conflict = |path: &str| ctx.changed_on_disk(path).then(|| CONFLICT.to_owned());
    let status = |path: &str| {
        if ctx.exists(path) {
            ChangeStatus::M
        } else {
            ChangeStatus::A
        }
    };
    let mut by_path = Ordered::default();

    for block in parse_markdown(response) {
        match block {
            Block::Code {
                code,
                path: Some(path),
                ..
            } if !path.is_empty() => {
                if ctx.read_only(&path) {
                    continue;
                }
                by_path.set(Change {
                    status: status(&path),
                    old: ctx.original(&path).unwrap_or_default(),
                    new: code,
                    conflict: conflict(&path),
                    edits: None,
                    unplaced: None,
                    path,
                });
            }
            Block::Edit {
                path,
                edits,
                partial,
            } => {
                if partial || ctx.read_only(&path) {
                    continue;
                }
                // Several EDIT sections for one file apply on top of each other.
                let prev = by_path.get(&path);
                let original = ctx.original(&path);
                let base = match prev {
                    Some(p) if p.status != ChangeStatus::D => Some(p.new.as_str()),
                    _ => original.as_deref(),
                };
                let result = match base {
                    Some(base) => apply_edits(base, &edits),
                    None => EditOutcome {
                        content: String::new(),
                        failed: edits.clone(),
                    },
                };
                let old = prev.map(|p| p.old.clone()).or(original).unwrap_or_default();
                let mut all_edits = prev.and_then(|p| p.edits.clone()).unwrap_or_default();
                all_edits.extend(edits);
                let mut unplaced = prev.and_then(|p| p.unplaced.clone()).unwrap_or_default();
                unplaced.extend(result.failed);
                by_path.set(Change {
                    status: status(&path),
                    old,
                    new: result.content,
                    conflict: conflict(&path),
                    edits: Some(all_edits),
                    unplaced: Some(unplaced),
                    path,
                });
            }
            Block::Delete { path } => {
                if !path.is_empty() && ctx.read_only(&path) {
                    continue;
                }
                by_path.set(Change {
                    status: ChangeStatus::D,
                    old: ctx.original(&path).unwrap_or_default(),
                    new: String::new(),
                    conflict: conflict(&path),
                    edits: None,
                    unplaced: None,
                    path,
                });
            }
            _ => {}
        }
    }
    // A "modified" file whose new content equals the old one isn't a change (unless edits still need placing).
    by_path
        .list
        .into_iter()
        .filter(|c| {
            !(c.status == ChangeStatus::M
                && c.old == c.new
                && c.unplaced.as_ref().is_none_or(Vec::is_empty))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ctx;
    impl ChangeContext for Ctx {
        fn exists(&self, path: &str) -> bool {
            path == "a.txt"
        }
        fn original(&self, path: &str) -> Option<String> {
            (path == "a.txt").then(|| "one\ntwo\n".to_owned())
        }
        fn changed_on_disk(&self, _: &str) -> bool {
            false
        }
    }

    #[test]
    fn file_edit_delete() {
        let r = "### EDIT: a.txt\n<<<<<<< SEARCH\ntwo\n=======\n2\n>>>>>>> REPLACE\n### FILE: b.txt\n```\nb\n```\n### DELETE: c.txt";
        let c = parse_changes(r, &Ctx);
        assert_eq!(c.len(), 3);
        assert_eq!(
            (c[0].status, c[0].new.as_str()),
            (ChangeStatus::M, "one\n2\n")
        );
        assert_eq!((c[1].status, c[1].new.as_str()), (ChangeStatus::A, "b"));
        assert_eq!(c[2].status, ChangeStatus::D);
    }
}
