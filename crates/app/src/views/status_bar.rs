//! 24px status bar: branch, Foundry connection, the read-only promise, scanning, Settings.

use contextual_core::logic::foundry_status::{FoundryStatus, Tone, describe_foundry};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};

use crate::state::{AppModel, Screen};
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};

const H: f32 = 24.0;
const ICON: f32 = 14.0;

fn item(icon: impl Into<Icon>, text: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap(sp(1))
        .whitespace_nowrap()
        .child(Icon::new(icon).size(px(ICON)))
        .child(text.into())
}

pub fn status_bar(app: &Entity<AppModel>, cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    let a = app.read(cx);
    let label = describe_foundry(&a.foundry);
    let tone = match label.tone {
        Tone::Ok => c.success,
        Tone::Warn => c.warning,
        _ => c.ink_muted,
    };
    let foundry_text = match &a.foundry {
        FoundryStatus::Ready { .. } => format!("Foundry · {}", a.deployment().name),
        _ => label.text.clone(),
    };
    let foundry = div()
        .flex()
        .items_center()
        .gap(sp(1))
        .whitespace_nowrap()
        .child(
            div()
                .text_color(hsla(tone))
                .child(Icon::new(IconName::Globe).size(px(ICON))),
        )
        .child(SharedString::from(foundry_text));
    // From git status when known (it follows switches); else the scan's branch.
    let scm = a.scm.clone();
    let status = scm.as_ref().and_then(|s| s.read(cx).status.clone());
    let branch = a.project.as_ref().map(|p| {
        let name = status
            .as_ref()
            .map(|st| st.branch.clone().unwrap_or_else(|| "detached".into()));
        let mut label = name
            .or_else(|| p.read(cx).branch.clone())
            .unwrap_or_else(|| "no git".into());
        if let Some(st) = status.as_ref().filter(|st| st.upstream.is_some()) {
            if st.behind > 0 {
                label.push_str(&format!(" {}↓", st.behind));
            }
            if st.ahead > 0 {
                label.push_str(&format!(" {}↑", st.ahead));
            }
        }
        label
    });
    let app_for_settings = app.clone();

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(sp(4))
        .h(px(H))
        .px(sp(3))
        .border_t_1()
        .border_color(hsla(c.border))
        .bg(hsla(c.surface_000))
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(c.ink_muted))
        .map(|d| match a.screen {
            Screen::Welcome => d.child(foundry).child(div().flex_1()).child("No project"),
            _ => d
                .when_some(branch, |d, b| {
                    d.child(
                        div()
                            .id("status-branch")
                            .cursor_pointer()
                            .hover(|s| s.text_color(hsla(c.ink)))
                            .when_some(scm.clone(), |d, scm| {
                                d.on_click(move |_, _, cx| {
                                    scm.update(cx, |s, cx| s.open_branch_picker(cx))
                                })
                            })
                            .child(item(gpui_kit::assets::IconName::GitBranch, b)),
                    )
                })
                .when_some(
                    a.project
                        .as_ref()
                        .map(|p| p.read(cx).location.clone())
                        .filter(|l| *l != contextual_core::location::Location::Local),
                    |d, l| {
                        d.child(
                            div()
                                .px(sp(1))
                                .rounded(px(contextual_theme::radius::SM))
                                .bg(hsla(c.surface_300))
                                .text_color(hsla(c.ink))
                                .child(SharedString::from(l.label())),
                        )
                    },
                )
                .child(foundry)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(sp(1))
                        .child(
                            div()
                                .text_color(hsla(c.success))
                                .child(Icon::new(IconName::Check).size(px(ICON))),
                        )
                        .child("Read-only: nothing is written until you save"),
                )
                .child(div().flex_1())
                .when(a.scanning, |d| d.child(item(IconName::Loader, "Scanning…")))
                .when(a.screen != Screen::Settings, |d| {
                    d.child(
                        div()
                            .id("status-settings")
                            .cursor_pointer()
                            .hover(|s| s.text_color(hsla(c.ink)))
                            .on_click(move |_, _, cx| {
                                app_for_settings.update(cx, |a, cx| a.open_settings(cx))
                            })
                            .child(item(IconName::Settings, "Settings")),
                    )
                }),
        })
}
