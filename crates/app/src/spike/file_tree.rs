//! Prototype 1: a virtualised file tree of 20,000 files with tri-state checkboxes and token sums.
//! Selection rules and sums come from `contextual_core::logic::tree`.

use std::collections::{HashMap, HashSet};

use contextual_core::logic::format::fmt_k;
use contextual_core::logic::tree::{
    NodeStats, Selection, check_state, node_stats, selectable_files,
};
use contextual_core::logic::types::{NodeKind, TreeNode};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div, px, uniform_list,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, row_height, sp};
use crate::ui::{Button, ButtonSize, Checkbox, pane_header};

const INDENT: f32 = 12.0;
const CHEVRON: f32 = 16.0;

struct Row {
    depth: usize,
    path: String,
}

pub struct FileTreeProto {
    roots: Vec<TreeNode>,
    by_path: HashMap<String, (Vec<usize>, usize)>,
    expanded: HashSet<String>,
    selected: Selection,
    stats: HashMap<String, NodeStats>,
    rows: Vec<Row>,
    scroll: UniformListScrollHandle,
    total_files: usize,
}

/// 40 top-level folders × 10 subfolders × 50 files = 20,000 files, with a few exclusions.
fn synthetic_tree() -> Vec<TreeNode> {
    let exts = ["cs", "ts", "rs", "json", "md"];
    (0..40)
        .map(|a| {
            let a_path = format!("module-{a:02}");
            let subs = (0..10)
                .map(|b| {
                    let b_path = format!("{a_path}/feature-{b}");
                    let files = (0..50)
                        .map(|f| {
                            let ext = exts[(a + b + f) % exts.len()];
                            let name = format!("File{f:02}.{ext}");
                            let excluded = (f == 49)
                                .then(|| "too large (612 KB)".to_string())
                                .or_else(|| (f == 48).then(|| "binary".to_string()));
                            TreeNode {
                                path: format!("{b_path}/{name}"),
                                name,
                                kind: NodeKind::File,
                                tokens: Some(
                                    ((a * 7919 + b * 104_729 + f * 1299) % 4000 + 120) as u64,
                                ),
                                children: None,
                                excluded,
                                changed_on_disk: (f == 3 && b == 2).then_some(true),
                                mtime_ms: None,
                                image: None,
                                condensed_from: None,
                            }
                        })
                        .collect();
                    dir(&b_path, files)
                })
                .collect();
            dir(&a_path, subs)
        })
        .collect()
}

fn dir(path: &str, children: Vec<TreeNode>) -> TreeNode {
    TreeNode {
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        path: path.to_string(),
        kind: NodeKind::Dir,
        tokens: None,
        children: Some(children),
        excluded: None,
        changed_on_disk: None,
        mtime_ms: None,
        image: None,
        condensed_from: None,
    }
}

