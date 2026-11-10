//! Debug-build automation, so a full flow can be checked without clicking (compiled out of
//! release builds). `CONTEXTUAL_DEV_SEND="text"` selects every file once the project opens and
//! sends `text`; `CONTEXTUAL_DEV_AFTER=diff|split|save|undo|chats|inspect` then takes one action after the
//! reply. `CONTEXTUAL_DEV_OPEN=scm|settings|branches|pr|inspect|gitdiff:<path>|commit:<message>|switch:<branch>`
//! opens that view, or runs that Source Control action, at startup.
//! `CONTEXTUAL_DEV_OPEN=envfile` with `--settings` opens the env-file picker in Settings.
//!
//! With `CONTEXTUAL_DEV_COMMANDS=1`,
//! `CONTEXTUAL_DEV_AFTER=command-run|command-stop|command-reject|command-edit` presses that
//! button on the first proposed command once its review card is ready (`command-stop` presses
//! Run, then Stop when the command has produced output; `command-edit` replaces every `ran`
//! in the command with `edited`, has the edit reviewed, then presses Run). `command-share`
//! presses Run, then chooses to share the output without the error output, then presses
//! Continue; `command-withhold` presses Run and then Continue without sharing;
//! `command-decline` presses Reject and then Continue. `command-quit` presses Run and quits the
//! app once the command has produced output; `command-close` closes the project instead.
//! These stand in for a person's actions and exist only in debug builds.

use contextual_core::chat::Role;
use gpui_kit::{App, Entity, Window};

use crate::state::chat::{CenterView, ChatModel, DiffMode};
use crate::state::{AppModel, Project};
use crate::views::chat_panel::ChatPanel;
use crate::views::chats_menu::ChatsMenu;

pub fn install(
    app: &Entity<AppModel>,
    project: &Entity<Project>,
    chat: &Entity<ChatModel>,
    panel: &Entity<ChatPanel>,
    chats: &Entity<ChatsMenu>,
    window: &mut Window,
    cx: &mut App,
) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Ok(text) = std::env::var("CONTEXTUAL_DEV_SEND") else {
        return;
    };
    let after = std::env::var("CONTEXTUAL_DEV_AFTER").unwrap_or_default();
    // With "Model picks files" the model asks for what it needs; otherwise tick everything.
    if !app.read(cx).settings.chat.model_picks_files {
        project.update(cx, |p, cx| {
            p.select_all();
            cx.notify()
        });
    }
    panel.update(cx, |p, cx| p.dev_send(text, window, cx));

    if let Some(action) = after.strip_prefix("command-") {
        // `CONTEXTUAL_DEV_EXPECT_FILE=<path>` reports on stderr when the project tree
        // contains that file, which shows that a rescan picked up what a command created.
        if let Ok(expected) = std::env::var("CONTEXTUAL_DEV_EXPECT_FILE") {
            let mut reported = false;
            cx.observe(project, move |project, cx| {
                if !reported && project.read(cx).node(&expected).is_some() {
                    reported = true;
                    eprintln!("contextual dev: the project tree now contains {expected}");
                }
            })
            .detach();
        }
        let (action, then) = match action {
            "share" => ("run", Some(Followup::Share)),
            "withhold" => ("run", Some(Followup::Continue)),
            "decline" => ("reject", Some(Followup::Continue)),
            "quit" | "close" => {
                leave_while_command_runs(app, chat, action == "quit", cx);
                ("run", None)
            }
            other => (other, None),
        };
        press_command_button(project, chat, action.to_string(), cx);
        if let Some(then) = then {
            continue_after_command(project, chat, panel, then, window, cx);
        }
        return;
    }
    let (app, chat_for_obs, chats) = (app.clone(), chat.clone(), chats.clone());
    let mut done = false;
    cx.observe(chat, move |chat, cx| {
        if done || after.is_empty() {
            return;
        }
        let finished =
            chat.read(cx).messages.last().is_some_and(|m| {
                m.role != Role::User && !m.streaming && !chat.read(cx).streaming()
            });
        if !finished || (after != "inspect" && chat.read(cx).changes().is_empty()) {
            return;
        }
        done = true;
        let app = app.clone();
        let after = after.clone();
        if after == "chats" {
            chats.update(cx, |m, cx| {
                m.open = true;
                cx.notify();
            });
            return;
        }
        chat_for_obs.update(cx, |c, cx| match after.as_str() {
            "inspect" => {
                c.set_center(CenterView::Inspect, cx);
            }
            "diff" | "split" => {
                c.diff_mode = if after == "split" {
                    DiffMode::Split
                } else {
                    DiffMode::Unified
                };
                c.set_center(CenterView::Diff, cx);
            }
            "save" | "undo" => {
                let undo = after == "undo";
                c.save(move |cx: &mut App| app.update(cx, |a, cx| a.rescan(cx)), cx);
                if undo {
                    // Undo once the save's toast (with its snapshot id) is up.
                    cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(200))
                                .await;
                            let ready = this
                                .update(cx, |c, _| {
                                    c.toast.as_ref().is_some_and(|t| t.backup_id.is_some())
                                })
                                .unwrap_or(true);
                            if ready {
                                break;
                            }
                        }
                        this.update(cx, |c, cx| c.undo(|_| {}, cx)).ok();
                    })
                    .detach();
                }
            }
            _ => {}
        });
    })
    .detach();
}

