//! Debug-build automation, so a full flow can be checked without clicking (compiled out of
//! release builds). `CONTEXTUAL_DEV_SEND="text"` selects every file once the project opens and
//! sends `text`; `CONTEXTUAL_DEV_AFTER=diff|split|save|undo|chats` then takes one action after the
//! reply.

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
    project.update(cx, |p, cx| {
        p.select_all();
        cx.notify()
    });
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
        if !finished || chat.read(cx).changes().is_empty() {
            return;
        }
        done = true;
        let app = app.clone();
        let after = after.clone();
        if after == "chats" {
            chats.update(cx, |m, cx| m.toggle(cx));
            return;
        }
        chat_for_obs.update(cx, |c, cx| match after.as_str() {
            "diff" | "split" => {
                c.center = CenterView::Diff;
                c.diff_mode = if after == "split" {
                    DiffMode::Split
                } else {
                    DiffMode::Unified
                };
                cx.notify();
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
