//! The open project: scanned tree, file selection, Skills & Rules and preview tabs.
//! Selection rules and sums come from `contextual_core::logic::tree`; this model only holds state.
//! Methods are plain state changes (testable without a window); views call `cx.notify()`.

use std::collections::{HashMap, HashSet};

use contextual_core::location::{Location, ProjectRoot};
use contextual_core::logic::tree::{
    NodeStats, Selection, check_state, filter_tree, is_overridable, node_stats, pickable_files,
    selectable_files,
};
use contextual_core::logic::types::{NodeKind, Rule, RuleType, TreeNode};
use contextual_protocol::fs::ScanResult;

pub use contextual_core::logic::tree::CheckState;

/// One visible row of the explorer.
#[derive(Clone, Debug)]
pub struct Row {
    pub depth: usize,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tab {
    Chat,
    Preview(String),
}

pub struct Project {
    pub root: String,
    /// Where the project lives (local, or a WSL distro).
    pub location: Location,
    pub name: String,
    pub branch: Option<String>,
    /// The scan stopped at the entry cap.
    pub truncated: bool,
    tree: Vec<TreeNode>,
    /// Node lookup by path in `tree`: indices from the root.
    index: HashMap<String, Vec<usize>>,
    pub selected: Selection,
    pub expanded: HashSet<String>,
    pub filter: String,
    pub rules: Vec<Rule>,
    /// Bumped by every rescan, so other models can refresh (Source Control).
    pub rescans: u64,
    pub rules_on: HashSet<String>,
    pub rule_preview: Option<String>,
    pub tabs: Vec<String>,
    pub active: Tab,
    stats: HashMap<String, NodeStats>,
    rows: Vec<Row>,
    /// The filtered tree the rows were built from (only while filtering).
    filtered: Option<Vec<TreeNode>>,
}

/// Scan JSON → the logic layer's types (same wire shape, so this is a straight conversion).
fn convert<T: serde::de::DeserializeOwned>(v: impl serde::Serialize) -> T {
    serde_json::from_value(serde_json::to_value(v).expect("scan types serialise"))
        .expect("scan JSON matches the logic types")
}

/// Folders first, then files; keeps the scan's order within each group (recursively).
fn sorted(nodes: Vec<TreeNode>) -> Vec<TreeNode> {
    let (mut dirs, files): (Vec<_>, Vec<_>) =
        nodes.into_iter().partition(|n| n.kind == NodeKind::Dir);
    for d in &mut dirs {
        if let Some(ch) = d.children.take() {
            d.children = Some(sorted(ch));
        }
    }
    dirs.extend(files);
    dirs
}

fn build_index(nodes: &[TreeNode], trail: &mut Vec<usize>, out: &mut HashMap<String, Vec<usize>>) {
    for (i, n) in nodes.iter().enumerate() {
        trail.push(i);
        out.insert(n.path.clone(), trail.clone());
        if let Some(ch) = &n.children {
            build_index(ch, trail, out);
        }
        trail.pop();
    }
}

impl Project {
    pub fn from_scan(scan: ScanResult, location: Location) -> Self {
        let tree = sorted(convert(&scan.tree));
        let rules: Vec<Rule> = convert(&scan.rules);
        let mut index = HashMap::new();
        build_index(&tree, &mut Vec::new(), &mut index);
        let mut this = Self {
            location,
            root: scan.root,
            name: scan.name,
            branch: scan.branch,
            truncated: scan.truncated,
            tree,
            index,
            selected: Selection::new(),
            expanded: HashSet::new(),
            filter: String::new(),
            // Rules are on by default, skills off (as before).
            rules_on: rules
                .iter()
                .filter(|r| r.kind == RuleType::Rule)
                .map(|r| r.path.clone())
                .collect(),
            rules,
            rescans: 0,
            rule_preview: None,
            tabs: Vec::new(),
            active: Tab::Chat,
            stats: HashMap::new(),
            rows: Vec::new(),
            filtered: None,
        };
        this.recompute();
        this
    }

