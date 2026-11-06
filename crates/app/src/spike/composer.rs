//! Prototype 4: the composer. Multi-line input with IME (gpui-component's textarea), image paste
//! and file drop. Pictures go through `contextual_core::logic::images` (2048px scaling, vision
//! token estimate), as they will when sent.

use contextual_core::logic::format::fmt_k;
use contextual_core::logic::images::{Attachment, to_attachment};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, ClipboardEntry, ClipboardItem, Context, Entity, ExternalPaths, ImageFormat,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Window, div, px,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonVariant};

pub struct ComposerProto {
    input: Entity<TextareaState>,
    attachments: Vec<Attachment>,
    sent: Vec<String>,
    error: Option<String>,
    _subscription: Subscription,
}

fn mime_for_format(f: ImageFormat) -> Option<&'static str> {
    Some(match f {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Webp => "image/webp",
        ImageFormat::Gif => "image/gif",
        _ => return None,
    })
}

fn mime_for_path(p: &std::path::Path) -> Option<&'static str> {
    match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

impl ComposerProto {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx).auto_grow(3, 12).placeholder(
                "Ask about the selected files… (Enter to send, Shift+Enter for a new line)",
            )
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    shift: false,
                    secondary: false,
                } = event
                {
                    let text = input.read(cx).value().trim().to_string();
                    if !text.is_empty() || !this.attachments.is_empty() {
                        this.sent
                            .push(format!("{text} [{} picture(s)]", this.attachments.len()));
                        this.attachments.clear();
                        input.update(cx, |s, cx| s.set_value("", window, cx));
                        cx.notify();
                    }
                }
            },
        );
        Self {
            input,
            attachments: Vec::new(),
            sent: Vec::new(),
            error: None,
            _subscription: subscription,
        }
    }

    fn add_image(&mut self, bytes: &[u8], mime: &str, name: &str, cx: &mut Context<Self>) {
        let id = format!("img-{}", self.attachments.len() + self.sent.len());
        match to_attachment(bytes, mime, name, &id) {
            Ok(a) => {
                self.attachments.push(a);
                self.error = None;
            }
            Err(e) => self.error = Some(e.message),
        }
        cx.notify();
    }

    /// Handles a paste; returns true when it took a picture (so the text input ignores it).
    fn paste(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let mut took = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(img) => {
                    if let Some(mime) = mime_for_format(img.format) {
                        self.add_image(&img.bytes, mime, "Pasted image", cx);
                        took = true;
                    }
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    self.drop_paths(paths, cx);
                    took = true;
                }
                _ => {}
            }
        }
        took
    }

    fn drop_paths(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        for p in paths.paths() {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match (mime_for_path(p), std::fs::read(p)) {
                (Some(mime), Ok(bytes)) => self.add_image(&bytes, mime, &name, cx),
                (None, _) => {
                    // Non-image files would be added to the context selection in the real app.
                    self.error = Some(format!("{name}: added to context (not a picture)"));
                    cx.notify();
                }
                (_, Err(e)) => {
                    self.error = Some(format!("{name}: {e}"));
                    cx.notify();
                }
            }
        }
    }
}

impl Render for ComposerProto {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let this = cx.entity().downgrade();
        let chips = self.attachments.iter().enumerate().map(|(i, a)| {
            div()
                .flex()
                .items_center()
                .gap(sp(1))
                .px(sp(2))
                .rounded(px(contextual_theme::radius::SM))
                .bg(hsla(c.pill_bg))
                .type_style(contextual_theme::type_style::UI_SMALL)
                .child(SharedString::from(format!(
                    "{} · {}×{} · ~{} tokens",
                    a.name,
                    a.width,
                    a.height,
                    fmt_k(a.tokens)
                )))
                .child(
                    Button::new(("rm", i), "×")
                        .variant(ButtonVariant::Ghost)
                        .size(crate::ui::ButtonSize::Xs)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.attachments.remove(i);
                            cx.notify();
                        })),
                )
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .p(sp(6))
            .gap(sp(4))
            .child(
                div()
                    .w_full()
                    .max_w(px(contextual_theme::layout::CONTENT_MAX_WIDTH))
                    .flex()
                    .flex_col()
                    .gap(sp(2))
                    .children(self.sent.iter().map(|s| {
                        div()
                            .p(sp(3))
                            .rounded(px(contextual_theme::radius::MD))
                            .bg(hsla(c.surface_200))
                            .type_style(contextual_theme::type_style::UI_PROSE)
                            .child(SharedString::from(s.clone()))
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("composer")
                    .w_full()
                    .max_w(px(contextual_theme::layout::CONTENT_MAX_WIDTH))
                    .flex()
                    .flex_col()
                    .gap(sp(2))
                    .p(sp(2))
                    .rounded(px(contextual_theme::radius::MD))
                    .border_1()
                    .border_color(hsla(c.border_strong))
                    .bg(hsla(c.surface_200))
                    .drag_over::<ExternalPaths>(|s, _, _, cx| {
                        s.border_color(hsla(cx.tokens().colors.accent))
                    })
                    .on_drop(
                        cx.listener(|this, paths: &ExternalPaths, _, cx| {
                            this.drop_paths(paths, cx)
                        }),
                    )
                    .when(!self.attachments.is_empty(), |d| {
                        d.child(div().flex().flex_wrap().gap(sp(1)).children(chips))
                    })
                    .child(Textarea::new(&self.input).appearance(false).on_paste(
                        move |item, _, cx| {
                            this.update(cx, |this, cx| this.paste(item, cx))
                                .unwrap_or(false)
                        },
                    ))
                    .when_some(self.error.clone(), |d, e| {
                        d.child(
                            div()
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child(SharedString::from(e)),
                        )
                    }),
            )
    }
}
