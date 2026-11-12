//! The review card for one proposed command, under the reply that proposed it.
//!
//! What the card shows and which actions it offers is decided in
//! `contextual_core::commands::card`; this view renders that and forwards each click to the
//! chat model. It decides nothing itself. Output shown here is held in memory only and has
//! not been sent to the model.

use contextual_core::chat::UiMessage;
use contextual_core::commands::CallId;
use std::collections::HashSet;

use contextual_core::commands::card::{
    Card, CardInput, CardLine, CardPhase, CardTone, EFFECTS_SHORT, LineWord, SAVED_NOTE,
    SHARE_WARNING, STOP_NOTE, WordKind, elapsed_between, review_card, short_duration,
};
use contextual_core::commands::execution::ExecutionProgress;
use contextual_core::commands::history::CommandRecord;
use contextual_core::logic::tree::CheckState;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, WeakEntity, div,
};

use crate::state::Project;
use crate::state::chat::ChatModel;
use crate::theme::{ActiveTheme, TypeExt, hsla, px, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, Checkbox, Tag, TagTone};
use crate::views::chat_panel::ChatPanel;

/// Lines of output shown on a card; the rest stays in memory until it is shared.
const OUTPUT_TAIL_LINES: usize = 24;

/// The share preview of one finished command: which streams to include, and their text,
/// which the user may edit before sharing. View state only; nothing here is sent.
pub struct ShareEdit {
    pub call: CallId,
    pub stdout_on: bool,
    pub stderr_on: bool,
    pub stdout: Entity<TextareaState>,
    pub stderr: Entity<TextareaState>,
    pub _subs: Vec<Subscription>,
}

impl ShareEdit {
    /// The streams as they would be shared: `None` for one that is left out.
    pub fn streams(&self, cx: &App) -> (Option<String>, Option<String>) {
        let text = |on: bool, input: &Entity<TextareaState>| {
            on.then(|| input.read(cx).value().to_string())
        };
        (
            text(self.stdout_on, &self.stdout),
            text(self.stderr_on, &self.stderr),
        )
    }
}

fn text(value: impl Into<String>) -> SharedString {
    SharedString::from(value.into())
}

fn button_id(action: &str, call: &CallId) -> SharedString {
    text(format!("command-{action}-{}-{}", call.turn, call.call))
}

/// What the panel remembers about how the cards are shown. View state only.
pub struct CardView<'a> {
    /// Cards whose output is folded away.
    pub collapsed: &'a HashSet<CallId>,
    /// Cards whose details are open.
    pub details: &'a HashSet<CallId>,
    /// Motion is allowed (the spinner turns).
    pub animate: bool,
    /// The proposal being edited, with its editor.
    pub editing: Option<&'a (CallId, Entity<TextareaState>)>,
    /// The open share preview.
    pub sharing: Option<&'a ShareEdit>,
}

/// The keys that run the waiting command while the composer is empty.
pub fn run_shortcut() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘Enter"
    } else {
        "Ctrl+Enter"
    }
}

fn icon(name: IconName) -> Icon {
    Icon::new(name).size(sp(4))
}

/// The label column of the details.
fn label(name: impl Into<String>, cx: &App) -> gpui_kit::Div {
    let c = &cx.tokens().colors;
    div()
        .flex_none()
        .w(sp(8) * 3.0)
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(c.ink_muted))
        .child(text(name))
}

/// One labelled value. Multi-line values keep their lines; an empty one is shown as such.
fn line(entry: &CardLine, cx: &App) -> AnyElement {
    let c = &cx.tokens().colors;
    let value = if entry.value.is_empty() {
        div()
            .text_color(hsla(c.ink_faint))
            .child(text("(empty)"))
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .type_style(if entry.mono {
                contextual_theme::type_style::CODE_SMALL
            } else {
                contextual_theme::type_style::UI_SMALL
            })
            .children(
                entry
                    .value
                    .split('\n')
                    .map(|part| div().whitespace_normal().child(text(part))),
            )
            .into_any_element()
    };
    div()
        .flex()
        .items_start()
        .gap(sp(3))
        .child(label(entry.label.clone(), cx))
        .child(div().flex_1().min_w_0().child(value))
        .into_any_element()
}

