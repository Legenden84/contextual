use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
};

use crate::theme::{ActiveTheme, hsla, px, row_height};

const GLYPH: f32 = 14.0;

type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// 22px square icon-only button used in pane headers and toolbars.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: AnyElement,
    label: SharedString,
    disabled: bool,
    on_click: Option<OnClick>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName, label: impl Into<SharedString>) -> Self {
        Self::with_glyph(id, Icon::new(icon).size(px(GLYPH)), label)
    }

    /// Custom artwork with the same hit target, tooltip and interaction as standard icons.
    pub fn with_glyph(
        id: impl Into<ElementId>,
        glyph: impl IntoElement,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon: glyph.into_any_element(),
            label: label.into(),
            disabled: false,
            on_click: None,
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let label = self.label.clone();
        div()
            .id(self.id)
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(row_height())
            .rounded(px(contextual_theme::radius::SM))
            .text_color(hsla(if self.disabled {
                c.ink_faint
            } else {
                c.ink_muted
            }))
            .when(self.disabled, |d| d.cursor_not_allowed())
            .when(!self.disabled, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(hsla(c.surface_hover)).text_color(hsla(c.ink)))
            })
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(label.clone()).build(window, cx)
            })
            .child(self.icon)
            .on_click(move |e, w, cx| {
                cx.stop_propagation();
                if !self.disabled && let Some(f) = &self.on_click {
                    f(e, w, cx);
                }
            })
    }
}
