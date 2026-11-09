//! Chat history: saved conversations of this project (search, open, rename, delete, delete all)
//! and New chat. Docked on the right, open by default when a project opens.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
};

use crate::screens::welcome::ago;
use crate::state::chat::ChatModel;
use crate::theme::{ActiveTheme, TypeExt, hsla, px, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, IconButton, LinkButton, pane_header};

pub struct ChatsMenu {
    chat: Entity<ChatModel>,
    search: Entity<InputState>,
    rename: Option<(String, Entity<InputState>)>,
    confirm_delete_all: bool,
    pub open: bool,
    _subs: Vec<Subscription>,
}

impl ChatsMenu {
    pub fn new(chat: Entity<ChatModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats"));
        let subs = vec![
            cx.observe(&chat, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |_, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify()
                }
            }),
        ];
        Self {
            chat,
            search,
            rename: None,
            confirm_delete_all: false,
            open: true,
            _subs: subs,
        }
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        self.confirm_delete_all = false;
        self.rename = None;
        cx.notify();
    }

    fn start_rename(
        &mut self,
        id: String,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        let rid = id.clone();
        cx.subscribe(&input, move |this: &mut Self, input, e: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = e {
                let title = input.read(cx).value().trim().to_string();
                if !title.is_empty() {
                    let id = rid.clone();
                    this.chat.update(cx, |c, cx| c.rename_thread(id, title, cx));
                }
                this.rename = None;
                cx.notify();
            }
        })
        .detach();
        input.update(cx, |i, cx| i.focus(window, cx));
        self.rename = Some((id, input));
        cx.notify();
    }
}

impl Render for ChatsMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let c = &cx.tokens().colors;
        let chat = self.chat.read(cx);
        let current = chat.thread.id.clone();
        let q = self.search.read(cx).value().trim().to_lowercase();
        let threads: Vec<_> = chat
            .threads
            .iter()
            .filter(|t| q.is_empty() || t.title.to_lowercase().contains(&q))
            .cloned()
            .collect();
        let empty = threads.is_empty();
        let rows: Vec<_> = threads
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                let active = t.id == current;
                let renaming = self
                    .rename
                    .as_ref()
                    .filter(|(id, _)| *id == t.id)
                    .map(|(_, input)| input.clone());
                let (open_id, rename_id, delete_id, title) =
                    (t.id.clone(), t.id.clone(), t.id.clone(), t.title.clone());
                div()
                    .id(("chat", i))
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .px(sp(3))
                    .py(sp(1))
                    .cursor_pointer()
                    .when(active, |d| d.bg(hsla(c.surface_selected)))
                    .when(!active, |d| d.hover(|s| s.bg(hsla(c.surface_hover))))
                    .on_click(cx.listener(move |m, _, _, cx| {
                        let id = open_id.clone();
                        m.chat.update(cx, |c, cx| c.open_thread(id, cx));
                        m.open = false;
                        cx.notify();
                    }))
                    .map(|d| match renaming {
                        Some(input) => d.child(div().flex_1().child(Input::new(&input).small())),
                        None => d
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .text_color(hsla(c.ink))
                                            .child(SharedString::from(t.title.clone())),
                                    )
                                    .child(
                                        div()
                                            .type_style(contextual_theme::type_style::UI_SMALL)
                                            .text_color(hsla(c.ink_muted))
                                            .child(SharedString::from(format!(
                                                "{} {} · {}",
                                                t.turns,
                                                if t.turns == 1 { "turn" } else { "turns" },
                                                ago(&t.updated_at)
                                            ))),
                                    ),
                            )
                            .child(
                                IconButton::new(("rename", i), IconName::Replace, "Rename")
                                    .on_click(cx.listener(move |m, _, w, cx| {
                                        cx.stop_propagation();
                                        m.start_rename(rename_id.clone(), title.clone(), w, cx)
                                    })),
                            )
                            .child(
                                IconButton::new(("delete", i), IconName::Delete, "Delete")
                                    .on_click(cx.listener(move |m, _, _, cx| {
                                        cx.stop_propagation();
                                        let id = delete_id.clone();
                                        m.chat.update(cx, |c, cx| c.delete_thread(id, cx));
                                    })),
                            ),
                    })
            })
            .collect();

        div()
            .id("chats-menu")
            .w_full()
            .min_w_0()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .flex_none()
            .border_l_1()
            .border_color(hsla(c.border))
            .bg(hsla(c.surface_000))
            .type_style(contextual_theme::type_style::UI_BODY)
            .child(pane_header(
                "Chat history",
                IconButton::new("close-history", IconName::Close, "Close chat history")
                    .on_click(cx.listener(|m, _, _, cx| m.toggle(cx))),
                cx,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .p(sp(2))
                    .border_b_1()
                    .border_color(hsla(c.border))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .small()
                                .prefix(Icon::new(IconName::Search).size(px(14.0))),
                        ),
                    )
                    .child(
                        Button::new("new-chat", "New chat")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Primary)
                            .on_click(cx.listener(|m, _, _, cx| {
                                m.chat.update(cx, |c, cx| c.start_fresh(cx));
                                m.open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("chat-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .py(sp(1))
                    .children(rows)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .px(sp(3))
                                .py(sp(2))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child(if q.is_empty() {
                                    "No saved chats for this project yet."
                                } else {
                                    "No chats match."
                                }),
                        )
                    }),
            )
            .when(!empty && q.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .px(sp(3))
                        .py(sp(2))
                        .border_t_1()
                        .border_color(hsla(c.border))
                        .child(LinkButton::new(
                            "delete-all",
                            if self.confirm_delete_all {
                                "Click again to delete all chats"
                            } else {
                                "Delete all"
                            },
                            cx.listener(|m, _, _, cx| {
                                if m.confirm_delete_all {
                                    m.chat.update(cx, |c, cx| c.delete_all_threads(cx));
                                    m.confirm_delete_all = false;
                                } else {
                                    m.confirm_delete_all = true;
                                }
                                cx.notify();
                            }),
                        )),
                )
            })
            .into_any_element()
    }
}
