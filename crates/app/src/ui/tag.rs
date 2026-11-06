use contextual_theme::Rgba;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, px,
};

use crate::theme::{ActiveTheme, hsla};

/// Tag height and type from the Controls spec (16px, 11px medium).
const H: f32 = 16.0;
const SIZE: f32 = 11.0;
const PAD_X: f32 = 5.0;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum TagTone {
    #[default]
    Default,
    Warn,
    Skill,
    Excluded,
}

/// Small label: "new file", "read-only", "Rule", "excluded"…
#[derive(IntoElement)]
pub struct Tag {
    label: SharedString,
    tone: TagTone,
}

impl Tag {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            tone: TagTone::Default,
        }
    }
    pub fn tone(mut self, tone: TagTone) -> Self {
        self.tone = tone;
        self
    }
}

impl RenderOnce for Tag {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let clear = Rgba {
            a: 0.0,
            ..c.surface_100
        };
        let (bg, fg) = match self.tone {
            TagTone::Default => (c.surface_300, c.ink_muted),
            TagTone::Warn => (c.warning_bg, c.warning),
            TagTone::Skill => (c.surface_300, c.accent_ink),
            TagTone::Excluded => (clear, c.ink_faint),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(H))
            .px(px(PAD_X))
            .rounded(px(contextual_theme::radius::SM))
            .bg(hsla(bg))
            .text_color(hsla(fg))
            .text_size(px(SIZE))
            .line_height(px(H))
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .when(self.tone == TagTone::Excluded, |d| {
                d.border_1().border_color(hsla(c.border_strong))
            })
            .child(self.label)
    }
}

/// Round count badge, e.g. next to "Skills & Rules".
pub fn count(n: usize, cx: &App) -> impl IntoElement {
    let c = &cx.tokens().colors;
    div()
        .flex()
        .flex_none()
        .justify_center()
        .h(px(H))
        .min_w(px(H))
        .px(px(contextual_theme::space::S1))
        .rounded_full()
        .bg(hsla(c.surface_300))
        .text_color(hsla(c.ink_muted))
        .text_size(px(SIZE))
        .line_height(px(H))
        .font_weight(FontWeight::MEDIUM)
        .child(SharedString::from(n.to_string()))
}
