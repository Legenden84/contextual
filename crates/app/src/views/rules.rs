//! Skills & Rules (design spec: RulesPanel): detected instruction files, each with a Rule/Skill
//! tag, token count and a switch that includes it in the system instructions.

use contextual_core::logic::format::fmt_k;
use contextual_core::logic::types::RuleType;
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::state::Project;
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Switch, Tag, TagTone, count, file_glyph};

/// Lines shown when a rule is previewed.
const PREVIEW_LINES: usize = 9;
const ROW_H: f32 = 24.0;
const ROW_INDENT: f32 = 22.0;
/// Below the explorer, the section scrolls past this height so the tree keeps its room.
const MAX_H: f32 = 240.0;

pub struct RulesPanel {
    project: Entity<Project>,
    collapsed: bool,
    /// Fill the sidebar (Rules tab) instead of sitting below the explorer.
    fill: bool,
    _sub: Subscription,
}

impl RulesPanel {
    pub fn new(project: Entity<Project>, fill: bool, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&project, |_, _, cx| cx.notify());
        Self {
            project,
            collapsed: false,
            fill,
            _sub: sub,
        }
    }
}

/// Skills are all called SKILL.md: show the skill's folder name, then where it lives.
fn display_name(path: &str) -> (String, String) {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 && parts[parts.len() - 1].eq_ignore_ascii_case("SKILL.md") {
        let name = parts[parts.len() - 2].to_string();
        let dir = parts[..parts.len() - 2].join("/");
        return (
            name,
            if dir.is_empty() {
                dir
            } else {
                format!("{dir}/")
            },
        );
    }
    match path.rsplit_once('/') {
        Some((dir, name)) => (name.to_string(), format!("{dir}/")),
        None => (path.to_string(), String::new()),
    }
}

impl Render for RulesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let p = self.project.read(cx);
        let on_count = p
            .rules
            .iter()
            .filter(|r| p.rules_on.contains(&r.path))
            .count();
        let preview = p
            .rule_preview
            .as_ref()
            .and_then(|path| p.rules.iter().find(|r| &r.path == path))
            .map(|r| {
                r.content
                    .split('\n')
                    .take(PREVIEW_LINES)
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        let rows: Vec<_> = p
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let on = p.rules_on.contains(&r.path);
                let previewing = p.rule_preview.as_deref() == Some(r.path.as_str());
                let (name, dir) = display_name(&r.path);
                let (pa, pb) = (r.path.clone(), r.path.clone());
                div()
                    .id(("rule", i))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(ROW_H))
                    .pl(px(ROW_INDENT))
                    .pr(sp(3))
                    .cursor_pointer()
                    .when(previewing, |d| d.bg(hsla(c.surface_selected)))
                    .when(!previewing, |d| d.hover(|s| s.bg(hsla(c.surface_hover))))
                    .on_click(cx.listener(move |t, _, _, cx| {
                        t.project.update(cx, |p, cx| {
                            p.preview_rule(&pa);
                            cx.notify()
                        })
                    }))
                    .child(file_glyph(&r.path, cx))
                    .child(
                        div()
                            .flex()
                            .gap(sp(1))
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .text_color(hsla(if on { c.ink } else { c.ink_muted }))
                            .child(SharedString::from(name))
                            .child(
                                div()
                                    .text_color(hsla(c.ink_muted))
                                    .child(SharedString::from(dir)),
                            ),
                    )
                    .child(
                        Tag::new(if r.kind == RuleType::Skill {
                            "Skill"
                        } else {
                            "Rule"
                        })
                        .tone(if r.kind == RuleType::Skill {
                            TagTone::Skill
                        } else {
                            TagTone::Default
                        }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(fmt_k(r.tokens))),
                    )
                    .child(Switch::new(("rule-on", i), on).on_toggle(cx.listener(
                        move |t, _, _, cx| {
                            t.project.update(cx, |p, cx| {
                                p.toggle_rule(&pb);
                                cx.notify()
                            })
                        },
                    )))
            })
            .collect();
        let empty = rows.is_empty();
        let fill = self.fill;

        div()
            .flex()
            .flex_col()
            .pb(sp(2))
            .when(self.fill, |d| d.flex_1().min_h_0())
            .when(!self.fill, |d| d.flex_none().border_t_1().border_color(hsla(c.border)))
            .child(
                div()
                    .id("rules-header")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .h(sp(8))
                    .pl(px(6.0))
                    .pr(sp(2))
                    .cursor_pointer()
                    .on_click(cx.listener(|t, _, _, cx| {
                        t.collapsed = !t.collapsed;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_color(hsla(c.ink_muted))
                            .child(Icon::new(if self.collapsed { IconName::ChevronRight } else { IconName::ChevronDown }).size(px(14.0))),
                    )
                    .child(div().type_style(contextual_theme::type_style::UI_CAPTION).text_color(hsla(c.ink_muted)).child("SKILLS & RULES"))
                    .child(count(on_count, cx)),
            )
            .when(!self.collapsed, |d| {
                d.child(
                div()
                    .id("rules-body")
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .map(|b| if fill { b.flex_1().min_h_0() } else { b.max_h(px(MAX_H)) })
                    .children(rows)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .pl(px(ROW_INDENT))
                                .pr(sp(3))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child("No instruction files found."),
                        )
                    })
                    .when_some(preview, |d, text| {
                        d.child(
                            div()
                                .mt(sp(1))
                                .mr(sp(3))
                                .mb(sp(2))
                                .ml(px(ROW_INDENT))
                                .p(sp(2))
                                .rounded(px(contextual_theme::radius::MD))
                                .border_1()
                                .border_color(hsla(c.border))
                                .bg(hsla(c.surface_100))
                                .type_style(contextual_theme::type_style::CODE_SMALL)
                                .text_color(hsla(c.syn_plain))
                                .whitespace_normal()
                                .child(SharedString::from(text)),
                        )
                    })
                    .child(
                        div()
                            .pt(sp(1))
                            .pl(px(ROW_INDENT))
                            .pr(sp(3))
                            .text_size(px(11.0))
                            .text_color(hsla(c.ink_muted))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child("Detected: AGENTS.md, CLAUDE.md, .github, .cursor/rules, .claude/skills"),
                    )
            )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::display_name;

    #[test]
    fn skill_and_rule_names() {
        assert_eq!(
            display_name(".claude/skills/shadcn/SKILL.md"),
            ("shadcn".into(), ".claude/skills/".into())
        );
        assert_eq!(
            display_name("tools/skill.md"),
            ("tools".into(), String::new())
        );
        assert_eq!(
            display_name("AGENTS.md"),
            ("AGENTS.md".into(), String::new())
        );
        assert_eq!(
            display_name(".cursor/rules/a.mdc"),
            ("a.mdc".into(), ".cursor/rules/".into())
        );
    }
}