/// The arguments side by side, one pill each, so that every boundary is exact.
fn arguments(values: &[&CardLine], cx: &App) -> AnyElement {
    let c = &cx.tokens().colors;
    div()
        .flex()
        .items_start()
        .gap(sp(3))
        .child(label("Arguments", cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .gap(sp(1))
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .children(values.iter().map(|entry| {
                    div()
                        .px(sp(1))
                        .rounded(px(contextual_theme::radius::SM))
                        .bg(hsla(c.pill_bg))
                        .when(entry.value.is_empty(), |d| d.text_color(hsla(c.ink_faint)))
                        .child(text(if entry.value.is_empty() {
                            "(empty)".to_owned()
                        } else {
                            entry.value.clone()
                        }))
                })),
        )
        .into_any_element()
}

/// Everything about the request, with exact boundaries: what Run approves.
fn details(card: &Card, cx: &App) -> AnyElement {
    let numbered = |entry: &&CardLine| entry.label.starts_with("Argument ");
    let args: Vec<&CardLine> = card.command.iter().filter(numbered).collect();
    let mut rows: Vec<AnyElement> = card
        .command
        .iter()
        .filter(|entry| !numbered(entry))
        .map(|entry| line(entry, cx))
        .collect();
    if !args.is_empty() {
        rows.push(arguments(&args, cx));
    }
    rows.extend(card.details.iter().map(|entry| line(entry, cx)));
    div()
        .flex()
        .flex_col()
        .gap(sp(1))
        .children(rows)
        .into_any_element()
}

/// A block of text in the code face on the editor ground.
fn inset(cx: &App) -> gpui_kit::Div {
    let c = &cx.tokens().colors;
    div()
        .p(sp(2))
        .rounded(px(contextual_theme::radius::SM))
        .bg(hsla(c.surface_100))
        .type_style(contextual_theme::type_style::CODE_SMALL)
        .flex()
        .flex_col()
}

/// The end of what was captured, in one block: output, then error output.
fn output(progress: &ExecutionProgress, call: &CallId, cx: &App) -> Option<AnyElement> {
    let c = &cx.tokens().colors;
    let (out, err) = (&progress.stdout, &progress.stderr);
    if out.text.is_empty() && err.text.is_empty() {
        return None;
    }
    let faint = |message: String| {
        div()
            .text_color(hsla(c.ink_faint))
            .child(text(message))
            .into_any_element()
    };
    let mut rows = Vec::new();
    for (captured, error) in [(out, false), (err, true)] {
        if captured.text.is_empty() {
            continue;
        }
        if error && !out.text.is_empty() {
            rows.push(faint("error output".into()));
        }
        let lines: Vec<&str> = captured.text.lines().collect();
        let hidden = lines.len().saturating_sub(OUTPUT_TAIL_LINES);
        if captured.truncated {
            rows.push(faint("… more was produced than was kept".into()));
        }
        if hidden > 0 {
            rows.push(faint(format!("… {hidden} earlier lines")));
        }
        rows.push(
            div()
                .whitespace_normal()
                .child(crate::views::markdown::selectable_plain(
                    format!(
                        "command-output-{}-{}-{}",
                        call.turn,
                        call.call,
                        if error { "stderr" } else { "stdout" }
                    ),
                    &lines[hidden..].join("\n"),
                ))
                .into_any_element(),
        );
    }
    Some(inset(cx).children(rows).into_any_element())
}

/// A short line with a mark in front, in the mark's colour.
fn marked(name: IconName, message: &str, color: contextual_theme::Rgba) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(sp(1))
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(color))
        .child(Icon::new(name).size(sp(3)))
        .child(div().whitespace_normal().child(text(message)))
        .into_any_element()
}

