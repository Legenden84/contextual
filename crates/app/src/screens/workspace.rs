//! Workspace (design spec: ScreenWorkspace): sidebar and persistent tabs for Chat, file previews,
//! Diff and Inspect. Feature tabs use the centre and right columns beneath the shared tab bar;
//! Chat and previews keep a stable column beside responsive Changes and history docks.

use std::collections::HashMap;

use contextual_theme::layout;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyView, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::state::chat::ChatModel;
use crate::state::scm::{OpenPullRequest, ScmModel};
use crate::state::{AppModel, Project, ReturnToChat, Tab};
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, IconButton, Tag, file_glyph};
use crate::views::branch_picker::BranchPicker;
use crate::views::changes_panel::{ChangesPanel, OpenDiffFile};
use crate::views::chat_panel::ChatPanel;
use crate::views::chats_menu::ChatsMenu;
use crate::views::code_preview::CodePreview;
use crate::views::context_bar::{ContextCache, context_bar};
use crate::views::diff_cache::DiffStore;
use crate::views::diff_screen::DiffScreen;
use crate::views::git_diff::GitDiffView;
use crate::views::inspect_screen::InspectScreen;
use crate::views::pr_dialog::PrDialog;
use crate::views::save_dialog::SaveDialog;
use crate::views::sidebar::Sidebar;
use crate::views::toast::toast;
use contextual_core::logic::git_tabs::parse_git_tab;

/// Centre tab bar height.
const TABS_H: f32 = 36.0;

struct ChatLayout {
    width: f32,
    show_changes: bool,
    show_history: bool,
}

/// `available` is the width after the fixed explorer column. Reserve one sidebar even
/// when it is empty, and cap the chat at its reading width plus message/composer padding.
/// Panel visibility must never participate in sizing the chat.
fn chat_layout(available: f32, has_changes: bool, history_open: bool) -> ChatLayout {
    let max_width = layout::CONTENT_MAX_WIDTH + 2.0 * contextual_theme::space::S4;
    let width = (available - layout::CHANGES_WIDTH).clamp(0.0, max_width);
    let fits_both = available - width >= 2.0 * layout::CHANGES_WIDTH;

    ChatLayout {
        width,
        show_changes: has_changes && (!history_open || fits_both),
        show_history: history_open,
    }
}

pub struct WorkspaceScreen {
    app: Entity<AppModel>,
    project: Entity<Project>,
    sidebar: Entity<Sidebar>,
    previews: HashMap<String, AnyView>,
    scm: Entity<ScmModel>,
    branch_picker: Option<Entity<BranchPicker>>,
    pr_dialog: Option<Entity<PrDialog>>,
    chat: Entity<ChatModel>,
    chat_panel: Entity<ChatPanel>,
    changes: Entity<ChangesPanel>,
    diff: Entity<DiffScreen>,
    inspect: Entity<InspectScreen>,
    chats: Entity<ChatsMenu>,
    save_dialog: Option<Entity<SaveDialog>>,
    context: ContextCache,
    _subs: Vec<Subscription>,
}

