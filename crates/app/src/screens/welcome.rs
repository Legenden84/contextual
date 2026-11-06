//! Welcome (design spec: ScreenWelcome): Open folder, four steps, the Foundry connection and
//! recent projects.

use contextual_core::logic::foundry_status::{FoundryStatus, Tone, describe_foundry};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::state::AppModel;
use crate::theme::{ActiveTheme, MONO, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonVariant, LinkButton, folder_glyph};

const WIDTH: f32 = 860.0;
const LEFT: f32 = 300.0;
const GAP: f32 = 56.0;
const STEP_BADGE: f32 = 20.0;

const STEPS: [&str; 4] = [
    "Tick the files the model should see",
    "Ask for a change in chat",
    "Review the diff, file by file",
    "Save what you want. The model never writes to disk.",
];

/// "just now", "5 min ago", "3 hours ago", "Yesterday", "4 days ago", "12 Oct".
pub fn ago(rfc3339: &str) -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    let Ok(t) = OffsetDateTime::parse(rfc3339, &Rfc3339) else {
        return String::new();
    };
    let mins = ((OffsetDateTime::now_utc() - t).whole_seconds() as f64 / 60.0).round() as i64;
    if mins < 1 {
        return "just now".into();
    }
    if mins < 60 {
        return format!("{mins} min ago");
    }
    let hours = (mins as f64 / 60.0).round() as i64;
    if hours < 24 {
        return format!(
            "{hours} {}",
            if hours == 1 { "hour ago" } else { "hours ago" }
        );
    }
    let days = (hours as f64 / 24.0).round() as i64;
    match days {
        1 => "Yesterday".into(),
        d if d < 7 => format!("{d} days ago"),
        _ => format!("{} {}", t.day(), &format!("{:?}", t.month())[..3]),
    }
}

pub struct WelcomeScreen {
    app: Entity<AppModel>,
    _sub: Subscription,
}

impl WelcomeScreen {
    pub fn new(app: Entity<AppModel>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&app, |_, _, cx| cx.notify());
        Self { app, _sub: sub }
    }
}

fn caption(text: &'static str, cx: &App) -> impl IntoElement + use<> {
    div()
        .mb(sp(3))
        .type_style(contextual_theme::type_style::UI_CAPTION)
        .text_color(hsla(cx.tokens().colors.ink_muted))
        .child(text)
}