fn note(message: &str, warn: bool, cx: &App) -> AnyElement {
    let c = &cx.tokens().colors;
    div()
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(if warn { c.warning } else { c.ink_muted }))
        .whitespace_normal()
        .child(text(message))
        .into_any_element()
}

/// The cards of the command proposals that belong under `message`.
pub fn command_cards(
    chat: &Entity<ChatModel>,
    project: &Entity<Project>,
    panel: &WeakEntity<ChatPanel>,
    view: &CardView<'_>,
    message: &UiMessage,
    cx: &App,
) -> Vec<AnyElement> {
    let Some(turn) = message.command_turn.as_deref() else {
        return Vec::new();
    };
    let model = chat.read(cx);
    let desk = &project.read(cx).commands;
    // The shortcut runs the first command that is waiting, so only that card shows it.
    let shortcut = runnable_call(chat, project, cx);
    let mut cards: Vec<AnyElement> = model
        .thread
        .commands
        .of_turn(turn)
        .map(|record| {
            let call = record.call.clone();
            let card = card_of(model, desk, record);
            if let Some(share) = view
                .sharing
                .filter(|share| share.call == call && card.actions.share)
            {
                return share_preview(chat, panel, share, &card, cx);
            }
            // The request on screen, as text the user can edit.
            let draft = desk
                .plan(&call)
                .map(|plan| &plan.request)
                .or(record.request())
                .and_then(|request| serde_json::to_string_pretty(request).ok())
                .unwrap_or_default();
            let live = model
                .running_output(&call)
                .cloned()
                .or_else(|| desk.output(&call).map(|(_, result)| result.output.clone()));
            let editor = view
                .editing
                .filter(|(edited, _)| edited == &call)
                .map(|(_, input)| input.clone());
            let shown = Shown {
                draft,
                live,
                editor,
                collapsed: view.collapsed.contains(&call),
                details: view.details.contains(&call),
                animate: view.animate,
                shortcut: shortcut.as_ref() == Some(&call),
            };
            render(chat, panel, call, card, shown, cx)
        })
        .collect();
    cards.extend(continue_row(chat, panel, turn, cx));
    cards
}

fn card_of(
    model: &ChatModel,
    desk: &contextual_core::commands::desk::CommandDesk,
    record: &CommandRecord,
) -> Card {
    let call = &record.call;
    let runtime = model.card_runtime(call);
    review_card(&CardInput {
        record,
        plan: desk.plan(call),
        review: desk.state(call),
        project_busy: desk.busy(),
        chat_busy: model.streaming() || model.saving,
        preparing: runtime.preparing,
        starting: runtime.starting,
        error: runtime.error.as_deref(),
        enabled: model.commands_enabled,
        output_held: desk.output(call).is_some(),
        staged: model.staged_share(call),
    })
}

/// The first proposal of the open turn that Run can start right now: the one the keyboard
/// shortcut runs.
pub fn runnable_call(
    chat: &Entity<ChatModel>,
    project: &Entity<Project>,
    cx: &App,
) -> Option<CallId> {
    let model = chat.read(cx);
    let desk = &project.read(cx).commands;
    let turn = model.open_command_turn()?;
    model
        .thread
        .commands
        .of_turn(turn)
        .find(|record| card_of(model, desk, record).actions.run)
        .map(|record| record.call.clone())
}

/// How one card is shown right now.
struct Shown {
    draft: String,
    live: Option<ExecutionProgress>,
    editor: Option<Entity<TextareaState>>,
    collapsed: bool,
    details: bool,
    animate: bool,
    shortcut: bool,
}

