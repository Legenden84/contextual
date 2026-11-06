//! Prototype 2: read-only code view of a 5,000-line file with tree-sitter highlighting in the
//! `syn-*` token colours (set on gpui-component's highlight theme in `theme.rs`).

use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, row_height, sp};

pub struct CodeViewProto {
    editor: Entity<EditorState>,
    lines: usize,
}

impl CodeViewProto {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = super::sample::rust_source(5_000, 0);
        let lines = text.lines().count();
        let editor = cx.new(|cx| {
            let mut state = EditorState::new(window, cx)
                .language("rust")
                .line_number(true)
                .default_value(text);
            state.set_readonly(true, cx);
            state
        });
        Self { editor, lines }
    }
}

impl Render for CodeViewProto {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(row_height() + sp(2))
                    .px(sp(3))
                    .bg(hsla(c.surface_200))
                    .border_b_1()
                    .border_color(hsla(c.border))
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .child(
                        div()
                            .text_color(hsla(c.ink))
                            .child("src/orders/handlers.rs"),
                    )
                    .child(
                        div()
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(format!(
                                "{} lines · read-only",
                                self.lines
                            ))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .type_style(contextual_theme::type_style::CODE)
                    .child(
                        Editor::new(&self.editor)
                            .readonly(true)
                            .bordered(false)
                            .h_full(),
                    ),
            )
    }
}
