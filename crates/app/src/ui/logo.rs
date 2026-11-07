use gpui_kit::{App, IntoElement, ParentElement, Styled, div, px, svg};

use crate::theme::{ActiveTheme, hsla};

/// The supplied Moss mark keeps its Moss/Algae colours even when previewed in another theme.
pub fn moss_mark(size: f32) -> impl IntoElement {
    let c = &contextual_theme::Theme::MOSS.colors;
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(hsla(c.surface_selected))
        .child(
            svg()
                .path(crate::assets::MOSS_ICON_PATH)
                .size_full()
                .text_color(hsla(c.ink)),
        )
}

/// The Contextual mark on a 16-unit grid: an accent sheet, an outlined sheet over it, and two
/// success-coloured lines (the old app's `Logo`), drawn from the theme tokens.
pub fn logo(size: f32, cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    let u = size / 16.0;
    let at = |x: f32, y: f32, w: f32, h: f32| {
        div()
            .absolute()
            .left(px(x * u))
            .top(px(y * u))
            .w(px(w * u))
            .h(px(h * u))
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(
            at(1.0, 1.5, 9.0, 11.0)
                .rounded(px(2.0 * u))
                .bg(hsla(c.accent)),
        )
        .child(
            at(6.0, 3.5, 9.0, 11.0)
                .rounded(px(2.0 * u))
                .bg(hsla(c.surface_100))
                .border(px((1.2 * u).max(1.0)))
                .border_color(hsla(c.ink_muted)),
        )
        .child(at(8.5, 6.8, 4.0, 1.4).rounded_full().bg(hsla(c.success)))
        .child(at(8.5, 9.3, 2.5, 1.4).rounded_full().bg(hsla(c.success)))
}
