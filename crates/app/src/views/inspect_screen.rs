//! The Inspect view: every round of the chat exactly as sent and received. A list of rounds on
//! the left; the selected round's input (message or system prompt) and raw output side by side.
//! Uses the centre and right columns beneath the workspace tabs, like the diff view.
//! The round streaming now is shown live.

use std::time::Instant;

use contextual_core::chat::Usage;
use contextual_core::inspect::{Round, rounds};
use contextual_core::logic::format::{estimate_tokens, fmt_k};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, ClipboardItem, Context, Entity, InteractiveElement, IntoElement, ListAlignment,
    ListOffset, ListState, ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Window, canvas, div, list,
};

use crate::state::chat::{CenterView, ChatModel};
use crate::theme::{ActiveTheme, ListZoom, TypeExt, hsla, px, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, Tag, TagTone, pane_header};

const ROUND_LIST_W: f32 = 260.0;
const TOOLBAR_H: f32 = 36.0;

const NOT_RECORDED: &str =
    "The system prompt was not recorded for this round: the chat was saved before the app kept it.";

/// Bound ingestion independently of source-line length. One pane advances per frame.
/// These bound submitted text, not the editor's deferred layout/highlighting time.
const LOAD_CHUNK_BYTES: usize = 16 * 1024;
const LOAD_CHUNK_LINES: usize = 256;

/// A viewport can show no more logical lines than visual rows. Wrapping only reduces that
/// number. Include two extra lines so a partially visible row does not leave a gap.
fn initial_line_count(height: Pixels) -> usize {
    ((height / px(contextual_theme::type_style::CODE.line_height))
        .ceil()
        .max(1.0) as usize)
        .saturating_add(2)
}

/// A bounded source slice, including its final newline when possible. Never split UTF-8 or
/// CRLF. A minimum four-byte budget guarantees progress even for a four-byte character.
/// `start` is always an endpoint returned by this function, beginning at zero.
fn chunk_end(text: &str, start: usize, lines: usize, bytes: usize) -> usize {
    if start >= text.len() {
        return text.len();
    }
    let mut end = start.saturating_add(bytes.max(4)).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some((offset, _)) = text[start..end].match_indices('\n').nth(lines.max(1) - 1) {
        end = start + offset + 1;
    }
    if end < text.len() && text.as_bytes()[end - 1] == b'\r' && text.as_bytes()[end] == b'\n' {
        end -= 1;
    }
    end
}

/// Opt-in CPU timings. Only static stage names and numeric metadata are logged.
/// Editor layout/highlighting may also run after these synchronous operations.
struct InspectTiming {
    stage: &'static str,
    started: Instant,
    bytes: usize,
    rounds: usize,
}

impl InspectTiming {
    fn start(stage: &'static str, bytes: usize, rounds: usize) -> Option<Self> {
        tracing::enabled!(
            target: "contextual::performance",
            tracing::Level::DEBUG
        )
        .then(|| Self {
            stage,
            started: Instant::now(),
            bytes,
            rounds,
        })
    }

    fn next_frame(&self, window: &mut Window) {
        let started = self.started;
        let stage = self.stage;
        window.on_next_frame(move |_, _| {
            // A scheduling/latency proxy, not a GPU presentation timestamp or proof
            // that asynchronous highlighting has finished.
            tracing::debug!(
                target: "contextual::performance",
                stage,
                next_frame_us = started.elapsed().as_micros() as u64,
                "inspect navigation reached next frame callback"
            );
        });
    }
}

impl Drop for InspectTiming {
    fn drop(&mut self) {
        tracing::debug!(
            target: "contextual::performance",
            stage = self.stage,
            bytes = self.bytes,
            rounds = self.rounds,
            elapsed_us = self.started.elapsed().as_micros() as u64,
            "inspect synchronous work"
        );
    }
}

