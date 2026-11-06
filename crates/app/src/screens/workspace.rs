//! Workspace (design spec: ScreenWorkspace): sidebar, centre tabs (Chat plus read-only previews)
//! and the composer area with the context bar. Chat and the Changes panel arrive in Phase 4.

use std::collections::HashMap;

use contextual_theme::layout;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::state::{AppModel, Project, Tab};
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Tag, file_glyph};
use crate::views::code_preview::CodePreview;
use crate::views::context_bar::context_bar;
use crate::views::sidebar::Sidebar;

/// Centre tab bar height.
const TABS_H: f32 = 36.0;

pub struct WorkspaceScreen {
    app: Entity<AppModel>,
    project: Entity<Project>,
    sidebar: Entity<Sidebar>,
    previews: HashMap<String, Entity<CodePreview>>,
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
        Self {
            sidebar: cx.new(|cx| Sidebar::new(app.clone(), project.clone(), window, cx)),
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

        let body = match &active {
            Tab::Preview(path) => match self.previews.get(path) {
                Some(view) => div().flex().flex_col().flex_1().min_h_0().child(view.clone()),
                None => div().flex_1(),
            },
            Tab::Chat => div()
                .flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(sp(2))
                        .max_w(px(layout::CONTENT_MAX_WIDTH))
                        .type_style(contextual_theme::type_style::UI_PROSE)
                        .text_color(hsla(c.ink_muted))
                        .child(div().text_color(hsla(c.ink_strong)).type_style(contextual_theme::type_style::UI_TITLE).child("Pick the files the model should see"))
                        .child("Tick files in the explorer, then ask for a change. Click a file to preview it."),
                ),
        };

        div()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(self.sidebar.clone())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .bg(hsla(c.surface_100))
                    .child(
                        div()
                            .id("center-tabs")
                            .flex()
                            .flex_none()
                            .h(px(TABS_H))
                            .border_b_1()
                            .border_color(hsla(c.border))
                            .bg(hsla(c.surface_000))
                            .type_style(contextual_theme::type_style::UI_BODY)
                            .overflow_x_scroll()
                            .child(chat_tab)
                            .children(preview_tabs),
                    )
                    .child(body)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .items_center()
                            .px(sp(4))
                            .pb(sp(3))
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(layout::CONTENT_MAX_WIDTH))
                                    .child(context_bar(&self.app, &self.project, cx))
                                    .child(
                                        div()
                                            .px(sp(3))
                                            .py(sp(3))
                                            .rounded(px(contextual_theme::radius::MD))
                                            .border_1()
                                            .border_color(hsla(c.border_strong))
                                            .bg(hsla(c.surface_200))
                                            .type_style(contextual_theme::type_style::UI_BODY)
                                            .text_color(hsla(c.ink_faint))
                                            .child("Chat arrives in the next phase."),
                                    ),
                            ),
                    ),
            )
    }
}
