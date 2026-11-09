//! Changes (design spec: ChangesPanel): the files the latest reply proposes, each selectable for
//! saving, with counts, +/− and conflicts; Open diff and Save N files….

use contextual_core::logic::types::ChangeStatus;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
};

use crate::state::chat::{CenterView, ChatModel};
use crate::theme::{ActiveTheme, TypeExt, hsla, px, row_height, sp};
use crate::ui::{Button, ButtonVariant, Checkbox, pane_header};
use crate::views::diff_cache::{DiffCache, ensure_message, message_key};

pub struct ChangesPanel {
    chat: Entity<ChatModel>,
    diffs: DiffCache,
    _subs: Vec<Subscription>,
}

pub struct OpenDiffFile {
    pub path: String,
}

impl EventEmitter<OpenDiffFile> for ChangesPanel {}

pub fn status_color(s: ChangeStatus, c: &contextual_theme::Colors) -> contextual_theme::Rgba {
    match s {
        ChangeStatus::A => c.success,
        ChangeStatus::M => c.modified,
        ChangeStatus::D => c.danger,
    }
}

pub fn status_letter(s: ChangeStatus) -> &'static str {
    match s {
        ChangeStatus::A => "A",
        ChangeStatus::M => "M",
        ChangeStatus::D => "D",
    }
}

impl ChangesPanel {
    pub fn new(chat: Entity<ChatModel>, diffs: DiffCache, cx: &mut Context<Self>) -> Self {
        let subs = vec![
            cx.observe(&chat, |_, _, cx| cx.notify()),
            cx.observe(&diffs, |_, _, cx| cx.notify()),
        ];
        Self {
            chat,
            diffs,
            _subs: subs,
        }
    }
}

impl Render for ChangesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ensure_message(&self.diffs, &self.chat, None, cx);
        let c = &cx.tokens().colors;
        let chat = self.chat.read(cx);
        let changes = chat.changes_ref();
        let ready = chat
            .change_message()
            .and_then(|m| self.diffs.read(cx).get(&message_key(chat, m)));
        let picked = changes
            .iter()
            .filter(|ch| chat.include.get(&ch.path).copied().unwrap_or(false))
            .count();
        let count = |s| changes.iter().filter(|ch| ch.status == s).count();
        let (mut adds, mut dels) = (0, 0);
        let rows: Vec<_> = changes
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let diff = ready.as_ref().and_then(|diffs| diffs.get(i));
                if let Some(diff) = diff {
                    adds += diff.adds;
                    dels += diff.dels;
                }
                let on = chat.include.get(&ch.path).copied().unwrap_or(false);
                let (dir, name) = match ch.path.rsplit_once('/') {
                    Some((dir, name)) => (dir.to_string(), name.to_string()),
                    None => (String::new(), ch.path.clone()),
                };
                let path = ch.path.clone();
                let diff_path = path.clone();
                div()
                    .id(("change", i))
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .h(row_height())
                    .px(sp(3))
                    .type_style(contextual_theme::type_style::UI_BODY)
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(c.surface_hover)))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(OpenDiffFile {
                            path: diff_path.clone(),
                        });
                    }))
                    .child(
                        Checkbox::new(
                            ("include", i),
                            if on {
                                crate::state::project::CheckState::On
                            } else {
                                crate::state::project::CheckState::Off
                            },
                        )
                        .on_toggle(cx.listener(move |t, _, _, cx| {
                            t.chat.update(cx, |c, cx| c.toggle_include(&path, cx))
                        })),
                    )
                    .child(
                        div()
                            .w(px(12.0))
                            .flex_none()
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(hsla(status_color(ch.status, c)))
                            .child(status_letter(ch.status)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(sp(1))
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(SharedString::from(name))
                            .child(
                                div()
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .text_color(hsla(c.ink_muted))
                                    .child(SharedString::from(dir)),
                            ),
                    )
                    .when(ch.conflict.is_some(), |d| {
                        d.child(
                            div()
                                .text_color(hsla(c.warning))
                                .child(Icon::new(IconName::TriangleAlert).size(px(14.0))),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(sp(1))
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .child(
                                div().text_color(hsla(c.success)).child(SharedString::from(
                                    diff.map(|d| format!("+{}", d.adds))
                                        .unwrap_or_else(|| "…".into()),
                                )),
                            )
                            .child(
                                div().text_color(hsla(c.danger)).child(SharedString::from(
                                    diff.map(|d| format!("−{}", d.dels))
                                        .unwrap_or_else(|| "…".into()),
                                )),
                            ),
                    )
            })
            .collect();
        let conflicts = changes.iter().any(|ch| ch.conflict.is_some());
        let summary = [
            (ChangeStatus::A, "added"),
            (ChangeStatus::M, "modified"),
            (ChangeStatus::D, "deleted"),
        ]
        .into_iter()
        .filter(|(s, _)| count(*s) > 0)
        .map(|(s, w)| format!("{} {w}", count(s)))
        .collect::<Vec<_>>()
        .join(" · ");

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .min_w_0()
            .min_h_0()
            .h_full()
            .border_l_1()
            .border_color(hsla(c.border))
            .bg(hsla(c.surface_000))
            .child(pane_header("Changes", div(), cx))
            .child(
                div()
                    .flex()
                    .gap(sp(2))
                    .px(sp(3))
                    .py(sp(2))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(SharedString::from(summary))
                    .child(div().flex_1())
                    .child(div().text_color(hsla(c.success)).child(SharedString::from(
                        if ready.is_some() { format!("+{adds}") } else { "…".into() },
                    )))
                    .child(div().text_color(hsla(c.danger)).child(SharedString::from(
                        if ready.is_some() { format!("−{dels}") } else { "…".into() },
                    ))),
            )
            .child(div().id("changes-list").flex_1().min_h_0().overflow_y_scroll().children(rows))
            .when(conflicts, |d| {
                d.child(
                    div()
                        .mx(sp(3))
                        .mb(sp(2))
                        .px(sp(2))
                        .py(sp(1))
                        .rounded(px(contextual_theme::radius::SM))
                        .bg(hsla(c.warning_bg))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .child("Files changed on disk since the model saw them are left out. Open the diff to check them."),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(sp(2))
                    .flex_wrap()
                    .p(sp(3))
                    .border_t_1()
                    .border_color(hsla(c.border))
                    .child(Button::new("open-diff", "Open diff").on_click(cx.listener(|t, _, _, cx| {
                        t.chat.update(cx, |c, cx| {
                            c.set_center(CenterView::Diff, cx);
                        })
                    })))
                    .child(div().flex_1())
                    .child(
                        Button::new("save", format!("Save {picked} {}…", if picked == 1 { "file" } else { "files" }))
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
    }
}