/// Retain the visible item across same-thread rebuilds, clamping if a live round disappeared.
fn retained_round_offset(mut top: ListOffset, count: usize) -> Option<ListOffset> {
    let last = count.checked_sub(1)?;
    if top.item_ix > last {
        top.item_ix = last;
        top.offset_in_item = px(0.0);
    }
    Some(top)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Message,
    System,
}

#[derive(Clone, Copy)]
enum CopySource {
    Input,
    Output,
}

/// A cancelled callback may still arrive after another load has been queued. Its ticket
/// must not consume the newer callback's scheduled flag.
#[derive(Default)]
struct LoadQueue {
    generation: u64,
    scheduled: bool,
}

impl LoadQueue {
    fn enqueue(&mut self) -> Option<u64> {
        if self.scheduled {
            return None;
        }
        self.scheduled = true;
        Some(self.generation)
    }

    fn begin(&mut self, ticket: u64) -> bool {
        if ticket != self.generation || !self.scheduled {
            return false;
        }
        self.scheduled = false;
        true
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.scheduled = false;
    }
}

/// Prepared only when rounds are rebuilt, never while rendering or receiving deltas.
struct RoundSummary {
    input: SharedString,
    reply: SharedString,
    reply_row: SharedString,
    usage: SharedString,
    system_tokens: Option<u64>,
}

impl RoundSummary {
    fn new(round: &Round) -> Self {
        let input = round.input_label();
        let reply = round.reply_label();
        Self {
            input: if input.is_empty() {
                "(no text)".into()
            } else {
                input.into()
            },
            reply_row: format!("→ {reply}").into(),
            reply: reply.into(),
            usage: match &round.usage {
                Some(usage) => usage_text(usage),
                None => format!("~{} in", fmt_k(round.input_tokens())),
            }
            .into(),
            system_tokens: round.system.as_deref().map(estimate_tokens),
        }
    }
}

/// Input mode is deliberately absent from output invalidation: switching Message/System
/// must not reset the unchanged output editor, its selection or its scroll position.
/// Reply length is a revision only within a round: ChatModel's live replies are append-only.
fn editor_changes(
    shown: Option<(usize, InputMode, usize)>,
    next: (usize, InputMode, usize),
) -> (bool, bool) {
    let Some(previous) = shown else {
        return (true, true);
    };
    (
        previous.0 != next.0 || previous.1 != next.1,
        previous.0 != next.0 || previous.2 != next.2,
    )
}

fn output_can_append(
    shown: Option<(usize, InputMode, usize)>,
    next: (usize, InputMode, usize),
) -> bool {
    shown.is_some_and(|previous| previous.0 == next.0 && previous.2 <= next.2)
}