    /// Replaces the tree after a rescan, keeping what still exists of the selection and tabs.
    pub fn apply_rescan(&mut self, scan: ScanResult) {
        let fresh = Self::from_scan(scan, self.location.clone());
        let pickable = pickable_files(&fresh.tree);
        let rules_on = self
            .rules_on
            .iter()
            .filter(|p| fresh.rules.iter().any(|r| &r.path == *p))
            .cloned()
            .collect();
        self.tree = fresh.tree;
        self.index = fresh.index;
        self.branch = fresh.branch;
        self.truncated = fresh.truncated;
        self.rules = fresh.rules;
        self.rules_on = rules_on;
        self.selected.retain(|p| pickable.contains(p));
        self.rescans += 1;
        self.tabs.retain(|t| self.index.contains_key(t));
        if let Tab::Preview(p) = &self.active
            && !self.index.contains_key(p)
        {
            self.active = Tab::Chat;
        }
        self.recompute();
    }

    /// Key for per-project app data (saved chats): see `ProjectRoot::key`.
    pub fn key(&self) -> String {
        ProjectRoot {
            location: self.location.clone(),
            path: self.root.clone(),
        }
        .key()
    }

    pub fn tree(&self) -> &[TreeNode] {
        &self.tree
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// A node by path (from the filtered tree while filtering, so rows always resolve).
    pub fn node(&self, path: &str) -> Option<&TreeNode> {
        if let Some(f) = &self.filtered {
            return contextual_core::logic::tree::find_node(f, path);
        }
        let mut nodes = &self.tree;
        let mut found = None;
        for &i in self.index.get(path)? {
            let n = nodes.get(i)?;
            found = Some(n);
            if let Some(ch) = &n.children {
                nodes = ch;
            }
        }
        found
    }

    pub fn folder_stats(&self, path: &str) -> NodeStats {
        self.stats.get(path).copied().unwrap_or_default()
    }

    /// Selected files and their tokens across the whole tree.
    pub fn selection_totals(&self) -> (usize, u64) {
        self.tree
            .iter()
            .map(|n| node_stats(n, &self.selected))
            .fold((0, 0), |(f, t), s| (f + s.selected, t + s.selected_tokens))
    }

    pub fn check_state(&self, node: &TreeNode) -> CheckState {
        check_state(node, &self.selected)
    }

    pub fn is_open(&self, path: &str) -> bool {
        self.filtered.is_some() || self.expanded.contains(path)
    }

    /// Rebuilds folder sums and the visible rows (after selection, expansion or filter changes).
    fn recompute(&mut self) {
        let mut stats = HashMap::new();
        fn walk(n: &TreeNode, sel: &Selection, out: &mut HashMap<String, NodeStats>) {
            if let Some(ch) = &n.children {
                out.insert(n.path.clone(), node_stats(n, sel));
                ch.iter().for_each(|c| walk(c, sel, out));
            }
        }
        self.tree
            .iter()
            .for_each(|n| walk(n, &self.selected, &mut stats));
        self.stats = stats;

        self.filtered =
            (!self.filter.trim().is_empty()).then(|| filter_tree(&self.tree, &self.filter));
        let mut rows = Vec::new();
        fn visible(
            nodes: &[TreeNode],
            depth: usize,
            open: &dyn Fn(&str) -> bool,
            out: &mut Vec<Row>,
        ) {
            for n in nodes {
                out.push(Row {
                    depth,
                    path: n.path.clone(),
                });
                if let Some(ch) = &n.children
                    && open(&n.path)
                {
                    visible(ch, depth + 1, open, out);
                }
            }
        }
        let filtering = self.filtered.is_some();
        let expanded = &self.expanded;
        let open = |p: &str| filtering || expanded.contains(p);
        visible(
            self.filtered.as_deref().unwrap_or(&self.tree),
            0,
            &open,
            &mut rows,
        );
        self.rows = rows;
    }

    pub fn toggle_expand(&mut self, path: &str) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_string());
        }
        self.recompute();
    }

    /// Ticks or unticks a file, or every selectable file in a folder.
    pub fn toggle_check(&mut self, path: &str) {
        let Some(node) = self.node(path) else { return };
        let files = match node.kind {
            NodeKind::Dir => selectable_files(std::slice::from_ref(node)),
            NodeKind::File if node.excluded.is_none() || is_overridable(node) => {
                vec![node.path.clone()]
            }
            NodeKind::File => return,
        };
        let all_on = files.iter().all(|f| self.selected.contains(f));
        for f in files {
            if all_on {
                self.selected.remove(&f);
            } else {
                self.selected.insert(f);
            }
        }
        self.recompute();
    }

    pub fn select_all(&mut self) {
        self.selected = selectable_files(&self.tree).into_iter().collect();
        self.recompute();
    }

    /// Ticks exactly these files (those that can still be picked), e.g. when a saved chat opens.
    pub fn select_paths(&mut self, paths: &[String]) {
        let pickable = pickable_files(&self.tree);
        self.selected = paths
            .iter()
            .filter(|p| pickable.contains(*p))
            .cloned()
            .collect();
        self.recompute();
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.recompute();
    }

    pub fn set_filter(&mut self, filter: String) {
        if self.filter != filter {
            self.filter = filter;
            self.recompute();
        }
    }

    pub fn toggle_rule(&mut self, path: &str) {
        if !self.rules_on.remove(path) {
            self.rules_on.insert(path.to_string());
        }
    }

    pub fn preview_rule(&mut self, path: &str) {
        self.rule_preview = if self.rule_preview.as_deref() == Some(path) {
            None
        } else {
            Some(path.to_string())
        };
    }

    pub fn rules_on(&self) -> Vec<Rule> {
        self.rules
            .iter()
            .filter(|r| self.rules_on.contains(&r.path))
            .cloned()
            .collect()
    }

    pub fn open_preview(&mut self, path: &str) {
        if !self.tabs.iter().any(|t| t == path) {
            self.tabs.push(path.to_string());
        }
        self.active = Tab::Preview(path.to_string());
    }

    pub fn close_tab(&mut self, path: &str) {
        let Some(i) = self.tabs.iter().position(|t| t == path) else {
            return;
        };
        self.tabs.remove(i);
        if self.active == Tab::Preview(path.to_string()) {
            self.active = match self
                .tabs
                .get(i)
                .or_else(|| self.tabs.get(i.wrapping_sub(1)))
            {
                Some(t) => Tab::Preview(t.clone()),
                None => Tab::Chat,
            };
        }
    }

    pub fn activate(&mut self, tab: Tab) {
        self.active = tab;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextual_protocol::fs::ScanOptions;

    fn fixture() -> ScanResult {
        let root = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../core/tests/fixtures/projects/basic"
        );
        contextual_core::workspace::scan::scan_project(root, &ScanOptions::default())
            .expect("fixture scans")
    }

    #[test]
    fn selection_filter_and_tabs() {
        let p = &mut Project::from_scan(fixture(), Location::Local);
        // Folders first, then files, at the top level.
        let kinds: Vec<_> = p.tree().iter().map(|n| n.kind).collect();
        let first_file = kinds
            .iter()
            .position(|k| *k == NodeKind::File)
            .unwrap_or(kinds.len());
        assert!(kinds[first_file..].iter().all(|k| *k == NodeKind::File));

        // Ticking a folder selects its selectable files and sums their tokens.
        p.toggle_check("src");
        let (files, tokens) = p.selection_totals();
        assert!(files > 0 && tokens > 0);
        assert_eq!(p.folder_stats("src").selected, files);
        assert_eq!(p.check_state(p.node("src").unwrap()), CheckState::On);
        p.toggle_check("src");
        assert_eq!(p.selection_totals(), (0, 0));

        // Collapsed: only top-level rows; expanding shows children.
        let top = p.rows().len();
        p.toggle_expand("src");
        assert!(p.rows().len() > top);

        // Filtering opens matching folders and hides the rest.
        p.set_filter("crlf".into());
        assert!(p.rows().iter().any(|r| r.path == "src/crlf.cs"));
        assert!(p.rows().iter().all(|r| "src/crlf.cs".starts_with(&r.path)));
        p.set_filter(String::new());

        // Preview tabs.
        p.open_preview("src/main.ts");
        p.open_preview("src/lower.ts");
        assert_eq!(p.active, Tab::Preview("src/lower.ts".into()));
        p.close_tab("src/lower.ts");
        assert_eq!(p.active, Tab::Preview("src/main.ts".into()));
        p.close_tab("src/main.ts");
        assert_eq!(p.active, Tab::Chat);
    }
}
