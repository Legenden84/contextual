//! Open folder: a browser over the workspace host's `list_dirs`, so it browses the machine the
//! project lives on (needed for WSL; on Linux it is also the picker). Accepts typed paths.

use contextual_core::location::Location;
use contextual_protocol::fs::{DirListing, ListDirs};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px,
};

use crate::runtime;
use crate::state::AppModel;
use crate::theme::{ActiveTheme, TypeExt, hsla, row_height, sp};
use crate::ui::{Button, ButtonVariant, IconButton, Switch, Tag, folder_glyph};

/// Dialog size from the old FolderDialog.
const W: f32 = 680.0;
const H: f32 = 560.0;

pub struct FolderDialog {
    app: Entity<AppModel>,
    input: Entity<InputState>,
    listing: Option<DirListing>,
    selected: Option<String>,
    hidden: bool,
    loading: bool,
    error: Option<String>,
    focus: FocusHandle,
    /// A path to put in the input on the next render (setting it needs the window).
    pending_input: Option<String>,
    /// Where the listed folders live; clicks stay here, typed paths may move elsewhere.
    location: Location,
    _sub: Subscription,
}

impl FolderDialog {
    pub fn new(
        app: Entity<AppModel>,
        start: Option<(String, Location)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("/home/you/project or C:\\src\\project")
        });
        let sub = cx.subscribe(&input, |this: &mut Self, input, e: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = e {
                let path = input.read(cx).value().to_string();
                this.go_typed(path, cx);
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            app,
            input,
            listing: None,
            selected: None,
            hidden: false,
            loading: false,
            error: None,
            focus,
            pending_input: None,
            location: start
                .as_ref()
                .map(|(_, l)| l.clone())
                .unwrap_or(Location::Local),
            _sub: sub,
        };
        this.go(start.map(|(p, _)| p), cx);
        this
    }

    /// A path the user typed: it may point into WSL (`\\wsl.localhost\…`) or back to Windows.
    fn go_typed(&mut self, text: String, cx: &mut Context<Self>) {
        let detected = self.app.read(cx).detect_location(&text);
        match detected {
            Ok(root) => {
                self.location = root.location;
                self.go(Some(root.path), cx);
            }
            Err(e) => {
                self.error = Some(e);
                cx.notify();
            }
        }
    }

    /// Browses another location from its home folder.
    fn go_to(&mut self, location: Location, cx: &mut Context<Self>) {
        self.location = location;
        self.go(None, cx);
    }

    /// Lists `path` (home when `None`) in the current location.
    fn go(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        let location = self.location.clone();
        let host = self.app.update(cx, |a, _| a.host_for(&location));
        let req = ListDirs {
            path,
            hidden: self.hidden,
        };
        self.loading = true;
        self.error = None;
        cx.notify();
        let job = runtime::spawn(cx, async move { host.list_dirs(req).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(listing) => {
                        let shown = listing.path.clone();
                        this.listing = Some(listing);
                        this.selected = None;
                        this.pending_input = Some(shown);
                    }
                    Err(e) => this.error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let target = self
            .selected
            .clone()
            .or_else(|| self.listing.as_ref().map(|l| l.path.clone()));
        if let Some(path) = target {
            let location = self.location.clone();
            self.app
                .update(cx, |a, cx| a.open_folder(path, Some(location), cx));
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.app.update(cx, |a, cx| {
            a.folder_dialog_open = false;
            cx.notify();
        });
    }
}

impl Render for FolderDialog {
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
        let truncated = listing.as_ref().is_some_and(|l| l.truncated);
        let target = self.selected.clone().unwrap_or_else(|| current.clone());
        let opening = self.app.read(cx).scanning;
        let app_error = self.app.read(cx).error.clone();
        let error = self.error.clone().or(app_error);

        let place_buttons = places.into_iter().enumerate().map(|(i, p)| {
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
        });
        // Other locations: each WSL distro, and back to this machine when browsing inside WSL.
        let mut others: Vec<(String, Location)> = self
            .app
            .read(cx)
            .distros()
            .iter()
            .map(|d| {
                (
                    format!("WSL: {}", d.name),
                    Location::Wsl {
                        distro: d.name.clone(),
                    },
                )
            })
            .filter(|(_, l)| *l != self.location)
            .collect();
        if self.location != Location::Local
            && !self.app.read(cx).distros().iter().any(|d| d.name == "dev")
        {
            others.insert(0, ("This PC".into(), Location::Local));
        }
        let location_buttons: Vec<_> = others
            .into_iter()
            .enumerate()
            .map(|(i, (label, loc))| {
                div()
                    .id(("location", i))
                    .flex()
                    .items_center()
                    .h(row_height())
                    .px(sp(2))
                    .rounded(px(contextual_theme::radius::SM))
                    .cursor_pointer()
                    .text_color(hsla(c.ink_muted))
                    .hover(|s| s.bg(hsla(c.surface_hover)).text_color(hsla(c.ink)))
                    .on_click(cx.listener(move |t, _, _, cx| t.go_to(loc.clone(), cx)))
                    .child(SharedString::from(label))
            })
            .collect();
        let location_label = (self.location != Location::Local).then(|| self.location.label());
        let rows = dirs.into_iter().enumerate().map(|(i, d)| {
            let selected = self.selected.as_deref() == Some(d.path.as_str());
            let (pick, enter) = (d.path.clone(), d.path.clone());
            div()
                .id(("dir", i))
                .flex()
                .items_center()
                .gap(sp(2))
                .h(row_height())
                .px(sp(4))
                .cursor_pointer()
                .when(selected, |x| x.bg(hsla(c.surface_selected)))
                .when(!selected, |x| x.hover(|s| s.bg(hsla(c.surface_hover))))
                .on_click(cx.listener(move |t, e: &gpui_kit::ClickEvent, _, cx| {
                    if e.click_count() >= 2 {
                        t.go(Some(enter.clone()), cx);
                    } else {
                        t.selected = Some(pick.clone());
                        cx.notify();
                    }
                }))
                .child(folder_glyph(false, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(SharedString::from(d.name)),
                )
                .when(d.project, |x| x.child(Tag::new("project")))
        });

        div()
            .id("folder-scrim")
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
                    .id("folder-dialog")
                    .flex()
                    .flex_col()
                    .w(px(W))
                    .h(px(H))
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
                                    .child("Open folder"),
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
                                t.go_typed(v, cx)
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
                            .when_some(location_label, |d, l| {
                                d.child(
                                    div()
                                        .px(sp(1))
                                        .text_color(hsla(c.accent_ink))
                                        .child(SharedString::from(l)),
                                )
                            })
                            .children(place_buttons)
                            .children(location_buttons)
                            .child(div().flex_1())
                            .child(Switch::new("hidden", self.hidden).on_toggle(cx.listener(
                                |t, _, _, cx| {
                                    t.hidden = !t.hidden;
                                    let path = t.listing.as_ref().map(|l| l.path.clone());
                                    t.go(path, cx);
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
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(SharedString::from(current.clone())),
                            ),
                    )
                    .child(
                        div()
                            .id("dirs")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .py(sp(1))
                            .children(rows)
                            .when(truncated, |d| {
                                d.child(
                                    div()
                                        .px(sp(4))
                                        .py(sp(1))
                                        .type_style(contextual_theme::type_style::UI_SMALL)
                                        .text_color(hsla(c.ink_muted))
                                        .child("Only the first 500 folders are shown."),
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
                                    .text_color(hsla(if error.is_some() {
                                        c.danger
                                    } else {
                                        c.ink_muted
                                    }))
                                    .child(SharedString::from(error.unwrap_or_else(|| {
                                        if self.loading {
                                            "Loading…".into()
                                        } else {
                                            target.clone()
                                        }
                                    }))),
                            )
                            .child(
                                Button::new("cancel", "Cancel")
                                    .on_click(cx.listener(|t, _, _, cx| t.close(cx))),
                            )
                            .child(
                                Button::new("open", if opening { "Opening…" } else { "Open" })
                                    .variant(ButtonVariant::Primary)
                                    .disabled(opening || target.is_empty())
                                    .on_click(cx.listener(|t, _, _, cx| t.open(cx))),
                            ),
                    ),
            )
    }
}
