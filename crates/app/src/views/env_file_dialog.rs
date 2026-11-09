//! Choose an env file to import (Settings → Azure AI Foundry): browses this machine's folders
//! with dotenv files (`.env`, `.env.local`, `prod.env`) listed first. In-app rather than the OS
//! picker, which Linux/WSLg often lacks (no desktop portal); typed paths work too, including
//! `\\wsl.localhost\…` on Windows.

use contextual_core::workspace::browse::{FileEntry, list_dirs, list_files};
use contextual_protocol::fs::DirListing;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, EventEmitter, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div,
};

use crate::runtime;
use crate::theme::{ActiveTheme, TypeExt, hsla, px, row_height, sp};
use crate::ui::{Button, ButtonVariant, IconButton, Switch, Tag, file_glyph, folder_glyph};

const W: f32 = 680.0;
const H: f32 = 560.0;

pub enum EnvFileEvent {
    Picked(String),
    Closed,
}

pub struct EnvFileDialog {
    input: Entity<InputState>,
    listing: Option<DirListing>,
    files: Vec<FileEntry>,
    selected: Option<String>,
    hidden: bool,
    all: bool,
    loading: bool,
    error: Option<String>,
    focus: FocusHandle,
    /// A path to put in the input on the next render (setting it needs the window).
    pending_input: Option<String>,
    _sub: Subscription,
}

impl EventEmitter<EnvFileEvent> for EnvFileDialog {}

