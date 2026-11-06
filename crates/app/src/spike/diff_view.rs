//! Prototype 3: side-by-side diff with word-level highlights and collapsed unchanged regions,
//! from `contextual_core::logic::diff`. Every visual line is one `uniform_list` row.

use std::collections::HashSet;

use contextual_core::logic::diff::{
    Diff, DiffItem, DiffOp, OpKind, compute_diff, hunk_header, make_hunks,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, HighlightStyle, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, StyledText, UniformListScrollHandle, Window, div, px,
    uniform_list,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, sp};

/// Line-number gutter width (5 digits of code-small).
const GUTTER: f32 = 48.0;
const CONTEXT: usize = 3;

enum Row {
    Header(String),
    Pair(Option<DiffOp>, Option<DiffOp>),
    Gap { item: usize, lines: usize },
}

pub struct DiffProto {
    diff: Diff,
    items: Vec<DiffItem>,
    open_gaps: HashSet<usize>,
    rows: Vec<Row>,
    scroll: UniformListScrollHandle,
}

impl DiffProto {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        let old = super::sample::rust_source(3_000, 0);
        let new = super::sample::rust_source(3_000, 1);
        let diff = compute_diff(&old, &new);
        let items = make_hunks(&diff.ops, CONTEXT);
        let mut this = Self {
            diff,
            items,
            open_gaps: HashSet::new(),
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
        };
        this.rebuild();
        this
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            match item {
                DiffItem::Gap { lines } if !self.open_gaps.contains(&i) => rows.push(Row::Gap {
                    item: i,
                    lines: lines.len(),
                }),
                DiffItem::Gap { lines } => rows.extend(
                    lines
                        .iter()
                        .map(|l| Row::Pair(Some(l.clone()), Some(l.clone()))),
                ),
                DiffItem::Hunk { lines } => {
                    rows.push(Row::Header(hunk_header(lines, &self.diff.new_lines)));
                    pair_lines(lines, &mut rows);
                }
            }
        }
        self.rows = rows;
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let c = &cx.tokens().colors;
        match &self.rows[ix] {
            Row::Header(h) => div()
                .w_full()
                .h(px(contextual_theme::type_style::CODE.line_height))
                .px(sp(3))
                .flex()
                .items_center()
                .bg(hsla(c.diff_hunk_bg))
                .text_color(hsla(c.ink_muted))
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .child(SharedString::from(h.clone()))
                .into_any_element(),
            Row::Gap { item, lines } => {
                let item = *item;
                div()
                    .id(("gap", ix))
                    .w_full()
                    .h(px(contextual_theme::type_style::CODE.line_height))
                    .px(sp(3))
                    .flex()
                    .items_center()
                    .bg(hsla(c.diff_hunk_bg))
                    .text_color(hsla(c.accent_ink))
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(c.surface_hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_gaps.insert(item);
                        this.rebuild();
                        cx.notify();
                    }))
                    .child(SharedString::from(format!("⋯ {lines} unchanged lines")))
                    .into_any_element()
            }
            Row::Pair(left, right) => div()
                .w_full()
                .flex()
                .h(px(contextual_theme::type_style::CODE.line_height))
                .child(side(left.as_ref(), Side::Old, cx))
                .child(div().w(px(1.0)).h_full().bg(hsla(c.border)))
                .child(side(right.as_ref(), Side::New, cx))
                .into_any_element(),
        }
    }
}

/// Within a hunk, runs of deletions pair with the following additions, line by line.
fn pair_lines(lines: &[DiffOp], rows: &mut Vec<Row>) {
    let mut i = 0;
    while i < lines.len() {
        match lines[i].k {
            OpKind::Same => {
                rows.push(Row::Pair(Some(lines[i].clone()), Some(lines[i].clone())));
                i += 1;
            }
            _ => {
                let dels: Vec<_> = lines[i..]
                    .iter()
                    .take_while(|l| l.k == OpKind::Del)
                    .cloned()
                    .collect();
                let j = i + dels.len();
                let adds: Vec<_> = lines[j..]
                    .iter()
                    .take_while(|l| l.k == OpKind::Add)
                    .cloned()
                    .collect();
                for k in 0..dels.len().max(adds.len()) {
                    rows.push(Row::Pair(dels.get(k).cloned(), adds.get(k).cloned()));
                }
                i = j + adds.len();
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Old,
    New,
}

fn side(op: Option<&DiffOp>, side: Side, cx: &gpui_kit::App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    let base = div()
        .flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .type_style(contextual_theme::type_style::CODE);
    let Some(op) = op else {
        return base.bg(hsla(c.diff_empty_bg));
    };
    let changed = op.k != OpKind::Same;
    let (bg, gutter, word) = match op.k {
        OpKind::Add => (c.diff_add_bg, c.diff_add_gutter, c.diff_add_word),
        OpKind::Del => (c.diff_del_bg, c.diff_del_gutter, c.diff_del_word),
        OpKind::Same => (c.surface_100, c.surface_100, c.surface_100),
    };
    let number = match side {
        Side::Old => op.a,
        Side::New => op.b,
    };
    let text = op.t.replace('\t', "    ");
    let highlights =
        op.r.filter(|_| changed && !op.t.contains('\t'))
            .map(|(s, e)| {
                vec![(
                    s..e,
                    HighlightStyle {
                        background_color: Some(hsla(word)),
                        ..Default::default()
                    },
                )]
            })
            .unwrap_or_default();
    base.when(changed, |d| d.bg(hsla(bg)))
        .child(
            div()
                .flex_none()
                .w(px(GUTTER))
                .pr(sp(2))
                .flex()
                .justify_end()
                .when(changed, |d| d.bg(hsla(gutter)))
                .text_color(hsla(c.editor_gutter_ink))
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .child(SharedString::from(
                    number.map(|n| n.to_string()).unwrap_or_default(),
                )),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pl(sp(2))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_color(hsla(c.syn_plain))
                .child(StyledText::new(text).with_highlights(highlights)),
        )
}

impl Render for DiffProto {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .h(px(contextual_theme::space::ROW_HEIGHT) + sp(2))
                    .px(sp(3))
                    .bg(hsla(c.surface_200))
                    .border_b_1()
                    .border_color(hsla(c.border))
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .child(
                        div()
                            .text_color(hsla(c.ink))
                            .child("src/orders/handlers.rs"),
                    )
                    .child(
                        div()
                            .text_color(hsla(c.success))
                            .child(SharedString::from(format!("+{}", self.diff.adds))),
                    )
                    .child(
                        div()
                            .text_color(hsla(c.danger))
                            .child(SharedString::from(format!("−{}", self.diff.dels))),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(format!("{} rows", self.rows.len()))),
                    ),
            )
            .child(
                uniform_list(
                    "diff",
                    self.rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1(),
            )
    }
}
