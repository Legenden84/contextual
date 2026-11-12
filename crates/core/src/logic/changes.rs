//! Turns an assistant response into proposed file changes (`lib/changes.ts`): whole files
//! (`### FILE:`), edit blocks applied to the local copy (`### EDIT:`, see [`super::edits`]) and
//! deletions (`### DELETE:`), plus lines added to the end of a file (`### APPEND:`).
//!
//! A reply cut off at the completion token limit still yields every section it finished
//! ([`parse_reply`]); the section it was writing is described in a [`Cutoff`] so that the
//! model can be asked to continue it (see [`super::cutoff`]).

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

/// The complete lines of a file that a cut-off reply was writing (a `FILE` or `APPEND`
/// section), held until the model continues it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartialFile {
    pub path: String,
    /// The file so far, without a final newline; empty when nothing of it was received.
    pub content: String,
}

impl PartialFile {
    /// Lines received so far.
    pub fn lines(&self) -> usize {
        if self.content.is_empty() {
            0
        } else {
            self.content.split('\n').count()
        }
    }
}

/// The section a cut-off reply was writing when it stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutSection {
    /// A whole file or an append: its complete lines so far.
    File(PartialFile),
    /// An `EDIT` section: its first `applied` SEARCH/REPLACE blocks were complete and are part
    /// of the proposal; the one being written was dropped.
    Edit { path: String, applied: usize },
}

/// What a cut-off reply delivered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cutoff {
    /// Files with sections received complete, in reply order.
    pub received: Vec<String>,
    /// The section being written when the reply stopped, if any.
    pub cut: Option<CutSection>,
    /// A file held from an earlier cut-off reply that this reply neither continued nor
    /// replaced: it is given up.
    pub abandoned: Option<String>,
}

impl Cutoff {
    /// The file still being written, to hold for the next reply.
    pub fn partial(&self) -> Option<&PartialFile> {
        match &self.cut {
            Some(CutSection::File(p)) => Some(p),
            _ => None,
        }
    }
}

/// A reply turned into changes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    pub changes: Vec<Change>,
    /// Set when the reply was cut off.
    pub cutoff: Option<Cutoff>,
}

/// `base` followed by `more` on the next line.
fn appended(base: &str, more: &str) -> String {
    if base.is_empty() {
        more.to_owned()
    } else if base.ends_with('\n') || more.is_empty() {
        format!("{base}{more}")
    } else {
        format!("{base}\n{more}")
    }
}

pub fn parse_changes(response: &str, ctx: &dyn ChangeContext) -> Vec<Change> {
    parse_reply(response, false, None, ctx).changes
}

/// Parses a reply into changes. With `cut_off`, the reply stopped at the token limit: its
/// last, possibly partial, line is dropped, every finished section is kept, and the section
/// it was writing is described in [`Parsed::cutoff`]. `held` is a file an earlier cut-off
/// reply left unfinished: an `APPEND` to its path continues it.
pub fn parse_reply(
    response: &str,
    cut_off: bool,
    held: Option<&PartialFile>,
    ctx: &dyn ChangeContext,
) -> Parsed {
    let body = if cut_off {
        response.rfind('\n').map_or("", |i| &response[..i])
    } else {
        response
    };
    let mut blocks = parse_markdown(body);
    let mut cut = None;
    if cut_off {
        cut = take_cut(&mut blocks, held, ctx);
    }
    let changes = parse_blocks(blocks, held, ctx);
    let cutoff = cut_off.then(|| {
        let mut received: Vec<String> = Vec::new();
        for c in &changes {
            if !received.contains(&c.path) {
                received.push(c.path.clone());
            }
        }
        if let Some(CutSection::Edit { path, .. }) = &cut {
            received.retain(|p| p != path);
        }
        let continued = |path: &str| {
            received.iter().any(|p| p == path)
                || matches!(&cut, Some(CutSection::File(p)) if p.path == path)
        };
        let abandoned = held.filter(|h| !continued(&h.path)).map(|h| h.path.clone());
        Cutoff {
            received,
            cut,
            abandoned,
        }
    });
    let abandoned_held =
        !cut_off && held.is_some_and(|h| !changes.iter().any(|c| c.path == h.path));
    Parsed {
        changes,
        cutoff: cutoff.or_else(|| {
            abandoned_held.then(|| Cutoff {
                abandoned: held.map(|h| h.path.clone()),
                ..Cutoff::default()
            })
        }),
    }
}

