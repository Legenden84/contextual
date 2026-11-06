//! Workspace (design spec: ScreenWorkspace): sidebar, centre tabs (Chat plus read-only previews)
//! with the chat and composer, and the Changes panel when a reply proposes changes. The diff view
//! replaces the centre and right columns.

use std::collections::HashMap;

use contextual_theme::layout;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::state::chat::{CenterView, ChatModel};
use crate::state::{AppModel, Project, Tab};
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, Tag, file_glyph};
use crate::views::changes_panel::ChangesPanel;
use crate::views::chat_panel::ChatPanel;
use crate::views::chats_menu::ChatsMenu;
use crate::views::code_preview::CodePreview;
use crate::views::context_bar::context_bar;
use crate::views::diff_cache::DiffCache;
use crate::views::diff_screen::DiffScreen;
use crate::views::save_dialog::SaveDialog;
use crate::views::sidebar::Sidebar;
use crate::views::toast::toast;

/// Centre tab bar height.
const TABS_H: f32 = 36.0;

pub struct WorkspaceScreen {
    app: Entity<AppModel>,
    project: Entity<Project>,
    sidebar: Entity<Sidebar>,
    previews: HashMap<String, Entity<CodePreview>>,
    chat: Entity<ChatModel>,
    chat_panel: Entity<ChatPanel>,
    changes: Entity<ChangesPanel>,
    diff: Entity<DiffScreen>,
    chats: Entity<ChatsMenu>,
    save_dialog: Option<Entity<SaveDialog>>,
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
                // Create previews for new tabs, drop closed ones.
                let tabs = project.read(cx).tabs.clone();
                this.previews.retain(|p, _| tabs.contains(p));
                for t in tabs {
                    if !this.previews.contains_key(&t) {
                        let host = this.app.read(cx).host();
                        let view = cx.new(|cx| {
                            CodePreview::new(t.clone(), project.clone(), host, window, cx)
                        });
                        this.previews.insert(t, view);
                    }
                }
                cx.notify();
            }),
            cx.observe(&app, |_, _, cx| cx.notify()),
        ];
        let host = app.read(cx).host();
        let chat = cx.new(|cx| ChatModel::new(project.clone(), host, cx));
        let diffs = DiffCache::default();
        let mut subs = subs;
        subs.push(
            cx.observe_in(&chat, window, |this: &mut Self, chat, window, cx| {
                // The Save dialog lives while the model says it's open.
                let open = chat.read(cx).save_dialog_open;
                if open && this.save_dialog.is_none() {
                    let (app, chat) = (this.app.clone(), chat.clone());
                    this.save_dialog = Some(cx.new(|cx| SaveDialog::new(app, chat, window, cx)));
                } else if !open {
                    this.save_dialog = None;
                }
                cx.notify();
            }),
        );
        let chat_panel =
            cx.new(|cx| ChatPanel::new(app.clone(), project.clone(), chat.clone(), window, cx));
        let chats = cx.new(|cx| ChatsMenu::new(chat.clone(), window, cx));
        crate::dev::install(&app, &project, &chat, &chat_panel, &chats, window, cx);
        Self {
            sidebar: cx.new(|cx| Sidebar::new(app.clone(), project.clone(), window, cx)),
            chat_panel,
            changes: cx.new(|cx| ChangesPanel::new(chat.clone(), diffs.clone(), cx)),
            diff: cx.new(|cx| DiffScreen::new(chat.clone(), diffs, cx)),
            chats,
            save_dialog: None,
            chat,
            app,
            project,
            previews: HashMap::new(),
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
            .items_center()
            .gap(px(6.0))
            .px(sp(3))
            .h_full()
            .border_r_1()
            .border_color(hsla(c.border))
            .cursor_pointer()
            .text_color(hsla(if active { c.ink_strong } else { c.ink_muted }))
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let p = self.project.read(cx);
        let active = p.active.clone();
        let tabs = p.tabs.clone();

        let chat_tab = self
            .tab("tab-chat".into(), active == Tab::Chat, cx)
            .on_click(cx.listener(|t, _, _, cx| {
                t.project.update(cx, |p, cx| {
                    p.activate(Tab::Chat);
                    cx.notify()
                })
            }))
            .child(
                div()
                    .text_color(hsla(c.accent_ink))
                    .child(Icon::new(IconName::Bot).size(px(14.0))),
            )
            .child("Chat");
        let preview_tabs: Vec<_> = tabs
            .iter()
            .map(|path| {
                let name = path.rsplit('/').next().unwrap_or(path).to_string();
                let (pa, pb) = (path.clone(), path.clone());
                self.tab(
                    SharedString::from(format!("tab:{path}")),
                    active == Tab::Preview(path.clone()),
                    cx,
                )
                .on_click(cx.listener(move |t, _, _, cx| {
                    t.project.update(cx, |p, cx| {
                        p.activate(Tab::Preview(pa.clone()));
                        cx.notify()
                    })
                }))
                .child(file_glyph(&name, cx))
                .child(SharedString::from(name))
                .child(Tag::new("read-only"))
                .child(
                    div()
                        .id(SharedString::from(format!("close:{path}")))
                        .rounded(px(contextual_theme::radius::SM))
                        .text_color(hsla(c.ink_muted))
                        .hover(|s| s.bg(hsla(c.surface_hover)).text_color(hsla(c.ink)))
                        .on_click(cx.listener(move |t, _, _, cx| {
                            cx.stop_propagation();
                            t.project.update(cx, |p, cx| {
                                p.close_tab(&pb);
                                cx.notify()
                            })
                        }))
                        .child(Icon::new(IconName::Close).size(px(14.0))),
                )
            })
            .collect();

        let chat = self.chat.read(cx);
        let has_changes = !chat.changes().is_empty();
        let diff_view = chat.center == CenterView::Diff && has_changes;

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
                                context_bar(&self.app, &self.project, Some(&self.chat), cx),
                            )),
                    )
                    .into_any_element()
            }
            Tab::Chat => self.chat_panel.clone().into_any_element(),
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
                    .children(preview_tabs),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(sp(1))
                    .px(sp(2))
                    .child(
                        Button::new("chats", "Chats")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .on_click(
                                cx.listener(|t, _, _, cx| t.chats.update(cx, |m, cx| m.toggle(cx))),
                            ),
                    ),
            );

        let main = if diff_view {
            div().flex().flex_1().min_w_0().child(self.diff.clone())
        } else {
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .bg(hsla(c.surface_100))
                        .child(tab_bar)
                        .child(body)
                        .child(
                            div()
                                .absolute()
                                .top(px(TABS_H))
                                .right_0()
                                .child(self.chats.clone()),
                        ),
                )
                .when(has_changes, |d| d.child(self.changes.clone()))
        };

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
    }
}
