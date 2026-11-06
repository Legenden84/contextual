//! Left column (312px): Files (explorer + Skills & Rules) or Rules on its own.
//! Source Control joins as a third tab in Phase 5.

use contextual_theme::layout;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::state::{AppModel, Project};
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::count;
use crate::views::explorer::Explorer;
use crate::views::rules::RulesPanel;

/// Sidebar tab bar: 30px tabs with a 2px underline.
const TAB_H: f32 = 30.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SideTab {
    Files,
    Rules,
}

pub struct Sidebar {
    project: Entity<Project>,
    tab: SideTab,
    explorer: Entity<Explorer>,
    rules_below: Entity<RulesPanel>,
    rules_full: Entity<RulesPanel>,
    _sub: Subscription,
}

impl Sidebar {
    pub fn new(
        app: Entity<AppModel>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            explorer: cx.new(|cx| Explorer::new(app, project.clone(), window, cx)),
            rules_below: cx.new(|cx| RulesPanel::new(project.clone(), false, cx)),
            rules_full: cx.new(|cx| RulesPanel::new(project.clone(), true, cx)),
            _sub: cx.observe(&project, |_, _, cx| cx.notify()),
            project,
            tab: SideTab::Files,
        }
    }

    fn tab_button(
        &self,
        id: &'static str,
        tab: SideTab,
        icon: IconName,
        label: &'static str,
        badge: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let active = self.tab == tab;
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(TAB_H))
            .px(sp(2))
            .border_b_2()
            .border_color(hsla(if active {
                c.accent
            } else {
                contextual_theme::Rgba { a: 0.0, ..c.accent }
            }))
            .type_style(contextual_theme::type_style::UI_SMALL)
            .text_color(hsla(if active { c.ink_strong } else { c.ink_muted }))
            .cursor_pointer()
            .on_click(cx.listener(move |t, _, _, cx| {
                t.tab = tab;
                cx.notify();
            }))
            .child(Icon::new(icon).size(px(14.0)))
            .child(SharedString::from(label))
            .when_some(badge, |d, n| d.child(count(n, cx)))
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let rules_on = self.project.read(cx).rules_on.len();
        let tabs = div()
            .flex()
            .flex_none()
            .gap(px(2.0))
            .px(sp(2))
            .pt(px(6.0))
            .border_b_1()
            .border_color(hsla(c.border))
            .child(self.tab_button(
                "tab-files",
                SideTab::Files,
                IconName::File,
                "Files",
                None,
                cx,
            ))
            .child(self.tab_button(
                "tab-rules",
                SideTab::Rules,
                IconName::BookOpen,
                "Rules",
                Some(rules_on),
                cx,
            ));
        div()
            .flex()
            .flex_col()
            .flex_none()
            .min_h_0()
            .w(px(layout::EXPLORER_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(hsla(c.border))
            .bg(hsla(c.surface_000))
            .child(tabs)
            .map(|d| match self.tab {
                SideTab::Files => d
                    .child(self.explorer.clone())
                    .child(self.rules_below.clone()),
                SideTab::Rules => d.child(self.rules_full.clone()),
            })
    }
}
