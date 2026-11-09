//! The Inspect view: every round of the chat exactly as sent and received. A list of rounds on
//! the left; the selected round's input (message or system prompt) and raw output side by side.
//! Uses the centre and right columns beneath the workspace tabs, like the diff view.
//! The round streaming now is shown live.

use contextual_core::chat::Usage;
use contextual_core::inspect::{Round, rounds};
use contextual_core::logic::format::fmt_k;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
};

use crate::state::chat::{CenterView, ChatModel};
use crate::theme::{ActiveTheme, TypeExt, hsla, px, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, Tag, TagTone, pane_header};

const ROUND_LIST_W: f32 = 260.0;
const TOOLBAR_H: f32 = 36.0;

const NOT_RECORDED: &str =
    "The system prompt was not recorded for this round: the chat was saved before the app kept it.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Message,
    System,
}

pub struct InspectScreen {
    chat: Entity<ChatModel>,
    rounds: Vec<Round>,
    /// What `rounds` was built from: thread generation, transcript length and a live round.
    built: (u64, usize, bool),
    /// Index into `rounds`; `None` follows the newest round.
    selected: Option<usize>,
    mode: InputMode,
    input: Entity<EditorState>,
    output: Entity<EditorState>,
    /// What the editors show (round, mode, reply length), to skip needless updates.
    shown: Option<(usize, InputMode, usize)>,
    _sub: Subscription,
}

fn editor(window: &mut Window, cx: &mut Context<InspectScreen>) -> Entity<EditorState> {
    cx.new(|cx| {
        let mut s = EditorState::new(window, cx)
            .language("markdown")
            .line_number(true)
            .soft_wrap(true);
        s.set_readonly(true, cx);
        s
    })
}

fn usage_text(u: &Usage) -> String {
    let mut s = format!("{} in", fmt_k(u.input_tokens));
    if u.cached_tokens > 0 {
        s.push_str(&format!(" ({} cached)", fmt_k(u.cached_tokens)));
    }
    s.push_str(&format!(" / {} out", fmt_k(u.output_tokens)));
    s
}

impl InspectScreen {
    pub fn new(chat: Entity<ChatModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe_in(&chat, window, |this: &mut Self, _, window, cx| {
            if this.chat.read(cx).center(cx) == CenterView::Inspect {
                this.refresh(window, cx);
            }
        });
        let mut this = Self {
            input: editor(window, cx),
            output: editor(window, cx),
            chat,
            rounds: Vec::new(),
            built: (u64::MAX, usize::MAX, false),
            selected: None,
            mode: InputMode::Message,
            shown: None,
            _sub: sub,
        };
        this.refresh(window, cx);
        this
    }

    fn current(&self) -> Option<usize> {
        match self.selected {
            Some(i) if i < self.rounds.len() => Some(i),
            _ => self.rounds.len().checked_sub(1),
        }
    }

