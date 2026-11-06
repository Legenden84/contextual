//! Read-only preview tab (design spec: ScreenCodePreview, CodeView): breadcrumb with language,
//! token count and context state, Copy and Wrap lines, then the highlighted code. Files are read
//! through the workspace host, like everything else that touches the project.

use std::path::Path;
use std::sync::Arc;

use contextual_core::host::WorkspaceHost;
use contextual_core::logic::format::fmt_n;
use contextual_core::logic::lang::lang_from_path;
use contextual_protocol::fs::{FileContent, ReadFiles};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div, img, px,
};

use crate::runtime;
use crate::state::Project;
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant};

/// Breadcrumb bar height.
const BAR_H: f32 = 28.0;

/// Extension → tree-sitter language name understood by the editor.
pub fn editor_language(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let file = Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    match ext.as_str() {
        "rs" => "rust",
        "cs" | "csx" => "csharp",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "json" | "jsonc" => "json",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kt",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "css" | "scss" => "css",
        "html" | "htm" | "xml" | "csproj" | "props" | "targets" | "svg" | "xaml" => "html",
        "md" | "mdc" | "markdown" => "markdown",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "bash",
        "lua" => "lua",
        "zig" => "zig",
        "scala" => "scala",
        "graphql" | "gql" => "graphql",
        "proto" => "proto",
        "svelte" => "svelte",
        "ex" | "exs" => "elixir",
        _ if file.eq_ignore_ascii_case("makefile") => "make",
        _ if file.eq_ignore_ascii_case("cmakelists.txt") => "cmake",
        _ => "text",
    }
}

/// `data:image/png;base64,…` → a GPUI image.
fn decode_data_url(url: &str) -> Option<Arc<gpui_kit::Image>> {
    use base64::Engine as _;
    let (head, data) = url.strip_prefix("data:")?.split_once(',')?;
    let format = match head.split(';').next()? {
        "image/png" => gpui_kit::ImageFormat::Png,
        "image/jpeg" => gpui_kit::ImageFormat::Jpeg,
        "image/gif" => gpui_kit::ImageFormat::Gif,
        "image/webp" => gpui_kit::ImageFormat::Webp,
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    Some(Arc::new(gpui_kit::Image::from_bytes(format, bytes)))
}

enum Body {
    Loading,
    Code(Entity<EditorState>),
    Image(Arc<gpui_kit::Image>),
    Error(String),
}

pub struct CodePreview {
    path: String,
    project: Entity<Project>,
    body: Body,
    text: Option<String>,
    wrap: bool,
    _sub: Subscription,
}

impl CodePreview {
    pub fn new(
        path: String,
        project: Entity<Project>,
        host: Arc<dyn WorkspaceHost>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let root = project.read(cx).root.clone();
        let req = ReadFiles {
            root,
            paths: vec![path.clone()],
            condense: false,
        };
        let job = runtime::spawn(cx, async move { host.read(req).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            this.update_in(cx, |this, window, cx| {
                this.body = match result.map(|r| r.files.into_iter().next()) {
                    Ok(Some(FileContent {
                        image: Some(url), ..
                    })) => match decode_data_url(&url) {
                        Some(image) => Body::Image(image),
                        None => Body::Error("Unsupported picture.".into()),
                    },
                    Ok(Some(FileContent {
                        content: Some(text),
                        ..
                    })) => {
                        let lang = editor_language(&this.path);
                        this.text = Some(text.clone());
                        Body::Code(cx.new(|cx| {
                            let mut s = EditorState::new(window, cx)
                                .language(lang)
                                .line_number(true)
                                .default_value(text);
                            s.set_readonly(true, cx);
                            s
                        }))
                    }
                    Ok(Some(FileContent { error: Some(e), .. })) => Body::Error(e),
                    Ok(_) => Body::Error("Nothing to show.".into()),
                    Err(e) => Body::Error(e.message),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
        let sub = cx.observe(&project, |_, _, cx| cx.notify());
        Self {
            path,
            project,
            body: Body::Loading,
            text: None,
            wrap: false,
            _sub: sub,
        }
    }

    fn toggle_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        if let Body::Code(editor) = &self.body {
            let wrap = self.wrap;
            editor.update(cx, |e, cx| e.set_soft_wrap(wrap, window, cx));
        }
        cx.notify();
    }
}

impl Render for CodePreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let p = self.project.read(cx);
        let node = p.node(&self.path);
        let tokens = node.and_then(|n| n.tokens).unwrap_or(0);
        let picture = node.and_then(|n| n.image).unwrap_or(false);
        let condensed = node.and_then(|n| n.condensed_from);
        let in_context = p.selected.contains(&self.path);
        let mut meta = if picture {
            "Picture".to_string()
        } else {
            format!("{} · UTF-8", lang_from_path(Some(&self.path)).label())
        };
        meta.push_str(&format!(" · ~{} tokens", fmt_n(tokens)));
        if let Some(full) = condensed {
            meta.push_str(&format!(" (condensed from ~{})", fmt_n(full)));
        }
        meta.push_str(if in_context {
            " · in context"
        } else {
            " · not in context"
        });

        let parts: Vec<String> = self.path.split('/').map(str::to_string).collect();
        let last = parts.len().saturating_sub(1);
        let crumbs = parts.into_iter().enumerate().flat_map(|(i, part)| {
            let sep = (i > 0).then(|| div().child("›").into_any_element());
            let label = if i == last {
                div()
                    .text_color(hsla(c.ink))
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(SharedString::from(part))
            } else {
                div().child(SharedString::from(part))
            };
            sep.into_iter().chain([label.into_any_element()])
        });
        let text = self.text.clone();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(BAR_H))
                    .pl(sp(4))
                    .pr(sp(3))
                    .border_b_1()
                    .border_color(hsla(c.border))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .children(crumbs)
                    .child(div().flex_1())
                    .child(SharedString::from(meta))
                    .child(
                        Button::new("copy", "Copy")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Xs)
                            .disabled(text.is_none())
                            .on_click(move |_, _, cx| {
                                if let Some(t) = &text {
                                    cx.write_to_clipboard(ClipboardItem::new_string(t.clone()));
                                }
                            }),
                    )
                    .child(
                        Button::new("wrap", "Wrap lines")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Xs)
                            .selected(self.wrap)
                            .on_click(cx.listener(|t, _, w, cx| t.toggle_wrap(w, cx))),
                    ),
            )
            .map(|d| match &self.body {
                Body::Code(editor) => d.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .type_style(contextual_theme::type_style::CODE)
                        .child(Editor::new(editor).readonly(true).bordered(false).h_full()),
                ),
                Body::Image(url) => d.child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .items_center()
                        .justify_center()
                        .p(sp(6))
                        .child(
                            img(url.clone())
                                .max_w_full()
                                .max_h_full()
                                .rounded(px(contextual_theme::radius::MD)),
                        ),
                ),
                Body::Loading | Body::Error(_) => d.child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(match &self.body {
                            Body::Error(e) => format!("Cannot preview this file: {e}"),
                            _ => "Loading…".to_string(),
                        })),
                ),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_data_urls() {
        assert_eq!(editor_language("src/main.rs"), "rust");
        assert_eq!(editor_language("App.CS"), "csharp");
        assert_eq!(editor_language("web/App.tsx"), "tsx");
        assert_eq!(editor_language("Makefile"), "make");
        assert_eq!(editor_language("notes"), "text");
        // 1×1 transparent PNG.
        let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
        assert!(decode_data_url(png).is_some());
        assert!(decode_data_url("data:text/plain;base64,aGk=").is_none());
        assert!(decode_data_url("not a url").is_none());
    }
}