impl WorkspaceScreen {
    pub fn new(
        app: Entity<AppModel>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subs = vec![
            cx.observe_in(&project, window, |this: &mut Self, project, window, cx| {
                // Feature tabs have their own persistent entities, not file previews.
                let tabs = project.read(cx).tabs.clone();
                this.previews
                    .retain(|p, _| tabs.contains(&Tab::Preview(p.clone())));
                for tab in tabs {
                    let Tab::Preview(t) = tab else {
                        continue;
                    };
                    if !this.previews.contains_key(&t) {
                        let host = this.app.update(cx, |a, _| a.host());
                        let view: AnyView = match parse_git_tab(&t) {
                            Some(g) => {
                                let scm = this.scm.clone();
                                cx.new(|cx| {
                                    GitDiffView::new(
                                        g.path,
                                        g.staged,
                                        project.clone(),
                                        scm,
                                        host,
                                        cx,
                                    )
                                })
                                .into()
                            }
                            None => cx
                                .new(|cx| {
                                    CodePreview::new(t.clone(), project.clone(), host, window, cx)
                                })
                                .into(),
                        };
                        this.previews.insert(t, view);
                    }
                }
                cx.notify();
            }),
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.subscribe_in(&app, window, |this, _, _: &ReturnToChat, window, cx| {
                this.project.update(cx, |p, cx| {
                    p.activate(Tab::Chat);
                    cx.notify();
                });
                this.chat_panel
                    .update(cx, |panel, cx| panel.focus_input(window, cx));
                cx.notify();
            }),
        ];
        let host = app.update(cx, |a, _| a.host());
        let scm = app.read(cx).scm.clone().unwrap_or_else(|| {
            let (project, host) = (project.clone(), host.clone());
            cx.new(|cx| ScmModel::new(project, host, cx))
        });
        let chat = cx.new(|cx| ChatModel::new(project.clone(), host, cx));
        let diffs = cx.new(|_| DiffStore::default());
        let mut subs = subs;
        subs.push(
            cx.observe_in(&chat, window, |this: &mut Self, chat, window, cx| {
                // The Save and pull request dialogs live while the model says they're open.
                let open = chat.read(cx).save_dialog_open;
                if open && this.save_dialog.is_none() {
                    let (app, chat) = (this.app.clone(), chat.clone());
                    this.save_dialog = Some(cx.new(|cx| SaveDialog::new(app, chat, window, cx)));
                } else if !open {
                    this.save_dialog = None;
                }
                let pr = chat.read(cx).pr_open;
                if pr && this.pr_dialog.is_none() {
                    this.sidebar.update(cx, |sidebar, cx| sidebar.show_scm(cx));
                    let (app, project, chat) =
                        (this.app.clone(), this.project.clone(), chat.clone());
                    this.pr_dialog =
                        Some(cx.new(|cx| PrDialog::new(app, project, chat, window, cx)));
                } else if !pr {
                    this.pr_dialog = None;
                }
                cx.notify();
            }),
        );
        subs.push(
            cx.observe_in(&scm, window, |this: &mut Self, scm, window, cx| {
                let open = scm.read(cx).branch_picker_open;
                if open && this.branch_picker.is_none() {
                    let (app, scm) = (this.app.clone(), scm.clone());
                    this.branch_picker = Some(cx.new(|cx| BranchPicker::new(app, scm, window, cx)));
                } else if !open {
                    this.branch_picker = None;
                }
                cx.notify();
            }),
        );
        subs.push(cx.subscribe(
            &scm,
            |this: &mut Self, _, _: &OpenPullRequest, cx| {
                this.chat.update(cx, |chat, cx| {
                    chat.pr_open = true;
                    cx.notify();
                });
            },
        ));
        let chat_panel = cx.new(|cx| {
            ChatPanel::new(
                app.clone(),
                project.clone(),
                chat.clone(),
                diffs.clone(),
                window,
                cx,
            )
        });
        // The settings warning is owned by the composer but overlays the whole workspace.
        subs.push(cx.observe(&chat_panel, |_, _, cx| cx.notify()));
        let chats = cx.new(|cx| ChatsMenu::new(chat.clone(), window, cx));
        // History visibility changes the workspace layout, not just the panel's contents.
        subs.push(cx.observe(&chats, |_, _, cx| cx.notify()));
        crate::dev::install(&app, &project, &chat, &chat_panel, &chats, window, cx);
        let sidebar =
            cx.new(|cx| Sidebar::new(app.clone(), project.clone(), scm.clone(), window, cx));
        crate::dev::open(&app, &sidebar, &project, &scm, &chat, cx);
        let changes = cx.new(|cx| ChangesPanel::new(chat.clone(), diffs.clone(), cx));
        let diff = cx.new(|cx| DiffScreen::new(chat.clone(), diffs, cx));
        subs.push(cx.subscribe(
            &changes,
            |this: &mut Self, _, event: &OpenDiffFile, cx| {
                this.diff
                    .update(cx, |diff, cx| diff.open_file(&event.path, cx));
            },
        ));
        Self {
            sidebar,
            scm,
            branch_picker: None,
            pr_dialog: None,
            chat_panel,
            changes,
            diff,
            inspect: cx.new(|cx| InspectScreen::new(chat.clone(), window, cx)),
            chats,
            save_dialog: None,
            chat,
            app,
            project,
            previews: HashMap::new(),
            context: ContextCache::default(),
            _subs: subs,
        }
    }

