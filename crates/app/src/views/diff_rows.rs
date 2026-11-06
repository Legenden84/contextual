//! Diff rows shared by the Diff screen, git diff tabs and the pull request view (design spec:
//! DiffFile): `@@` headers with the enclosing member, collapsed unchanged runs, unified or split
//! lines with word-level highlights. Callers render their own file headers (`Row::File`).

use std::collections::HashSet;

use contextual_core::logic::diff::{Diff, DiffItem, DiffOp, OpKind, hunk_header, make_hunks};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, HighlightStyle, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, StyledText, div, px,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, sp};

/// Unchanged lines kept around each change.
const CONTEXT: usize = 3;
const GUTTER: f32 = 44.0;

pub enum Row {
    File(usize),
    /// Conflict or "not shown" note under a file header.
    Banner(usize),
    Hunk(String),
    Gap {
        file: usize,
        item: usize,
        lines: usize,
    },
    Unified(DiffOp),
    Split(Option<DiffOp>, Option<DiffOp>),
    Spacer,
}

/// One file to show: its diff and an optional banner (conflict, "not shown: binary").
pub struct FileRows<'a> {
    pub diff: &'a Diff,
    pub banner: bool,
    /// Skip the lines (e.g. binary files): header and banner only.
    pub hide_lines: bool,
}

fn pair(lines: &[DiffOp], rows: &mut Vec<Row>) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].k == OpKind::Same {
            rows.push(Row::Split(Some(lines[i].clone()), Some(lines[i].clone())));
            i += 1;
            continue;
        }
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
            rows.push(Row::Split(dels.get(k).cloned(), adds.get(k).cloned()));
        }
        i = j + adds
            .len()
            .max(usize::from(dels.is_empty() && adds.is_empty()));
    }
}

/// Rows for all files, and the row index of each file's header.
pub fn build(
    files: &[FileRows<'_>],
    split: bool,
    expand_all: bool,
    open_gaps: &HashSet<(usize, usize)>,
) -> (Vec<Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut file_rows = Vec::new();
    for (f, file) in files.iter().enumerate() {
        file_rows.push(rows.len());
        rows.push(Row::File(f));
        if file.banner {
            rows.push(Row::Banner(f));
        }
        if !file.hide_lines {
            for (i, item) in make_hunks(&file.diff.ops, CONTEXT).into_iter().enumerate() {
                let (lines, header) = match item {
                    DiffItem::Gap { lines } if !expand_all && !open_gaps.contains(&(f, i)) => {
                        rows.push(Row::Gap {
                            file: f,
                            item: i,
                            lines: lines.len(),
                        });
                        continue;
                    }
                    DiffItem::Gap { lines } => (lines, None),
                    DiffItem::Hunk { lines } => {
                        let h = hunk_header(&lines, &file.diff.new_lines);
                        (lines, Some(h))
                    }
                };
                if let Some(h) = header {
                    rows.push(Row::Hunk(h));
                }
                if split {
                    pair(&lines, &mut rows)
                } else {
                    rows.extend(lines.into_iter().map(Row::Unified))
                }
            }
        }
        rows.push(Row::Spacer);
    }
    (rows, file_rows)
}

fn line_text(op: &DiffOp, word: contextual_theme::Rgba) -> StyledText {
    let text = op.t.replace('\t', "    ");
    let hl =
        op.r.filter(|_| op.k != OpKind::Same && !op.t.contains('\t'))
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
    StyledText::new(text).with_highlights(hl)
}

fn gutter(
    n: Option<usize>,
    bg: Option<contextual_theme::Rgba>,
    cx: &App,
) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    div()
        .flex_none()
        .w(px(GUTTER))
        .pr(sp(2))
        .flex()
        .justify_end()
        .when_some(bg, |d, b| d.bg(hsla(b)))
        .text_color(hsla(c.editor_gutter_ink))
        .type_style(contextual_theme::type_style::CODE_SMALL)
        .child(SharedString::from(
            n.map(|n| n.to_string()).unwrap_or_default(),
        ))
}

