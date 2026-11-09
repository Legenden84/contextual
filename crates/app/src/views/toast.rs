//! Toasts (design spec: Toast): persistent save confirmation with Undo and transient zoom feedback.

use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, Entity, IntoElement, ParentElement, SharedString, Styled, div};

use crate::state::AppModel;
use crate::state::chat::ChatModel;
use crate::theme::{ActiveTheme, TypeExt, hsla, px, shadow, sp};
use crate::ui::{IconButton, LinkButton};

const W: f32 = 360.0;

/// Shared floating surface; positioning and lifetime belong to each notification.
fn toast_surface(cx: &App) -> gpui_kit::Div {
    let c = &cx.tokens().colors;
    div()
        .flex()
        .gap(sp(3))
        .p(sp(3))
        .rounded(px(contextual_theme::radius::LG))
        .border_1()
        .border_color(hsla(c.border_strong))
        .bg(hsla(c.surface_200))
        .shadow(vec![shadow(cx.tokens().shadows.modal)])
        .type_style(contextual_theme::type_style::UI_BODY)
}

/// Separate from Save/Undo: zoom feedback never replaces an actionable notification.
pub fn zoom_toast(cx: &App) -> Option<impl IntoElement + use<>> {
    let percent = crate::theme::zoom_notice_percent(cx)?;
    Some(
        toast_surface(cx)
            .absolute()
            .right(sp(4))
            .top(sp(4))
            .text_color(hsla(cx.tokens().colors.ink_strong))
            .child(SharedString::from(format!("Zoom {percent}%"))),
    )
}

pub fn toast(
    app: &Entity<AppModel>,
    chat: &Entity<ChatModel>,
    cx: &App,
) -> Option<impl IntoElement + use<>> {
    let c = &cx.tokens().colors;
    let t = chat.read(cx).toast.clone()?;
    let (undo_chat, close_chat, app) = (chat.clone(), chat.clone(), app.clone());
    Some(
        toast_surface(cx)
            .absolute()
            .right(sp(4))
            .bottom(sp(4))
            .w(px(W))
            .child(
                div()
                    .text_color(hsla(if t.error { c.danger } else { c.success }))
                    .child(
                        Icon::new(if t.error {
                            IconName::CircleX
                        } else {
                            IconName::CircleCheck
                        })
                        .size(px(16.0)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(hsla(c.ink_strong))
                            .child(SharedString::from(t.title)),
                    )
                    .child(
                        div()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(t.detail)),
                    )
                    .when(t.backup_id.is_some(), |d| {
                        d.child(div().mt(sp(1)).child(LinkButton::new(
                            "undo",
                            "Undo",
                            move |_, _, cx| {
                                let app = app.clone();
                                undo_chat.update(cx, |c, cx| {
                                    c.undo(
                                        move |cx: &mut App| app.update(cx, |a, cx| a.rescan(cx)),
                                        cx,
                                    )
                                })
                            },
                        )))
                    }),
            )
            .child(
                IconButton::new("close-toast", IconName::Close, "Dismiss")
                    .on_click(move |_, _, cx| close_chat.update(cx, |c, cx| c.dismiss_toast(cx))),
            ),
    )
}
