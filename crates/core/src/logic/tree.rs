//! Explorer tree helpers (`lib/tree.ts`): selection, token sums, check state, filter, sort.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::types::{NodeKind, TreeNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckState {
    On,
    Off,
    Mixed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStats {
    /// Selectable (not excluded) files below this node.
    pub files: usize,
    /// Selected files below this node.
    pub selected: usize,
    /// Tokens of the selected files below this node.
    pub selected_tokens: u64,
}

/// The selected file paths.
pub type Selection = HashSet<String>;

/// Excluded only for size: can still be ticked on its own ("Include anyway").
pub fn is_overridable(n: &TreeNode) -> bool {
    n.is_file()
        && n.is_excluded()
        && n.excluded
            .as_deref()
            .is_some_and(|r| r.starts_with("too large"))
}

fn children(n: &TreeNode) -> &[TreeNode] {
    n.children.as_deref().unwrap_or_default()
}

pub fn node_stats(node: &TreeNode, selected: &Selection) -> NodeStats {
    let file = |on: bool, files: usize| NodeStats {
        files,
        selected: usize::from(on),
        selected_tokens: if on { node.tokens.unwrap_or(0) } else { 0 },
    };
    if is_overridable(node) {
        // Counted only once ticked, so it never makes a folder look partly selected on its own.
        let on = selected.contains(&node.path);
        return file(on, usize::from(on));
    }
    if node.is_excluded() {
        return NodeStats::default();
    }
    if node.is_file() {
        return file(selected.contains(&node.path), 1);
    }
    children(node)
        .iter()
        .fold(NodeStats::default(), |mut s, c| {
            let cs = node_stats(c, selected);
            s.files += cs.files;
            s.selected += cs.selected;
            s.selected_tokens += cs.selected_tokens;
            s
        })
}

pub fn check_state(node: &TreeNode, selected: &Selection) -> CheckState {
    if is_overridable(node) {
        return if selected.contains(&node.path) {
            CheckState::On
        } else {
            CheckState::Off
        };
    }
    if node.is_excluded() {
        return CheckState::Off;
    }
    let s = node_stats(node, selected);
    if s.selected == 0 {
        CheckState::Off
    } else if s.selected == s.files {
        CheckState::On
    } else {
        CheckState::Mixed
    }
}

/// Files that can be ticked at all: selectable ones plus too-large ones (opt-in).
pub fn pickable_files(nodes: &[TreeNode]) -> Selection {
    fn walk(n: &TreeNode, out: &mut Selection) {
        if n.is_file() && (!n.is_excluded() || is_overridable(n)) {
            out.insert(n.path.clone());
        } else if !n.is_excluded() {
            children(n).iter().for_each(|c| walk(c, out));
        }
    }
    let mut out = Selection::new();
    nodes.iter().for_each(|n| walk(n, &mut out));
    out
}

/// All selectable file paths at or below the given nodes (not including too-large opt-ins).
pub fn selectable_files(nodes: &[TreeNode]) -> Vec<String> {
    fn walk(n: &TreeNode, out: &mut Vec<String>) {
        if n.is_excluded() {
            return;
        }
        if n.is_file() {
            out.push(n.path.clone());
        } else {
            children(n).iter().for_each(|c| walk(c, out));
        }
    }
    let mut out = Vec::new();
    nodes.iter().for_each(|n| walk(n, &mut out));
    out
}

pub fn find_node<'a>(nodes: &'a [TreeNode], path: &str) -> Option<&'a TreeNode> {
    for n in nodes {
        if n.path == path {
            return Some(n);
        }
        let below = path
            .strip_prefix(n.path.as_str())
            .is_some_and(|rest| rest.starts_with('/'));
        if let (Some(kids), true) = (&n.children, below) {
            if let Some(hit) = find_node(kids, path) {
                return Some(hit);
            }
        }
    }
    None
}

/// Directories first, then files, each in their original order.
pub fn sort_nodes(nodes: &[TreeNode]) -> Vec<&TreeNode> {
    let mut out: Vec<&TreeNode> = nodes.iter().collect();
    out.sort_by_key(|n| n.kind != NodeKind::Dir);
    out
}

/// Keeps nodes whose name matches the filter, plus their ancestors.
pub fn filter_tree(nodes: &[TreeNode], query: &str) -> Vec<TreeNode> {
    let q = super::js::trim(query).to_lowercase();
    if q.is_empty() {
        return nodes.to_vec();
    }
    filter_lowered(nodes, &q)
}

fn filter_lowered(nodes: &[TreeNode], q: &str) -> Vec<TreeNode> {
    let mut out = Vec::new();
    for n in nodes {
        if n.name.to_lowercase().contains(q) {
            out.push(n.clone());
        } else if let Some(kids) = &n.children {
            let kids = filter_lowered(kids, q);
            if !kids.is_empty() {
                out.push(TreeNode {
                    children: Some(kids),
                    ..n.clone()
                });
            }
        }
    }
    out
}

/// The folder (by name) holding the most selected tokens, for the "near limit" hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiggestFolder {
    pub name: String,
    pub tokens: u64,
}

pub fn biggest_folder(nodes: &[TreeNode], selected: &Selection) -> Option<BiggestFolder> {
    // Keyed by folder name in first-seen order, like the TS `Map`.
    fn walk<'a>(
        n: &'a TreeNode,
        parent: &'a str,
        selected: &Selection,
        by: &mut Vec<(&'a str, u64)>,
    ) {
        if n.is_excluded() {
            return;
        }
        if n.is_file() {
            if selected.contains(&n.path) {
                let tokens = n.tokens.unwrap_or(0);
                match by.iter_mut().find(|(name, _)| *name == parent) {
                    Some((_, t)) => *t += tokens,
                    None => by.push((parent, tokens)),
                }
            }
        } else {
            children(n)
                .iter()
                .for_each(|c| walk(c, &n.name, selected, by));
        }
    }
    let mut by = Vec::new();
    nodes.iter().for_each(|n| walk(n, "", selected, &mut by));
    let mut best: Option<BiggestFolder> = None;
    for (name, tokens) in by {
        if !name.is_empty() && best.as_ref().is_none_or(|b| tokens > b.tokens) {
            best = Some(BiggestFolder {
                name: name.to_owned(),
                tokens,
            });
        }
    }
    best
}
