//! Bridges `contextual-theme` (generated from tokens.json) into GPUI.
//!
//! - Views read colours, spacing and type styles through [`ActiveTheme::tokens`] and the
//!   helpers here; nothing in `crates/app` writes a literal colour (CI checks this).
//! - `gpui-component`'s own theme is set from the same tokens, so its widgets (inputs, dialogs,
//!   selects, notifications, markdown) match the design system.

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use contextual_theme::{
    Appearance, FontFamily, Rgba, Shadow, Theme as Tokens, TypeStyle, radius, space,
};
#[cfg(test)]
use gpui_kit::component::Rope;
use gpui_kit::component::highlighter::HighlightTheme;
#[cfg(test)]
use gpui_kit::component::highlighter::SyntaxHighlighter;
use gpui_kit::component::theme::{Theme as ComponentTheme, ThemeMode};
use gpui_kit::{
    App, BoxShadow, FontFeatures, FontWeight, Global, Hsla, ListState, Pixels, Styled, Window,
    point,
};
use serde_json::json;

/// Bundled font families (see `assets.rs`).
pub const SANS: &str = "Inter";
pub const MONO: &str = "JetBrains Mono";

thread_local! {
    // GPUI renders on its UI thread. Keep the scale here so existing context-free token
    // helpers and render-once primitives use the same application-wide scale. Background
    // work must not use UI dimensions. This is session state, not a project setting.
    static UI_ZOOM: Cell<u16> = const { Cell::new(100) };
}

pub fn zoom_percent() -> u16 {
    UI_ZOOM.with(Cell::get)
}

pub fn zoom_factor() -> f32 {
    f32::from(zoom_percent()) / 100.0
}

/// Converts a design-space pixel dimension to a GPUI logical-pixel dimension.
/// Use this in views; use `gpui_kit::px` for native window bounds and already-measured values.
/// OS display scaling remains GPUI's responsibility and is not included here.
pub fn px(value: f32) -> Pixels {
    gpui_kit::px(value * zoom_factor())
}

fn bounded_zoom(percent: i16) -> u16 {
    percent.clamp(50, 200) as u16
}

/// Transient feedback is shared by keyboard, menu and toolkit-popup zoom actions.
struct ZoomNotice {
    percent: u16,
    shown_at: Instant,
}

impl Global for ZoomNotice {}

const ZOOM_NOTICE_DURATION: Duration = Duration::from_secs(2);

pub fn zoom_notice_percent(cx: &App) -> Option<u16> {
    let notice = cx.try_global::<ZoomNotice>()?;
    (notice.shown_at.elapsed() < ZOOM_NOTICE_DURATION).then_some(notice.percent)
}

/// Updates both explicit pixel dimensions and the toolkit's rem-based widgets.
/// gpui-component's root sets the window's rem size from its theme's font_size every frame;
/// changing window.rem_size directly would therefore be overwritten.
pub fn set_zoom(percent: i16, cx: &mut App) {
    let next = bounded_zoom(percent);
    if next != zoom_percent() {
        UI_ZOOM.with(|zoom| zoom.set(next));
        ComponentTheme::update(cx, configure_component_sizes);
    }

    // Show the actual bounded value even when already at a limit or at actual size.
    let shown_at = Instant::now();
    cx.set_global(ZoomNotice {
        percent: next,
        shown_at,
    });
    cx.refresh_windows();

    let timer = cx.background_executor().timer(ZOOM_NOTICE_DURATION);
    cx.spawn(async move |cx| {
        timer.await;
        cx.update(|cx| {
            // An older timer must not dismiss a more recent zoom notification.
            if cx
                .try_global::<ZoomNotice>()
                .is_some_and(|notice| notice.shown_at == shown_at)
            {
                cx.refresh_windows();
            }
        });
    })
    .detach();
}

/// Invalidates cached variable-height list measurements after zoom, including lists that
/// were hidden when the shortcut was pressed. Keep the visible item rather than rebuilding
/// the view (which would discard drafts, expanded code blocks and other interaction state).
pub struct ListZoom(u16);

impl Default for ListZoom {
    fn default() -> Self {
        Self(zoom_percent())
    }
}

