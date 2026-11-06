//! The diff view for one response (design specs: ScreenDiff, ScreenDiffSplit, DiffFile).
//! Every visual line of every file is one row of a virtualised `list`, so large diffs stay smooth.

use std::collections::HashSet;

use contextual_core::logic::diff::{
    DiffItem, DiffOp, OpKind, hunk_header, make_hunks, unified_patch,
};
use contextual_core::logic::types::{Change, ChangeStatus};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Entity, HighlightStyle, InteractiveElement,
    IntoElement, ListAlignment, ListState, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Subscription, Window, div, list, px,
};

use crate::state::chat::{CenterView, ChatModel, DiffMode};
use crate::state::project::CheckState;
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, Checkbox};
use crate::views::changes_panel::{status_color, status_letter};
use crate::views::diff_cache::DiffCache;

const CONTEXT: usize = 3;
const GUTTER: f32 = 44.0;
const FILE_LIST_W: f32 = 240.0;
const TOOLBAR_H: f32 = 36.0;
const FILE_HEADER_H: f32 = 34.0;

enum Row {
    File(usize),
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

pub struct DiffScreen {
    chat: Entity<ChatModel>,
    diffs: DiffCache,
    changes: Vec<Change>,
    open_gaps: HashSet<(usize, usize)>,
    expand_all: bool,
    /// The file list in split mode (folded away by default to give each side room).
    split_files: bool,
    rows: Vec<Row>,
    /// Row index of each file's header, for the file list.
    file_rows: Vec<usize>,
    list: ListState,
    _sub: Subscription,
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

impl DiffScreen {
    pub fn new(chat: Entity<ChatModel>, diffs: DiffCache, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&chat, |this: &mut Self, _, cx| {
            this.rebuild(cx);
            cx.notify();
        });
        let mut this = Self {
            chat,
            diffs,
            changes: Vec::new(),
            open_gaps: HashSet::new(),
            expand_all: false,
            split_files: false,
            rows: Vec::new(),
            file_rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(600.0)),
            _sub: sub,
        };
        this.rebuild(cx);
        this
    }

