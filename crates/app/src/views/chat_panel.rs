//! The chat (design specs: ChatMessage, Composer): the thread as a virtualised list, then the
//! context bar and the composer. Ctrl+Enter sends; pictures can be pasted or dropped.

use std::collections::HashSet;
use std::io::Read as _;
use std::path::PathBuf;

use contextual_core::chat::{ReadLog, Role, Tone, UiMessage};
use contextual_core::logic::format::fmt_k;
use contextual_core::logic::images::{
    Attachment, IMAGE_TYPES, MAX_SOURCE_BYTES, attachment_name, to_attachment,
};
use contextual_theme::layout;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardEntry, ClipboardItem, Context, Entity, ExternalPaths,
    ImageFormat, InteractiveElement, IntoElement, ListAlignment, ListState, ParentElement,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    WeakEntity, Window, div, list, px,
};

use crate::runtime;
use crate::state::chat::{CenterView, ChatModel, SendEnv};
use crate::state::{AppModel, Project};
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant};
use crate::views::context_bar::context_bar;
use crate::views::markdown::render_reply;

pub struct ChatPanel {
    app: Entity<AppModel>,
    project: Entity<Project>,
    chat: Entity<ChatModel>,
    input: Entity<TextareaState>,
    list: ListState,
    /// Code blocks the user expanded ("Show N more lines").
    expanded: HashSet<String>,
    rendered: usize,
    image_loading: bool,
    image_error: Option<String>,
    _subs: Vec<Subscription>,
}

/// These are explicitly chosen local attachments, not project paths resolved through the host.
enum ImageSource {
    File(PathBuf),
    Clipboard {
        bytes: Vec<u8>,
        mime: &'static str,
        name: String,
    },
}