impl ListZoom {
    pub fn refresh(&mut self, list: &ListState, count: usize) {
        let next = zoom_percent();
        if self.0 == next {
            return;
        }
        let ratio = f32::from(next) / f32::from(self.0);
        self.0 = next;
        if count == 0 {
            return;
        }
        let mut top = list.logical_scroll_top();
        top.offset_in_item *= ratio;
        list.reset(count);
        list.scroll_to(top);
    }
}

pub fn hsla(c: Rgba) -> Hsla {
    gpui_kit::Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
    .into()
}

fn hex(c: Rgba) -> String {
    format!("#{:08x}", c.to_u32())
}

/// The tokens of the active theme.
pub struct ThemeState {
    pub tokens: &'static Tokens,
}

impl Global for ThemeState {}

pub trait ActiveTheme {
    fn tokens(&self) -> &'static Tokens;
}

impl ActiveTheme for App {
    fn tokens(&self) -> &'static Tokens {
        self.try_global::<ThemeState>()
            .map(|s| s.tokens)
            .unwrap_or(&Tokens::DARK)
    }
}

/// Registers grammars gpui-component lacks highlight queries for. Call once at startup.
pub fn register_languages() {
    use gpui_kit::component::highlighter::{LanguageConfig, LanguageRegistry};
    LanguageRegistry::singleton().register(
        "csharp",
        &LanguageConfig::new(
            "csharp",
            tree_sitter_c_sharp::LANGUAGE.into(),
            vec![],
            tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
            "",
            "",
        ),
    );
}

/// Switches the whole app to `appearance`: our tokens, then gpui-component's theme.
pub fn apply(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    let tokens = Tokens::get(appearance);
    cx.set_global(ThemeState { tokens });
    let mode = match appearance {
        Appearance::Dark => ThemeMode::Dark,
        Appearance::Light | Appearance::Moss => ThemeMode::Light,
    };
    // `change` reloads the component theme's colours, so edit them in a second update.
    ComponentTheme::change(mode, window, cx);
    ComponentTheme::update(cx, |t| configure_components(t, tokens));
    cx.refresh_windows();
}

fn configure_components(t: &mut ComponentTheme, tokens: &Tokens) {
    let c = &tokens.colors;
    let h = |v: Rgba| hsla(v);
    let k = &mut t.colors;

    k.background = h(c.surface_100);
    k.foreground = h(c.ink);
    k.border = h(c.border_strong);
    k.input = h(c.border_strong);
    k.ring = h(c.focus_ring);
    k.caret = h(c.ink_strong);
    k.selection = h(c.editor_selection);
    k.muted = h(c.surface_300);
    k.muted_foreground = h(c.ink_muted);
    k.accent = h(c.surface_hover); // gpui-component's "accent" is a hover fill
    k.accent_foreground = h(c.ink);
    k.primary = h(c.accent);
    k.primary_hover = h(c.accent_hover);
    k.primary_active = h(c.accent_hover);
    k.primary_foreground = h(c.on_accent);
    k.secondary = h(c.surface_300);
    k.secondary_hover = h(c.surface_hover);
    k.secondary_active = h(c.surface_hover);
    k.secondary_foreground = h(c.ink);
    k.button_primary = h(c.accent);
    k.button_primary_hover = h(c.accent_hover);
    k.button_primary_active = h(c.accent_hover);
    k.button_primary_foreground = h(c.on_accent);
    k.button_secondary = h(c.surface_300);
    k.button_secondary_hover = h(c.surface_hover);
    k.button_secondary_active = h(c.surface_hover);
    k.button_secondary_foreground = h(c.ink);
    k.danger = h(c.danger);
    k.danger_foreground = h(c.on_accent);
    k.success = h(c.success);
    k.success_foreground = h(c.on_accent);
    k.warning = h(c.warning);
    k.warning_foreground = h(c.ink_strong);
    k.info = h(c.accent);
    k.info_foreground = h(c.on_accent);
    k.link = h(c.accent_ink);
    k.link_hover = h(c.accent_ink);
    k.link_active = h(c.accent_ink);
    k.popover = h(c.surface_200);
    k.popover_foreground = h(c.ink);
    k.overlay = h(c.surface_scrim);
    k.list = h(c.surface_100);
    k.list_hover = h(c.surface_hover);
    k.list_active = h(c.surface_selected);
    k.list_active_border = h(c.focus_ring);
    k.scrollbar = h(Rgba {
        a: 0.0,
        ..c.surface_100
    });
    k.scrollbar_thumb = h(Rgba {
        a: 0.4,
        ..c.ink_faint
    });
    k.scrollbar_thumb_hover = h(Rgba {
        a: 0.6,
        ..c.ink_faint
    });
    k.sidebar = h(c.surface_000);
    k.sidebar_foreground = h(c.ink);
    k.sidebar_border = h(c.border);
    k.title_bar = h(c.surface_000);
    k.title_bar_border = h(c.border);
    k.status_bar = h(c.surface_000);
    k.status_bar_border = h(c.border);
    k.tab_bar = h(c.surface_000);
    k.tab = h(c.surface_000);
    k.tab_active = h(c.surface_100);
    k.tab_foreground = h(c.ink_muted);
    k.tab_active_foreground = h(c.ink_strong);
    k.switch = h(c.surface_300);
    k.window_border = h(c.border);

    t.font_family = SANS.into();
    t.mono_font_family = MONO.into();
    configure_component_sizes(t);
    t.shadow = true;
    t.highlight_theme = Arc::new(highlight_theme(tokens));
}

