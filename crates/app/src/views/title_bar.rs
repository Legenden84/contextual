//! The window's title bar (design README: "a title bar, a three-column body and a status bar"):
//! the mark, the window title centred, and a theme switch. Window controls, dragging and
//! double-click come from gpui-component's `TitleBar`, which draws them where the OS doesn't
//! (Linux with client-side decorations, Windows with a custom title bar) and leaves room for the
//! macOS traffic lights.

use contextual_theme::Appearance;
use gpui_kit::component::{IconName, TitleBar};
use gpui_kit::{App, IntoElement, ParentElement, SharedString, Styled, div};

use crate::ToggleTheme;
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{IconButton, logo, moss_mark};

/// Mark size in the title bar.
const LOGO: f32 = 16.0;

pub fn title_bar(title: &str, cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    let toggle = match cx.tokens().appearance {
        Appearance::Light => IconButton::new(
            "theme-toggle",
            IconName::Sun,
            "Light theme · Switch to Moss",
        ),
        Appearance::Moss => IconButton::with_glyph(
            "theme-toggle",
            moss_mark(LOGO),
            "Moss theme · Switch to dark",
        ),
        Appearance::Dark => IconButton::new(
            "theme-toggle",
            IconName::Moon,
            "Dark theme · Switch to light",
        ),
    };
    TitleBar::new()
        .bg(hsla(c.surface_000))
        .border_b_1()
        .border_color(hsla(c.border))
        .child(
            div()
                .flex()
                .flex_1()
                .items_center()
                .gap(sp(3))
                .type_style(contextual_theme::type_style::UI_SMALL)
                .child(logo(LOGO, cx))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .justify_center()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(title.to_string())),
                )
                .child(div().pr(sp(2)).child(
                    toggle.on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ToggleTheme), cx)
                    }),
                )),
        )
}