/// Removes the section a cut-off reply was writing from `blocks` and describes it.
fn take_cut(
    blocks: &mut Vec<Block>,
    held: Option<&PartialFile>,
    ctx: &dyn ChangeContext,
) -> Option<CutSection> {
    let last = blocks.iter().rposition(|b| {
        matches!(
            b,
            Block::File { .. }
                | Block::Append { .. }
                | Block::Edit { .. }
                | Block::Delete { .. }
                | Block::Code { path: Some(_), .. }
        )
    })?;
    let after_append = |path: &str| {
        last > 0 && matches!(&blocks[last - 1], Block::Append { path: p } if p == path)
    };
    // The base an APPEND continues: the held file, or the proposal or file as sent.
    let base = |path: &str| match held {
        Some(h) if h.path == path => h.content.clone(),
        _ => ctx.original(path).unwrap_or_default(),
    };
    let (cut, keep) = match &blocks[last] {
        Block::Code {
            path: Some(path),
            code,
            open: true,
            ..
        } => {
            let content = if after_append(path) {
                appended(&base(path), code)
            } else {
                code.clone()
            };
            let path = path.clone();
            (CutSection::File(PartialFile { path, content }), false)
        }
        Block::File { path } => (
            CutSection::File(PartialFile {
                path: path.clone(),
                content: String::new(),
            }),
            false,
        ),
        Block::Append { path } => (
            CutSection::File(PartialFile {
                path: path.clone(),
                content: base(path),
            }),
            false,
        ),
        // The finished SEARCH/REPLACE blocks stand; the one being written is not in `edits`.
        Block::Edit {
            path,
            edits,
            partial: true,
        } => (
            CutSection::Edit {
                path: path.clone(),
                applied: edits.len(),
            },
            true,
        ),
        _ => return None,
    };
    if keep {
        if let Block::Edit { partial, .. } = &mut blocks[last] {
            *partial = false;
        }
    } else {
        // An APPEND header before a cut fence goes too: the held file carries its lines.
        let from = match &cut {
            CutSection::File(p) if after_append(&p.path) => last - 1,
            _ => last,
        };
        blocks.truncate(from);
    }
    Some(cut)
}

fn parse_blocks(
    blocks: Vec<Block>,
    held: Option<&PartialFile>,
    ctx: &dyn ChangeContext,
) -> Vec<Change> {
    let conflict = |path: &str| ctx.changed_on_disk(path).then(|| CONFLICT.to_owned());
    let status = |path: &str| {
        if ctx.exists(path) {
            ChangeStatus::M
        } else {
            ChangeStatus::A
        }
    };
    let mut by_path = Ordered::default();
    // The path of the `APPEND` header just seen: its fence adds to the file.
    let mut appending: Option<String> = None;

    for block in blocks {
        let append_to = appending.take();
        match block {
            Block::Append { path } => appending = Some(path),
            Block::Code {
                code,
                path: Some(path),
                ..
            } if !path.is_empty() => {
                if ctx.read_only(&path) {
                    continue;
                }
                let new = if append_to.as_deref() == Some(path.as_str()) {
                    let base = match (by_path.get(&path), held) {
                        (Some(p), _) if p.status != ChangeStatus::D => p.new.clone(),
                        (Some(_), _) => String::new(),
                        (None, Some(h)) if h.path == path => h.content.clone(),
                        (None, _) => ctx.original(&path).unwrap_or_default(),
                    };
                    appended(&base, &code)
                } else {
                    code
                };
                by_path.set(Change {
                    status: status(&path),
                    old: ctx.original(&path).unwrap_or_default(),
                    new,
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