type Colors3 = (
    contextual_theme::Rgba,
    contextual_theme::Rgba,
    contextual_theme::Rgba,
);

fn colors(k: OpKind, c: &contextual_theme::Colors) -> Option<Colors3> {
    match k {
        OpKind::Add => Some((c.diff_add_bg, c.diff_add_gutter, c.diff_add_word)),
        OpKind::Del => Some((c.diff_del_bg, c.diff_del_gutter, c.diff_del_word)),
        OpKind::Same => None,
    }
}

fn side(op: Option<&DiffOp>, old: bool, cx: &App) -> AnyElement {
    let c = &cx.tokens().colors;
    let base = div().flex().flex_1().min_w_0();
    let Some(op) = op else {
        return base.bg(hsla(c.diff_empty_bg)).into_any_element();
    };
    let k = colors(op.k, c);
    base.when_some(k, |d, (bg, _, _)| d.bg(hsla(bg)))
        .child(gutter(if old { op.a } else { op.b }, k.map(|x| x.1), cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pl(sp(2))
                .pr(sp(2))
                .text_color(hsla(c.syn_plain))
                .child(line_text(op, k.map(|x| x.2).unwrap_or(c.surface_100))),
        )
        .into_any_element()
}

/// Renders every row but `Row::File`. `banner` gives a file's banner text; `on_gap` expands a
/// collapsed run.
pub fn render(
    row: &Row,
    ix: usize,
    banner: impl Fn(usize) -> String,
    on_gap: impl Fn((usize, usize), &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let c = &cx.tokens().colors;
    let line_h = px(contextual_theme::type_style::CODE.line_height);
    let base = div()
        .w_full()
        .h(line_h)
        .type_style(contextual_theme::type_style::CODE);
    match row {
        Row::File(_) | Row::Spacer => base.into_any_element(),
        Row::Banner(f) => base
            .px(sp(3))
            .flex()
            .items_center()
            .bg(hsla(c.warning_bg))
            .type_style(contextual_theme::type_style::UI_SMALL)
            .text_color(hsla(c.ink))
            .child(SharedString::from(banner(*f)))
            .into_any_element(),
        Row::Hunk(h) => base
            .px(sp(3))
            .flex()
            .items_center()
            .bg(hsla(c.diff_hunk_bg))
            .text_color(hsla(c.ink_muted))
            .type_style(contextual_theme::type_style::CODE_SMALL)
            .child(SharedString::from(h.clone()))
            .into_any_element(),
        Row::Gap { file, item, lines } => {
            let key = (*file, *item);
            base.id(("gap", ix))
                .px(sp(3))
                .flex()
                .items_center()
                .bg(hsla(c.diff_hunk_bg))
                .text_color(hsla(c.accent_ink))
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .cursor_pointer()
                .hover(|s| s.bg(hsla(c.surface_hover)))
                .on_click(move |_, _, cx| on_gap(key, cx))
                .child(SharedString::from(format!("⋯ Expand {lines} lines")))
                .into_any_element()
        }
        Row::Unified(op) => {
            let k = colors(op.k, c);
            base.flex()
                .when_some(k, |d, (bg, _, _)| d.bg(hsla(bg)))
                .child(gutter(op.a, k.map(|x| x.1), cx))
                .child(gutter(op.b, k.map(|x| x.1), cx))
                .child(
                    div()
                        .w(px(16.0))
                        .flex_none()
                        .pl(sp(1))
                        .text_color(hsla(c.ink_muted))
                        .child(op.k.sign().to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(hsla(c.syn_plain))
                        .child(line_text(op, k.map(|x| x.2).unwrap_or(c.surface_100))),
                )
                .into_any_element()
        }
        // Split lines wrap inside their half, so both sides stay aligned (ScreenDiffSplit).
        Row::Split(old, new) => div()
            .w_full()
            .min_h(line_h)
            .type_style(contextual_theme::type_style::CODE)
            .flex()
            .child(side(old.as_ref(), true, cx))
            .child(div().w(px(1.0)).bg(hsla(c.border)))
            .child(side(new.as_ref(), false, cx))
            .into_any_element(),
    }
}
