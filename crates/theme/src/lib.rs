//! The Contextual design system as typed Rust.
//!
//! Everything in this crate is generated at build time from
//! `docs/design/contextual/tokens.json` (see `build.rs`). Views use these values only:
//! no literal colours, spacing, radii or shadows anywhere else.
//!
//! This crate has no GUI dependency; the app converts [`Rgba`] into its toolkit's colour type.

/// A colour in non-premultiplied sRGB, every channel in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// `0xRRGGBBAA`, the layout `gpui::rgba` takes.
    pub fn to_u32(self) -> u32 {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        (c(self.r) << 24) | (c(self.g) << 16) | (c(self.b) << 8) | c(self.a)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontFamily {
    Sans,
    Mono,
}

impl FontFamily {
    /// The family stack, bundled font first, then platform fallbacks.
    pub fn stack(self) -> &'static [&'static str] {
        match self {
            FontFamily::Sans => families::SANS,
            FontFamily::Mono => families::MONO,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeStyle {
    pub family: FontFamily,
    /// px
    pub size: f32,
    /// px
    pub line_height: f32,
    pub weight: u16,
    /// Tracking in em (multiply by `size` for px).
    pub letter_spacing_em: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    Moss,
}

impl Appearance {
    /// The title-bar and keyboard toggle cycle through the three explicit schemes.
    pub const fn next(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Moss,
            Self::Moss => Self::Dark,
        }
    }
}

/// One complete theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub appearance: Appearance,
    pub colors: Colors,
    pub shadows: Shadows,
}

impl Theme {
    pub const DARK: Theme = Theme {
        appearance: Appearance::Dark,
        colors: DARK_COLORS,
        shadows: DARK_SHADOWS,
    };
    pub const LIGHT: Theme = Theme {
        appearance: Appearance::Light,
        colors: LIGHT_COLORS,
        shadows: LIGHT_SHADOWS,
    };
    pub const MOSS: Theme = Theme {
        appearance: Appearance::Moss,
        colors: MOSS_COLORS,
        shadows: MOSS_SHADOWS,
    };

    pub fn get(appearance: Appearance) -> &'static Theme {
        match appearance {
            Appearance::Dark => &Self::DARK,
            Appearance::Light => &Self::LIGHT,
            Appearance::Moss => &Self::MOSS,
        }
    }
}

/// Fixed layout widths from the design README (px).
pub mod layout {
    /// Left side bar (Files, Skills & Rules).
    pub const EXPLORER_WIDTH: f32 = 312.0;
    /// Right side bar (Changes panel).
    pub const CHANGES_WIDTH: f32 = 320.0;
    /// Chat thread reading width.
    pub const CONTENT_MAX_WIDTH: f32 = 820.0;
    /// Minimum comfortable chat column before auxiliary panes become on-demand views.
    pub const CHAT_MIN_WIDTH: f32 = 480.0;
    /// Compact warning before sending with incomplete AI settings.
    pub const SETTINGS_PROMPT_WIDTH: f32 = 440.0;
    /// Tab width in code views.
    pub const TAB_SIZE: usize = 4;
}

include!(concat!(env!("OUT_DIR"), "/tokens.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_and_rgba() {
        assert_eq!(DARK_COLORS.surface_100.to_u32(), 0x1f1f1fff);
        assert_eq!(LIGHT_COLORS.surface_100.to_u32(), 0xffffffff);
        // rgba(0, 0, 0, 0.55)
        assert_eq!(DARK_COLORS.surface_scrim.to_u32(), 0x0000008c);
    }

    #[test]
    fn spacing_radius_type() {
        assert_eq!(space::S2, 8.0);
        assert_eq!(space::ROW_HEIGHT, 22.0);
        assert_eq!(space::CONTROL_HEIGHT, 26.0);
        assert_eq!(radius::MD, 6.0);
        assert_eq!(type_style::UI_BODY.size, 13.0);
        assert_eq!(type_style::CODE.family, FontFamily::Mono);
        assert_eq!(families::SANS[0], "Inter");
        assert_eq!(families::MONO[0], "JetBrains Mono");
        assert_eq!(Theme::DARK.shadows.popover.blur, 16.0);
    }

    #[test]
    fn every_token_present() {
        assert!(COLOR_TOKEN_NAMES.contains(&"syn-keyword"));
        assert!(COLOR_TOKEN_NAMES.len() >= 50);
    }

    #[test]
    fn appearance_cycles_through_three_complete_themes() {
        let mut appearance = Appearance::default();
        for expected in [Appearance::Light, Appearance::Moss, Appearance::Dark] {
            appearance = appearance.next();
            assert_eq!(appearance, expected);
            assert_eq!(Theme::get(appearance).appearance, expected);
        }
        assert_eq!(MOSS_COLORS.ink.to_u32(), 0x173630ff);
        assert_eq!(MOSS_COLORS.surface_selected.to_u32(), 0xbfe6c4ff);
        assert_eq!(MOSS_COLORS.surface_100.to_u32(), 0xf2fff3ff);
    }

    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let luminance = |c: Rgba| {
            let linear = |v: f32| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn moss_text_and_code_remain_readable() {
        let c = MOSS_COLORS;
        for bg in [
            c.surface_000,
            c.surface_100,
            c.surface_200,
            c.surface_300,
            c.surface_hover,
            c.surface_selected,
        ] {
            for fg in [
                c.ink,
                c.ink_strong,
                c.ink_muted,
                c.accent_ink,
                c.success,
                c.warning,
                c.danger,
                c.modified,
            ] {
                assert!(contrast(fg, bg) >= 4.5, "{fg:?} on {bg:?}");
            }
            assert!(contrast(c.border_strong, bg) >= 3.0);
            assert!(contrast(c.focus_ring, bg) >= 3.0);
        }
        for bg in [
            c.surface_100,
            c.editor_selection,
            c.diff_add_bg,
            c.diff_add_word,
            c.diff_del_bg,
            c.diff_del_word,
        ] {
            for fg in [
                c.syn_plain,
                c.syn_keyword,
                c.syn_control,
                c.syn_type,
                c.syn_function,
                c.syn_string,
                c.syn_number,
                c.syn_comment,
                c.syn_variable,
                c.syn_tag,
                c.syn_heading,
            ] {
                assert!(contrast(fg, bg) >= 4.5, "{fg:?} on {bg:?}");
            }
        }
        for bg in [c.surface_100, c.diff_add_gutter, c.diff_del_gutter] {
            assert!(contrast(c.editor_gutter_ink, bg) >= 4.5);
        }
        for bg in [c.accent, c.accent_hover] {
            assert!(contrast(c.on_accent, bg) >= 4.5);
        }
    }
}
