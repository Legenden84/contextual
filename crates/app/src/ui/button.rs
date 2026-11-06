use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::theme::{ActiveTheme, TypeExt, control_height, hsla, row_height, sp};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Secondary,
    Primary,
    Ghost,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// Row height, small type (pane headers, inline actions).
    Xs,
    #[default]
    Md,
}

type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The design system's button: secondary (default), primary or ghost; `xs` or `md`.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    variant: ButtonVariant,
    size: ButtonSize,
    selected: bool,
    disabled: bool,
    on_click: Option<OnClick>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            selected: false,
            disabled: false,
            on_click: None,
        }
    }
    pub fn variant(mut self, v: ButtonVariant) -> Self {
        self.variant = v;
        self
    }
    pub fn size(mut self, s: ButtonSize) -> Self {
        self.size = s;
        self
    }
    /// Shows the ghost variant as pressed (segmented controls, nav).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
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

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let (bg, hover, fg, border) = match self.variant {
            ButtonVariant::Secondary => (c.surface_300, c.surface_hover, c.ink, c.border_strong),
            ButtonVariant::Primary => (c.accent, c.accent_hover, c.on_accent, c.accent),
            ButtonVariant::Ghost if self.selected => (
                c.surface_selected,
                c.surface_selected,
                c.ink_strong,
                c.surface_selected,
            ),
            ButtonVariant::Ghost => (
                clear(c.surface_100),
                c.surface_hover,
                c.ink_muted,
                clear(c.surface_100),
            ),
        };
        let ink_hover = if self.variant == ButtonVariant::Ghost {
            c.ink
        } else {
            fg
        };
        let disabled = self.disabled;
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(sp(1))
            .rounded(px(contextual_theme::radius::MD))
            .border_1()
            .border_color(hsla(border))
            .bg(hsla(bg))
            .text_color(hsla(fg))
            .map(|d| match self.size {
                ButtonSize::Xs => d
                    .h(row_height())
                    .px(sp(2))
                    .type_style(contextual_theme::type_style::UI_SMALL),
                ButtonSize::Md => d
                    .h(control_height())
                    .px(sp(3))
                    .type_style(contextual_theme::type_style::UI_BODY),
            })
            .when(self.variant == ButtonVariant::Primary, |d| {
                d.font_weight(gpui_kit::FontWeight::MEDIUM)
            })
            .when(disabled, |d| d.opacity(0.5).cursor_not_allowed())
            .when(!disabled, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(hsla(hover)).text_color(hsla(ink_hover)))
            })
            .when_some(self.on_click.filter(|_| !disabled), |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
            .child(self.label)
    }
}

/// The same token colour, fully transparent ("no fill").
fn clear(c: contextual_theme::Rgba) -> contextual_theme::Rgba {
    contextual_theme::Rgba { a: 0.0, ..c }
}