pub struct InspectScreen {
    chat: Entity<ChatModel>,
    rounds: Vec<Round>,
    summaries: Vec<RoundSummary>,
    usage_totals: (u64, u64),
    round_list: ListState,
    list_zoom: ListZoom,
    /// What `rounds` was built from: thread generation, transcript length and a live round.
    built: (u64, usize, bool),
    /// Index into `rounds`; `None` follows the newest round.
    selected: Option<usize>,
    mode: InputMode,
    input: Entity<EditorState>,
    output: Entity<EditorState>,
    /// Source byte offsets already submitted to each editor.
    input_loaded: usize,
    output_loaded: usize,
    input_height: Option<Pixels>,
    output_height: Option<Pixels>,
    load_queue: LoadQueue,
    next_pane: CopySource,
    /// The source being displayed (round, mode, reply length). The loaded offsets can lag
    /// behind this revision while the remaining text is ingested between frames.
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

fn show_prefix(
    editor: &Entity<EditorState>,
    text: &str,
    height: Pixels,
    window: &mut Window,
    cx: &mut Context<InspectScreen>,
) -> usize {
    let end = chunk_end(text, 0, initial_line_count(height), LOAD_CHUNK_BYTES);
    let prefix = text[..end].to_owned();
    editor.update(cx, |editor, cx| editor.set_value(prefix, window, cx));
    end
}

/// Use an incremental edit, not set_value with an ever-growing prefix. The readonly flag
/// is restored before returning to the event loop; this only changes the display buffer.
/// Keep the user's selection rather than leaving it at the insertion point.
fn append_chunk(
    editor: &Entity<EditorState>,
    text: String,
    window: &mut Window,
    cx: &mut Context<InspectScreen>,
) {
    editor.update(cx, |editor, cx| {
        let selection = editor.selected_range();
        let end = editor.value().len();
        editor.set_readonly(false, cx);
        editor.set_selected_range(end..end, cx);
        editor.replace(text, window, cx);
        editor.set_selected_range(selection, cx);
        editor.set_readonly(true, cx);
    });
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
            } else {
                // Keep the already-loaded buffers, but do not ingest hidden documents.
                this.load_queue.cancel();
            }
        });
        let mut this = Self {
            input: editor(window, cx),
            output: editor(window, cx),
            chat,
            rounds: Vec::new(),
            summaries: Vec::new(),
            usage_totals: (0, 0),
            round_list: ListState::new(0, ListAlignment::Top, sp(8)),
            list_zoom: ListZoom::default(),
            built: (u64::MAX, usize::MAX, false),
            selected: None,
            mode: InputMode::Message,
            input_loaded: 0,
            output_loaded: 0,
            input_height: None,
            output_height: None,
            load_queue: LoadQueue::default(),
            next_pane: CopySource::Input,
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
        let rebuilt = key != self.built;
        if rebuilt {
            let same_thread = key.0 == self.built.0;
            let top = same_thread.then(|| self.round_list.logical_scroll_top());
            if !same_thread {
                self.selected = None;
            }
            {
                let _timing = InspectTiming::start("build_rounds", 0, self.rounds.len());
                self.rounds = rounds(&chat.thread, chat.live.as_ref());
            }
            {
                let _timing = InspectTiming::start("build_summaries", 0, self.rounds.len());
                self.summaries = self.rounds.iter().map(RoundSummary::new).collect();
                self.usage_totals = self
                    .rounds
                    .iter()
                    .filter_map(|r| r.usage)
                    .fold((0, 0), |a, u| (a.0 + u.input_tokens, a.1 + u.output_tokens));
            }
            // Re-measure changed labels/badges, but do not jump the user's viewport.
            // Ordinary live deltas never reset the list.
            self.round_list.reset(self.rounds.len());
            if let Some(top) = top.and_then(|top| retained_round_offset(top, self.rounds.len())) {
                self.round_list.scroll_to(top);
            }
            self.built = key;
            self.shown = None;
            if self.selected.is_some_and(|i| i >= self.rounds.len()) {
                self.selected = None;
            }
        } else if let (Some(live), Some(last)) = (&chat.live, self.rounds.last_mut())
            && last.live
            && live.reply.len() > last.reply.len()
        {
            // Within this build key the live round is unchanged and only receives appends.
            // Unrelated notifications do no copying, and deltas copy only the new suffix.
            last.reply.push_str(&live.reply[last.reply.len()..]);
        }
        let editors_changed = self.sync_editors(window, cx);
        if rebuilt || editors_changed {
            cx.notify();
        }
    }

    fn sync_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(i) = self.current() else {
            self.load_queue.cancel();
            let changed =
                self.shown.take().is_some() || self.input_loaded > 0 || self.output_loaded > 0;
            if changed {
                self.input.update(cx, |e, cx| e.set_value("", window, cx));
                self.output.update(cx, |e, cx| e.set_value("", window, cx));
            }
            self.input_loaded = 0;
            self.output_loaded = 0;
            return changed;
        };
        let r = &self.rounds[i];
        let key = (i, self.mode, r.reply.len());
        let (input_changed, output_changed) = editor_changes(self.shown, key);
        let reset_output = output_changed && !output_can_append(self.shown, key);
        if input_changed || reset_output {
            self.load_queue.cancel();
            self.next_pane = CopySource::Input;
        }

        // Before the first layout, window height is a conservative upper bound on the pane
        // height. Later selections use the actual measured pane, at the current zoom.
        let fallback_height = window.viewport_size().height;
        if input_changed {
            let text = match self.mode {
                InputMode::Message => r.input.as_str(),
                InputMode::System => r.system.as_deref().unwrap_or(NOT_RECORDED),
            };
            let _timing = InspectTiming::start("input_editor", text.len(), self.rounds.len());
            self.input_loaded = show_prefix(
                &self.input,
                text,
                self.input_height.unwrap_or(fallback_height),
                window,
                cx,
            );
        }
        if reset_output {
            let _timing = InspectTiming::start("output_editor", r.reply.len(), self.rounds.len());
            self.output_loaded = show_prefix(
                &self.output,
                &r.reply,
                self.output_height.unwrap_or(fallback_height),
                window,
                cx,
            );
        }

        // A live revision merely extends the target. It neither replaces the editor nor
        // restarts its queued work, even when deltas arrive faster than it can display them.
        self.shown = Some(key);
        self.schedule_loading(window, cx);
        input_changed || output_changed
    }

    fn source_text(&self, source: CopySource) -> Option<&str> {
        let round = &self.rounds[self.current()?];
        Some(match source {
            CopySource::Input => match self.mode {
                InputMode::Message => round.input.as_str(),
                InputMode::System => round.system.as_deref().unwrap_or(NOT_RECORDED),
            },
            CopySource::Output => round.reply.as_str(),
        })
    }

    fn loading(&self, source: CopySource) -> bool {
        let loaded = match source {
            CopySource::Input => self.input_loaded,
            CopySource::Output => self.output_loaded,
        };
        self.source_text(source)
            .is_some_and(|text| loaded < text.len())
    }

    fn schedule_loading(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.loading(CopySource::Input) && !self.loading(CopySource::Output) {
            return;
        }
        let Some(ticket) = self.load_queue.enqueue() else {
            return;
        };
        let weak = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = weak.update(cx, |this, cx| {
                if !this.load_queue.begin(ticket) {
                    return;
                }
                if this.chat.read(cx).center(cx) != CenterView::Inspect {
                    this.load_queue.cancel();
                    return;
                }
                this.load_one_chunk(window, cx);
                this.schedule_loading(window, cx);
                cx.notify();
            });
        });
        // Loading is scheduled from observers and next-frame callbacks, outside painting.
        // Entity notification requests a redraw without requiring an active painted view.
        cx.notify();
    }

    fn load_one_chunk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = match (
            self.loading(CopySource::Input),
            self.loading(CopySource::Output),
        ) {
            (false, false) => return,
            (true, false) => CopySource::Input,
            (false, true) => CopySource::Output,
            (true, true) => self.next_pane,
        };
        let loaded = match source {
            CopySource::Input => self.input_loaded,
            CopySource::Output => self.output_loaded,
        };
        let Some(text) = self.source_text(source) else {
            return;
        };
        let end = chunk_end(text, loaded, LOAD_CHUNK_LINES, LOAD_CHUNK_BYTES);
        let chunk = text[loaded..end].to_owned();
        match source {
            CopySource::Input => {
                append_chunk(&self.input, chunk, window, cx);
                self.input_loaded = end;
                self.next_pane = CopySource::Output;
            }
            CopySource::Output => {
                append_chunk(&self.output, chunk, window, cx);
                self.output_loaded = end;
                self.next_pane = CopySource::Input;
            }
        }
    }

    fn select(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.current() == Some(i) {
            return;
        }
        let timing = InspectTiming::start("select_round", 0, self.rounds.len());
        // Picking the newest round follows new rounds as they come in.
        self.selected = (i + 1 < self.rounds.len()).then_some(i);
        self.sync_editors(window, cx);
        cx.notify();
        if let Some(timing) = &timing {
            timing.next_frame(window);
        }
    }

    fn set_mode(&mut self, mode: InputMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        let timing = InspectTiming::start("input_mode", 0, self.rounds.len());
        self.mode = mode;
        self.sync_editors(window, cx);
        cx.notify();
        if let Some(timing) = &timing {
            timing.next_frame(window);
        }
    }

    fn round_row(&self, i: usize, active: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let c = &cx.tokens().colors;
        let r = &self.rounds[i];
        let summary = &self.summaries[i];
        div()
            .id(SharedString::from(format!("round-{i}")))
            .w_full()
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
                    .when(r.routing, |d| d.child(Tag::new("routing")))
                    .when(r.live, |d| d.child(Tag::new("live").tone(TagTone::Skill)))
                    .when_some(r.model.clone(), |d, model| {
                        d.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(SharedString::from(model)),
                        )
                    }),
            )
            .child(
                div()
                    .type_style(contextual_theme::type_style::UI_BODY)
                    .text_color(hsla(c.ink_strong))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(summary.input.clone()),
            )
            .child(
                div()
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink))
                    .child(summary.reply_row.clone()),
            )
            .child(
                div()
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(summary.usage.clone()),
            )
            .into_any_element()
    }

    fn column(
        title: &'static str,
        actions: impl IntoElement,
        summary: impl IntoElement,
        editor: &Entity<EditorState>,
        source: CopySource,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let weak = cx.entity().downgrade();
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
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                let weak = weak.clone();
                                // Record layout metadata after this frame; measuring a pane
                                // must not mutate its view while GPUI is laying it out.
                                cx.defer(move |cx| {
                                    let _ = weak.update(cx, |this, _| match source {
                                        CopySource::Input => {
                                            this.input_height = Some(bounds.size.height);
                                        }
                                        CopySource::Output => {
                                            this.output_height = Some(bounds.size.height);
                                        }
                                    });
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(Editor::new(editor).readonly(true).bordered(false).h_full()),
            )
    }

    fn copy_button(&self, id: &'static str, source: CopySource, cx: &Context<Self>) -> Button {
        Button::new(id, "Copy")
            .size(ButtonSize::Xs)
            .variant(ButtonVariant::Ghost)
            .on_click(cx.listener(move |this, _, _, cx| {
                let Some(i) = this.current() else {
                    return;
                };
                let round = &this.rounds[i];
                let text = match source {
                    CopySource::Input => match this.mode {
                        InputMode::Message => round.input.as_str(),
                        InputMode::System => round.system.as_deref().unwrap_or_default(),
                    },
                    CopySource::Output => round.reply.as_str(),
                };
                cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
            }))
    }
}