/// The status on the right of the header: a mark, the status and how long it took.
fn status(card: &Card, animate: bool, cx: &App) -> AnyElement {
    let c = &cx.tokens().colors;
    let color = match card.tone {
        CardTone::Neutral => c.ink_muted,
        CardTone::Attention => c.accent_ink,
        CardTone::Ok => c.success,
        CardTone::Warn => c.warning,
        CardTone::Danger => c.danger,
    };
    let took = match (&card.duration, &card.started_at) {
        (Some(duration), _) => Some(duration.clone()),
        (None, Some(started)) => elapsed_between(started, &contextual_core::threads::now_iso())
            .map(|elapsed| short_duration(contextual_core::commands::card::whole_seconds(elapsed))),
        (None, None) => None,
    };
    let mark: Option<AnyElement> = match (card.phase, card.tone) {
        (CardPhase::Running | CardPhase::Starting, _) if animate => Some(
            Spinner::new()
                .xsmall()
                .color(hsla(color))
                .into_any_element(),
        ),
        (CardPhase::Running | CardPhase::Starting, _) => {
            Some(Icon::new(IconName::Loader).size(sp(3)).into_any_element())
        }
        (CardPhase::Finished, CardTone::Ok) => {
            Some(Icon::new(IconName::Check).size(sp(3)).into_any_element())
        }
        (CardPhase::Finished, CardTone::Danger) if card.timed_out => {
            Some(Icon::new(IconName::Timer).size(sp(3)).into_any_element())
        }
        (CardPhase::Finished, CardTone::Danger) => {
            Some(Icon::new(IconName::Close).size(sp(3)).into_any_element())
        }
        (_, CardTone::Warn) => Some(
            Icon::new(IconName::TriangleAlert)
                .size(sp(3))
                .into_any_element(),
        ),
        _ => None,
    };
    let mut label = card.status.clone();
    if let Some(took) = took {
        label.push_str(" · ");
        label.push_str(&took);
    }
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(sp(1))
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(color))
        .children(mark)
        .child(text(label))
        .into_any_element()
}

/// The command as one line in the code face.
fn command_line(words: &[LineWord], cx: &App) -> gpui_kit::Div {
    let c = &cx.tokens().colors;
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_wrap()
        .gap_x(sp(2))
        .type_style(contextual_theme::type_style::CODE)
        .child(div().text_color(hsla(c.ink_faint)).child(text("$")))
        .children(words.iter().map(|word| {
            div()
                .min_w_0()
                .whitespace_normal()
                .text_color(hsla(match word.kind {
                    WordKind::Program => c.ink_strong,
                    WordKind::Flag => c.syn_keyword,
                    WordKind::Quoted => c.syn_string,
                    WordKind::Plain => c.ink,
                }))
                .child(text(word.text.clone()))
        }))
}

/// The one-line header: what would run, or ran, and where it stands. With output to show,
/// the whole line folds and unfolds it.
fn card_header(
    panel: &WeakEntity<ChatPanel>,
    call: &CallId,
    card: &Card,
    fold: Option<bool>,
    animate: bool,
    cx: &App,
) -> AnyElement {
    let c = &cx.tokens().colors;
    let lead = match fold {
        Some(true) => IconName::ChevronRight,
        Some(false) => IconName::ChevronDown,
        None => IconName::SquareTerminal,
    };
    let row = div()
        .id(button_id("header", call))
        .flex()
        .items_start()
        .gap(sp(2))
        .px(sp(3))
        .py(sp(2))
        .child(
            div()
                .flex_none()
                .text_color(hsla(c.ink_muted))
                .child(icon(lead)),
        )
        .child(command_line(&card.line, cx))
        .when(card.edited, |d| {
            d.child(Tag::new("Edited by you").tone(TagTone::Warn))
        })
        .child(status(card, animate, cx));
    match fold {
        Some(_) => {
            let (toggle, toggled) = (panel.clone(), call.clone());
            row.cursor_pointer()
                .on_click(move |_, _, cx| {
                    toggle
                        .update(cx, |panel, cx| panel.toggle_command_output(&toggled, cx))
                        .ok();
                })
                .into_any_element()
        }
        None => row.into_any_element(),
    }
}

/// The body of a card, indented to sit under the command text.
fn card_body() -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(sp(2))
        .pl(sp(3) + sp(4) + sp(2))
        .pr(sp(3))
        .pb(sp(3))
        .type_style(contextual_theme::type_style::UI_BODY)
}

