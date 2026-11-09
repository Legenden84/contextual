use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    StatefulInteractiveElement, Styled, Window, div,
};

use crate::theme::{ActiveTheme, hsla, px};

/// Track and knob sizes from the Controls spec.
const TRACK_W: f32 = 26.0;
const TRACK_H: f32 = 14.0;
const KNOB: f32 = 8.0;
const INSET: f32 = 2.0;

type OnToggle = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// 26×14 toggle switch.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    on: bool,
    on_toggle: Option<OnToggle>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, on: bool) -> Self {
        Self {
            id: id.into(),
            on,
            on_toggle: None,
        }
    }
    pub fn on_toggle(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Box::new(f));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let c = &cx.tokens().colors;
        // Knob sits INSET from the edge; the 1px border is inside the track.
        let left = if self.on {
            TRACK_W - KNOB - INSET - 2.0
        } else {
            INSET - 1.0
        };
        div()
            .id(self.id)
            .relative()
            .flex_none()
            .w(px(TRACK_W))
            .h(px(TRACK_H))
            .rounded_full()
            .border_1()
            .border_color(hsla(if self.on { c.accent } else { c.border_strong }))
            .bg(hsla(if self.on { c.accent } else { c.surface_300 }))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top(px(INSET - 1.0))
                    .left(px(left))
                    .size(px(KNOB))
                    .rounded_full()
                    .bg(hsla(if self.on { c.on_accent } else { c.ink_muted })),
            )
            .when_some(self.on_toggle, |d, f| {
                d.on_click(move |e, w, cx| {
                    cx.stop_propagation();
                    f(e, w, cx)
                })
            })
    }
}