    /// Rebuilds the rounds when the transcript changed; while streaming, only the live reply.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat = self.chat.read(cx);
        let key = (
            chat.thread_generation,
            chat.thread.transcript.len(),
            chat.live.is_some(),
        );
        if key != self.built {
            if key.0 != self.built.0 {
                self.selected = None;
            }
            self.rounds = rounds(&chat.thread, chat.live.as_ref());
            self.built = key;
            self.shown = None;
            if self.selected.is_some_and(|i| i >= self.rounds.len()) {
                self.selected = None;
            }
        } else if let (Some(live), Some(last)) = (&chat.live, self.rounds.last_mut())
            && last.live
        {
            last.reply.clone_from(&live.reply);
        }
        self.sync_editors(window, cx);
        cx.notify();
    }

    fn sync_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(i) = self.current() else {
            if self.shown.take().is_some() {
                self.input.update(cx, |e, cx| e.set_value("", window, cx));
                self.output.update(cx, |e, cx| e.set_value("", window, cx));
            }
            return;
        };
        let r = &self.rounds[i];
        let key = (i, self.mode, r.reply.len());
        if self.shown == Some(key) {
            return;
        }
        let input_changed = self
            .shown
            .is_none_or(|(si, sm, _)| si != i || sm != self.mode);
        if input_changed {
            let text = match self.mode {
                InputMode::Message => r.input.clone(),
                InputMode::System => r.system.clone().unwrap_or_else(|| NOT_RECORDED.into()),
            };
            self.input.update(cx, |e, cx| e.set_value(text, window, cx));
        }
        let reply = r.reply.clone();
        self.output
            .update(cx, |e, cx| e.set_value(reply, window, cx));
        self.shown = Some(key);
    }

    fn select(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Picking the newest round follows new rounds as they come in.
        self.selected = (i + 1 < self.rounds.len()).then_some(i);
        self.sync_editors(window, cx);
        cx.notify();
    }

    fn set_mode(&mut self, mode: InputMode, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = mode;
        self.sync_editors(window, cx);
        cx.notify();
    }

    fn round_row(&self, i: usize, active: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let c = &cx.tokens().colors;
        let r = &self.rounds[i];
        let label = r.input_label();
        div()
            .id(SharedString::from(format!("round-{i}")))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(sp(3))
            .py(sp(2))
            .border_b_1()
            .border_color(hsla(c.border))
            .cursor_pointer()
            .when(active, |d| d.bg(hsla(c.surface_selected)))
            .when(!active, |d| d.hover(|s| s.bg(hsla(c.surface_hover))))
            .on_click(cx.listener(move |t, _, window, cx| t.select(i, window, cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(SharedString::from(format!("Round {}", r.number)))
                    .when(r.followup, |d| d.child(Tag::new("app")))
                    .when(r.live, |d| d.child(Tag::new("live").tone(TagTone::Skill))),
            )
            .child(
                div()
                    .type_style(contextual_theme::type_style::UI_BODY)
                    .text_color(hsla(c.ink_strong))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(SharedString::from(if label.is_empty() {
                        "(no text)".to_string()
                    } else {
                        label
                    })),
            )
            .child(
                div()
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink))
                    .child(SharedString::from(format!("→ {}", r.reply_label()))),
            )
            .child(
                div()
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(SharedString::from(match &r.usage {
                        Some(u) => usage_text(u),
                        None => format!("~{} in", fmt_k(r.input_tokens())),
                    })),
            )
            .into_any_element()
    }

    fn column(
        title: &'static str,
        actions: impl IntoElement,
        summary: impl IntoElement,
        editor: &Entity<EditorState>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let c = &cx.tokens().colors;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(pane_header(title, actions, cx))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .flex_wrap()
                    .items_center()
                    .gap(sp(1))
                    .px(sp(3))
                    .py(sp(2))
                    .border_b_1()
                    .border_color(hsla(c.border))
                    .bg(hsla(c.surface_200))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(summary),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(Editor::new(editor).readonly(true).bordered(false).h_full()),
            )
    }

    fn copy_button(id: &'static str, text: String) -> Button {
        Button::new(id, "Copy")
            .size(ButtonSize::Xs)
            .variant(ButtonVariant::Ghost)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
            })
    }
}