/// One stream in the share preview: a tick box, and its editable text when included.
fn share_stream(
    panel: &WeakEntity<ChatPanel>,
    id: &'static str,
    label: &str,
    stdout: bool,
    on: bool,
    input: &Entity<TextareaState>,
    cx: &App,
) -> AnyElement {
    let c = &cx.tokens().colors;
    let toggle = panel.clone();
    div()
        .flex()
        .flex_col()
        .gap(sp(1))
        .child(
            div()
                .flex()
                .items_center()
                .gap(sp(2))
                .type_style(contextual_theme::type_style::UI_SMALL)
                .child(
                    Checkbox::new(id, if on { CheckState::On } else { CheckState::Off }).on_toggle(
                        move |_, _, cx| {
                            toggle
                                .update(cx, |panel, cx| panel.toggle_share_stream(stdout, cx))
                                .ok();
                        },
                    ),
                )
                .child(text(label)),
        )
        .when(on, |d| {
            d.child(
                div()
                    .rounded(px(contextual_theme::radius::SM))
                    .border_1()
                    .border_color(hsla(c.border_strong))
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .child(Textarea::new(input).appearance(false)),
            )
        })
        .into_any_element()
}

/// The preview of exactly what the model would receive for one finished command, with the
/// warning, the choice to leave each stream out, and the chance to edit what is included.
fn share_preview(
    chat: &Entity<ChatModel>,
    panel: &WeakEntity<ChatPanel>,
    share: &ShareEdit,
    card: &Card,
    cx: &App,
) -> AnyElement {
    let (stdout, stderr) = share.streams(cx);
    let preview = chat
        .read(cx)
        .share_preview(&share.call, stdout.clone(), stderr.clone(), cx);
    let (use_chat, use_panel, use_call) = (chat.clone(), panel.clone(), share.call.clone());
    let cancel_panel = panel.clone();
    let sendable = preview.is_ok();
    let body = card_body()
        .child(note(SHARE_WARNING, true, cx))
        .child(note(
            "Choose what to include. You can edit the text before sharing, for example to \
             remove a secret.",
            false,
            cx,
        ))
        .child(share_stream(
            panel,
            "share-stdout",
            "Include output",
            true,
            share.stdout_on,
            &share.stdout,
            cx,
        ))
        .child(share_stream(
            panel,
            "share-stderr",
            "Include error output",
            false,
            share.stderr_on,
            &share.stderr,
            cx,
        ))
        .child(note("The model will receive exactly this:", false, cx))
        .child(match &preview {
            Ok(json) => inset(cx)
                .children(
                    json.split('\n')
                        .map(|part| div().whitespace_normal().child(text(part))),
                )
                .into_any_element(),
            Err(problem) => note(problem, true, cx),
        })
        .child(note(SAVED_NOTE, false, cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap(sp(2))
                .child(
                    Button::new(button_id("use-share", &share.call), "Use this result")
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Primary)
                        .disabled(!sendable)
                        .on_click(move |_, _, cx| {
                            let staged = use_chat.update(cx, |chat, cx| {
                                chat.stage_share(&use_call, stdout.clone(), stderr.clone(), cx)
                            });
                            if staged {
                                use_panel
                                    .update(cx, |panel, cx| panel.end_command_share(cx))
                                    .ok();
                            }
                        }),
                )
                .child(
                    Button::new(button_id("cancel-share", &share.call), "Cancel")
                        .size(ButtonSize::Xs)
                        .on_click(move |_, _, cx| {
                            cancel_panel
                                .update(cx, |panel, cx| panel.end_command_share(cx))
                                .ok();
                        }),
                ),
        );
    frame(
        card,
        card_header(panel, &share.call, card, None, false, cx),
        body,
        cx,
    )
}