/// File I/O and image decoding run on the blocking pool, never on the UI thread.
fn prepare_image(source: ImageSource) -> contextual_core::Result<Attachment> {
    let (bytes, mime, name) = match source {
        ImageSource::Clipboard { bytes, mime, name } => (bytes, mime, name),
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
    to_attachment(
        &bytes,
        mime,
        &name,
        &contextual_core::chat::new_id("img"),
    )
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

impl ChatPanel {
    pub fn new(
        app: Entity<AppModel>,
        project: Entity<Project>,
        chat: Entity<ChatModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 12)
                .placeholder("Ask for a change… (Ctrl+Enter to send)")
        });
        let count = chat.read(cx).messages.len();
        let subs = vec![
            cx.observe(&chat, |this: &mut Self, chat, cx| {
                let n = chat.read(cx).messages.len();
                if n != this.rendered {
                    this.list.reset(n);
                    this.list.scroll_to_end();
                } else if n > 0 {
                    // The last message grows while streaming: re-measure it.
                    this.list.splice(n - 1..n, 1);
                }
                this.rendered = n;
                cx.notify();
            }),
            cx.observe(&project, |_, _, cx| cx.notify()),
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = e
                {
                    this.send(window, cx);
                }
            }),
        ];
        Self {
            app,
            project,
            chat,
            input,
            list: ListState::new(count, ListAlignment::Bottom, px(400.0)),
            expanded: HashSet::new(),
            rendered: count,
            image_loading: false,
            image_error: None,
            _subs: subs,
        }
    }

    /// Sends `text` as if typed (debug automation, see `dev.rs`).
    pub fn dev_send(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.set_value(text, window, cx));
        self.send(window, cx);
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        let chat = self.chat.read(cx);
        if self.image_loading
            || chat.streaming()
            || (text.is_empty() && chat.attachments.is_empty())
        {
            return;
        }
        let a = self.app.read(cx);
        let Some(config) = a.config.clone() else {
            self.app.update(cx, |a, cx| {
                a.error = Some("Foundry is not configured: open Settings.".into());
                cx.notify();
            });
            return;
        };
        let auto = a.settings.chat.model_picks_files;
        let p = self.project.read(cx);
        let env = SendEnv {
            config,
            deployment: a.deployment(),
            output_prompt: a.settings.workspace.output_prompt.clone(),
            auto,
            skill_index: if auto { p.skill_index() } else { String::new() },
        };
        let rules = p.rules_on();
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.chat.update(cx, |c, cx| c.send(text, env, rules, cx));
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
        self.image_loading = true;
        self.image_error = None;
        let thread = self.chat.read(cx).thread.id.clone();
        cx.notify();
        let job = runtime::spawn(cx, async move {
            tokio::task::spawn_blocking(move || {
                sources.into_iter().map(prepare_image).collect::<Vec<_>>()
            })
            .await
            .map_err(|e| {
                contextual_core::Error::internal(format!("Could not prepare images: {e}"))
            })
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
        self.image_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = picker.await;
            this.update(cx, |this, cx| {
                this.image_loading = false;
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

    fn paste_image(&mut self, cx: &mut Context<Self>) {
        let handled = cx
            .read_from_clipboard()
            .is_some_and(|item| self.paste(&item, cx));
        if !handled {
            self.image_error = Some(
                "No supported image on the clipboard. Copy an image or choose Attach images…"
                    .into(),
            );
            cx.notify();
        }
    }

    fn paste(&mut self, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let mut sources = Vec::new();
        let mut unsupported = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(img) => {
                    if let Some(mime) = mime_for_format(img.format) {
                        sources.push(ImageSource::Clipboard {
                            bytes: img.bytes.to_vec(),
                            mime,
                            name: attachment_name("", runtime::now_local().time()),
                        });
                    } else {
                        unsupported = true;
                    }
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
        let handled = !sources.is_empty() || unsupported;
        self.load_images(sources, cx);
        if unsupported {
            self.image_error =
                Some("Clipboard image format not supported. Use PNG, JPEG, GIF or WebP.".into());
            cx.notify();
        }
        // Ordinary text (including non-image file paths) keeps the textarea's default paste.
        handled
    }

    fn drop_paths(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        self.load_images(
            paths.paths().iter().cloned().map(ImageSource::File).collect(),
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
    let row = div().w_full().flex().justify_center().px(sp(4)).py(sp(3));
    let column = div()
        .w_full()
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
            let changes = m.changes.clone().unwrap_or_default();
            let weak = entity.clone();
            let body = render_reply(
                &m.id,
                &m.text,
                &changes,
                m.streaming,
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
                .when(m.streaming && m.text.is_empty(), |d| {
                    d.child(div().text_color(hsla(c.ink_muted)).child("Thinking…"))
                })
                .when_some(m.reads.clone(), |d, log| {
                    d.child(read_row(this, &m.id, &log, entity, cx))
                })
                .when(!changes.is_empty() && !m.streaming, |d| {
                    d.child(
                        div()
                            .mt(sp(2))
                            .flex()
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
                                            c.center = CenterView::Diff;
                                            cx.notify();
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
    let pending = this
        .chat
        .read(cx)
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
                .child(Icon::new(IconName::File).size(px(12.0)))
                .child(SharedString::from(log.summary())),
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
        let c = &cx.tokens().colors;
        let chat = self.chat.read(cx);
        let streaming = chat.streaming();
        let resend = chat.resend_all;
        let auto = self.app.read(cx).settings.chat.model_picks_files;
        let empty = chat.messages.is_empty();
        let attachments: Vec<_> = chat
            .attachments
            .iter()
            .map(|a| (a.id.clone(), a.name.clone(), a.width, a.height, a.tokens))
            .collect();
        let entity = cx.entity().downgrade();
        let weak = entity.clone();
        let paste_target = entity.clone();

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
                        .when(auto, |d| {
                            d.child(div().type_style(contextual_theme::type_style::UI_TITLE).text_color(hsla(c.ink_strong)).child("Ask for a change"))
                                .child("The model gets the project tree and asks for the files and skills it needs. Tick files to include them yourself.")
                        })
                        .when(!auto, |d| {
                            d.child(div().type_style(contextual_theme::type_style::UI_TITLE).text_color(hsla(c.ink_strong)).child("Pick the files the model should see"))
                                .child("Tick files in the explorer, then ask for a change. Click a file to preview it.")
                        }),
                )
                .into_any_element()
        } else {
            list(self.list.clone(), move |ix, _, cx| {
                let Some(this) = weak.upgrade() else {
                    return div().into_any_element();
                };
                let panel = this.read(cx);
                render_message(panel, ix, &weak, cx)
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
                    .items_center()
                    .gap(sp(1))
                    .pl(sp(2))
                    .rounded(px(contextual_theme::radius::SM))
                    .bg(hsla(c.pill_bg))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .child(SharedString::from(format!(
                        "{name} · {w}×{h} · ~{}",
                        fmt_k(tokens)
                    )))
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
        let mode = |id: &'static str, label: &'static str, on: bool| {
            Button::new(id, label)
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Ghost)
                .selected(on)
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(thread)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .items_center()
                    .px(sp(4))
                    .pb(sp(3))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(layout::CONTENT_MAX_WIDTH))
                            .child(context_bar(&self.app, &self.project, Some(&self.chat), cx))
                            .child(
                                div()
                                    .id("composer")
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
                                    .on_drop(cx.listener(|t, paths: &ExternalPaths, _, cx| {
                                        t.drop_paths(paths, cx)
                                    }))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .items_center()
                                            .gap(sp(2))
                                            .child(
                                                Button::new("attach-images", "Attach images…")
                                                    .size(ButtonSize::Xs)
                                                    .disabled(streaming || self.image_loading)
                                                    .on_click(cx.listener(|t, _, _, cx| {
                                                        t.choose_images(cx)
                                                    })),
                                            )
                                            .child(
                                                Button::new("paste-image", "Paste image")
                                                    .size(ButtonSize::Xs)
                                                    .variant(ButtonVariant::Ghost)
                                                    .disabled(streaming || self.image_loading)
                                                    .on_click(cx.listener(|t, _, _, cx| {
                                                        t.paste_image(cx)
                                                    })),
                                            )
                                            .child(
                                                div()
                                                    .type_style(
                                                        contextual_theme::type_style::UI_SMALL,
                                                    )
                                                    .text_color(hsla(c.ink_muted))
                                                    .child(if self.image_loading {
                                                        "Preparing images…"
                                                    } else {
                                                        "PNG, JPEG, GIF or WebP · up to 20 MiB each"
                                                    }),
                                            ),
                                    )
                                    .when_some(self.image_error.clone(), |d, error| {
                                        d.child(
                                            div()
                                                .type_style(contextual_theme::type_style::UI_SMALL)
                                                .text_color(hsla(c.danger))
                                                .whitespace_normal()
                                                .child(SharedString::from(error)),
                                        )
                                    })
                                    .when(!chips.is_empty(), |d| {
                                        d.child(div().flex().flex_wrap().gap(sp(1)).children(chips))
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
                                            .child(
                                                mode("only-new", "Only send new message", !resend)
                                                    .on_click(cx.listener(|t, _, _, cx| {
                                                        t.chat.update(cx, |c, cx| {
                                                            c.resend_all = false;
                                                            cx.notify();
                                                        })
                                                    })),
                                            )
                                            .child(
                                                mode("resend", "Resend full context", resend)
                                                    .on_click(cx.listener(|t, _, _, cx| {
                                                        t.chat.update(cx, |c, cx| {
                                                            c.resend_all = true;
                                                            cx.notify();
                                                        })
                                                    })),
                                            )
                                            .child(div().w(sp(2)))
                                            .child(
                                                mode("auto-pick", "Model picks files", auto)
                                                    .on_click(cx.listener(|t, _, _, cx| {
                                                        t.app.update(cx, |a, cx| {
                                                            let mut s = a.settings.clone();
                                                            s.chat.model_picks_files =
                                                                !s.chat.model_picks_files;
                                                            a.save_settings(s, None, cx);
                                                        })
                                                    })),
                                            )
                                            .child(div().flex_1())
                                            .map(|d| {
                                                if streaming {
                                                    d.child(Button::new("stop", "Stop").on_click(
                                                        cx.listener(|t, _, _, cx| {
                                                            t.chat.update(cx, |c, cx| c.stop(cx))
                                                        }),
                                                    ))
                                                } else {
                                                    d.child(
                                                        Button::new("send", "Send")
                                                            .variant(ButtonVariant::Primary)
                                                            .disabled(self.image_loading)
                                                            .on_click(cx.listener(
                                                                |t, _, w, cx| t.send(w, cx),
                                                            )),
                                                    )
                                                }
                                            }),
                                    ),
                            ),
                    ),
            )
    }
}