/// Recalculate from the original tokens, never from an already-scaled component theme.
/// Theme switches call this too, preserving zoom without compounding it.
fn configure_component_sizes(t: &mut ComponentTheme) {
    t.font_size = px(contextual_theme::type_style::UI_BODY.size);
    t.mono_font_size = px(contextual_theme::type_style::CODE.size);
    t.radius = px(radius::MD);
    t.radius_lg = px(radius::LG);
}

/// Tree-sitter capture → `syn-*` token (the design README's code-rendering rules).
fn highlight_theme(tokens: &Tokens) -> HighlightTheme {
    let c = &tokens.colors;
    let s = |v: Rgba| json!({ "color": hex(v) });
    let syntax = json!({
        "attribute": s(c.syn_variable),
        "boolean": s(c.syn_keyword),
        "comment": s(c.syn_comment),
        "comment.doc": s(c.syn_comment),
        "constant": s(c.syn_number),
        "constructor": s(c.syn_type),
        "enum": s(c.syn_type),
        "function": s(c.syn_function),
        "keyword": s(c.syn_keyword),
        "label": s(c.syn_variable),
        "number": s(c.syn_number),
        "operator": s(c.syn_plain),
        "preproc": s(c.syn_control),
        "property": s(c.syn_variable),
        "punctuation": s(c.syn_plain),
        "punctuation.bracket": s(c.syn_plain),
        "punctuation.delimiter": s(c.syn_plain),
        "punctuation.special": s(c.syn_control),
        "string": s(c.syn_string),
        "string.escape": s(c.syn_control),
        "string.regex": s(c.syn_string),
        "string.special": s(c.syn_string),
        "tag": s(c.syn_tag),
        "title": s(c.syn_heading),
        "type": s(c.syn_type),
        "variable": s(c.syn_variable),
        "variable.special": s(c.syn_keyword),
        "variant": s(c.syn_type),
        "text.literal": s(c.syn_string),
        "link_uri": s(c.accent_ink),
        "link_text": s(c.accent_ink),
    });
    let value = json!({
        "name": "Contextual",
        "appearance": if tokens.appearance == Appearance::Dark { "dark" } else { "light" },
        "style": {
            "editor.background": hex(c.surface_100),
            "editor.foreground": hex(c.syn_plain),
            "editor.active_line.background": hex(c.editor_line_current),
            "editor.line_number": hex(c.editor_gutter_ink),
            "editor.active_line_number": hex(c.editor_gutter_active),
            "editor.invisible": hex(c.editor_indent_guide),
            "editor.gutter.background": hex(c.surface_100),
            "syntax": syntax,
        }
    });
    serde_json::from_value(value).expect("highlight theme JSON matches gpui-component's schema")
}

