//! Context budget above the input (design spec: ContextBar): files, rules, estimated tokens, a
//! meter against the deployment's limit and the cost per message. Numbers come from
//! `contextual_core::logic::context::totals`.

use std::collections::{HashMap, HashSet};

use contextual_core::logic::context::{ContextInput, Level, totals};
use contextual_core::logic::format::{fmt_k, fmt_n};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, Entity, IntoElement, ParentElement, SharedString, Styled, div, px, relative};

use crate::state::chat::ChatModel;
use crate::state::{AppModel, Project};
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use contextual_core::logic::prompt::ApiContent;

const BAR_H: f32 = 28.0;
const METER_W: f32 = 140.0;
const METER_H: f32 = 4.0;

pub fn context_bar(
    app: &Entity<AppModel>,
    project: &Entity<Project>,
    chat: Option<&Entity<ChatModel>>,
    cx: &App,
) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    let a = app.read(cx);
    let p = project.read(cx);
    let deployment = a.deployment();
    let rules_on = p.rules_on();
    let chat = chat.map(|c| c.read(cx));
    let transcript: Vec<ApiContent> = chat
        .map(|c| {
            c.thread
                .transcript
                .iter()
                .map(|m| m.content.clone())
                .collect()
        })
        .unwrap_or_default();
    let versions: HashMap<String, String> = chat
        .map(|c| {
            c.thread
                .file_versions
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    let stale: HashSet<String> = p
        .selected
        .iter()
        .filter(|s| p.node(s).is_some_and(|n| n.changed_on_disk == Some(true)))
        .cloned()
        .collect();
    let pictures: u64 = chat
        .map(|c| c.attachments.iter().map(|a| a.tokens).sum())
        .unwrap_or(0);
    let resend_all = chat.is_some_and(|c| c.resend_all);
    // Model picks files: the read protocol and skill index, and the tree until it is sent.
    let auto_tokens = if a.settings.chat.model_picks_files {
        use contextual_core::logic::format::estimate_tokens;
        use contextual_core::logic::reads;
        let tree_sent = chat.is_some_and(|c| {
            c.thread.tree_version.as_deref() == Some(p.model_tree_version.as_str())
        });
        estimate_tokens(reads::READ_FORMAT)
            + estimate_tokens(&p.skill_index())
            + if tree_sent {
                0
            } else {
                estimate_tokens(&reads::tree_section(&p.model_tree))
            }
    } else {
        0
    };
    let t = totals(&ContextInput {
        tree: p.tree(),
        selected: &p.selected,
        rules_on: &rules_on,
        output_format: &a.settings.workspace.output_prompt,
        transcript: &transcript,
        file_versions: &versions,
        stale: &stale,
        resend_all,
        picture_tokens: pictures,
        auto_tokens,
        deployment: &deployment,
    });
    let (fill, ink) = match t.level {
        Level::Ok => (c.accent, c.ink_muted),
        Level::Warn => (c.warning, c.warning),
        Level::Danger => (c.danger, c.danger),
    };
    let pct = (t.share * 100.0).round() as u64;
    let priced = deployment.priced();

    div()
        .flex()
        .items_center()
        .gap(sp(2))
        .h(px(BAR_H))
        .px(sp(1))
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(c.ink_muted))
        .child(Icon::new(IconName::File).size(px(14.0)))
        .child(
            div()
                .flex()
                .gap(sp(1))
                .whitespace_nowrap()
                .child(
                    div()
                        .text_color(hsla(c.ink))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .child(SharedString::from(format!("{} files", t.files))),
                )
                .child(SharedString::from(format!(
                    "· {} rules · ~{} tokens",
                    t.rules,
                    fmt_n(t.tokens)
                ))),
        )
        .child(
            div()
                .flex_none()
                .w(px(METER_W))
                .h(px(METER_H))
                .rounded_full()
                .bg(hsla(c.surface_300))
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .bg(hsla(fill))
                        .w(relative(t.share.min(1.0) as f32)),
                ),
        )
        .child(
            div()
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .text_color(hsla(ink))
                .whitespace_nowrap()
                .child(SharedString::from(format!(
                    "{pct}% of {}",
                    fmt_k(deployment.limit)
                ))),
        )
        .when(t.level != Level::Ok, |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(sp(1))
                    .text_color(hsla(ink))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child(Icon::new(IconName::TriangleAlert).size(px(14.0)))
                    .child(if t.level == Level::Warn {
                        "Near limit"
                    } else {
                        "Over limit"
                    }),
            )
            .when_some(t.hint.clone(), |d, h| {
                d.child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(SharedString::from(h)),
                )
            })
        })
        .child(div().flex_1())
        .child(
            div()
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .whitespace_nowrap()
                .child(SharedString::from(if priced && t.long_context {
                    // The whole request is billed at the higher rates past the threshold.
                    let above = deployment.long_context.as_ref().map_or(0, |l| l.above);
                    format!(
                        "~${:.3} / message (long-context rates, over {})",
                        t.cost,
                        fmt_k(above)
                    )
                } else if priced {
                    format!("~${:.3} / message", t.cost)
                } else {
                    "Cost: prices not set".into()
                })),
        )
}