/// Under the cards of the last reply while the model has not been told about them: what it
/// will be told, and the button that tells it. Continuing is always the user's action.
fn continue_row(
    chat: &Entity<ChatModel>,
    panel: &WeakEntity<ChatPanel>,
    turn: &str,
    cx: &App,
) -> Option<AnyElement> {
    let c = &cx.tokens().colors;
    let model = chat.read(cx);
    if model.open_command_turn() != Some(turn) {
        return None;
    }
    let answers = model.pending_answers();
    let busy = model.streaming() || model.saving || model.command_running();
    let sharing = answers
        .iter()
        .any(|planned| planned.result.stdout.is_some());
    let summary = if model.command_running() {
        "A command is running. Stop it or wait for it before continuing.".to_owned()
    } else {
        contextual_core::chat::continuation_summary(&answers)
    };
    let continue_panel = panel.clone();
    Some(
        div()
            .mt(sp(3))
            .flex()
            .flex_col()
            .gap(sp(1))
            .type_style(contextual_theme::type_style::UI_SMALL)
            .text_color(hsla(c.ink_muted))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(sp(2))
                    .child(
                        Button::new(text(format!("command-continue-{turn}")), "Continue")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Primary)
                            .disabled(busy || answers.is_empty())
                            .on_click(move |_, window, cx| {
                                continue_panel
                                    .update(cx, |panel, cx| panel.continue_commands(window, cx))
                                    .ok();
                            }),
                    )
                    .child(div().whitespace_normal().child(text(summary))),
            )
            .when(sharing, |d| d.child(note(SAVED_NOTE, false, cx)))
            .into_any_element(),
    )
}