impl EnvFileDialog {
    /// `start`: a file or folder to open at (the path already typed in Settings), else home.
    pub fn new(start: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("/home/you/old-app/.env.local or a folder")
        });
        let sub = cx.subscribe(&input, |this: &mut Self, input, e: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = e {
                let path = input.read(cx).value().to_string();
                this.go(Some(path), cx);
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            input,
            listing: None,
            files: Vec::new(),
            selected: None,
            hidden: false,
            all: false,
            loading: false,
            error: None,
            focus,
            pending_input: None,
            _sub: sub,
        };
        this.go(start.filter(|s| !s.trim().is_empty()), cx);
        this
    }

    /// Lists a folder (home when `None`). A path to a file lists its folder with the file picked.
    fn go(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let (hidden, all) = (self.hidden, self.all);
        let previous = self.selected.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        let job = runtime::spawn(cx, async move {
            let mut dir = path;
            let mut file = None;
            if let Some(p) = &dir {
                let target = contextual_core::workspace::paths::normalize_input_path(Some(p));
                if target.is_file() {
                    file = Some(contextual_core::workspace::paths::path_string(&target));
                    dir = target
                        .parent()
                        .map(contextual_core::workspace::paths::path_string);
                }
            }
            let listing = list_dirs(dir.as_deref(), hidden)?;
            let files = list_files(Some(&listing.path), hidden, all)?;
            Ok((listing, files, file))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((listing, files, file)) => {
                        this.pending_input = Some(file.clone().unwrap_or(listing.path.clone()));
                        // Pick the given file, else keep the pick, else the only dotenv file.
                        let envs: Vec<&FileEntry> = files.iter().filter(|f| f.env).collect();
                        this.selected = file
                            .or_else(|| previous.filter(|p| files.iter().any(|f| &f.path == p)))
                            .or_else(|| (envs.len() == 1).then(|| envs[0].path.clone()));
                        this.listing = Some(listing);
                        this.files = files;
                    }
                    Err(e) => this.error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let path = self.listing.as_ref().map(|l| l.path.clone());
        self.go(path, cx);
    }

    fn pick(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.selected.clone() {
            cx.emit(EnvFileEvent::Picked(p));
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(EnvFileEvent::Closed);
    }
}

impl Render for EnvFileDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(v) = self.pending_input.take() {
            self.input.update(cx, |s, cx| s.set_value(v, window, cx));
        }
        let c = &cx.tokens().colors;
        let listing = self.listing.clone();
        let current = listing.as_ref().map(|l| l.path.clone()).unwrap_or_default();
        let places = listing
            .as_ref()
            .map(|l| l.places.clone())
            .unwrap_or_default();
        let parent = listing.as_ref().and_then(|l| l.parent.clone());
        let dirs = listing.as_ref().map(|l| l.dirs.clone()).unwrap_or_default();
        let envs = self.files.iter().filter(|f| f.env).count();

        let row = |id: (&'static str, usize), selected: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(sp(2))
                .h(row_height())
                .px(sp(4))
                .cursor_pointer()
                .when(selected, |x| x.bg(hsla(c.surface_selected)))
                .when(!selected, |x| x.hover(|s| s.bg(hsla(c.surface_hover))))
        };
        let name = |text: String| {
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(SharedString::from(text))
        };
        let place_buttons: Vec<_> = places
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                let active = p.path == current;
                let path = p.path.clone();
                div()
                    .id(("place", i))
                    .flex()
                    .items_center()
                    .h(row_height())
                    .px(sp(2))
                    .rounded(px(contextual_theme::radius::SM))
                    .cursor_pointer()
                    .text_color(hsla(if active { c.ink } else { c.ink_muted }))
                    .when(active, |d| d.bg(hsla(c.surface_300)))
                    .hover(|s| s.bg(hsla(c.surface_hover)).text_color(hsla(c.ink)))
                    .on_click(cx.listener(move |t, _, _, cx| t.go(Some(path.clone()), cx)))
                    .child(SharedString::from(p.label))
            })
            .collect();
        let dir_rows: Vec<_> = dirs
            .into_iter()
            .enumerate()
            .map(|(i, d)| {
                let path = d.path.clone();
                row(("dir", i), false)
                    .on_click(cx.listener(move |t, _, _, cx| t.go(Some(path.clone()), cx)))
                    .child(folder_glyph(false, cx))
                    .child(name(d.name))
            })
            .collect();
        let file_rows: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let selected = self.selected.as_deref() == Some(f.path.as_str());
                let path = f.path.clone();
                row(("file", i), selected)
                    .on_click(cx.listener(move |t, e: &gpui_kit::ClickEvent, _, cx| {
                        t.selected = Some(path.clone());
                        if e.click_count() >= 2 {
                            t.pick(cx);
                        }
                        cx.notify();
                    }))
                    .child(file_glyph(&f.name, cx))
                    .child(name(f.name.clone()))
                    .when(f.env, |x| x.child(Tag::new("env")))
            })
            .collect();
        let empty = dir_rows.is_empty() && file_rows.is_empty() && !self.loading;

        div()
            .id("env-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(c.surface_scrim))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|t, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    t.close(cx);
                }
            }))
            .on_click(cx.listener(|t, _, _, cx| t.close(cx)))
            .child(
                div()
                    .id("env-dialog")
                    .flex()
                    .flex_col()
                    .w(px(W))
                    .h(px(H))
                    .max_w_full()
                    .max_h_full()
                    .overflow_hidden()
                    .rounded(px(contextual_theme::radius::LG))
                    .border_1()
                    .border_color(hsla(c.border_strong))
                    .bg(hsla(c.surface_100))
                    .shadow(vec![crate::theme::shadow(cx.tokens().shadows.modal)])
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .type_style(contextual_theme::type_style::UI_BODY)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .pt(sp(3))
                            .pr(sp(3))
                            .pb(sp(2))
                            .pl(sp(4))
                            .child(
                                div()
                                    .type_style(contextual_theme::type_style::UI_TITLE)
                                    .text_color(hsla(c.ink_strong))
                                    .child("Import .env file"),
                            )
                            .child(div().flex_1())
                            .child(
                                IconButton::new("close", IconName::Close, "Close")
                                    .on_click(cx.listener(|t, _, _, cx| t.close(cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(sp(2))
                            .px(sp(4))
                            .child(
                                div().flex_1().child(
                                    Input::new(&self.input)
                                        .small()
                                        .prefix(Icon::new(IconName::Folder).size(px(14.0))),
                                ),
                            )
                            .child(Button::new("go", "Go").on_click(cx.listener(|t, _, _, cx| {
                                let v = t.input.read(cx).value().to_string();
                                t.go(Some(v), cx)
                            }))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(1))
                            .px(sp(4))
                            .pt(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .children(place_buttons)
                            .child(div().flex_1())
                            .child(Switch::new("all-files", self.all).on_toggle(cx.listener(
                                |t, _, _, cx| {
                                    t.all = !t.all;
                                    t.reload(cx);
                                },
                            )))
                            .child("All files")
                            .child(div().w(sp(2)))
                            .child(Switch::new("hidden", self.hidden).on_toggle(cx.listener(
                                |t, _, _, cx| {
                                    t.hidden = !t.hidden;
                                    t.reload(cx);
                                },
                            )))
                            .child("Show hidden"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(1))
                            .h(px(28.0))
                            .px(sp(4))
                            .mt(sp(2))
                            .border_b_1()
                            .border_color(hsla(c.border))
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(
                                IconButton::new("up", IconName::ArrowUp, "Parent folder").on_click(
                                    cx.listener(move |t, _, _, cx| {
                                        if let Some(p) = parent.clone() {
                                            t.go(Some(p), cx)
                                        }
                                    }),
                                ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(SharedString::from(current)),
                            )
                            .child(
                                div()
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .child(SharedString::from(match envs {
                                        0 => "No env files here".to_string(),
                                        1 => "1 env file".to_string(),
                                        n => format!("{n} env files"),
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("env-entries")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .py(sp(1))
                            .children(dir_rows)
                            .children(file_rows)
                            .when(empty, |d| {
                                d.child(
                                    div()
                                        .px(sp(4))
                                        .py(sp(2))
                                        .type_style(contextual_theme::type_style::UI_SMALL)
                                        .text_color(hsla(c.ink_muted))
                                        .child("This folder is empty."),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(2))
                            .px(sp(4))
                            .py(sp(3))
                            .border_t_1()
                            .border_color(hsla(c.border))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .text_color(hsla(if self.error.is_some() {
                                        c.danger
                                    } else {
                                        c.ink_muted
                                    }))
                                    .child(SharedString::from(self.error.clone().unwrap_or_else(
                                        || {
                                            self.selected
                                                .clone()
                                                .unwrap_or_else(|| "No file chosen".into())
                                        },
                                    ))),
                            )
                            .child(
                                Button::new("cancel", "Cancel")
                                    .on_click(cx.listener(|t, _, _, cx| t.close(cx))),
                            )
                            .child(
                                Button::new("import", "Import file")
                                    .variant(ButtonVariant::Primary)
                                    .disabled(self.selected.is_none())
                                    .on_click(cx.listener(|t, _, _, cx| t.pick(cx))),
                            ),
                    ),
            )
    }
}