    fn mode(&self, cx: &App) -> DiffMode {
        self.chat.read(cx).diff_mode
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.changes = self.chat.read(cx).changes();
        let split = self.mode(cx) == DiffMode::Split;
        let mut rows = Vec::new();
        let mut file_rows = Vec::new();
        for (f, ch) in self.changes.iter().enumerate() {
            file_rows.push(rows.len());
            rows.push(Row::File(f));
            if ch.conflict.is_some() {
                rows.push(Row::Banner(f));
            }
            let diff = self.diffs.get(ch);
            for (i, item) in make_hunks(&diff.ops, CONTEXT).into_iter().enumerate() {
                match item {
                    DiffItem::Gap { lines }
                        if !self.expand_all && !self.open_gaps.contains(&(f, i)) =>
                    {
                        rows.push(Row::Gap {
                            file: f,
                            item: i,
                            lines: lines.len(),
                        })
                    }
                    DiffItem::Gap { lines } => {
                        if split {
                            pair(&lines, &mut rows)
                        } else {
                            rows.extend(lines.into_iter().map(Row::Unified))
                        }
                    }
                    DiffItem::Hunk { lines } => {
                        rows.push(Row::Hunk(hunk_header(&lines, &diff.new_lines)));
                        if split {
                            pair(&lines, &mut rows)
                        } else {
                            rows.extend(lines.into_iter().map(Row::Unified))
                        }
                    }
                }
            }
            rows.push(Row::Spacer);
        }
        self.list.reset(rows.len());
        self.rows = rows;
        self.file_rows = file_rows;
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

    fn colors(
        k: OpKind,
        c: &contextual_theme::Colors,
    ) -> Option<(
        contextual_theme::Rgba,
        contextual_theme::Rgba,
        contextual_theme::Rgba,
    )> {
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
        let colors = Self::colors(op.k, c);
        base.when_some(colors, |d, (bg, _, _)| d.bg(hsla(bg)))
            .child(Self::gutter(
                if old { op.a } else { op.b },
                colors.map(|x| x.1),
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(sp(2))
                    .pr(sp(2))
                    .text_color(hsla(c.syn_plain))
                    .child(Self::line_text(
                        op,
                        colors.map(|x| x.2).unwrap_or(c.surface_100),
                    )),
            )
            .into_any_element()
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let c = &cx.tokens().colors;
        let line_h = px(contextual_theme::type_style::CODE.line_height);
        let row = div()
            .w_full()
            .h(line_h)
            .type_style(contextual_theme::type_style::CODE);
        match &self.rows[ix] {
            Row::Spacer => row.into_any_element(),
            Row::File(f) => self.file_header(*f, cx),
            Row::Banner(f) => row
                .px(sp(3))
                .flex()
                .items_center()
                .bg(hsla(c.warning_bg))
                .type_style(contextual_theme::type_style::UI_SMALL)
                .text_color(hsla(c.ink))
                .child(SharedString::from(
                    self.changes[*f].conflict.clone().unwrap_or_default(),
                ))
                .into_any_element(),
            Row::Hunk(h) => row
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
                row.id(("gap", ix))
                    .px(sp(3))
                    .flex()
                    .items_center()
                    .bg(hsla(c.diff_hunk_bg))
                    .text_color(hsla(c.accent_ink))
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(c.surface_hover)))
                    .on_click(cx.listener(move |t, _, _, cx| {
                        t.open_gaps.insert(key);
                        t.rebuild(cx);
                        cx.notify();
                    }))
                    .child(SharedString::from(format!("⋯ Expand {lines} lines")))
                    .into_any_element()
            }
            Row::Unified(op) => {
                let colors = Self::colors(op.k, c);
                row.flex()
                    .when_some(colors, |d, (bg, _, _)| d.bg(hsla(bg)))
                    .child(Self::gutter(op.a, colors.map(|x| x.1), cx))
                    .child(Self::gutter(op.b, colors.map(|x| x.1), cx))
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
                            .child(Self::line_text(
                                op,
                                colors.map(|x| x.2).unwrap_or(c.surface_100),
                            )),
                    )
                    .into_any_element()
            }
            Row::Split(old, new) => div()
                .w_full()
                .min_h(line_h)
                .type_style(contextual_theme::type_style::CODE)
                .flex()
                .child(Self::side(old.as_ref(), true, cx))
                .child(div().w(px(1.0)).h_full().bg(hsla(c.border)))
                .child(Self::side(new.as_ref(), false, cx))
                .into_any_element(),
        }
    }

    fn file_header(&self, f: usize, cx: &mut Context<Self>) -> AnyElement {
        let c = &cx.tokens().colors;
        let ch = &self.changes[f];
        let d = self.diffs.get(ch);
        let on = self
            .chat
            .read(cx)
            .include
            .get(&ch.path)
            .copied()
            .unwrap_or(false);
        let (path, content, patch) = (
            ch.path.clone(),
            ch.new.clone(),
            unified_patch(&ch.path, &d, ch.status),
        );
        let path_toggle = path.clone();
        div()
            .w_full()
            .h(px(FILE_HEADER_H))
            .flex()
            .items_center()
            .gap(sp(2))
            .px(sp(3))
            .bg(hsla(c.surface_200))
            .border_t_1()
            .border_b_1()
            .border_color(hsla(c.border))
            .type_style(contextual_theme::type_style::CODE_SMALL)
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(hsla(status_color(ch.status, c)))
                    .child(status_letter(ch.status)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_color(hsla(c.ink))
                    .child(SharedString::from(path)),
            )
            .child(
                div()
                    .text_color(hsla(c.success))
                    .child(SharedString::from(format!("+{}", d.adds))),
            )
            .child(
                div()
                    .text_color(hsla(c.danger))
                    .child(SharedString::from(format!("−{}", d.dels))),
            )
            .when(ch.status != ChangeStatus::D, |d| {
                d.child(
                    Button::new(("copy-file", f), "Copy file")
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(content.clone()))
                        }),
                )
            })
            .child(
                Button::new(("copy-diff", f), "Copy diff")
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(patch.clone()))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(sp(1))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .child(
                        Checkbox::new(
                            ("inc", f),
                            if on { CheckState::On } else { CheckState::Off },
                        )
                        .on_toggle(cx.listener(move |t, _, _, cx| {
                            t.chat
                                .update(cx, |c, cx| c.toggle_include(&path_toggle, cx))
                        })),
                    )
                    .child("Include in save"),
            )
            .into_any_element()
    }
}

