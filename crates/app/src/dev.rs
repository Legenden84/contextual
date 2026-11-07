//! Debug-build automation, so a full flow can be checked without clicking (compiled out of
//! release builds). `CONTEXTUAL_DEV_SEND="text"` selects every file once the project opens and
//! sends `text`; `CONTEXTUAL_DEV_AFTER=diff|split|save|undo|chats|inspect` then takes one action after the
//! reply. `CONTEXTUAL_DEV_OPEN=scm|settings|branches|pr|inspect|gitdiff:<path>|commit:<message>|switch:<branch>`
//! opens that view, or runs that Source Control action, at startup.
//! `CONTEXTUAL_DEV_OPEN=envfile` with `--settings` opens the env-file picker in Settings.

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
