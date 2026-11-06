use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div,
};

use crate::theme::{ActiveTheme, TypeExt, hsla};

type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Text-only action in accent ink ("Select all", "Settings", "Clear recent").
#[derive(IntoElement)]
pub struct LinkButton {
    id: ElementId,
    label: SharedString,
    on_click: OnClick,
}

impl LinkButton {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            on_click: Box::new(on_click),
        }
    }
}

impl RenderOnce for LinkButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let f = self.on_click;
        div()
            .id(self.id)
            .flex_none()
            .cursor_pointer()
            .type_style(contextual_theme::type_style::UI_SMALL)
            .text_color(hsla(c.accent_ink))
            .hover(|s| s.underline())
            .on_click(move |e, w, cx| f(e, w, cx))
            .child(self.label)
    }
}