impl Render for InspectScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current();
        let mut rows = Vec::with_capacity(self.rounds.len());
        for i in 0..self.rounds.len() {
            rows.push(self.round_row(i, Some(i) == current, cx));
        }
        let c = &cx.tokens().colors;
        let (tokens_in, tokens_out) = self
            .rounds
            .iter()
            .filter_map(|r| r.usage)
            .fold((0, 0), |a, u| (a.0 + u.input_tokens, a.1 + u.output_tokens));
        let n = self.rounds.len();

        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(sp(2))
            .h(px(TOOLBAR_H))
            .px(sp(3))
            .border_b_1()
            .border_color(hsla(c.border))
            .bg(hsla(c.surface_000))
            .type_style(contextual_theme::type_style::UI_SMALL)
            .child(SharedString::from(format!(
                "{n} {}",
                if n == 1 { "round" } else { "rounds" }
            )))
            .when(tokens_in + tokens_out > 0, |d| {
                d.child(
                    div()
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(format!(
                            "{} in / {} out",
                            fmt_k(tokens_in),
                            fmt_k(tokens_out)
                        ))),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .text_color(hsla(c.ink_muted))
                    .child("Raw, exactly as sent and received"),
            );

        let body = match current.map(|i| &self.rounds[i]) {
            None => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .type_style(contextual_theme::type_style::UI_BODY)
                .text_color(hsla(c.ink_muted))
                .child("Nothing sent in this chat yet.")
                .into_any_element(),
            Some(r) => {
                let mode = self.mode;
                let toggle = |id: &'static str, label: &'static str, m: InputMode| {
                    Button::new(id, label)
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .selected(mode == m)
                };
                let input_text = match mode {
                    InputMode::Message => r.input.clone(),
                    InputMode::System => r.system.clone().unwrap_or_default(),
                };
                let input_actions = div()
                    .flex()
                    .items_center()
                    .gap(sp(1))
                    .child(
                        toggle("mode-message", "Message", InputMode::Message).on_click(
                            cx.listener(|t, _, w, cx| t.set_mode(InputMode::Message, w, cx)),
                        ),
                    )
                    .child(
                        toggle("mode-system", "System prompt", InputMode::System).on_click(
                            cx.listener(|t, _, w, cx| t.set_mode(InputMode::System, w, cx)),
                        ),
                    )
                    .child(Self::copy_button("copy-input", input_text));
                let input_summary: Vec<gpui_kit::AnyElement> = match mode {
                    InputMode::Message => {
                        let mut v: Vec<gpui_kit::AnyElement> = r
                            .sections
                            .iter()
                            .map(|s| {
                                Tag::new(format!("{} ~{}", s.title, fmt_k(s.tokens)))
                                    .into_any_element()
                            })
                            .collect();
                        if r.images > 0 {
                            v.push(
                                Tag::new(format!(
                                    "{} {}",
                                    r.images,
                                    if r.images == 1 { "image" } else { "images" }
                                ))
                                .into_any_element(),
                            );
                        }
                        v
                    }
                    InputMode::System => match &r.system {
                        None => vec![
                            Tag::new("not recorded")
                                .tone(TagTone::Warn)
                                .into_any_element(),
                        ],
                        Some(s) => vec![
                            Tag::new(format!(
                                "~{} tokens",
                                fmt_k(contextual_core::logic::format::estimate_tokens(s),)
                            ))
                            .into_any_element(),
                            Tag::new(if r.system_changed {
                                "sent with this round"
                            } else {
                                "same as the round before"
                            })
                            .into_any_element(),
                        ],
                    },
                };
                let output_summary = div()
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .child(SharedString::from(r.reply_label()))
                    .when_some(r.usage, |d, u| {
                        d.child(
                            div()
                                .type_style(contextual_theme::type_style::CODE_SMALL)
                                .child(SharedString::from(usage_text(&u))),
                        )
                    });
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(Self::column(
                        "Input",
                        input_actions,
                        div().flex().flex_wrap().gap(sp(1)).children(input_summary),
                        &self.input,
                        cx,
                    ))
                    .child(div().w(px(1.0)).h_full().bg(hsla(c.border)))
                    .child(Self::column(
                        "Output",
                        Self::copy_button("copy-output", r.reply.clone()),
                        output_summary,
                        &self.output,
                        cx,
                    ))
                    .into_any_element()
            }
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .bg(hsla(c.surface_100))
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("inspect-rounds")
                            .flex_none()
                            .w(px(ROUND_LIST_W))
                            .h_full()
                            .overflow_y_scroll()
                            .border_r_1()
                            .border_color(hsla(c.border))
                            .bg(hsla(c.surface_000))
                            .children(rows),
                    )
                    .child(body),
            )
    }
}