    fn tab(
        &self,
        id: SharedString,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let c = &cx.tokens().colors;
        div()
            .id(id)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(sp(2))
            .whitespace_nowrap()
            .px(sp(3))
            .h_full()
            .border_r_1()
            .border_color(hsla(c.border))
            .cursor_pointer()
            .text_color(hsla(if active { c.ink_strong } else { c.ink_muted }))
            .when(!active, |d| d.hover(|s| s.bg(hsla(c.surface_hover))))
            .when(active, |d| {
                d.bg(hsla(c.surface_100)).child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(2.0))
                        .bg(hsla(c.accent)),
                )
            })
    }
}

impl Render for WorkspaceScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let p = self.project.read(cx);
        let active = p.active.clone();
        let tabs = p.tabs.clone();

        let chat_tab = self
            .tab("tab-chat".into(), active == Tab::Chat, cx)
            .on_click(cx.listener(|t, _, window, cx| {
                t.project.update(cx, |p, cx| {
                    p.activate(Tab::Chat);
                    cx.notify()
                });
                t.chat_panel
                    .update(cx, |panel, cx| panel.focus_input(window, cx));
            }))
            .child(
                div()
                    .text_color(hsla(c.accent_ink))
                    .child(Icon::new(IconName::Bot).size(px(14.0))),
            )
            .child("Chat");
        let open_tabs: Vec<_> = tabs
            .iter()
            .map(|tab| {
                let (id, name, tag, glyph) = match tab {
                    Tab::Preview(path) => {
                        let git = parse_git_tab(path);
                        let shown = git
                            .as_ref()
                            .map(|g| g.path.as_str())
                            .unwrap_or(path);
                        let name = shown.rsplit('/').next().unwrap_or(shown).to_string();
                        let tag = match &git {
                            Some(g) if g.staged => "staged",
                            Some(_) => "changes",
                            None => "read-only",
                        };
                        let glyph = file_glyph(&name, cx).into_any_element();
                        (format!("preview:{path}"), name, Some(tag), glyph)
                    }
                    Tab::Diff => (
                        "diff".into(),
                        "Diff".into(),
                        None,
                        Icon::new(IconName::File).size(sp(4)).into_any_element(),
                    ),
                    Tab::Inspect => (
                        "inspect".into(),
                        "Inspect".into(),
                        None,
                        Icon::new(gpui_kit::assets::IconName::Brain)
                            .size(sp(4))
                            .into_any_element(),
                    ),
                    Tab::Chat => (
                        "chat".into(),
                        "Chat".into(),
                        None,
                        Icon::new(IconName::Bot).size(sp(4)).into_any_element(),
                    ),
                };
                let (activate, close) = (tab.clone(), tab.clone());
                let close_label = format!("Close {name}");
                self.tab(
                    SharedString::from(format!("tab:{id}")),
                    &active == tab,
                    cx,
                )
                .on_click(cx.listener(move |t, _, _, cx| {
                    t.project.update(cx, |p, cx| {
                        p.activate(activate.clone());
                        cx.notify()
                    })
                }))
                .child(glyph)
                .child(SharedString::from(name))
                .when_some(tag, |d, tag| d.child(Tag::new(tag)))
                .child(
                    IconButton::new(
                        SharedString::from(format!("close:{id}")),
                        IconName::Close,
                        close_label,
                    )
                    .on_click(cx.listener(move |t, _, window, cx| {
                        cx.stop_propagation();
                        t.project.update(cx, |p, cx| {
                            p.close_tab(&close);
                            cx.notify()
                        });
                        if t.project.read(cx).active == Tab::Chat {
                            t.chat_panel
                                .update(cx, |panel, cx| panel.focus_input(window, cx));
                        }
                    })),
                )
            })
            .collect();

        let chat = self.chat.read(cx);
        let has_changes = chat.change_message().is_some();
        let feature_view = matches!(active, Tab::Diff | Tab::Inspect);
        let history_open = self.chats.read(cx).open && !feature_view;
        // The workspace fills the viewport; only the fixed explorer consumes width
        // before this area. Recompute on resize, never from panel visibility.
        let panes = chat_layout(
            f32::from(window.viewport_size().width) - layout::EXPLORER_WIDTH,
            has_changes,
            history_open,
        );

        let body = match &active {
            Tab::Preview(path) => {
                let view = self
                    .previews
                    .get(path)
                    .map(|v| v.clone().into_any_element())
                    .unwrap_or_else(|| div().into_any_element());
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex().flex_col().flex_1().min_h_0().child(view))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .justify_center()
                            .px(sp(4))
                            .pb(sp(2))
                            .child(div().w_full().max_w(px(layout::CONTENT_MAX_WIDTH)).child(
                                context_bar(
                                    &self.app,
                                    &self.project,
                                    Some(&self.chat),
                                    &self.context,
                                    cx,
                                ),
                            )),
                    )
                    .into_any_element()
            }
            Tab::Chat => self.chat_panel.clone().into_any_element(),
            Tab::Diff => self.diff.clone().into_any_element(),
            Tab::Inspect => self.inspect.clone().into_any_element(),
        };

        let tab_bar = div()
            .id("center-tabs")
            .flex()
            .flex_none()
            .h(px(TABS_H))
            .border_b_1()
            .border_color(hsla(c.border))
            .bg(hsla(c.surface_000))
            .type_style(contextual_theme::type_style::UI_BODY)
            .child(
                div()
                    .id("tab-strip")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_scroll()
                    .child(chat_tab)
                    .children(open_tabs),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(sp(1))
                    .px(sp(2))
                    .child(
                        Button::new("chats", if history_open { "Hide history" } else { "Chat history" })
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|t, _, _, cx| {
                                if matches!(t.project.read(cx).active, Tab::Diff | Tab::Inspect) {
                                    t.project.update(cx, |p, cx| {
                                        p.activate(Tab::Chat);
                                        cx.notify();
                                    });
                                    t.chats.update(cx, |m, cx| {
                                        m.open = true;
                                        cx.notify();
                                    });
                                } else {
                                    t.chats.update(cx, |m, cx| m.toggle(cx));
                                }
                            })),
                    ),
            );

        let main = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .bg(hsla(c.surface_100))
            .child(tab_bar)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .when(!feature_view, |d| d.flex_none().w(px(panes.width)))
                            .child(body),
                    )
                    .when(!feature_view, |d| {
                        d.child(
                            div()
                                .id("workspace-right-dock")
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .min_h_0()
                                .h_full()
                                .justify_end()
                                .when(panes.show_changes, |d| d.child(self.changes.clone()))
                                .when(panes.show_history, |d| d.child(self.chats.clone())),
                        )
                    }),
            );

        div()
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(self.sidebar.clone())
            .child(main)
            .when_some(toast(&self.app, &self.chat, cx), |d, t| d.child(t))
            .when_some(self.save_dialog.clone(), |d, s| d.child(s))
            .when_some(self.pr_dialog.clone(), |d, s| d.child(s))
            .when_some(self.branch_picker.clone(), |d, s| d.child(s))
            .when_some(
                self.chat_panel.update(cx, |panel, cx| panel.settings_prompt(cx)),
                |d, prompt| d.child(prompt),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_chat_width() -> f32 {
        layout::CONTENT_MAX_WIDTH + 2.0 * contextual_theme::space::S4
    }

    #[test]
    fn panel_visibility_never_changes_chat_width() {
        for available in [
            full_chat_width(),
            full_chat_width() + layout::CHANGES_WIDTH,
            full_chat_width() + 2.0 * layout::CHANGES_WIDTH,
            full_chat_width() + 3.0 * layout::CHANGES_WIDTH,
        ] {
            let expected = chat_layout(available, false, false).width;
            for has_changes in [false, true] {
                for history_open in [false, true] {
                    assert_eq!(
                        chat_layout(available, has_changes, history_open).width,
                        expected,
                    );
                }
            }
        }
    }

    #[test]
    fn both_panels_require_two_full_sidebar_widths_beside_chat() {
        let threshold = full_chat_width() + 2.0 * layout::CHANGES_WIDTH;

        let below = chat_layout(threshold - 1.0, true, true);
        assert!(!below.show_changes);
        assert!(below.show_history);
        assert_eq!(below.width, full_chat_width());

        for available in [threshold, threshold + layout::CHANGES_WIDTH] {
            let panes = chat_layout(available, true, true);
            assert!(panes.show_changes);
            assert!(panes.show_history);
            assert_eq!(panes.width, full_chat_width());
        }
    }

    #[test]
    fn narrow_layout_reserves_the_same_sidebar_when_empty() {
        let available = full_chat_width();
        for has_changes in [false, true] {
            for history_open in [false, true] {
                let panes = chat_layout(available, has_changes, history_open);
                assert_eq!(panes.width, available - layout::CHANGES_WIDTH);
                assert_eq!(panes.show_changes, has_changes && !history_open);
                assert_eq!(panes.show_history, history_open);
            }
        }
    }
}
