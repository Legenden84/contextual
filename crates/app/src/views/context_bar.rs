//! Context budget above the input. Cache accounting separately from rendering: streaming
//! deltas do not change the next request's stored conversation, selection or settings.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use contextual_core::logic::context::{
    ContextInput, Level, Totals, content_tokens, totals_with_conversation_tokens,
};
use contextual_core::logic::format::{fmt_k, fmt_n};
use contextual_core::logic::types::Deployment;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{App, Entity, IntoElement, ParentElement, SharedString, Styled, div, relative};

use crate::state::chat::ChatModel;
use crate::state::{AppModel, Project};
use crate::theme::{ActiveTheme, TypeExt, hsla, px, sp};

const METER_W: f32 = 140.0;
const METER_H: f32 = 4.0;

/// Each owning view keeps one entry. Keys contain paths and settings, never file contents
/// or transcript text. They are copied only on a miss.
#[derive(Default)]
pub struct ContextCache(RefCell<Option<(BudgetKey, Totals)>>);

struct BudgetKey {
    rescans: u64,
    selected: HashSet<String>,
    rules_on: HashSet<String>,
    skills_never: HashSet<String>,
    context_revision: Option<u64>,
    pictures: u64,
    deployment: Deployment,
    output_format: String,
}

impl BudgetKey {
    fn matches(
        &self,
        a: &AppModel,
        p: &Project,
        chat: Option<&ChatModel>,
        deployment: &Deployment,
    ) -> bool {
        self.rescans == p.rescans
            && self.selected == p.selected
            && self.rules_on == p.rules_on
            && self.skills_never == p.skills_never
            && self.context_revision == chat.map(|c| c.context_revision)
            && self.pictures == picture_tokens(chat)
            && self.deployment == *deployment
            && self.output_format == a.settings.workspace.output_prompt
    }
}

fn picture_tokens(chat: Option<&ChatModel>) -> u64 {
    chat.map(|c| c.attachments.iter().map(|a| a.tokens).sum())
        .unwrap_or(0)
}

impl ContextCache {
    fn get(
        &self,
        a: &AppModel,
        p: &Project,
        chat: Option<&ChatModel>,
        deployment: &Deployment,
    ) -> Totals {
        if let Some((key, totals)) = self.0.borrow().as_ref()
            && key.matches(a, p, chat, deployment)
        {
            return totals.clone();
        }

        let key = BudgetKey {
            rescans: p.rescans,
            selected: p.selected.clone(),
            rules_on: p.rules_on.clone(),
            skills_never: p.skills_never.clone(),
            context_revision: chat.map(|c| c.context_revision),
            pictures: picture_tokens(chat),
            deployment: deployment.clone(),
            output_format: a.settings.workspace.output_prompt.clone(),
        };
        let totals = compute_totals(a, p, chat, deployment);
        *self.0.borrow_mut() = Some((key, totals.clone()));
        totals
    }
}

fn compute_totals(
    a: &AppModel,
    p: &Project,
    chat: Option<&ChatModel>,
    deployment: &Deployment,
) -> Totals {
    let rules_on = p.rules_on();
    // Borrow each stored message. In particular, never clone image data URLs for this meter.
    let conversation_tokens = chat
        .map(|c| {
            c.thread
                .transcript
                .iter()
                .map(|m| content_tokens(&m.content))
                .sum()
        })
        .unwrap_or(0);
    let versions: HashMap<String, String> = chat
        .map(|c| {
            c.thread
                .file_versions
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    // Accounting must not depend on the explorer's current filter.
    let stale: HashSet<String> = p
        .selected
        .iter()
        .filter(|path| {
            contextual_core::logic::tree::find_node(p.tree(), path)
                .is_some_and(|n| n.changed_on_disk == Some(true))
        })
        .cloned()
        .collect();
    let auto_tokens = {
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
    };
    totals_with_conversation_tokens(
        &ContextInput {
            tree: p.tree(),
            selected: &p.selected,
            rules_on: &rules_on,
            output_format: &a.settings.workspace.output_prompt,
            transcript: &[],
            file_versions: &versions,
            stale: &stale,
            resend_all: false,
            picture_tokens: picture_tokens(chat),
            auto_tokens,
            deployment,
        },
        conversation_tokens,
    )
}

pub fn context_bar(
    app: &Entity<AppModel>,
    project: &Entity<Project>,
    chat: Option<&Entity<ChatModel>>,
    cache: &ContextCache,
    cx: &App,
) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    let a = app.read(cx);
    let p = project.read(cx);
    let deployment = a.deployment();
    let chat = chat.map(|c| c.read(cx));
    let t = cache.get(a, p, chat, &deployment);
    let (fill, ink) = match t.level {
        Level::Ok => (c.accent, c.ink_muted),
        Level::Warn => (c.warning, c.warning),
        Level::Danger => (c.danger, c.danger),
    };
    let pct = (t.share * 100.0).round() as u64;
    let priced = deployment.priced();

    div()
        .flex()
        .flex_wrap()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(sp(2))
        .py(sp(1))
        .type_style(contextual_theme::type_style::UI_SMALL)
        .text_color(hsla(c.ink_muted))
        .child(Icon::new(IconName::File).size(px(14.0)))
        .child(
            div()
                .flex()
                .flex_wrap()
                .min_w_0()
                .gap(sp(1))
                .child(
                    div()
                        .text_color(hsla(c.ink))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .child(SharedString::from(format!("{} files", t.files))),
                )
                .child(SharedString::from(format!("· {} rules", t.rules)))
                .child(
                    div()
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .child(SharedString::from(format!("· ~{} tokens", fmt_n(t.tokens)))),
                ),
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
                        .whitespace_normal()
                        .child(SharedString::from(h)),
                )
            })
        })
        .child(
            div()
                .min_w_0()
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .whitespace_normal()
                .child(SharedString::from(if priced && t.long_context {
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