/// Applies a token type style (size, line height, weight, tracking, family) to an element.
pub trait TypeExt: Styled + Sized {
    fn type_style(self, s: TypeStyle) -> Self {
        let (family, features) = match s.family {
            FontFamily::Sans => (SANS, FontFeatures::default()),
            // Code ligatures off (design README: code rendering rules).
            FontFamily::Mono => (MONO, FontFeatures::disable_ligatures()),
        };
        self.font_family(family)
            .font_features(features)
            .text_size(px(s.size))
            .line_height(px(s.line_height))
            .font_weight(FontWeight(s.weight as f32))
    }
}

impl<T: Styled> TypeExt for T {}

/// Token spacing step in px: only 1, 2, 3, 4, 6 and 8 exist.
pub fn sp(step: u8) -> Pixels {
    px(match step {
        1 => space::S1,
        2 => space::S2,
        3 => space::S3,
        4 => space::S4,
        6 => space::S6,
        8 => space::S8,
        _ => panic!("space-{step} is not a token"),
    })
}

pub fn row_height() -> Pixels {
    px(space::ROW_HEIGHT)
}

pub fn control_height() -> Pixels {
    px(space::CONTROL_HEIGHT)
}

pub fn shadow(s: Shadow) -> BoxShadow {
    BoxShadow {
        color: hsla(s.color),
        offset: point(px(s.offset_x), px(s.offset_y)),
        blur_radius: px(s.blur),
        spread_radius: px(s.spread),
        inset: false,
    }
}

/// The appearance for a preference; `System` follows the window's (OS) appearance.
pub fn resolve(pref: contextual_core::settings::ThemePreference, window: &Window) -> Appearance {
    use contextual_core::settings::ThemePreference;
    match pref {
        ThemePreference::Dark => Appearance::Dark,
        ThemePreference::Light => Appearance::Light,
        ThemePreference::Moss => Appearance::Moss,
        ThemePreference::System => match window.appearance() {
            gpui_kit::WindowAppearance::Light | gpui_kit::WindowAppearance::VibrantLight => {
                Appearance::Light
            }
            _ => Appearance::Dark,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_is_bounded_and_reset_is_exact() {
        assert_eq!(bounded_zoom(i16::MIN), 50);
        assert_eq!(bounded_zoom(50), 50);
        assert_eq!(bounded_zoom(100), 100);
        assert_eq!(bounded_zoom(150), 150);
        assert_eq!(bounded_zoom(200), 200);
        assert_eq!(bounded_zoom(i16::MAX), 200);
    }

    #[test]
    fn pixel_spacing_and_row_metrics_share_one_scale() {
        UI_ZOOM.with(|zoom| {
            let original = zoom.get();
            for percent in [50, 100, 150, 200, 100] {
                zoom.set(percent);
                let scale = f32::from(percent) / 100.0;
                assert_eq!(px(80.0), gpui_kit::px(80.0 * scale));
                assert_eq!(sp(4), gpui_kit::px(space::S4 * scale));
                assert_eq!(row_height(), gpui_kit::px(space::ROW_HEIGHT * scale));
                assert_eq!(
                    control_height(),
                    gpui_kit::px(space::CONTROL_HEIGHT * scale)
                );
                assert_eq!(px(0.0), gpui_kit::px(0.0));
            }
            zoom.set(original);
        });
    }

    #[test]
    fn highlights_code_in_syn_colours() {
        register_languages();
        for tokens in [&Tokens::DARK, &Tokens::LIGHT, &Tokens::MOSS] {
            let theme = highlight_theme(tokens);
            for (lang, code) in [
                ("rust", "fn main() { let x = 1; }"),
                ("csharp", "public class A { int x = 1; }"),
            ] {
                let rope = Rope::from_str(code);
                let mut h = SyntaxHighlighter::new(lang);
                h.update(None, &rope, None);
                let styles = h.styles(&(0..code.len()), &theme);
                assert!(
                    styles
                        .iter()
                        .any(|(_, s)| s.color == Some(hsla(tokens.colors.syn_keyword))),
                    "{:?}/{lang}: missing themed keywords in {styles:?}",
                    tokens.appearance,
                );
            }
        }
    }
}