fn render(
    chat: &Entity<ChatModel>,
    panel: &WeakEntity<ChatPanel>,
    call: CallId,
    card: Card,
    shown: Shown,
    cx: &App,
) -> AnyElement {
    let c = &cx.tokens().colors;
    let Shown {
        draft,
        live,
        editor,
        collapsed,
        details: details_open,
        animate,
        shortcut,
    } = shown;

    if let Some(input) = editor {
        // Editing: approval was voided when the editor opened.
        let (review_chat, review_panel, review_call) = (chat.clone(), panel.clone(), call.clone());
        let review_input = input.clone();
        let cancel_panel = panel.clone();
        let body = card_body()
            .child(note(
                "Edit the command. It will be checked again and needs your review before it can run.",
                false,
                cx,
            ))
            .child(
                div()
                    .rounded(px(contextual_theme::radius::SM))
                    .border_1()
                    .border_color(hsla(c.border_strong))
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .child(Textarea::new(&input).appearance(false)),
            )
            .when_some(card.notice.clone(), |d, notice| {
                d.child(note(&notice, true, cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .child(
                        Button::new(button_id("review-edit", &call), "Review changes")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Primary)
                            .on_click(move |_, _, cx| {
                                let edited = review_input.read(cx).value().to_string();
                                let accepted = review_chat.update(cx, |chat, cx| {
                                    chat.review_edit(review_call.clone(), &edited, cx)
                                });
                                if accepted {
                                    review_panel
                                        .update(cx, |panel, cx| panel.end_command_edit(cx))
                                        .ok();
                                }
                            }),
                    )
                    .child(
                        Button::new(button_id("cancel-edit", &call), "Cancel")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .on_click(move |_, _, cx| {
                                cancel_panel
                                    .update(cx, |panel, cx| panel.end_command_edit(cx))
                                    .ok();
                            }),
                    ),
            );
        let header = card_header(panel, &call, &card, None, animate, cx);
        return frame(&card, header, body, cx);
    }

    let captured = live
        .as_ref()
        .and_then(|progress| output(progress, &call, cx));
    let fold = captured.as_ref().map(|_| collapsed);
    let header = card_header(panel, &call, &card, fold, animate, cx);
    let can_run = matches!(
        card.phase,
        CardPhase::Review | CardPhase::Unprepared | CardPhase::Starting
    );
    let mut body = card_body();
    let mut any = false;

    // What it is for, in the model's words, while that still matters for a decision.
    if let Some(purpose) = card.purpose.as_ref().filter(|_| can_run) {
        body = body.child(div().whitespace_normal().child(text(purpose.clone())));
        any = true;
    }
    // A longer script is shown whole: Run approves all of it.
    if let Some(script) = &card.script {
        body = body.child(
            inset(cx).children(
                script
                    .split('\n')
                    .map(|part| div().whitespace_normal().child(text(part))),
            ),
        );
        any = true;
    }
    // Where and with what, only when it is not the default.
    let mut facts: Vec<AnyElement> = Vec::new();
    if let Some(cwd) = &card.cwd {
        facts.push(
            div()
                .flex()
                .gap(sp(1))
                .child(text("in"))
                .child(
                    div()
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .child(text(cwd.clone())),
                )
                .into_any_element(),
        );
    }
    facts.extend(card.environment.iter().map(|variable| {
        div()
            .type_style(contextual_theme::type_style::CODE_SMALL)
            .child(text(variable.clone()))
            .into_any_element()
    }));
    if can_run {
        facts.push(text(card.timeout.clone()).into_any_element());
        facts.push(marked(IconName::TriangleAlert, EFFECTS_SHORT, c.warning));
    }
    if !facts.is_empty() {
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(sp(3))
                .gap_y(sp(1))
                .type_style(contextual_theme::type_style::UI_SMALL)
                .text_color(hsla(c.ink_muted))
                .children(facts),
        );
        any = true;
    }

    if let Some(captured) = captured.filter(|_| !collapsed) {
        body = body.child(captured);
        any = true;
    } else if card.phase == CardPhase::Finished && live.is_some() {
        body = body.child(note("No output", false, cx));
        any = true;
    }
    if details_open {
        body = body.child(details(&card, cx));
        any = true;
    }
    if let Some(staged) = &card.staged {
        body = body.child(note(staged, false, cx));
        any = true;
    }
    if let Some(notice) = &card.notice {
        body = body.child(note(notice, true, cx));
        any = true;
    }
    if let Some(told) = &card.told {
        body = body.child(if card.shared {
            marked(IconName::Check, told, c.success)
        } else {
            note(told, false, cx)
        });
        any = true;
    }

    // Decisions on the left, with the one that matters first; looking closer on the right.
    let actions = card.actions;
    let mut left: Vec<AnyElement> = Vec::new();
    let mut right: Vec<AnyElement> = Vec::new();
    let quiet = |message: &str| {
        div()
            .type_style(contextual_theme::type_style::UI_SMALL)
            .text_color(hsla(c.ink_muted))
            .whitespace_normal()
            .child(text(message))
            .into_any_element()
    };
    if card.phase == CardPhase::Review {
        let (run_chat, run_call) = (chat.clone(), call.clone());
        left.push(
            Button::new(
                button_id("run", &call),
                if actions.again { "Run again" } else { "Run" },
            )
            .icon(IconName::Play)
            .size(ButtonSize::Xs)
            .variant(ButtonVariant::Primary)
            .disabled(!actions.run)
            .on_click(move |_, _, cx| {
                run_chat.update(cx, |chat, cx| chat.run_command(run_call.clone(), cx));
            })
            .into_any_element(),
        );
        if shortcut {
            left.push(
                div()
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_faint))
                    .child(text(run_shortcut()))
                    .into_any_element(),
            );
        }
        let (edit_panel, edit_call, edit_draft) = (panel.clone(), call.clone(), draft.clone());
        left.push(
            Button::new(button_id("edit", &call), "Edit")
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Ghost)
                .disabled(!actions.edit)
                .on_click(move |_, window, cx| {
                    edit_panel
                        .update(cx, |panel, cx| {
                            panel.begin_command_edit(
                                edit_call.clone(),
                                edit_draft.clone(),
                                window,
                                cx,
                            )
                        })
                        .ok();
                })
                .into_any_element(),
        );
    }
    if actions.prepare {
        let (prepare_chat, prepare_call) = (chat.clone(), call.clone());
        let again = card.phase == CardPhase::Finished;
        let button = Button::new(
            button_id("prepare", &call),
            if again {
                "Run again…"
            } else {
                "Prepare again"
            },
        )
        .size(ButtonSize::Xs)
        .on_click(move |_, _, cx| {
            prepare_chat.update(cx, |chat, cx| {
                chat.prepare_command(prepare_call.clone(), None, cx)
            });
        });
        if again {
            // Opens the review again; nothing runs until Run is pressed there.
            right.push(
                button
                    .icon(IconName::RotateCw)
                    .variant(ButtonVariant::Ghost)
                    .into_any_element(),
            );
        } else {
            left.push(button.into_any_element());
        }
    }
    if actions.reject {
        let (reject_chat, reject_call) = (chat.clone(), call.clone());
        left.push(
            Button::new(button_id("reject", &call), "Skip")
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Ghost)
                .on_click(move |_, _, cx| {
                    reject_chat.update(cx, |chat, cx| chat.reject_command(&reject_call, cx));
                })
                .into_any_element(),
        );
    }
    if actions.stop {
        let stop_chat = chat.clone();
        left.push(
            Button::new(button_id("stop", &call), "Stop")
                .icon(IconName::Square)
                .size(ButtonSize::Xs)
                .on_click(move |_, _, cx| {
                    stop_chat.update(cx, |chat, cx| chat.stop_command(cx));
                })
                .into_any_element(),
        );
        left.push(quiet(STOP_NOTE));
    }
    if actions.reconcile {
        let check_chat = chat.clone();
        left.push(
            Button::new(button_id("reconcile", &call), "Check whether it still runs")
                .size(ButtonSize::Xs)
                .on_click(move |_, _, cx| {
                    check_chat.update(cx, |chat, cx| chat.reconcile_commands(cx));
                })
                .into_any_element(),
        );
    }
    if actions.share && card.staged.is_none() {
        left.push(quiet("Output stays local."));
    }
    if actions.unshare {
        let (unshare_chat, unshare_call) = (chat.clone(), call.clone());
        left.push(
            Button::new(button_id("unshare", &call), "Don't share")
                .size(ButtonSize::Xs)
                .on_click(move |_, _, cx| {
                    unshare_chat.update(cx, |chat, cx| chat.unstage_share(&unshare_call, cx));
                })
                .into_any_element(),
        );
    }
    if card.phase != CardPhase::Unusable {
        let (details_panel, details_call) = (panel.clone(), call.clone());
        right.insert(
            0,
            Button::new(
                button_id("details", &call),
                if details_open {
                    "Hide details"
                } else {
                    "Details"
                },
            )
            .size(ButtonSize::Xs)
            .variant(ButtonVariant::Ghost)
            .on_click(move |_, _, cx| {
                details_panel
                    .update(cx, |panel, cx| {
                        panel.toggle_command_details(&details_call, cx)
                    })
                    .ok();
            })
            .into_any_element(),
        );
    }
    if !left.is_empty() || !right.is_empty() {
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(sp(2))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(sp(2))
                        .children(left),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(sp(1))
                        .children(right),
                ),
        );
        any = true;
    }
    if any {
        frame(&card, header, body, cx)
    } else {
        frame(&card, header, div(), cx)
    }
}

/// The card's outline says where it stands: waiting for a decision, ended badly, or neither.
fn frame(card: &Card, header: impl IntoElement, body: impl IntoElement, cx: &App) -> AnyElement {
    let c = &cx.tokens().colors;
    let outline = match (card.phase, card.tone) {
        (CardPhase::Review, _) => c.accent,
        (CardPhase::Finished, CardTone::Danger) => c.danger,
        _ => c.border,
    };
    div()
        .mt(sp(3))
        .rounded(px(contextual_theme::radius::MD))
        .border_1()
        .border_color(hsla(outline))
        .bg(hsla(c.surface_200))
        .overflow_hidden()
        .child(header)
        .child(body)
        .into_any_element()
}
