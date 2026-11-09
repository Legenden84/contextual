//! Save (design specs: ApplyDialog, ScreenApply): what will happen to each selected file before
//! anything is written. An undo snapshot is always kept; files left out are listed as skipped.

use contextual_core::logic::types::ChangeStatus;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div,
};

use crate::state::AppModel;
use crate::state::chat::ChatModel;
use crate::theme::{ActiveTheme, TypeExt, hsla, px, shadow, sp};
use crate::ui::{Button, ButtonVariant};
use crate::views::changes_panel::status_color;

const W: f32 = 560.0;

pub struct SaveDialog {
    app: Entity<AppModel>,
    chat: Entity<ChatModel>,
    focus: FocusHandle,
    _sub: Subscription,
}

impl SaveDialog {
    pub fn new(
        app: Entity<AppModel>,
        chat: Entity<ChatModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let sub = cx.observe(&chat, |_, _, cx| cx.notify());
        Self {
            app,
            chat,
            focus,
            _sub: sub,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.chat.update(cx, |c, cx| {
            c.save_dialog_open = false;
            cx.notify();
        });
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let app = self.app.clone();
        self.chat.update(cx, |c, cx| {
            c.save(move |cx: &mut App| app.update(cx, |a, cx| a.rescan(cx)), cx)
        });
    }
}

impl Render for SaveDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let chat = self.chat.read(cx);
        let changes = chat.changes();
        let (picked, skipped): (Vec<_>, Vec<_>) = changes
            .iter()
            .partition(|ch| chat.include.get(&ch.path).copied().unwrap_or(false));
        let n = picked.len();
        let saving = chat.saving;
        let error = chat.save_error.clone();
        let branch = self
            .app
            .read(cx)
            .project
            .as_ref()
            .and_then(|p| p.read(cx).branch.clone());

        let row = |verb: &'static str,
                   color: contextual_theme::Rgba,
                   path: String,
                   note: Option<String>| {
            div()
                .flex()
                .items_center()
                .gap(sp(3))
                .py(px(3.0))
                .child(
                    div()
                        .w(px(72.0))
                        .flex_none()
                        .text_color(hsla(color))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .child(verb),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .child(SharedString::from(path)),
                )
                .when_some(note, |d, note| {
                    d.child(
                        div()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(note)),
                    )
                })
        };
        let picked_rows: Vec<_> = picked
            .iter()
            .map(|ch| {
                let verb = match ch.status {
                    ChangeStatus::A => "create",
                    ChangeStatus::M => "overwrite",
                    ChangeStatus::D => "delete",
                };
                row(verb, status_color(ch.status, c), ch.path.clone(), None)
            })
            .collect();
        let skipped_rows: Vec<_> = skipped
            .iter()
            .map(|ch| {
                let why = if ch.conflict.is_some() {
                    "changed on disk"
                } else {
                    "not selected"
                };
                row("skip", c.ink_faint, ch.path.clone(), Some(why.into()))
            })
            .collect();

        div()
            .id("save-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(c.surface_scrim))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|t, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    t.close(cx);
                }
            }))
            .on_click(cx.listener(|t, _, _, cx| {
                cx.stop_propagation();
                t.close(cx);
            }))
            .child(
                div()
                    .id("save-dialog")
                    .w(px(W))
                    .max_w_full()
                    .max_h(gpui_kit::relative(0.9))
                    .flex()
                    .flex_col()
                    .rounded(px(contextual_theme::radius::LG))
                    .border_1()
                    .border_color(hsla(c.border_strong))
                    .bg(hsla(c.surface_100))
                    .shadow(vec![shadow(cx.tokens().shadows.modal)])
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .type_style(contextual_theme::type_style::UI_BODY)
                    .child(
                        div()
                            .px(sp(4))
                            .pt(sp(4))
                            .pb(sp(2))
                            .child(div().type_style(contextual_theme::type_style::UI_TITLE).text_color(hsla(c.ink_strong)).child("Save changes"))
                            .child(
                                div()
                                    .mt(sp(1))
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .text_color(hsla(c.ink_muted))
                                    .child(SharedString::from(format!(
                                        "Written in place, keeping each file's line endings and BOM. Nothing is staged or committed{}.",
                                        branch.map(|b| format!(" on {b}")).unwrap_or_default()
                                    ))),
                            ),
                    )
                    .child(div().id("save-rows").flex_1().min_h_0().overflow_y_scroll().px(sp(4)).py(sp(2)).children(picked_rows).children(skipped_rows))
                    .child(
                        div()
                            .px(sp(4))
                            .py(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("An undo snapshot of the originals is kept, so Undo can restore them."),
                    )
                    .when_some(error, |d, e| {
                        d.child(div().px(sp(4)).pb(sp(2)).type_style(contextual_theme::type_style::UI_SMALL).text_color(hsla(c.danger)).child(SharedString::from(e)))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(sp(2))
                            .px(sp(4))
                            .py(sp(3))
                            .border_t_1()
                            .border_color(hsla(c.border))
                            .child(Button::new("cancel", "Cancel").on_click(cx.listener(|t, _, _, cx| t.close(cx))))
                            .child(
                                Button::new("confirm", if saving { "Saving…".to_string() } else { format!("Save {n} {}", if n == 1 { "file" } else { "files" }) })
                                    .variant(ButtonVariant::Primary)
                                    .disabled(saving || n == 0)
                                    .on_click(cx.listener(|t, _, _, cx| t.save(cx))),
                            ),
                    ),
            )
    }
}
