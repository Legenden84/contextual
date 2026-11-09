//! The chat (design specs: ChatMessage, Composer): the thread as a virtualised list, then the
//! context bar and the composer. Ctrl+Enter sends; pictures can be pasted or dropped.

use std::collections::HashSet;
use std::io::Read as _;
use std::path::PathBuf;

use contextual_core::chat::{ReadLog, Role, Tone, UiMessage};
use contextual_core::llm::config;
use contextual_core::logic::composer::{self, ComposerEdit};
use contextual_core::logic::format::fmt_k;
use contextual_core::logic::foundry_status::FoundryStatus;
use contextual_core::logic::images::{
    Attachment, IMAGE_TYPES, MAX_SOURCE_BYTES, attachment_name, clipboard_attachment, to_attachment,
};
use contextual_theme::layout;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardEntry, ClipboardItem, Context, ElementId, Entity,
    EntityInputHandler, ExternalPaths, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, ListAlignment, ListState, MouseButton, ParentElement, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, list,
};

use crate::runtime;
use crate::state::chat::{CenterView, ChatActivity, ChatModel, SendEnv};
use crate::state::{AppModel, Project};
use crate::theme::{ActiveTheme, ListZoom, TypeExt, control_height, hsla, px, shadow, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, IconButton, activity_mark, completed_mark};
use crate::views::context_bar::{ContextCache, context_bar};
use crate::views::diff_cache::{DiffCache, ensure_message, message_key};
use crate::views::markdown::{ReplyCache, render_reply};

pub struct ChatPanel {
    app: Entity<AppModel>,
    project: Entity<Project>,
    chat: Entity<ChatModel>,
    input: Entity<TextareaState>,
    /// Last draft seen by the typing helper; distinguishes typing from bulk paste and undo.
    composer_snapshot: String,
    list: ListState,
    list_zoom: ListZoom,
    /// Code blocks the user expanded ("Show N more lines").
    expanded: HashSet<String>,
    rendered: usize,
    thread_generation: u64,
    replies: ReplyCache,
    context: ContextCache,
    diffs: DiffCache,
    image_loading: bool,
    image_picker_open: bool,
    image_error: Option<String>,
    attachment_menu_open: bool,
    attachment_focus: FocusHandle,
    inspect_focus: FocusHandle,
    settings_prompt_open: bool,
    settings_prompt_focus: FocusHandle,
    settings_cancel_focus: FocusHandle,
    settings_open_focus: FocusHandle,
    _subs: Vec<Subscription>,
}

/// These are explicitly chosen local attachments, not project paths resolved through the host.
enum ImageSource {
    File(PathBuf),
    Clipboard { bytes: Vec<u8>, name: String },
}

/// File I/O and image decoding run on the blocking pool, never on the UI thread.
fn prepare_image(source: ImageSource) -> contextual_core::Result<Attachment> {
    let (bytes, mime, name) = match source {
        ImageSource::Clipboard { bytes, name } => {
            return clipboard_attachment(&bytes, &name, &contextual_core::chat::new_id("img"));
        }
        ImageSource::File(path) => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let mime = mime_for_path(&path).ok_or_else(|| {
                contextual_core::Error::invalid(format!(
                    "{name}: only PNG, JPEG, GIF and WebP pictures can be sent."
                ))
            })?;
            let mut bytes = Vec::new();
            let file = std::fs::File::open(&path).map_err(|e| {
                contextual_core::Error::invalid(format!("Could not open {name}: {e}"))
            })?;
            file.take(MAX_SOURCE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| {
                    contextual_core::Error::invalid(format!("Could not read {name}: {e}"))
                })?;
            (bytes, mime, name)
        }
    };
    to_attachment(&bytes, mime, &name, &contextual_core::chat::new_id("img"))
}

fn mime_for_path(p: &std::path::Path) -> Option<&'static str> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    let m = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    };
    IMAGE_TYPES.contains(&m).then_some(m)
}

fn settings_prompt_detail(status: &FoundryStatus) -> String {
    fn label(name: &str) -> &str {
        match name {
            config::ENV_ENDPOINT => "Endpoint",
            config::ENV_API_KEY => "API key",
            config::ENV_API_VERSION => "API version",
            config::ENV_DEPLOYMENT => "Deployment",
            config::ENV_MODEL_NAME => "Model name",
            config::ENV_MAX_COMPLETION_TOKENS => "Max output tokens",
            config::ENV_CONTEXT_LIMIT => "Context limit",
            config::ENV_INPUT_PRICE => "Input price / 1M",
            config::ENV_OUTPUT_PRICE => "Output price / 1M",
            other => other,
        }
    }

    let mut lines = Vec::new();
    if let FoundryStatus::Unconfigured { missing, invalid } = status {
        for (title, fields) in [("Missing", missing), ("Invalid", invalid)] {
            if !fields.is_empty() {
                let names = fields.iter().map(|f| label(f)).collect::<Vec<_>>();
                lines.push(format!("{title}: {}.", names.join(", ")));
            }
        }
    }
    if lines.is_empty() {
        lines.push("Azure AI Foundry is not configured.".into());
    }
    lines.join("\n")
}