impl Render for InspectScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // This measures element construction, not deferred list rendering or editor layout.
        let _timing = InspectTiming::start("render_elements", 0, self.rounds.len());
        self.list_zoom.refresh(&self.round_list, self.rounds.len());
        let current = self.current();
        let c = &cx.tokens().colors;
        let (tokens_in, tokens_out) = self.usage_totals;
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

        let body = match current.map(|i| (&self.rounds[i], &self.summaries[i])) {
            None => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .type_style(contextual_theme::type_style::UI_BODY)
                .text_color(hsla(c.ink_muted))
                .child("Nothing sent in this chat yet.")
                .into_any_element(),
            Some((r, summary)) => {
                let mode = self.mode;
                let toggle = |id: &'static str, label: &'static str, m: InputMode| {
                    Button::new(id, label)
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .selected(mode == m)
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
                    .child(self.copy_button("copy-input", CopySource::Input, cx));
                let mut input_summary: Vec<gpui_kit::AnyElement> = match mode {
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
                    InputMode::System => match summary.system_tokens {
                        None => vec![
                            Tag::new("not recorded")
                                .tone(TagTone::Warn)
                                .into_any_element(),
                        ],
                        Some(tokens) => vec![
                            Tag::new(format!("~{} tokens", fmt_k(tokens))).into_any_element(),
                            Tag::new(if r.system_changed {
                                "sent with this round"
                            } else {
                                "same as the round before"
                            })
                            .into_any_element(),
                        ],
                    },
                };
                if self.loading(CopySource::Input) {
                    input_summary.push(Tag::new("Loading remaining text…").into_any_element());
                }
                let output_summary = div()
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .child(summary.reply.clone())
                    .when(self.loading(CopySource::Output), |d| {
                        d.child(Tag::new("Loading remaining text…"))
                    })
                    .when(r.usage.is_some(), |d| {
                        d.child(
                            div()
                                .type_style(contextual_theme::type_style::CODE_SMALL)
                                .child(summary.usage.clone()),
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
                        CopySource::Input,
                        cx,
                    ))
                    .child(div().w(px(1.0)).h_full().bg(hsla(c.border)))
                    .child(Self::column(
                        "Output",
                        self.copy_button("copy-output", CopySource::Output, cx),
                        output_summary,
                        &self.output,
                        CopySource::Output,
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
                            .flex()
                            .flex_col()
                            .flex_none()
                            .w(px(ROUND_LIST_W))
                            .h_full()
                            .min_h_0()
                            .overflow_hidden()
                            .border_r_1()
                            .border_color(hsla(c.border))
                            .bg(hsla(c.surface_000))
                            .child(
                                list(self.round_list.clone(), {
                                    let weak = cx.entity().downgrade();
                                    move |i, _, cx| {
                                        weak.upgrade()
                                            .map(|entity| {
                                                entity.update(cx, |this, cx| {
                                                    let active = this.current() == Some(i);
                                                    this.round_row(i, active, cx)
                                                })
                                            })
                                            .unwrap_or_else(|| div().into_any_element())
                                    }
                                })
                                .flex_1()
                                .min_h_0()
                                .w_full(),
                            ),
                    )
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_prefix_tracks_the_scaled_line_height() {
        let line_height = px(contextual_theme::type_style::CODE.line_height);
        assert_eq!(initial_line_count(line_height * 20.0), 22);
        assert_eq!(initial_line_count(line_height * 40.0), 42);
        assert_eq!(initial_line_count(line_height * 20.5), 23);
    }

    #[test]
    fn chunks_include_complete_line_endings() {
        let text = "one\r\ntwo\nthree";
        assert_eq!(&text[..chunk_end(text, 0, 1, 1024)], "one\r\n");
        assert_eq!(&text[..chunk_end(text, 0, 2, 1024)], "one\r\ntwo\n");
    }

    #[test]
    fn byte_budget_does_not_split_crlf() {
        let text = "abc\r\ndef";
        assert_eq!(chunk_end(text, 0, 10, 4), 3);
        assert_eq!(chunk_end(text, 3, 1, 4), 5);
    }

    #[test]
    fn long_lines_are_bounded_without_losing_unicode_or_whitespace() {
        let text = format!("{}\r\n\tcafé ☕ 😀\n\n", "😀".repeat(100));
        let mut start = 0;
        let mut reconstructed = String::new();
        while start < text.len() {
            let end = chunk_end(&text, start, 2, 17);
            assert!(end > start);
            assert!(end - start <= 17);
            assert!(text.is_char_boundary(end));
            reconstructed.push_str(&text[start..end]);
            start = end;
        }
        assert_eq!(reconstructed, text);
    }

    #[test]
    fn tiny_budget_still_advances_over_a_four_byte_character() {
        assert_eq!(chunk_end("😀next", 0, 1, 1), 4);
    }

    #[test]
    fn empty_and_exhausted_sources_need_no_chunk() {
        assert_eq!(chunk_end("", 0, 1, 16), 0);
        assert_eq!(chunk_end("done", 4, 1, 16), 4);
    }

    #[test]
    fn repeated_notifications_queue_only_one_callback() {
        let mut queue = LoadQueue::default();
        let ticket = queue.enqueue().unwrap();
        assert_eq!(queue.enqueue(), None);
        assert!(queue.begin(ticket));
        assert!(!queue.begin(ticket));
        assert!(queue.enqueue().is_some());
    }

    #[test]
    fn cancelled_callback_cannot_consume_a_new_rounds_callback() {
        let mut queue = LoadQueue::default();
        let old = queue.enqueue().unwrap();
        queue.cancel();
        let current = queue.enqueue().unwrap();
        assert!(!queue.begin(old));
        assert!(queue.begin(current));
    }

    #[test]
    fn cancellation_without_a_replacement_stops_loading() {
        let mut queue = LoadQueue::default();
        let ticket = queue.enqueue().unwrap();
        queue.cancel();
        assert!(!queue.begin(ticket));
    }

    #[test]
    fn live_output_extends_the_existing_editor() {
        assert!(output_can_append(
            Some((2, InputMode::Message, 120)),
            (2, InputMode::Message, 140),
        ));
    }

    #[test]
    fn switching_input_mode_does_not_require_output_replacement() {
        assert!(output_can_append(
            Some((2, InputMode::Message, 120)),
            (2, InputMode::System, 120),
        ));
    }

    #[test]
    fn another_round_or_shorter_reply_cannot_append() {
        assert!(!output_can_append(
            Some((2, InputMode::Message, 120)),
            (3, InputMode::Message, 140),
        ));
        assert!(!output_can_append(
            Some((2, InputMode::Message, 120)),
            (2, InputMode::Message, 100),
        ));
        assert!(!output_can_append(None, (2, InputMode::Message, 140)));
    }

    #[test]
    fn list_rebuild_retains_the_visible_round_and_offset() {
        let top = retained_round_offset(
            ListOffset {
                item_ix: 3,
                offset_in_item: gpui_kit::px(12.0),
            },
            8,
        )
        .unwrap();
        assert_eq!((top.item_ix, top.offset_in_item), (3, gpui_kit::px(12.0)));
    }

    #[test]
    fn removed_live_round_clamps_the_list_to_the_last_remaining_round() {
        let top = retained_round_offset(
            ListOffset {
                item_ix: 7,
                offset_in_item: gpui_kit::px(12.0),
            },
            7,
        )
        .unwrap();
        assert_eq!((top.item_ix, top.offset_in_item), (6, gpui_kit::px(0.0)));
    }

    #[test]
    fn empty_list_has_no_scroll_anchor() {
        assert!(
            retained_round_offset(
                ListOffset {
                    item_ix: 0,
                    offset_in_item: gpui_kit::px(0.0),
                },
                0,
            )
            .is_none()
        );
    }

    #[test]
    fn switching_input_mode_leaves_output_untouched() {
        assert_eq!(
            editor_changes(
                Some((0, InputMode::Message, 120)),
                (0, InputMode::System, 120),
            ),
            (true, false),
        );
    }

    #[test]
    fn streaming_updates_only_output() {
        assert_eq!(
            editor_changes(
                Some((0, InputMode::System, 120)),
                (0, InputMode::System, 140),
            ),
            (false, true),
        );
    }

    #[test]
    fn unchanged_selection_updates_neither_editor() {
        assert_eq!(
            editor_changes(
                Some((0, InputMode::Message, 120)),
                (0, InputMode::Message, 120),
            ),
            (false, false),
        );
    }

    #[test]
    fn different_round_updates_both_even_with_equal_reply_lengths() {
        assert_eq!(
            editor_changes(
                Some((0, InputMode::Message, 120)),
                (1, InputMode::Message, 120),
            ),
            (true, true),
        );
    }

    #[test]
    fn rebuilt_transcript_invalidates_both_editors() {
        assert_eq!(
            editor_changes(None, (0, InputMode::Message, 120)),
            (true, true),
        );
    }
}
