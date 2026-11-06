use gpui_kit::{App, IntoElement, ParentElement, SharedString, Styled, div};

use crate::theme::{ActiveTheme, TypeExt, hsla, row_height, sp};

/// Uppercase pane title bar (EXPLORER, CHANGES) with optional trailing actions.
pub fn pane_header(
    title: impl Into<SharedString>,
    actions: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    let c = &cx.tokens().colors;
    let title: SharedString = title.into();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .h(row_height() + sp(2))
        .px(sp(3))
        .border_b_1()
        .border_color(hsla(c.border))
        .child(
            div()
                .type_style(contextual_theme::type_style::UI_CAPTION)
                .text_color(hsla(c.ink_muted))
                .child(SharedString::from(title.to_uppercase())),
        )
        .child(actions)
}