/// Opens a view at startup (`CONTEXTUAL_DEV_OPEN`), debug builds only.
pub fn open(
    app: &Entity<crate::state::AppModel>,
    sidebar: &Entity<crate::views::sidebar::Sidebar>,
    project: &Entity<Project>,
    scm: &Entity<crate::state::scm::ScmModel>,
    chat: &Entity<ChatModel>,
    cx: &mut App,
) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Ok(what) = std::env::var("CONTEXTUAL_DEV_OPEN") else {
        return;
    };
    match what.as_str() {
        "scm" => sidebar.update(cx, |s, cx| s.show_scm(cx)),
        "settings" => {
            let app = app.clone();
            cx.defer(move |cx| app.update(cx, |a, cx| a.open_settings(cx)));
        }
        "branches" => scm.update(cx, |s, cx| s.open_branch_picker(cx)),
        "pr" => chat.update(cx, |c, cx| {
            c.pr_open = true;
            cx.notify();
        }),
        "inspect" => chat.update(cx, |c, cx| {
            c.set_center(CenterView::Inspect, cx);
        }),
        other => {
            if let Some(message) = other.strip_prefix("commit:") {
                let message = message.to_string();
                sidebar.update(cx, |s, cx| s.show_scm(cx));
                // Wait for the first status, then commit through the same path as the button.
                let scm = scm.clone();
                cx.spawn(async move |cx| {
                    for _ in 0..50 {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(100))
                            .await;
                        if scm.read_with(cx, |s, _| s.status.is_some()) {
                            break;
                        }
                    }
                    scm.update(cx, |s, cx| s.commit(message, cx));
                })
                .detach();
            } else if let Some(branch) = other.strip_prefix("switch:") {
                let branch = branch.to_string();
                scm.update(cx, |s, cx| s.switch_branch(branch, |_| {}, cx));
            } else if let Some(path) = other.strip_prefix("gitdiff:") {
                sidebar.update(cx, |s, cx| s.show_scm(cx));
                let tab = contextual_core::logic::git_tabs::git_tab_id(path, false);
                project.update(cx, |p, cx| {
                    p.open_preview(&tab);
                    cx.notify()
                });
            }
        }
    }
}

