//! The review card for one proposed command, under the reply that proposed it.
//!
//! What the card shows and which actions it offers is decided in
//! `contextual_core::commands::card`; this view renders that and forwards each click to the
//! chat model. It decides nothing itself. Output shown here is held in memory only and has
//! not been sent to the model.

use contextual_core::chat::UiMessage;
use contextual_core::commands::CallId;
use contextual_core::commands::card::{
    Card, CardInput, CardLine, CardPhase, CardTone, EFFECTS_NOTE, SAVED_NOTE, SHARE_WARNING,
    STOP_NOTE, review_card,
};
use contextual_core::commands::execution::ExecutionProgress;
use contextual_core::logic::tree::CheckState;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Entity, FontWeight, IntoElement, ParentElement, SharedString, Styled,
    Subscription, WeakEntity, div,
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
            .when(entry.mono, |d| {
                d.type_style(contextual_theme::type_style::CODE_SMALL)
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
        .child(
            div()
                .flex_none()
                .w(sp(8) * 3.0)
                .type_style(contextual_theme::type_style::UI_SMALL)
                .text_color(hsla(c.ink_muted))
                .child(text(entry.label.clone())),
        )
        .child(div().flex_1().min_w_0().child(value))
        .into_any_element()
}

/// The end of one captured stream.
fn stream(label: &str, captured: &str, truncated: bool, cx: &App) -> Option<AnyElement> {
    if captured.is_empty() {
        return None;
    }
    let c = &cx.tokens().colors;
    let lines: Vec<&str> = captured.lines().collect();
    let hidden = lines.len().saturating_sub(OUTPUT_TAIL_LINES);
    let mut caption = label.to_owned();
    if hidden > 0 {
        caption.push_str(&format!(
            " · last {OUTPUT_TAIL_LINES} of {} lines",
            lines.len()
        ));
    }
    if truncated {
        caption.push_str(" · more was produced than was kept");
    }
    Some(
        div()
            .flex()
            .flex_col()
            .gap(sp(1))
            .child(
                div()
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(text(caption)),
            )
            .child(
                div()
                    .p(sp(2))
                    .rounded(px(contextual_theme::radius::SM))
                    .bg(hsla(c.surface_200))
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .flex()
                    .flex_col()
                    .children(
                        lines[hidden..]
                            .iter()
                            .map(|part| div().whitespace_normal().child(text(*part))),
                    ),
            )
            .into_any_element(),
    )
}

fn output(progress: &ExecutionProgress, cx: &App) -> Vec<AnyElement> {
    [
        stream(
            "Output",
            &progress.stdout.text,
            progress.stdout.truncated,
            cx,
        ),
        stream(
            "Error output",
            &progress.stderr.text,
            progress.stderr.truncated,
            cx,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
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
    editing: Option<&(CallId, Entity<TextareaState>)>,
    sharing: Option<&ShareEdit>,
    message: &UiMessage,
    cx: &App,
) -> Vec<AnyElement> {
    let Some(turn) = message.command_turn.as_deref() else {
        return Vec::new();
    };
    let model = chat.read(cx);
    let desk = &project.read(cx).commands;
    let mut cards: Vec<AnyElement> = model
        .thread
        .commands
        .of_turn(turn)
        .map(|record| {
            let call = record.call.clone();
            let runtime = model.card_runtime(&call);
            let card = review_card(&CardInput {
                record,
                plan: desk.plan(&call),
                review: desk.state(&call),
                project_busy: desk.busy(),
                chat_busy: model.streaming() || model.saving,
                preparing: runtime.preparing,
                starting: runtime.starting,
                error: runtime.error.as_deref(),
                enabled: model.commands_enabled,
                output_held: desk.output(&call).is_some(),
                staged: model.staged_share(&call),
            });
            if let Some(share) = sharing.filter(|share| share.call == call && card.actions.share) {
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
            let editor = editing
                .filter(|(edited, _)| edited == &call)
                .map(|(_, input)| input.clone());
            render(chat, panel, call, card, draft, live, editor, cx)
        })
        .collect();
    cards.extend(continue_row(chat, panel, turn, cx));
    cards
}

fn card_header(card: &Card, cx: &App) -> gpui_kit::Div {
    let c = &cx.tokens().colors;
    let status_color = match card.tone {
        CardTone::Neutral => c.ink_muted,
        CardTone::Ok => c.success,
        CardTone::Warn => c.warning,
        CardTone::Danger => c.danger,
    };
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(sp(2))
        .px(sp(3))
        .py(sp(2))
        .bg(hsla(c.surface_200))
        .border_b_1()
        .border_color(hsla(c.border))
        .type_style(contextual_theme::type_style::UI_SMALL)
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(hsla(c.ink_strong))
                .child(text("Proposed command")),
        )
        .child(
            div()
                .text_color(hsla(status_color))
                .child(text(card.status.clone())),
        )
        .when(card.edited, |d| {
            d.child(Tag::new("Edited by you").tone(TagTone::Warn))
        })
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
    let c = &cx.tokens().colors;
    let (stdout, stderr) = share.streams(cx);
    let preview = chat
        .read(cx)
        .share_preview(&share.call, stdout.clone(), stderr.clone(), cx);
    let (use_chat, use_panel, use_call) = (chat.clone(), panel.clone(), share.call.clone());
    let cancel_panel = panel.clone();
    let sendable = preview.is_ok();
    let body = div()
        .flex()
        .flex_col()
        .gap(sp(2))
        .p(sp(3))
        .type_style(contextual_theme::type_style::UI_BODY)
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
            Ok(json) => div()
                .p(sp(2))
                .rounded(px(contextual_theme::radius::SM))
                .bg(hsla(c.surface_200))
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .flex()
                .flex_col()
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
    frame(card_header(card, cx), body, cx)
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

#[allow(clippy::too_many_arguments)]
fn render(
    chat: &Entity<ChatModel>,
    panel: &WeakEntity<ChatPanel>,
    call: CallId,
    card: Card,
    draft: String,
    live: Option<ExecutionProgress>,
    editor: Option<Entity<TextareaState>>,
    cx: &App,
) -> AnyElement {
    let c = &cx.tokens().colors;
    let header = card_header(&card, cx);

    let mut body = div()
        .flex()
        .flex_col()
        .gap(sp(2))
        .p(sp(3))
        .type_style(contextual_theme::type_style::UI_BODY);

    if let Some(input) = editor {
        // Editing: approval was voided when the editor opened.
        let (review_chat, review_panel, review_call) = (chat.clone(), panel.clone(), call.clone());
        let review_input = input.clone();
        let cancel_panel = panel.clone();
        body = body
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
                            .on_click(move |_, _, cx| {
                                cancel_panel
                                    .update(cx, |panel, cx| panel.end_command_edit(cx))
                                    .ok();
                            }),
                    ),
            );
        return frame(header, body, cx);
    }

    if let Some(purpose) = &card.purpose {
        body = body.child(line(
            &CardLine {
                label: "Purpose (model-provided)".into(),
                value: purpose.clone(),
                mono: false,
            },
            cx,
        ));
    }
    body = body
        .children(card.command.iter().map(|entry| line(entry, cx)))
        .children(card.details.iter().map(|entry| line(entry, cx)));

    if let Some(progress) = &live {
        body = body.children(output(progress, cx));
        if card.phase == CardPhase::Finished {
            body = body.child(note(
                "This output is kept in memory only. It has not been sent to the model.",
                false,
                cx,
            ));
        }
    }
    if matches!(
        card.phase,
        CardPhase::Review | CardPhase::Unprepared | CardPhase::Starting
    ) {
        body = body.child(note(EFFECTS_NOTE, false, cx));
    }
    if card.phase == CardPhase::Running {
        body = body.child(note(STOP_NOTE, false, cx));
    }
    if let Some(staged) = &card.staged {
        body = body.child(note(staged, false, cx));
    }
    if let Some(notice) = &card.notice {
        body = body.child(note(notice, true, cx));
    }

    let actions = card.actions;
    let mut buttons = div().flex().flex_wrap().items_center().gap(sp(2));
    let mut any = false;
    if card.phase == CardPhase::Review {
        let (run_chat, run_call) = (chat.clone(), call.clone());
        buttons = buttons.child(
            Button::new(
                button_id("run", &call),
                if actions.again { "Run again" } else { "Run" },
            )
            .size(ButtonSize::Xs)
            .variant(ButtonVariant::Primary)
            .disabled(!actions.run)
            .on_click(move |_, _, cx| {
                run_chat.update(cx, |chat, cx| chat.run_command(run_call.clone(), cx));
            }),
        );
        let (edit_panel, edit_call, edit_draft) = (panel.clone(), call.clone(), draft.clone());
        buttons = buttons.child(
            Button::new(button_id("edit", &call), "Edit")
                .size(ButtonSize::Xs)
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
                }),
        );
        any = true;
    }
    if actions.prepare {
        let (prepare_chat, prepare_call) = (chat.clone(), call.clone());
        let label = match card.phase {
            CardPhase::Finished => "Review to run again",
            _ => "Prepare again",
        };
        buttons = buttons.child(
            Button::new(button_id("prepare", &call), label)
                .size(ButtonSize::Xs)
                .on_click(move |_, _, cx| {
                    prepare_chat.update(cx, |chat, cx| {
                        chat.prepare_command(prepare_call.clone(), None, cx)
                    });
                }),
        );
        any = true;
    }
    if actions.reject {
        let (reject_chat, reject_call) = (chat.clone(), call.clone());
        buttons = buttons.child(
            Button::new(button_id("reject", &call), "Reject")
                .size(ButtonSize::Xs)
                .variant(ButtonVariant::Ghost)
                .on_click(move |_, _, cx| {
                    reject_chat.update(cx, |chat, cx| chat.reject_command(&reject_call, cx));
                }),
        );
        any = true;
    }
    if actions.stop {
        let stop_chat = chat.clone();
        buttons = buttons.child(
            Button::new(button_id("stop", &call), "Stop")
                .size(ButtonSize::Xs)
                .on_click(move |_, _, cx| {
                    stop_chat.update(cx, |chat, cx| chat.stop_command(cx));
                }),
        );
        any = true;
    }
    if actions.reconcile {
        let check_chat = chat.clone();
        buttons = buttons.child(
            Button::new(button_id("reconcile", &call), "Check whether it still runs")
                .size(ButtonSize::Xs)
                .on_click(move |_, _, cx| {
                    check_chat.update(cx, |chat, cx| chat.reconcile_commands(cx));
                }),
        );
        any = true;
    }
    if actions.share {
        let (share_panel, share_call) = (panel.clone(), call.clone());
        buttons = buttons.child(
            Button::new(button_id("share", &call), "Share result…")
                .size(ButtonSize::Xs)
                .on_click(move |_, window, cx| {
                    share_panel
                        .update(cx, |panel, cx| {
                            panel.begin_command_share(share_call.clone(), window, cx)
                        })
                        .ok();
                }),
        );
        any = true;
    }
    if actions.unshare {
        let (unshare_chat, unshare_call) = (chat.clone(), call.clone());
        buttons = buttons.child(
            Button::new(button_id("unshare", &call), "Don't share")
                .size(ButtonSize::Xs)
                .on_click(move |_, _, cx| {
                    unshare_chat.update(cx, |chat, cx| chat.unstage_share(&unshare_call, cx));
                }),
        );
        any = true;
    }
    if any {
        body = body.child(buttons);
    }
    frame(header, body, cx)
}

fn frame(header: impl IntoElement, body: impl IntoElement, cx: &App) -> AnyElement {
    let c = &cx.tokens().colors;
    div()
        .mt(sp(3))
        .rounded(px(contextual_theme::radius::MD))
        .border_1()
        .border_color(hsla(c.border))
        .overflow_hidden()
        .child(header)
        .child(body)
        .into_any_element()
}
