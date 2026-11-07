//! The diff view for one response (design specs: ScreenDiff, ScreenDiffSplit, DiffFile).
//! Every visual line of every file is one row of a virtualised `list`, so large diffs stay smooth.

use std::collections::HashSet;
use std::sync::Arc;

use contextual_core::logic::diff::{Diff, unified_patch};
use contextual_core::logic::types::{Change, ChangeStatus};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Context, Entity, InteractiveElement, IntoElement,
    ListAlignment, ListState, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Window, div, list, px,
};

use crate::state::chat::{CenterView, ChatModel, DiffMode};
use crate::state::project::CheckState;
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, Checkbox};
use crate::views::changes_panel::{status_color, status_letter};
use crate::views::diff_cache::{DiffCache, ensure_message, message_key};
use crate::views::diff_rows::{self, FileRows, Row};

const FILE_LIST_W: f32 = 240.0;
const TOOLBAR_H: f32 = 36.0;
const FILE_HEADER_H: f32 = 34.0;

pub struct DiffScreen {
    chat: Entity<ChatModel>,
    diffs: DiffCache,
    changes: Arc<[Change]>,
    source: Option<String>,
    ready: Option<Arc<Vec<Diff>>>,
    built_mode: DiffMode,
    open_gaps: HashSet<(usize, usize)>,
    expand_all: bool,
    /// The file list in split mode (folded away by default to give each side room).
    split_files: bool,
    rows: Vec<Row>,
    /// Row index of each file's header, for the file list.
    file_rows: Vec<usize>,
    list: ListState,
    _subs: Vec<Subscription>,
}

impl DiffScreen {
    pub fn new(chat: Entity<ChatModel>, diffs: DiffCache, cx: &mut Context<Self>) -> Self {
        let subs = vec![
            cx.observe(&chat, |this: &mut Self, _, cx| {
                if this.chat.read(cx).center(cx) == CenterView::Diff {
                    this.refresh(cx);
                    cx.notify();
                }
            }),
            cx.observe(&diffs, |this: &mut Self, _, cx| {
                if this.chat.read(cx).center(cx) == CenterView::Diff {
                    this.refresh(cx);
                    cx.notify();
                }
            }),
        ];
        Self {
            chat,
            diffs,
            changes: Vec::new().into(),
            source: None,
            ready: None,
            built_mode: DiffMode::Unified,
            open_gaps: HashSet::new(),
            expand_all: false,
            split_files: false,
            rows: Vec::new(),
            file_rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(600.0)),
            _subs: subs,
        }
    }

    fn mode(&self, cx: &App) -> DiffMode {
        self.chat.read(cx).diff_mode
    }

    /// A streaming delta, checkbox, toast or dialog change does not change the row layout.
    /// Hidden screens do no work; entering the screen catches up with the current source.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let chat = self.chat.read(cx);
        if chat.center(cx) != CenterView::Diff {
            return;
        }
        let source = chat.change_message().map(|m| message_key(chat, m));
        if self.source != source {
            self.changes = chat.changes_ref().to_vec().into();
            self.source = source;
            self.ready = None;
            self.open_gaps.clear();
            self.rows.clear();
            self.file_rows.clear();
            self.list.reset(0);
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.ready.is_none() {
            ensure_message(&self.diffs, &self.chat, None, cx);
            self.ready = self.diffs.read(cx).get(&source);
            if self.ready.is_some() {
                self.rebuild(cx);
            }
        } else if self.built_mode != self.mode(cx) {
            self.rebuild(cx);
        }
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let Some(diffs) = self.ready.as_ref() else {
            return;
        };
        let split = self.mode(cx) == DiffMode::Split;
        let files: Vec<FileRows> = diffs
            .iter()
            .zip(self.changes.iter())
            .map(|(d, ch)| FileRows {
                diff: d,
                banner: ch.conflict.is_some(),
                hide_lines: false,
            })
            .collect();
        let (rows, file_rows) = diff_rows::build(&files, split, self.expand_all, &self.open_gaps);
        self.list.reset(rows.len());
        self.rows = rows;
        self.file_rows = file_rows;
        self.built_mode = self.mode(cx);
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match &self.rows[ix] {
            Row::File(f) => self.file_header(*f, cx),
            row => {
                let changes = &self.changes;
                let weak = cx.entity().downgrade();
                diff_rows::render(
                    row,
                    ix,
                    |f| changes[f].conflict.clone().unwrap_or_default(),
                    move |key, cx| {
                        weak.update(cx, |t, cx| {
                            t.open_gaps.insert(key);
                            t.rebuild(cx);
                            cx.notify();
                        })
                        .ok();
                    },
                    cx,
                )
            }
        }
    }

    fn file_header(&self, f: usize, cx: &mut Context<Self>) -> AnyElement {
        let c = &cx.tokens().colors;
        let ch = &self.changes[f];
        let Some(diffs) = self.ready.as_ref() else {
            return div().into_any_element();
        };
        let d = &diffs[f];
        let on = self
            .chat
            .read(cx)
            .include
            .get(&ch.path)
            .copied()
            .unwrap_or(false);
        let path = ch.path.clone();
        let path_toggle = path.clone();
        let copy_changes = self.changes.clone();
        let copy_diffs = diffs.clone();
        let patch_path = path.clone();
        let status = ch.status;
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
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                copy_changes[f].new.clone(),
                            ))
                        }),
                )
            })
            .child(
                Button::new(("copy-diff", f), "Copy diff")
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(unified_patch(
                            &patch_path,
                            &copy_diffs[f],
                            status,
                        )))
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
        self.refresh(cx);
        let c = &cx.tokens().colors;
        let mode = self.mode(cx);
        let chat = self.chat.read(cx);
        let picked = self
            .changes
            .iter()
            .filter(|ch| chat.include.get(&ch.path).copied().unwrap_or(false))
            .count();
        let (adds, dels) = self
            .ready
            .iter()
            .flat_map(|diffs| diffs.iter())
            .fold((0, 0), |(a, d), x| (a + x.adds, d + x.dels));
        let waiting = self.ready.is_none() && !self.changes.is_empty();
        let loading_text = self
            .source
            .as_deref()
            .and_then(|key| self.diffs.read(cx).error(key))
            .unwrap_or("Calculating diff…")
            .to_string();
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
                    .child(SharedString::from(format!(
                        "{} {}",
                        self.changes.len(),
                        if self.changes.len() == 1 {
                            "file"
                        } else {
                            "files"
                        }
                    )))
                    .when(self.ready.is_some(), |d| {
                        d.child(
                            div()
                                .text_color(hsla(c.success))
                                .child(SharedString::from(format!("+{adds}"))),
                        )
                        .child(
                            div()
                                .text_color(hsla(c.danger))
                                .child(SharedString::from(format!("−{dels}"))),
                        )
                    })
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
            .when(waiting, |d| {
                d.child(
                    div()
                        .px(sp(3))
                        .py(sp(2))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(loading_text)),
                )
            })
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