impl Render for WelcomeScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let a = self.app.read(cx);
        let label = describe_foundry(&a.foundry);
        let tone = match label.tone {
            Tone::Ok => c.success,
            Tone::Warn => c.warning,
            _ => c.ink_muted,
        };
        let foundry_text = match &a.foundry {
            FoundryStatus::Ready { health } => format!("Foundry · {}", health.deployment),
            _ => label.text.clone(),
        };
        let recents = a.state.recents.clone();
        let error = a.error.clone();
        let opening = a.scanning;
        let app = self.app.clone();

        let steps: Vec<_> = STEPS
            .iter()
            .enumerate()
            .map(|(i, s)| {
                div()
                    .flex()
                    .items_center()
                    .gap(sp(3))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(STEP_BADGE))
                            .rounded_full()
                            .border_1()
                            .border_color(hsla(c.border_strong))
                            .font_family(MONO)
                            .text_size(px(11.0))
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from((i + 1).to_string())),
                    )
                    .child(div().flex_1().min_w_0().child(*s))
            })
            .collect();
        let recent_rows = recents
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let path = r.path.clone();
                let location = r.location.clone();
                let app = app.clone();
                div()
                    .id(("recent", i))
                    .flex()
                    .items_center()
                    .gap(sp(3))
                    .px(sp(3))
                    .py(sp(2))
                    .when(i > 0, |d| d.border_t_1().border_color(hsla(c.border)))
                    .cursor_pointer()
                    .hover(|s| s.bg(hsla(c.surface_hover)))
                    .on_click(move |_, _, cx| {
                        app.update(cx, |a, cx| {
                            a.open_folder(path.clone(), Some(location.clone()), cx)
                        })
                    })
                    .child(folder_glyph(false, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                                    .text_color(hsla(c.ink_strong))
                                    .child(SharedString::from(r.name.clone())),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .type_style(contextual_theme::type_style::CODE_SMALL)
                                    .text_color(hsla(c.ink_muted))
                                    .child(SharedString::from(r.path.clone())),
                            ),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .flex()
                            .justify_end()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(ago(&r.opened_at))),
                    )
            })
            .collect::<Vec<_>>();
        let has_recents = !recents.is_empty();

        div()
            .id("welcome")
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .overflow_y_scroll()
            .p(sp(8))
            .bg(hsla(c.surface_100))
            .child(
                div()
                    .w(px(WIDTH))
                    .child(
                        div()
                            .mb(sp(8))
                            .child(
                                div()
                                    .type_style(contextual_theme::type_style::UI_DISPLAY)
                                    .text_color(hsla(c.ink_strong))
                                    .child("Contextual"),
                            )
                            .child(
                                div()
                                    .type_style(contextual_theme::type_style::UI_PROSE)
                                    .text_color(hsla(c.ink_muted))
                                    .child("Pick files, chat, review diffs, save what you want."),
                            ),
                    )
                    .when_some(error, |d, e| {
                        let app = app.clone();
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(sp(2))
                                .mb(sp(6))
                                .px(sp(3))
                                .py(sp(2))
                                .rounded(px(contextual_theme::radius::MD))
                                .border_1()
                                .border_color(hsla(c.warning_border))
                                .bg(hsla(c.warning_bg))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .child(
                                    div()
                                        .text_color(hsla(c.warning))
                                        .child(Icon::new(IconName::TriangleAlert).size(px(14.0))),
                                )
                                .child(div().flex_1().child(SharedString::from(e)))
                                .child(LinkButton::new("dismiss", "Dismiss", move |_, _, cx| {
                                    app.update(cx, |a, cx| a.dismiss_error(cx))
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap(px(GAP))
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(LEFT))
                                    .child(caption("START", cx))
                                    .child(
                                        div().flex().child(
                                            Button::new(
                                                "open",
                                                if opening {
                                                    "Opening…"
                                                } else {
                                                    "Open folder…"
                                                },
                                            )
                                            .variant(ButtonVariant::Primary)
                                            .disabled(opening)
                                            .on_click(
                                                {
                                                    let app = app.clone();
                                                    move |_, _, cx| {
                                                        app.update(cx, |a, cx| {
                                                            a.folder_dialog_open = true;
                                                            cx.notify();
                                                        })
                                                    }
                                                },
                                            ),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(sp(2))
                                            .my(sp(6))
                                            .children(steps),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(sp(2))
                                            .px(sp(3))
                                            .py(sp(2))
                                            .rounded(px(contextual_theme::radius::MD))
                                            .border_1()
                                            .border_color(hsla(c.border))
                                            .type_style(contextual_theme::type_style::UI_SMALL)
                                            .text_color(hsla(c.ink_muted))
                                            .child(
                                                div().text_color(hsla(tone)).child(
                                                    Icon::new(IconName::Globe).size(px(14.0)),
                                                ),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .whitespace_nowrap()
                                                    .child(SharedString::from(foundry_text)),
                                            )
                                            .child(LinkButton::new("settings", "Settings", {
                                                let app = app.clone();
                                                move |_, _, cx| {
                                                    app.update(cx, |a, cx| a.open_settings(cx))
                                                }
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(caption("RECENT", cx))
                                    .when(!has_recents, |d| {
                                        d.child(
                                            div()
                                                .px(sp(3))
                                                .py(sp(4))
                                                .rounded(px(contextual_theme::radius::MD))
                                                .border_1()
                                                .border_dashed()
                                                .border_color(hsla(c.border))
                                                .type_style(contextual_theme::type_style::UI_SMALL)
                                                .text_color(hsla(c.ink_muted))
                                                .child("Folders you open appear here."),
                                        )
                                    })
                                    .when(has_recents, |d| {
                                        d.child(
                                            div()
                                                .mb(sp(3))
                                                .overflow_hidden()
                                                .rounded(px(contextual_theme::radius::MD))
                                                .border_1()
                                                .border_color(hsla(c.border))
                                                .children(recent_rows),
                                        )
                                        .child(
                                            LinkButton::new(
                                                "clear-recents",
                                                "Clear recent",
                                                move |_, _, cx| {
                                                    app.update(cx, |a, cx| a.clear_recents(cx))
                                                },
                                            ),
                                        )
                                    }),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::ago;
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    fn before(secs: i64) -> String {
        (OffsetDateTime::now_utc() - time::Duration::seconds(secs))
            .format(&Rfc3339)
            .unwrap()
    }

    #[test]
    fn relative_times() {
        assert_eq!(ago(&before(10)), "just now");
        assert_eq!(ago(&before(5 * 60)), "5 min ago");
        assert_eq!(ago(&before(3600)), "1 hour ago");
        assert_eq!(ago(&before(3 * 3600)), "3 hours ago");
        assert_eq!(ago(&before(26 * 3600)), "Yesterday");
        assert_eq!(ago(&before(4 * 86400)), "4 days ago");
        assert_eq!(ago("garbage"), "");
    }
}
