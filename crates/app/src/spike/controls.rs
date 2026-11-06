//! Prototype 6: a dropdown, a modal dialog and a toast, all styled from the tokens through the
//! gpui-component theme set in `theme.rs`.

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div, px,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonVariant};

pub struct ControlsProto {
    deployment: Entity<SelectState<SearchableVec<SharedString>>>,
}

impl ControlsProto {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let items = SearchableVec::new(vec![
            SharedString::from("gpt-5.1 (400k)"),
            SharedString::from("gpt-5.1-mini (400k)"),
            SharedString::from("o4-mini (200k)"),
        ]);
        let deployment = cx.new(|cx| SelectState::new(items, Some(Default::default()), window, cx));
        Self { deployment }
    }
}

impl Render for ControlsProto {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let label = |t: &'static str| {
            div()
                .type_style(contextual_theme::type_style::UI_CAPTION)
                .text_color(hsla(c.ink_muted))
                .child(t)
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(sp(6))
            .p(sp(8))
            .child(div().type_style(contextual_theme::type_style::UI_DISPLAY).text_color(hsla(c.ink_strong)).child("Contextual"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(sp(2))
                    .w(px(contextual_theme::layout::CHANGES_WIDTH))
                    .child(label("DEPLOYMENT"))
                    .child(Select::new(&self.deployment).menu_width(px(contextual_theme::layout::CHANGES_WIDTH))),
            )
            .child(
                div()
                    .flex()
                    .gap(sp(2))
                    .child(Button::new("apply", "Save 4 files…").variant(ButtonVariant::Primary).on_click(|_, window, cx| {
                        window.open_dialog(cx, |dialog, _, cx| {
                            let c = &cx.tokens().colors;
                            dialog
                                .title("Save changes")
                                .w(px(520.0))
                                .content(move |content, _, cx| {
                                    let c = &cx.tokens().colors;
                                    content.child(
                                        div()
                                            .type_style(contextual_theme::type_style::UI_BODY)
                                            .text_color(hsla(c.ink))
                                            .child("4 files will be saved in place. Each keeps its line endings and BOM; you can undo the save."),
                                    )
                                })
                                .footer(
                                    div()
                                        .flex()
                                        .justify_end()
                                        .gap(sp(2))
                                        .text_color(hsla(c.ink))
                                        .child(Button::new("cancel", "Cancel").on_click(|_, w, cx| w.close_dialog(cx)))
                                        .child(Button::new("ok", "Save").variant(ButtonVariant::Primary).on_click(|_, w, cx| {
                                            w.close_dialog(cx);
                                            w.push_notification(Notification::success("Saved 4 files.").title("Saved"), cx);
                                        })),
                                )
                        });
                    }))
                    .child(Button::new("toast", "Show toast").on_click(|_, window, cx| {
                        window.push_notification(Notification::info("Context changed: 3 files were edited on disk."), cx);
                    }))
                    .child(Button::new("ghost", "Ghost").variant(ButtonVariant::Ghost))
                    .child(Button::new("disabled", "Disabled").disabled(true)),
            )
    }
}