impl Render for DiffScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let mode = self.mode(cx);
        let chat = self.chat.read(cx);
        let picked = self
            .changes
            .iter()
            .filter(|ch| chat.include.get(&ch.path).copied().unwrap_or(false))
            .count();
        let (adds, dels) = self
            .changes
            .iter()
            .map(|ch| self.diffs.get(ch))
            .fold((0, 0), |(a, d), x| (a + x.adds, d + x.dels));
        let show_files = mode == DiffMode::Unified || self.split_files;
        let files: Vec<_> = self
            .changes
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let row = self.file_rows.get(i).copied().unwrap_or(0);
                let name = ch.path.rsplit('/').next().unwrap_or(&ch.path).to_string();
                div()
                    .id(("file-link", i))
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .h(px(contextual_theme::space::ROW_HEIGHT))
                    .px(sp(3))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(c.surface_hover)))
                    .on_click(cx.listener(move |t, _, _, cx| {
                        t.list.scroll_to(gpui_kit::ListOffset {
                            item_ix: row,
                            offset_in_item: px(0.0),
                        });
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(px(12.0))
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .text_color(hsla(status_color(ch.status, c)))
                            .child(status_letter(ch.status)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(SharedString::from(name)),
                    )
            })
            .collect();
        let toggle = |id: &'static str, label: &'static str, on: bool| {
            Button::new(id, label)
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Ghost)
                .selected(on)
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .bg(hsla(c.surface_100))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(sp(2))
                    .h(px(TOOLBAR_H))
                    .px(sp(3))
                    .border_b_1()
                    .border_color(hsla(c.border))
                    .bg(hsla(c.surface_000))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .child(
                        Button::new("back", "Back to chat")
                            .size(ButtonSize::Xs)
                            .on_click(cx.listener(|t, _, _, cx| {
                                t.chat.update(cx, |c, cx| {
                                    c.center = CenterView::Chat;
                                    cx.notify();
                                })
                            })),
                    )
                    .child(SharedString::from(format!(
                        "{} {}",
                        self.changes.len(),
                        if self.changes.len() == 1 {
                            "file"
                        } else {
                            "files"
                        }
                    )))
                    .child(
                        div()
                            .text_color(hsla(c.success))
                            .child(SharedString::from(format!("+{adds}"))),
                    )
                    .child(
                        div()
                            .text_color(hsla(c.danger))
                            .child(SharedString::from(format!("−{dels}"))),
                    )
                    .child(div().flex_1())
                    .when(mode == DiffMode::Split, |d| {
                        d.child(
                            toggle("files", "Files", self.split_files).on_click(cx.listener(
                                |t, _, _, cx| {
                                    t.split_files = !t.split_files;
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .child(
                        toggle("unified", "Unified", mode == DiffMode::Unified).on_click(
                            cx.listener(|t, _, _, cx| {
                                t.chat.update(cx, |c, cx| {
                                    c.diff_mode = DiffMode::Unified;
                                    cx.notify();
                                })
                            }),
                        ),
                    )
                    .child(
                        toggle("split", "Split", mode == DiffMode::Split).on_click(cx.listener(
                            |t, _, _, cx| {
                                t.chat.update(cx, |c, cx| {
                                    c.diff_mode = DiffMode::Split;
                                    cx.notify();
                                })
                            },
                        )),
                    )
                    .child(
                        toggle("expand-all", "Expand all", self.expand_all).on_click(cx.listener(
                            |t, _, _, cx| {
                                t.expand_all = !t.expand_all;
                                t.rebuild(cx);
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        Button::new(
                            "save",
                            format!(
                                "Save {picked} {}…",
                                if picked == 1 { "file" } else { "files" }
                            ),
                        )
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Primary)
                        .disabled(picked == 0)
                        .on_click(cx.listener(|t, _, _, cx| {
                            t.chat.update(cx, |c, cx| {
                                c.save_dialog_open = true;
                                c.save_error = None;
                                cx.notify();
                            })
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .when(show_files, |d| {
                        d.child(
                            div()
                                .id("diff-files")
                                .flex_none()
                                .w(px(FILE_LIST_W))
                                .h_full()
                                .overflow_y_scroll()
                                .py(sp(1))
                                .border_r_1()
                                .border_color(hsla(c.border))
                                .bg(hsla(c.surface_000))
                                .type_style(contextual_theme::type_style::UI_BODY)
                                .children(files),
                        )
                    })
                    .child(
                        list(self.list.clone(), {
                            let weak = cx.entity().downgrade();
                            move |ix, _, cx| {
                                weak.upgrade()
                                    .map(|e| e.update(cx, |t, cx| t.render_row(ix, cx)))
                                    .unwrap_or_else(|| div().into_any_element())
                            }
                        })
                        .flex_1()
                        .min_w_0(),
                    ),
            )
    }
}
