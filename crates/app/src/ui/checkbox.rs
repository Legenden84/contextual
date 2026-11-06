use contextual_core::logic::tree::CheckState;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::theme::{ActiveTheme, hsla};

/// Box size and glyph size from the Controls spec (14px box, 12px check).
const BOX: f32 = 14.0;
const GLYPH: f32 = 12.0;

type OnToggle = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// 14px tri-state checkbox.
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    state: CheckState,
    disabled: bool,
    on_toggle: Option<OnToggle>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, state: CheckState) -> Self {
        Self {
            id: id.into(),
            state,
            disabled: false,
            on_toggle: None,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn on_toggle(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let on = self.state != CheckState::Off;
        let glyph = match self.state {
            CheckState::On => Some(IconName::Check),
            CheckState::Mixed => Some(IconName::Minus),
            CheckState::Off => None,
        };
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(BOX))
            .rounded(px(contextual_theme::radius::SM))
            .border_1()
            .border_color(hsla(if on { c.accent } else { c.border_strong }))
            .bg(hsla(if on { c.accent } else { c.surface_100 }))
            .text_color(hsla(c.on_accent))
            .when(self.disabled, |d| d.opacity(0.45))
            .when(!self.disabled, |d| d.cursor_pointer())
            .when_some(glyph, |d, g| d.child(Icon::new(g).size(px(GLYPH))))
            .when_some(self.on_toggle.filter(|_| !self.disabled), |d, f| {
                d.on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    f(e, w, cx)
                })
            })
    }
}