/// Presses Run, Stop or Reject on the first proposed command, as a person would, once the
/// card offers it. Debug builds only; see the module documentation.
fn press_command_button(
    project: &Entity<Project>,
    chat: &Entity<ChatModel>,
    action: String,
    cx: &mut App,
) {
    use contextual_core::commands::CommandState;

    let project = project.clone();
    let (mut pressed, mut stopped, mut edited) = (false, false, false);
    cx.observe(chat, move |chat, cx| {
        let model = chat.read(cx);
        let Some(call) = model
            .thread
            .commands
            .records()
            .first()
            .map(|record| record.call.clone())
        else {
            return;
        };
        if pressed {
            // Run was pressed for `command-stop`: press Stop once there is output.
            let has_output = model
                .running_output(&call)
                .is_some_and(|output| !output.stdout.text.is_empty());
            if action == "stop" && !stopped && has_output {
                stopped = true;
                chat.update(cx, |chat, cx| chat.stop_command(cx));
            }
            return;
        }
        let desk = &project.read(cx).commands;
        let reviewable = desk.state(&call) == Some(CommandState::NeedsReview)
            && !model.streaming()
            && !model.card_runtime(&call).preparing;
        if !reviewable {
            return;
        }
        if action == "edit" && !edited {
            // Open the editor, change the command, and send the edit back for review.
            let Some(draft) = desk
                .plan(&call)
                .and_then(|plan| serde_json::to_string(&plan.request).ok())
            else {
                return;
            };
            edited = true;
            let draft = draft.replace("ran", "edited");
            chat.update(cx, |chat, cx| {
                chat.edit_command(&call, cx);
                chat.review_edit(call, &draft, cx);
            });
            return;
        }
        pressed = true;
        chat.update(cx, |chat, cx| match action.as_str() {
            "reject" => chat.reject_command(&call, cx),
            _ => chat.run_command(call, cx),
        });
    })
    .detach();
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Followup {
    /// Choose to share the output (not the error output), then continue.
    Share,
    /// Continue without sharing anything.
    Continue,
}

/// Once the first proposed command has been decided (it finished, or was rejected), presses
/// Continue under its card, after choosing what to share. Debug builds only.
fn continue_after_command(
    project: &Entity<Project>,
    chat: &Entity<ChatModel>,
    panel: &Entity<ChatPanel>,
    then: Followup,
    window: &mut Window,
    cx: &mut App,
) {
    use contextual_core::commands::history::RecordStatus;

    let (project, panel) = (project.clone(), panel.clone());
    let mut done = false;
    window
        .observe(chat, cx, move |chat, window, cx| {
            if done {
                return;
            }
            let model = chat.read(cx);
            let Some(record) = model.thread.commands.records().first() else {
                return;
            };
            let decided = matches!(
                record.status(),
                RecordStatus::Finished(_) | RecordStatus::Rejected
            );
            if !decided || model.streaming() || model.command_running() {
                return;
            }
            let call = record.call.clone();
            done = true;
            if then == Followup::Share {
                let Some(stdout) = project
                    .read(cx)
                    .commands
                    .output(&call)
                    .map(|(_, result)| result.output.stdout.text.clone())
                else {
                    return;
                };
                chat.update(cx, |chat, cx| {
                    chat.stage_share(&call, Some(stdout), None, cx);
                });
            }
            panel.update(cx, |panel, cx| panel.continue_commands(window, cx));
        })
        .detach();
}

/// Once the running command has produced output, quits the app or closes the project, as a
/// person might while a command is still going. Debug builds only.
fn leave_while_command_runs(
    app: &Entity<AppModel>,
    chat: &Entity<ChatModel>,
    quit: bool,
    cx: &mut App,
) {
    let app = app.clone();
    let mut left = false;
    cx.observe(chat, move |chat, cx| {
        let model = chat.read(cx);
        let Some(call) = model
            .thread
            .commands
            .records()
            .first()
            .map(|record| record.call.clone())
        else {
            return;
        };
        let has_output = model
            .running_output(&call)
            .is_some_and(|output| !output.stdout.text.is_empty());
        if left || !has_output {
            return;
        }
        left = true;
        if quit {
            cx.quit();
        } else {
            let app = app.clone();
            cx.defer(move |cx| app.update(cx, |app, cx| app.close_project(cx)));
        }
    })
    .detach();
}