impl FileTreeProto {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        let roots = synthetic_tree();
        let mut by_path = HashMap::new();
        fn index(
            nodes: &[TreeNode],
            trail: &[usize],
            out: &mut HashMap<String, (Vec<usize>, usize)>,
        ) {
            for (i, n) in nodes.iter().enumerate() {
                let mut t = trail.to_vec();
                t.push(i);
                out.insert(n.path.clone(), (t.clone(), trail.len()));
                if let Some(ch) = &n.children {
                    index(ch, &t, out);
                }
            }
        }
        index(&roots, &[], &mut by_path);
        let total_files = selectable_files(&roots).len();
        let selected: Selection = selectable_files(&roots)
            .into_iter()
            .filter(|p| p.ends_with(".cs"))
            .collect();
        let mut this = Self {
            expanded: roots.iter().take(3).map(|n| n.path.clone()).collect(),
            roots,
            by_path,
            selected,
            stats: HashMap::new(),
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            total_files,
        };
        this.recompute();
        this
    }

    fn node(&self, path: &str) -> Option<&TreeNode> {
        let (trail, _) = self.by_path.get(path)?;
        let mut nodes = &self.roots;
        let mut found = None;
        for &i in trail {
            let n = nodes.get(i)?;
            found = Some(n);
            nodes = n.children.as_ref().map_or(nodes, |c| c);
        }
        found
    }

    /// Rebuilds folder sums and the visible rows (after selection or expansion changes).
    fn recompute(&mut self) {
        let mut stats = HashMap::new();
        fn walk(n: &TreeNode, sel: &Selection, out: &mut HashMap<String, NodeStats>) {
            if let Some(ch) = &n.children {
                out.insert(n.path.clone(), node_stats(n, sel));
                ch.iter().for_each(|c| walk(c, sel, out));
            }
        }
        self.roots
            .iter()
            .for_each(|n| walk(n, &self.selected, &mut stats));
        self.stats = stats;

        let mut rows = Vec::new();
        fn visible(
            nodes: &[TreeNode],
            depth: usize,
            expanded: &HashSet<String>,
            out: &mut Vec<Row>,
        ) {
            for n in nodes {
                out.push(Row {
                    depth,
                    path: n.path.clone(),
                });
                if let Some(ch) = &n.children
                    && expanded.contains(&n.path)
                {
                    visible(ch, depth + 1, expanded, out);
                }
            }
        }
        visible(&self.roots, 0, &self.expanded, &mut rows);
        self.rows = rows;
    }

    fn toggle_expand(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_string());
        }
        self.recompute();
        cx.notify();
    }

    fn toggle_check(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(node) = self.node(path) else { return };
        let files = if node.children.is_some() {
            selectable_files(std::slice::from_ref(node))
        } else {
            vec![node.path.clone()]
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
        cx.notify();
    }

    fn expand_all(&mut self, cx: &mut Context<Self>) {
        let all_expanded = self.by_path.keys().all(|p| {
            self.node(p)
                .is_none_or(|n| n.children.is_none() || self.expanded.contains(p))
        });
        self.expanded = if all_expanded {
            HashSet::new()
        } else {
            self.by_path
                .keys()
                .filter(|p| self.node(p).is_some_and(|n| n.children.is_some()))
                .cloned()
                .collect()
        };
        self.recompute();
        cx.notify();
    }

    fn set_all(&mut self, on: bool, cx: &mut Context<Self>) {
        self.selected = if on {
            selectable_files(&self.roots).into_iter().collect()
        } else {
            Selection::new()
        };
        self.recompute();
        cx.notify();
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        let row = &self.rows[ix];
        let node = self.node(&row.path).expect("row paths come from the tree");
        let is_dir = node.kind == NodeKind::Dir;
        let expanded = self.expanded.contains(&node.path);
        let excluded = node.excluded.clone();
        let tokens = if is_dir {
            self.stats
                .get(&node.path)
                .map(|s| s.selected_tokens)
                .unwrap_or(0)
        } else if self.selected.contains(&node.path) {
            node.tokens.unwrap_or(0)
        } else {
            0
        };
        let path_for_check: SharedString = node.path.clone().into();
        let path_for_row: SharedString = node.path.clone().into();
        let state = check_state(node, &self.selected);
        let overridable = contextual_core::logic::tree::is_overridable(node);
        let dimmed = excluded.is_some() && !overridable;

        div()
            .id(("row", ix))
            .w_full()
            .flex()
            .items_center()
            .gap(sp(1))
            .h(row_height())
            .pl(sp(2) + px(row.depth as f32 * INDENT))
            .pr(sp(3))
            .type_style(contextual_theme::type_style::UI_SMALL)
            .text_color(hsla(if dimmed { c.ink_faint } else { c.ink }))
            .cursor_pointer()
            .hover(|s| s.bg(hsla(c.surface_hover)))
            .when(is_dir, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| this.toggle_expand(&path_for_row, cx)))
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(px(CHEVRON))
                    .when(is_dir, |d| {
                        d.text_color(hsla(c.ink_muted)).child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(12.0)),
                        )
                    }),
            )
            .child(
                Checkbox::new(("check", ix), state)
                    .disabled(dimmed)
                    .on_toggle(
                        cx.listener(move |this, _, _, cx| this.toggle_check(&path_for_check, cx)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .when(is_dir, |d| d.text_color(hsla(c.ink_strong)))
                    .child(SharedString::from(node.name.clone())),
            )
            .when(node.changed_on_disk == Some(true), |d| {
                d.child(div().text_color(hsla(c.modified)).child("M"))
            })
            .when_some(excluded, |d, reason| {
                d.child(
                    div()
                        .px(sp(1))
                        .rounded(px(contextual_theme::radius::SM))
                        .bg(hsla(c.pill_bg))
                        .text_color(hsla(c.ink_muted))
                        .type_style(contextual_theme::type_style::UI_CAPTION)
                        .child(if reason.starts_with("too large") {
                            "too large"
                        } else {
                            "excluded"
                        }),
                )
            })
            .when(tokens > 0, |d| {
                d.child(
                    div()
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(format!("~{}", fmt_k(tokens)))),
                )
            })
    }
}

impl Render for FileTreeProto {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let selected_tokens: u64 = self
            .roots
            .iter()
            .filter_map(|r| self.stats.get(&r.path))
            .map(|s| s.selected_tokens)
            .sum();
        let actions = div()
            .flex()
            .gap(sp(1))
            .child(
                Button::new("expand", "Expand all")
                    .size(ButtonSize::Xs)
                    .on_click(cx.listener(|t, _, _, cx| t.expand_all(cx))),
            )
            .child(
                Button::new("all", "Select all")
                    .size(ButtonSize::Xs)
                    .on_click(cx.listener(|t, _, _, cx| t.set_all(true, cx))),
            )
            .child(
                Button::new("none", "Clear")
                    .size(ButtonSize::Xs)
                    .on_click(cx.listener(|t, _, _, cx| t.set_all(false, cx))),
            );
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(hsla(c.surface_000))
            .child(pane_header("Explorer", actions, cx))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .px(sp(3))
                    .h(row_height())
                    .items_center()
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(SharedString::from(format!(
                        "{} of {} files · {} visible rows",
                        self.selected.len(),
                        self.total_files,
                        self.rows.len()
                    )))
                    .child(SharedString::from(format!(
                        "~{} tokens",
                        fmt_k(selected_tokens)
                    ))),
            )
            .child(
                uniform_list(
                    "tree",
                    self.rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|ix| this.render_row(ix, cx).into_any_element())
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1(),
            )
    }
}