impl ChatPanel {
    pub fn new(
        app: Entity<AppModel>,
        project: Entity<Project>,
        chat: Entity<ChatModel>,
        diffs: DiffCache,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 6)
                // Plain Enter is handled below. Shift+Enter stays a native newline.
                .submit_on_enter(true)
                .placeholder("Ask for a change… (Ctrl+Enter to send)")
        });
        let count = chat.read(cx).messages.len();
        let thread_generation = chat.read(cx).thread_generation;
        let subs = vec![
            cx.observe(&chat, |this: &mut Self, chat, cx| {
                let chat = chat.read(cx);
                let n = chat.messages.len();
                let replaced = chat.thread_generation != this.thread_generation;
                if replaced {
                    this.thread_generation = chat.thread_generation;
                    this.expanded.clear();
                    this.replies.clear();
                    this.image_error = None;
                }
                if replaced || chat.streaming() {
                    this.attachment_menu_open = false;
                }
                if replaced || n != this.rendered {
                    this.list.reset(n);
                    this.list.scroll_to_end();
                } else if n > 0 {
                    // The last message grows while streaming: re-measure it.
                    this.list.splice(n - 1..n, 1);
                }
                this.rendered = n;
                cx.notify();
            }),
            cx.observe(&diffs, |_, _, cx| cx.notify()),
            cx.observe(&project, |_, _, cx| cx.notify()),
            cx.observe(&app, |this, app, cx| {
                if app.read(cx).screen != crate::state::Screen::Workspace {
                    this.settings_prompt_open = false;
                }
                cx.notify();
            }),
            cx.subscribe_in(
                &input,
                window,
                |this, _, e: &InputEvent, window, cx| match e {
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => {
                        if !this.composer_is_composing(window, cx) {
                            this.send(window, cx);
                        }
                    }
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    } => this.composer_enter(window, cx),
                    InputEvent::Change => this.composer_changed(window, cx),
                    _ => {}
                },
            ),
        ];
        Self {
            app,
            project,
            chat,
            input,
            composer_snapshot: String::new(),
            list: ListState::new(count, ListAlignment::Bottom, px(400.0)),
            list_zoom: ListZoom::default(),
            expanded: HashSet::new(),
            rendered: count,
            thread_generation,
            replies: ReplyCache::default(),
            context: ContextCache::default(),
            diffs,
            image_loading: false,
            image_picker_open: false,
            image_error: None,
            attachment_menu_open: false,
            attachment_focus: cx.focus_handle(),
            inspect_focus: cx.focus_handle(),
            settings_prompt_open: false,
            settings_prompt_focus: cx.focus_handle(),
            settings_cancel_focus: cx.focus_handle(),
            settings_open_focus: cx.focus_handle(),
            _subs: subs,
        }
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.input.read(cx).focus_handle(cx), cx);
    }

    /// Sends `text` as if typed (debug automation, see `dev.rs`).
    pub fn dev_send(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.set_value(text, window, cx));
        self.composer_snapshot = self.input.read(cx).value().to_string();
        self.send(window, cx);
    }

    fn composer_is_composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.input
            .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
    }

    fn edit_composer(&mut self, edit: ComposerEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| {
            if s.selected_range() != edit.range {
                s.set_selected_range(edit.range, cx);
            }
            // Unlike set_value, replace is an atomic, undoable local edit.
            s.replace(edit.text, window, cx);
            if s.selected_range() != (edit.cursor..edit.cursor) {
                s.set_selected_range(edit.cursor..edit.cursor, cx);
            }
        });
        // Programmatic replacements do not emit Change.
        self.composer_snapshot = self.input.read(cx).value().to_string();
    }

    fn composer_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.input.read(cx);
        let text = input.value().to_string();
        let selection = input.selected_range();
        let previous = std::mem::replace(&mut self.composer_snapshot, text.clone());
        if self.composer_is_composing(window, cx) {
            return;
        }
        if let Some(edit) = composer::indent_on_space(&previous, &text, selection) {
            self.edit_composer(edit, window, cx);
        }
    }

    fn composer_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_is_composing(window, cx) {
            return;
        }
        let input = self.input.read(cx);
        let text = input.value().to_string();
        if let Some(edit) = composer::newline(&text, input.selected_range()) {
            self.edit_composer(edit, window, cx);
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        let chat = self.chat.read(cx);
        if self.image_loading
            || self.settings_prompt_open
            || chat.streaming()
            || chat.saving
            || (text.is_empty() && chat.attachments.is_empty())
        {
            return;
        }
        let a = self.app.read(cx);
        let Some(config) = a.config.clone() else {
            self.attachment_menu_open = false;
            self.settings_prompt_open = true;
            window.focus(&self.settings_open_focus, cx);
            cx.notify();
            return;
        };
        let p = self.project.read(cx);
        let env = SendEnv {
            config,
            deployment: a.deployment(),
            output_prompt: a.settings.workspace.output_prompt.clone(),
            auto: true,
            skill_index: p.skill_index(),
        };
        let rules = p.rules_on();
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.composer_snapshot.clear();
        self.chat.update(cx, |c, cx| c.send(text, env, rules, cx));
    }

    fn close_settings_prompt(
        &mut self,
        open_settings: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_prompt_open = false;
        self.focus_input(window, cx);
        if open_settings {
            self.app.update(cx, |a, cx| a.open_settings(cx));
        }
        cx.notify();
    }

    /// The parent only needs notification when this overlay opens or closes.
    /// Ordinary chat/composer updates are rendered by this entity independently.
    pub fn settings_prompt_open(&self) -> bool {
        self.settings_prompt_open
    }

    /// Rendered above the whole workspace, not clipped to the chat column.
    pub fn settings_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.settings_prompt_open {
            return None;
        }
        let c = &cx.tokens().colors;
        let detail = settings_prompt_detail(&self.app.read(cx).foundry);
        let problem_rows = detail.lines().map(|line| {
            div()
                .whitespace_normal()
                .child(SharedString::from(line.to_string()))
        });

        Some(
            div()
                .id("ai-settings-scrim")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .p(sp(4))
                .bg(hsla(c.surface_scrim))
                .track_focus(&self.settings_prompt_focus)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|t, _, window, cx| {
                    t.close_settings_prompt(false, window, cx);
                }))
                .on_key_down(cx.listener(|t, e: &KeyDownEvent, window, cx| {
                    match e.keystroke.key.as_str() {
                        "escape" => t.close_settings_prompt(false, window, cx),
                        "tab" => {
                            // Two actions: Tab and Shift+Tab both move to the other one.
                            let next = if t.settings_open_focus.is_focused(window) {
                                &t.settings_cancel_focus
                            } else {
                                &t.settings_open_focus
                            };
                            window.focus(next, cx);
                        }
                        "enter" | "space" => {
                            let open = !t.settings_cancel_focus.is_focused(window);
                            t.close_settings_prompt(open, window, cx);
                        }
                        _ => {}
                    }
                    cx.stop_propagation();
                }))
                .child(
                    div()
                        .id("ai-settings-dialog")
                        .w_full()
                        .max_w(px(layout::SETTINGS_PROMPT_WIDTH))
                        .flex()
                        .flex_col()
                        .rounded(px(contextual_theme::radius::LG))
                        .border_1()
                        .border_color(hsla(c.border_strong))
                        .bg(hsla(c.surface_100))
                        .shadow(vec![shadow(cx.tokens().shadows.modal)])
                        .type_style(contextual_theme::type_style::UI_BODY)
                        .text_color(hsla(c.ink))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .p(sp(4))
                                .flex()
                                .flex_col()
                                .gap(sp(3))
                                .child(
                                    div()
                                        .type_style(contextual_theme::type_style::UI_TITLE)
                                        .text_color(hsla(c.ink_strong))
                                        .child("Check AI settings"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(sp(1))
                                        .children(problem_rows),
                                )
                                .child(
                                    div()
                                        .type_style(contextual_theme::type_style::UI_SMALL)
                                        .text_color(hsla(c.ink_muted))
                                        .whitespace_normal()
                                        .child(
                                            "Nothing sent. Your draft and attachments stay in Chat. Update settings, then send again.",
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(sp(2))
                                .px(sp(4))
                                .py(sp(3))
                                .border_t_1()
                                .border_color(hsla(c.border))
                                .child(
                                    div()
                                        .id("ai-settings-cancel-focus")
                                        .track_focus(&self.settings_cancel_focus)
                                        .rounded(px(contextual_theme::radius::MD))
                                        .border_1()
                                        .border_color(hsla(c.surface_100))
                                        .focus(|s| s.border_color(hsla(c.focus_ring)))
                                        .child(
                                            Button::new("ai-settings-cancel", "Cancel")
                                                .on_click(cx.listener(|t, _, window, cx| {
                                                    t.close_settings_prompt(false, window, cx);
                                                })),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("ai-settings-open-focus")
                                        .track_focus(&self.settings_open_focus)
                                        .rounded(px(contextual_theme::radius::MD))
                                        .border_1()
                                        .border_color(hsla(c.surface_100))
                                        .focus(|s| s.border_color(hsla(c.focus_ring)))
                                        .child(
                                            Button::new("ai-settings-open", "Open settings")
                                                .variant(ButtonVariant::Primary)
                                                .on_click(cx.listener(|t, _, window, cx| {
                                                    t.close_settings_prompt(true, window, cx);
                                                })),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    fn load_images(&mut self, sources: Vec<ImageSource>, cx: &mut Context<Self>) {
        if sources.is_empty() {
            return;
        }
        if self.image_loading || self.chat.read(cx).streaming() {
            self.image_error =
                Some("Wait for the current operation to finish before attaching images.".into());
            cx.notify();
            return;
        }
        self.attachment_menu_open = false;
        self.image_loading = true;
        self.image_error = None;
        let thread = self.chat.read(cx).thread.id.clone();
        cx.notify();
        let job = runtime::spawn(cx, async move {
            let mut resolved = Vec::with_capacity(sources.len());
            for source in sources {
                resolved.push(match source {
                    ImageSource::File(path) => {
                        contextual_core::platform::local_attachment_path(path)
                            .await
                            .map(ImageSource::File)
                    }
                    clipboard => Ok(clipboard),
                });
            }
            tokio::task::spawn_blocking(move || {
                resolved
                    .into_iter()
                    .map(|source| source.and_then(prepare_image))
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| contextual_core::Error::internal(format!("Could not prepare images: {e}")))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.image_loading = false;
                // A picker or decode operation must never attach to a different conversation.
                if this.chat.read(cx).thread.id != thread {
                    cx.notify();
                    return;
                }
                let mut errors = Vec::new();
                match result {
                    Ok(results) => {
                        for result in results {
                            match result {
                                Ok(image) => {
                                    this.chat.update(cx, |c, cx| c.add_attachment(image, cx));
                                }
                                Err(e) => errors.push(e.message),
                            }
                        }
                    }
                    Err(e) => errors.push(e.message),
                }
                this.image_error = (!errors.is_empty()).then(|| errors.join("\n"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn choose_images(&mut self, cx: &mut Context<Self>) {
        self.attachment_menu_open = false;
        cx.notify();
        if self.image_loading || self.chat.read(cx).streaming() {
            return;
        }
        let thread = self.chat.read(cx).thread.id.clone();
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach images".into()),
        });
        self.image_loading = true;
        self.image_picker_open = true;
        self.image_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = picker.await;
            this.update(cx, |this, cx| {
                this.image_loading = false;
                this.image_picker_open = false;
                if this.chat.read(cx).thread.id != thread {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        this.load_images(paths.into_iter().map(ImageSource::File).collect(), cx);
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => {
                        this.image_error = Some(format!(
                            "Could not open the file picker: {e}. You can also paste or drop images."
                        ));
                    }
                    Err(_) => {
                        this.image_error = Some(
                            "The file picker closed unexpectedly. You can also paste or drop images."
                                .into(),
                        );
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn attachment_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = &cx.tokens().colors;
        let disabled = self.image_loading || self.chat.read(cx).streaming();
        div()
            .id("attachment-control")
            .relative()
            .track_focus(&self.attachment_focus)
            .tab_index(0)
            .rounded(px(contextual_theme::radius::SM))
            .border_1()
            .border_color(hsla(c.surface_200))
            .focus(|s| s.border_color(hsla(c.focus_ring)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(|t, e: &KeyDownEvent, window, cx| {
                match e.keystroke.key.as_str() {
                    "escape" if t.attachment_menu_open => {
                        t.attachment_menu_open = false;
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "enter" | "space" if !t.image_loading && !t.chat.read(cx).streaming() => {
                        cx.stop_propagation();
                        if t.attachment_menu_open {
                            t.choose_images(cx);
                            t.focus_input(window, cx);
                        } else {
                            t.attachment_menu_open = true;
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            }))
            .child(
                IconButton::new("attach", IconName::Plus, "Add attachment")
                    .disabled(disabled)
                    .on_click(cx.listener(|t, _, window, cx| {
                        window.focus(&t.attachment_focus, cx);
                        t.attachment_menu_open = !t.attachment_menu_open;
                        cx.notify();
                    })),
            )
            .when(self.attachment_menu_open && !disabled, |d| {
                d.child(
                    div()
                        .id("attachment-menu")
                        .absolute()
                        .bottom(control_height() + sp(1))
                        .left_0()
                        .p(sp(1))
                        .rounded(px(contextual_theme::radius::MD))
                        .border_1()
                        .border_color(hsla(c.border_strong))
                        .bg(hsla(c.surface_200))
                        .shadow(vec![shadow(cx.tokens().shadows.popover)])
                        .child(
                            Button::new("attach-images", "Images")
                                .icon(gpui_kit::assets::IconName::Image)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|t, _, window, cx| {
                                    t.choose_images(cx);
                                    t.focus_input(window, cx);
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    fn inspect_control(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = &cx.tokens().colors;
        let chat = self.chat.read(cx);
        let enabled = !chat.thread.transcript.is_empty() || chat.live.is_some();
        div()
            .id("inspect-control")
            .flex_none()
            .track_focus(&self.inspect_focus)
            .when(enabled, |d| d.tab_index(0))
            .rounded(px(contextual_theme::radius::SM))
            .border_1()
            .border_color(hsla(c.surface_100))
            .focus(|s| s.border_color(hsla(c.focus_ring)))
            .on_key_down(cx.listener(|t, e: &KeyDownEvent, _, cx| {
                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                    let chat = t.chat.read(cx);
                    if !chat.thread.transcript.is_empty() || chat.live.is_some() {
                        cx.stop_propagation();
                        t.chat.update(cx, |c, cx| {
                            c.set_center(CenterView::Inspect, cx);
                        });
                    }
                }
            }))
            .child(
                IconButton::with_glyph(
                    "inspect",
                    Icon::new(gpui_kit::assets::IconName::Brain).size(sp(4)),
                    "Inspect conversation",
                )
                .disabled(!enabled)
                .on_click(cx.listener(|t, _, _, cx| {
                    t.chat.update(cx, |c, cx| {
                        c.set_center(CenterView::Inspect, cx);
                    });
                })),
            )
            .into_any_element()
    }

    fn paste(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let mut sources = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(img) => {
                    sources.push(ImageSource::Clipboard {
                        bytes: img.bytes.to_vec(),
                        name: attachment_name("", runtime::now_local().time()),
                    });
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    sources.extend(
                        paths
                            .paths()
                            .iter()
                            .filter(|p| mime_for_path(p).is_some())
                            .cloned()
                            .map(ImageSource::File),
                    );
                }
                _ => {}
            }
        }
        let handled = !sources.is_empty();
        self.load_images(sources, cx);
        // Ordinary text (including non-image file paths) keeps the textarea's default paste.
        handled
    }

    fn drop_paths(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        self.load_images(
            paths
                .paths()
                .iter()
                .cloned()
                .map(ImageSource::File)
                .collect(),
            cx,
        );
    }
}

/// One message of the thread.
fn render_message(
    this: &ChatPanel,
    ix: usize,
    entity: &WeakEntity<ChatPanel>,
    cx: &App,
) -> AnyElement {
    let c = &cx.tokens().colors;
    let chat = this.chat.read(cx);
    let Some(m) = chat.messages.get(ix) else {
        return div().into_any_element();
    };
    let row = div()
        .w_full()
        .min_w_0()
        .flex()
        .justify_center()
        .px(sp(4))
        .py(sp(3));
    let column = div()
        .w_full()
        .min_w_0()
        .max_w(px(layout::CONTENT_MAX_WIDTH))
        .type_style(contextual_theme::type_style::UI_PROSE);
    let el = match m.role {
        Role::Note => column
            .type_style(contextual_theme::type_style::UI_SMALL)
            .text_color(hsla(match m.tone {
                Some(Tone::Error) => c.danger,
                Some(Tone::Warn) => c.warning,
                _ => c.ink_muted,
            }))
            .child(SharedString::from(m.text.clone())),
        Role::User => column.child(
            div()
                .p(sp(4))
                .rounded(px(contextual_theme::radius::LG))
                .bg(hsla(c.surface_200))
                .child(header("You", m, cx))
                .child(
                    div()
                        .mt(sp(2))
                        .whitespace_normal()
                        .child(SharedString::from(m.text.clone())),
                )
                .when_some(m.images.as_ref().filter(|i| !i.is_empty()), |d, imgs| {
                    d.child(
                        div()
                            .mt(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(
                                imgs.iter()
                                    .map(|i| i.name.clone())
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                            )),
                    )
                }),
        ),
        Role::Assistant => {
            let changes = m.changes.as_deref().unwrap_or_default();
            let prepared = chat
                .reply_preview
                .as_ref()
                .filter(|(id, _)| id == &m.id)
                .map(|(_, blocks)| blocks);
            let parsed = this.replies.get(&m.id, &m.text, prepared);
            let diffs = this.diffs.read(cx).get(&message_key(chat, m));
            let weak = entity.clone();
            let body = render_reply(
                &m.id,
                &parsed,
                changes,
                diffs.as_ref().map(|diffs| diffs.as_slice()),
                m.streaming,
                m.streaming && chat.activity() == Some(ChatActivity::WritingResponse),
                this.app.read(cx).settings.reduce_motion,
                &this.expanded,
                move |key, cx| {
                    weak.update(cx, |t, cx| {
                        t.expanded.insert(key);
                        t.list.reset(t.rendered);
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            );
            let chat_entity = this.chat.clone();
            column
                .child(header(m.model.as_deref().unwrap_or("Assistant"), m, cx))
                .child(div().mt(sp(2)).children(body))
                .when_some(m.reads.clone(), |d, log| {
                    d.child(read_row(this, &m.id, &log, entity, cx))
                })
                .when(!changes.is_empty() && !m.streaming, |d| {
                    d.child(
                        div()
                            .mt(sp(2))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(format!(
                                "{} {} changed",
                                changes.len(),
                                if changes.len() == 1 { "file" } else { "files" }
                            )))
                            .child(
                                Button::new(("review", ix), "Review diff")
                                    .size(ButtonSize::Xs)
                                    .on_click(move |_, _, cx| {
                                        chat_entity.update(cx, |c, cx| {
                                            c.set_center(CenterView::Diff, cx);
                                        })
                                    }),
                            ),
                    )
                })
        }
    };
    row.child(el).into_any_element()
}

/// "Loaded skill frontend-design · read 4 files · ~12k tokens", expandable to the paths; with
/// the buttons when a request over the limit waits for the user.
fn read_row(
    this: &ChatPanel,
    id: &str,
    log: &ReadLog,
    entity: &WeakEntity<ChatPanel>,
    cx: &App,
) -> AnyElement {
    let c = &cx.tokens().colors;
    let key = format!("reads-{id}");
    let open = this.expanded.contains(&key);
    let chat = this.chat.read(cx);
    let reading = chat.activity() == Some(ChatActivity::ReadingFiles)
        && chat.messages.last().is_some_and(|message| message.id == id);
    let pending = chat
        .pending
        .as_ref()
        .filter(|p| p.ai_id() == id)
        .map(|p| p.size());
    let toggle = entity.clone();
    let toggle_key = key.clone();
    let paths: Vec<String> = log
        .skills
        .iter()
        .chain(&log.files)
        .chain(log.folders.iter())
        .cloned()
        .chain(log.refused.iter().map(|r| format!("{r}: not sent")))
        .chain(log.held_back.iter().map(|h| format!("{h}: held back")))
        .collect();
    div()
        .mt(sp(2))
        .flex()
        .flex_col()
        .gap(sp(1))
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(c.ink_muted))
        .child(
            div()
                .id(SharedString::from(key.clone()))
                .flex()
                .items_center()
                .gap(sp(1))
                .cursor_pointer()
                .hover(|s| s.text_color(hsla(c.ink)))
                .on_click(move |_, _, cx| {
                    toggle
                        .update(cx, |t, cx| {
                            if !t.expanded.remove(&toggle_key) {
                                t.expanded.insert(toggle_key.clone());
                            }
                            t.list.reset(t.rendered);
                            cx.notify();
                        })
                        .ok();
                })
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(12.0)),
                )
                .child(if reading {
                    activity_mark(
                        ElementId::Name(format!("{key}-activity").into()),
                        !this.app.read(cx).settings.reduce_motion,
                        cx,
                    )
                } else if log.completed
                    && pending.is_none()
                    && log.refused.is_empty()
                    && log.held_back.is_empty()
                {
                    completed_mark(cx).into_any_element()
                } else {
                    Icon::new(IconName::File).size(sp(4)).into_any_element()
                })
                .child(SharedString::from(if reading {
                    "Reading requested context…".to_string()
                } else if pending.is_some() {
                    "Context request · waiting for your approval".to_string()
                } else {
                    log.summary()
                })),
        )
        .when(open, |d| {
            d.child(
                div()
                    .pl(sp(6))
                    .flex()
                    .flex_col()
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .children(paths.into_iter().map(|p| div().child(SharedString::from(p)))),
            )
        })
        .when_some(pending, |d, (files, tokens)| {
            let (send_chat, skip_chat) = (this.chat.clone(), this.chat.clone());
            let (send_project, skip_project) = (this.project.clone(), this.project.clone());
            d.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(sp(2))
                    .text_color(hsla(c.warning))
                    .child(SharedString::from(format!(
                        "The model wants {files} more {} (~{} tokens), past the limit for one message.",
                        if files == 1 { "file" } else { "files" },
                        fmt_k(tokens)
                    )))
                    .child(
                        Button::new("reads-send", "Send them")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Primary)
                            .on_click(move |_, _, cx| {
                                let rules = send_project.read(cx).rules_on();
                                send_chat.update(cx, |c, cx| c.answer_pending(true, rules, cx));
                            }),
                    )
                    .child(
                        Button::new("reads-skip", "Skip")
                            .size(ButtonSize::Xs)
                            .on_click(move |_, _, cx| {
                                let rules = skip_project.read(cx).rules_on();
                                skip_chat.update(cx, |c, cx| c.answer_pending(false, rules, cx));
                            }),
                    ),
            )
        })
        .into_any_element()
}

fn header(who: &str, m: &UiMessage, cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    div()
        .flex()
        .flex_wrap()
        .min_w_0()
        .items_center()
        .gap(sp(2))
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(c.ink_muted))
        .child(
            div()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(hsla(c.ink_strong))
                .child(SharedString::from(who.to_string())),
        )
        .child(SharedString::from(m.time.clone()))
        .when_some(m.meta.clone(), |d, meta| {
            d.child(SharedString::from(format!("· {meta}")))
        })
}

impl Render for ChatPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.list_zoom.refresh(&self.list, self.rendered);
        let c = &cx.tokens().colors;
        let chat = self.chat.read(cx);
        let streaming = chat.streaming();
        let saving = chat.saving;
        let empty = chat.messages.is_empty();
        let reduce_motion = self.app.read(cx).settings.reduce_motion;
        // One stable location for all foreground work. User decisions are not busy work,
        // so approval and the native picker show a static mark rather than an animation.
        let activity = chat
            .activity()
            .map(|activity| (activity.label(), true))
            .or_else(|| {
                chat.pending
                    .as_ref()
                    .map(|_| ("Waiting for your approval", false))
            })
            .or_else(|| {
                if self.image_picker_open {
                    Some(("Choose images", false))
                } else {
                    self.image_loading.then_some(("Preparing images", true))
                }
            });
        let attachments: Vec<_> = chat
            .attachments
            .iter()
            .map(|a| (a.id.clone(), a.name.clone(), a.width, a.height, a.tokens))
            .collect();
        let entity = cx.entity().downgrade();
        let weak = entity.clone();
        let paste_target = entity.clone();
        let attachment_control = self.attachment_control(cx);
        let inspect_control = self.inspect_control(cx);

        let thread = if empty {
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(sp(2))
                        .type_style(contextual_theme::type_style::UI_PROSE)
                        .text_color(hsla(c.ink_muted))
                        .child(
                            div()
                                .type_style(contextual_theme::type_style::UI_TITLE)
                                .text_color(hsla(c.ink_strong))
                                .child("Ask for a change"),
                        ),
                )
                .into_any_element()
        } else {
            list(self.list.clone(), move |ix, _, cx| {
                let Some(this) = weak.upgrade() else {
                    return div().into_any_element();
                };
                this.update(cx, |panel, cx| {
                    ensure_message(&panel.diffs, &panel.chat, Some(ix), cx);
                    render_message(panel, ix, &weak, cx)
                })
            })
            .flex_1()
            .min_h_0()
            .into_any_element()
        };

        let chips = attachments
            .into_iter()
            .enumerate()
            .map(|(i, (id, name, w, h, tokens))| {
                div()
                    .flex()
                    .max_w_full()
                    .min_w_0()
                    .items_center()
                    .gap(sp(1))
                    .pl(sp(2))
                    .rounded(px(contextual_theme::radius::SM))
                    .bg(hsla(c.pill_bg))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .child(
                        div()
                            .min_w_0()
                            .whitespace_normal()
                            .child(SharedString::from(format!(
                                "{name} · {w}×{h} · ~{}",
                                fmt_k(tokens)
                            ))),
                    )
                    .child(
                        Button::new(("rm-img", i), "Remove")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .disabled(streaming)
                            .on_click(cx.listener(move |t, _, _, cx| {
                                t.chat.update(cx, |c, cx| {
                                    if !c.streaming() {
                                        c.attachments.retain(|a| a.id != id);
                                        cx.notify();
                                    }
                                })
                            })),
                    )
            })
            .collect::<Vec<_>>();

        div()
            .id("chat-image-drop-target")
            .on_mouse_down(MouseButton::Left, cx.listener(|t, _, _, cx| {
                if t.attachment_menu_open {
                    t.attachment_menu_open = false;
                    cx.notify();
                }
            }))
            .on_mouse_down_out(cx.listener(|t, _, _, cx| {
                if t.attachment_menu_open {
                    t.attachment_menu_open = false;
                    cx.notify();
                }
            }))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .when(!streaming && !self.image_loading, |d| {
                d.drag_over::<ExternalPaths>(|s, _, _, cx| {
                    s.bg(hsla(cx.tokens().colors.surface_selected))
                })
            })
            .on_drop(cx.listener(|t, paths: &ExternalPaths, _, cx| {
                t.drop_paths(paths, cx)
            }))
            .child(thread)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .min_w_0()
                    .w_full()
                    .items_center()
                    .px(sp(4))
                    .pb(sp(3))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .min_w_0()
                            .w_full()
                            .max_w(px(layout::CONTENT_MAX_WIDTH))
                            .child(
                                div()
                                    .flex()
                                    .min_w_0()
                                    .items_start()
                                    .justify_end()
                                    .gap(sp(2))
                                    .child(
                                        div().flex_1().min_w_0().child(context_bar(
                                            &self.app,
                                            &self.project,
                                            Some(&self.chat),
                                            &self.context,
                                            cx,
                                        )),
                                    )
                                    .child(inspect_control),
                            )
                            .child(
                                div()
                                    .id("composer")
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .w_full()
                                    .gap(sp(2))
                                    .p(sp(2))
                                    .rounded(px(contextual_theme::radius::MD))
                                    .border_1()
                                    .border_color(hsla(c.border_strong))
                                    .bg(hsla(c.surface_200))
                                    .when(!streaming && !self.image_loading, |d| {
                                        d.drag_over::<ExternalPaths>(|s, _, _, cx| {
                                            s.border_color(hsla(cx.tokens().colors.accent))
                                        })
                                    })
                                    .when_some(self.image_error.clone(), |d, error| {
                                        d.child(
                                            div()
                                                .p(sp(2))
                                                .rounded(px(contextual_theme::radius::SM))
                                                .border_1()
                                                .border_color(hsla(c.warning_border))
                                                .bg(hsla(c.warning_bg))
                                                .type_style(contextual_theme::type_style::UI_SMALL)
                                                .text_color(hsla(c.ink))
                                                .whitespace_normal()
                                                .child(
                                                    div()
                                                        .text_color(hsla(c.warning))
                                                        .child("Attachment warning"),
                                                )
                                                .child(div().child(SharedString::from(error))),
                                        )
                                    })
                                    .when(!chips.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .id("composer-attachments")
                                                .flex()
                                                .flex_wrap()
                                                .min_w_0()
                                                .max_h(px(contextual_theme::space::ROW_HEIGHT * 3.0))
                                                .overflow_y_scroll()
                                                .gap(sp(1))
                                                .children(chips),
                                        )
                                    })
                                    .child(Textarea::new(&self.input).appearance(false).on_paste(
                                        move |item, _, cx| {
                                            paste_target
                                                .update(cx, |t, cx| t.paste(item, cx))
                                                .unwrap_or(false)
                                        },
                                    ))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(sp(1))
                                            .child(attachment_control)
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .when_some(activity, |d, (label, animate)| {
                                                        d.child(
                                                            div()
                                                                .flex()
                                                                .items_center()
                                                                .gap(sp(2))
                                                                .px(sp(1))
                                                                .type_style(
                                                                    contextual_theme::type_style::UI_SMALL,
                                                                )
                                                                .text_color(hsla(c.ink_muted))
                                                                .child(activity_mark(
                                                                    "chat-activity",
                                                                    animate && !reduce_motion,
                                                                    cx,
                                                                ))
                                                                .child(
                                                                    div()
                                                                        .min_w_0()
                                                                        .whitespace_normal()
                                                                        .child(label),
                                                                ),
                                                        )
                                                    }),
                                            )
                                            .map(|d| {
                                                if streaming {
                                                    d.child(Button::new("stop", "Stop").on_click(
                                                        cx.listener(|t, _, _, cx| {
                                                            t.chat.update(cx, |c, cx| c.stop(cx))
                                                        }),
                                                    ))
                                                } else {
                                                    d.child(
                                                        div()
                                                            .id("send-tooltip")
                                                            .flex_none()
                                                            .tooltip(|window, cx| {
                                                                gpui_kit::component::tooltip::Tooltip::new(
                                                                    "Send (Ctrl+Enter)",
                                                                )
                                                                .build(window, cx)
                                                            })
                                                            .child(
                                                                Button::new("send", "")
                                                                    .icon(gpui_kit::assets::IconName::Send)
                                                                    .variant(ButtonVariant::Primary)
                                                                    .disabled(
                                                                        self.image_loading
                                                                            || self.settings_prompt_open
                                                                            || saving,
                                                                    )
                                                                    .on_click(cx.listener(
                                                                        |t, _, w, cx| t.send(w, cx),
                                                                    )),
                                                            ),
                                                    )
                                                }
                                            }),
                                    ),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_prompt_names_missing_and_invalid_fields() {
        let detail = settings_prompt_detail(&FoundryStatus::Unconfigured {
            missing: vec![config::ENV_API_KEY.into(), config::ENV_DEPLOYMENT.into()],
            invalid: vec![config::ENV_ENDPOINT.into()],
        });
        assert_eq!(detail, "Missing: API key, Deployment.\nInvalid: Endpoint.");
    }

    #[test]
    fn settings_prompt_omits_empty_problem_lists() {
        let detail = settings_prompt_detail(&FoundryStatus::Unconfigured {
            missing: vec![config::ENV_API_VERSION.into()],
            invalid: Vec::new(),
        });
        assert_eq!(detail, "Missing: API version.");
    }

    #[test]
    fn settings_prompt_has_a_fallback_without_field_problems() {
        assert_eq!(
            settings_prompt_detail(&FoundryStatus::Checking),
            "Azure AI Foundry is not configured."
        );
    }
}
