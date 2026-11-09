use contextual_core::logic::lang::{Lang, lang_from_path};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::{App, FontWeight, IntoElement, ParentElement, SharedString, Styled, div};

use crate::theme::{ActiveTheme, MONO, hsla, px};

/// Glyph box width and type (the design's 16px box, 9px semibold mono).
const BOX: f32 = 16.0;
const SIZE: f32 = 9.0;
const ICON: f32 = 15.0;

/// Small coloured file-type glyph used in trees, tabs and headers.
pub fn file_glyph(name: &str, cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    let lower = name.to_ascii_lowercase();
    let text = |t: &'static str, color| Some((t, color));
    let glyph = if lower.ends_with(".sln") {
        text("sln", c.file_xml)
    } else if [
        ".png", ".jpg", ".jpeg", ".gif", ".ico", ".dll", ".exe", ".pdb",
    ]
    .iter()
    .any(|e| lower.ends_with(e))
    {
        None
    } else {
        match lang_from_path(Some(name)) {
            Lang::Csharp => text("C#", c.file_csharp),
            Lang::Json => text("{ }", c.file_json),
            Lang::Xml => text("</>", c.file_xml),
            Lang::Markdown => text("M↓", c.file_markdown),
            Lang::Yaml => text("!", c.file_xml),
            Lang::Typescript => text("TS", c.file_markdown),
            Lang::Python => text("py", c.file_markdown),
            Lang::Shell => text("$", c.ink_muted),
            Lang::Sql => text("db", c.file_json),
            Lang::Plain => None,
        }
    };
    let base = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(BOX));
    match glyph {
        Some((t, color)) => base
            .font_family(MONO)
            .text_size(px(SIZE))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(hsla(color))
            .child(SharedString::from(t)),
        None => base
            .text_color(hsla(c.ink_muted))
            .child(Icon::new(IconName::File).size(px(ICON))),
    }
}

/// A completed action, not an indication that a proposal has been saved. The editor-ground
/// token gives the check contrasting ink on the success fill in Dark, Light and Moss.
pub fn completed_mark(cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    div()
        .flex_none()
        .size(crate::theme::sp(4))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(contextual_theme::radius::FULL))
        .bg(hsla(c.success))
        .text_color(hsla(c.surface_100))
        .child(Icon::new(IconName::Check).size(crate::theme::sp(3)))
}

pub fn folder_glyph(open: bool, cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(BOX))
        .text_color(hsla(c.file_folder))
        .child(
            Icon::new(if open {
                IconName::FolderOpen
            } else {
                IconName::Folder
            })
            .size(px(ICON)),
        )
}
