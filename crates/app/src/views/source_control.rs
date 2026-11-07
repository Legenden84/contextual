//! Source Control tab (VS Code-style): branch with ahead/behind, commit message (drafted by the
//! model on request), Commit / Commit all, Push / Publish, then staged and unstaged changes.
//! Clicking a file opens its diff in a tab.

use contextual_core::logic::git_tabs::git_tab_id;
use contextual_protocol::git::{ChangeLetter, StatusEntry};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::state::scm::ScmModel;
use crate::state::{AppModel, Project, Tab};
use crate::theme::{ActiveTheme, TypeExt, hsla, row_height, sp};
use crate::ui::{Button, ButtonVariant, IconButton, count, file_glyph};

fn letter_color(l: ChangeLetter, c: &contextual_theme::Colors) -> contextual_theme::Rgba {
    match l {
        ChangeLetter::M | ChangeLetter::R => c.modified,
        ChangeLetter::A | ChangeLetter::U => c.success,
        ChangeLetter::D | ChangeLetter::C => c.danger,
    }
}

fn letter(l: ChangeLetter) -> &'static str {
    match l {
        ChangeLetter::M => "M",
        ChangeLetter::A => "A",
        ChangeLetter::D => "D",
        ChangeLetter::R => "R",
        ChangeLetter::U => "U",
        ChangeLetter::C => "C",
    }
}

pub struct SourceControl {
    app: Entity<AppModel>,
    project: Entity<Project>,
    scm: Entity<ScmModel>,
    message: Entity<TextareaState>,
    staged_open: bool,
    changes_open: bool,
    _subs: Vec<Subscription>,
}

impl SourceControl {
    pub fn new(
        app: Entity<AppModel>,
        project: Entity<Project>,
        scm: Entity<ScmModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 10)
                .placeholder("Message (Ctrl+Enter to commit)")
        });
        let subs = vec![
            cx.observe(&scm, |_, _, cx| cx.notify()),
            cx.observe(&project, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &message,
                window,
                |this, _, e: &InputEvent, window, cx| match e {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.commit(window, cx),
                    _ => {}
                },
            ),
        ];
        Self {
            app,
            project,
            scm,
            message,
            staged_open: true,
            changes_open: true,
            _subs: subs,
        }
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.message.read(cx).value().trim().to_string();
        let s = self.scm.read(cx);
        let has = s
            .status
            .as_ref()
            .is_some_and(|st| !st.staged.is_empty() || !st.changes.is_empty());
        if text.is_empty() || !has || s.busy.is_some() {
            return;
        }
        self.message.update(cx, |m, cx| m.set_value("", window, cx));
        self.scm.update(cx, |s, cx| s.commit(text, cx));
    }

    fn row(
        &self,
        i: usize,
        e: &StatusEntry,
        staged: bool,
        active: bool,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        let (dir, name) = match e.path.rsplit_once('/') {
            Some((d, n)) => (d.to_string(), n.to_string()),
            None => (String::new(), e.path.clone()),
        };
        let (open_path, act_path) = (e.path.clone(), e.path.clone());
        let full_path = SharedString::from(e.path.clone());
        div()
            .id((if staged { "staged" } else { "change" }, i))
            .flex()
            .items_center()
            .gap(sp(1))
            .h(row_height())
            .pl(sp(6))
            .pr(sp(2))
            .type_style(contextual_theme::type_style::UI_SMALL)
            .cursor_pointer()
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(full_path.clone()).build(window, cx)
            })
            .when(active, |d| d.bg(hsla(c.surface_selected)))
            .when(!active, |d| d.hover(|s| s.bg(hsla(c.surface_hover))))
            .on_click(cx.listener(move |t, _, _, cx| {
                let tab = git_tab_id(&open_path, staged);
                t.project.update(cx, |p, cx| {
                    p.open_preview(&tab);
                    cx.notify()
                })
            }))
            .child(file_glyph(&name, cx))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .when(e.letter == ChangeLetter::D, |d| d.line_through())
                    .child(SharedString::from(name)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(SharedString::from(dir)),
            )
            .child(
                IconButton::new(
                    (if staged { "unstage" } else { "stage" }, i),
                    if staged {
                        IconName::Minus
                    } else {
                        IconName::Plus
                    },
                    if staged {
                        "Unstage changes"
                    } else {
                        "Stage changes"
                    },
                )
                .disabled(busy)
                .on_click(cx.listener(move |t, _, _, cx| {
                    cx.stop_propagation();
                    let p = vec![act_path.clone()];
                    t.scm.update(cx, |s, cx| {
                        if staged {
                            s.unstage(p, cx)
                        } else {
                            s.stage(p, cx)
                        }
                    })
                })),
            )
            .child(
                div()
                    .w(sp(3))
                    .flex_none()
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(hsla(letter_color(e.letter, c)))
                    .child(letter(e.letter)),
            )
    }

    fn section(
        &self,
        title: &'static str,
        staged: bool,
        entries: &[StatusEntry],
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        let open = if staged {
            self.staged_open
        } else {
            self.changes_open
        };
        let busy = self.scm.read(cx).busy.is_some();
        let active = match &self.project.read(cx).active {
            Tab::Preview(t) => Some(t.clone()),
            Tab::Chat => None,
        };
        let paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
        let rows: Vec<_> = if open {
            entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let is_active = active.as_deref() == Some(git_tab_id(&e.path, staged).as_str());
                    self.row(i, e, staged, is_active, busy, cx)
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        div().when(!entries.is_empty(), |d| {
            d.child(
                div()
                    .id(title)
                    .flex()
                    .items_center()
                    .gap(sp(1))
                    .h(row_height())
                    .pl(sp(1))
                    .pr(sp(2))
                    .cursor_pointer()
                    .bg(hsla(c.surface_100))
                    .hover(|s| s.bg(hsla(c.surface_hover)))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(hsla(c.ink))
                    .on_click(cx.listener(move |t, _, _, cx| {
                        if staged {
                            t.staged_open = !t.staged_open
                        } else {
                            t.changes_open = !t.changes_open
                        }
                        cx.notify();
                    }))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(sp(4)),
                    )
                    .child(div().flex_1().child(title))
                    .child(
                        IconButton::new(
                            if staged { "unstage-all" } else { "stage-all" },
                            if staged {
                                IconName::Minus
                            } else {
                                IconName::Plus
                            },
                            if staged {
                                "Unstage all changes"
                            } else {
                                "Stage all changes"
                            },
                        )
                        .disabled(busy)
                        .on_click(cx.listener(move |t, _, _, cx| {
                            cx.stop_propagation();
                            let p = paths.clone();
                            t.scm.update(cx, |s, cx| {
                                if staged {
                                    s.unstage(p, cx)
                                } else {
                                    s.stage(p, cx)
                                }
                            })
                        })),
                    )
                    .child(count(entries.len(), cx)),
            )
            .children(rows)
        })
    }
}

impl Render for SourceControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(text) = self.scm.update(cx, |s, _| s.drafted.take()) {
            self.message
                .update(cx, |m, cx| m.set_value(text, window, cx));
        }
        let c = &cx.tokens().colors;
        let s = self.scm.read(cx);
        let configured = self.app.read(cx).config.is_some();
        let has_changes = s
            .status
            .as_ref()
            .is_some_and(|st| !st.staged.is_empty() || !st.changes.is_empty());
        let can_draft = configured && s.busy.is_none() && has_changes;
        let draft_hint = if !configured {
            "Configure Foundry in Settings to draft a commit message"
        } else if s.busy == Some("Drafting") {
            "Drafting commit message…"
        } else if !has_changes {
            "No changes to draft a commit message for"
        } else {
            "Draft commit message with AI"
        };
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .h(sp(8))
            .pl(sp(3))
            .pr(sp(2))
            .child(
                div()
                    .flex_1()
                    .type_style(contextual_theme::type_style::UI_CAPTION)
                    .text_color(hsla(c.ink_muted))
                    .child("SOURCE CONTROL"),
            )
            .child(
                IconButton::new("draft", IconName::Bot, draft_hint)
                    .disabled(!can_draft)
                    .on_click(cx.listener(|t, _, _, cx| {
                        if let Some(cfg) = t.app.read(cx).config.clone() {
                            t.scm.update(cx, |s, cx| s.draft_message(cfg, cx));
                        }
                    })),
            )
            .child(
                IconButton::new("scm-refresh", IconName::Redo, "Refresh changes")
                    .disabled(s.busy.is_some())
                    .on_click(cx.listener(|t, _, _, cx| t.scm.update(cx, |s, cx| s.refresh(cx)))),
            );
        let Some(st) = s.status.clone() else {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .child(header)
                .child(
                    div()
                        .px(sp(3))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(if s.not_repo {
                            "This folder is not a git repository."
                        } else {
                            "Loading…"
                        }),
                )
                .into_any_element();
        };
        let busy = s.busy;
        let (error, done) = (s.error.clone(), s.done.clone());
        let all = st.staged.is_empty() && !st.changes.is_empty();
        let has_message = !self.message.read(cx).value().trim().is_empty();
        let can_commit =
            has_message && (!st.staged.is_empty() || !st.changes.is_empty()) && busy.is_none();
        let commit_label = match busy {
            Some("Commit") => "Committing…",
            Some("Drafting") => "Drafting…",
            _ if all => "Commit all",
            _ => "Commit",
        };
        let show_push = st.branch.is_some() && (st.ahead > 0 || st.upstream.is_none());
        let push_label = match (busy, &st.upstream) {
            (Some("Push"), _) => "Pushing…".to_string(),
            (_, Some(_)) => format!(
                "Push {} {}",
                st.ahead,
                if st.ahead == 1 { "commit" } else { "commits" }
            ),
            (_, None) => "Publish branch".into(),
        };
        let empty = st.staged.is_empty() && st.changes.is_empty() && st.conflicts.is_empty();
        let ahead_behind = (st.upstream.is_some() && (st.ahead > 0 || st.behind > 0)).then(|| {
            let mut s = String::new();
            if st.ahead > 0 {
                s.push_str(&format!("↑{}", st.ahead));
            }
            if st.behind > 0 {
                s.push_str(&format!(" ↓{}", st.behind));
            }
            s
        });

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .type_style(contextual_theme::type_style::UI_BODY)
            .text_color(hsla(c.ink))
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(sp(2))
                    .px(sp(3))
                    .pb(sp(3))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(
                                div()
                                    .id("scm-branch")
                                    .flex()
                                    .flex_1()
                                    .items_center()
                                    .gap(sp(1))
                                    .min_w_0()
                                    .h(row_height())
                                    .rounded(px(contextual_theme::radius::SM))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(hsla(c.surface_hover)).text_color(hsla(c.ink)))
                                    .on_click(cx.listener(|t, _, _, cx| {
                                        t.scm.update(cx, |s, cx| s.open_branch_picker(cx))
                                    }))
                                    .child(
                                        Icon::new(gpui_kit::assets::IconName::GitBranch)
                                            .size(sp(4)),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .type_style(contextual_theme::type_style::CODE_SMALL)
                                            .text_color(hsla(c.ink))
                                            .child(SharedString::from(
                                                st.branch
                                                    .clone()
                                                    .unwrap_or_else(|| "(detached)".into()),
                                            )),
                                    )
                                    .child(Icon::new(IconName::ChevronDown).size(sp(3))),
                            )
                            .when_some(ahead_behind, |d, ab| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .type_style(contextual_theme::type_style::CODE_SMALL)
                                        .child(SharedString::from(ab)),
                                )
                            }),
                    )
                    .child(Textarea::new(&self.message))
                    .child(
                        div().flex().flex_col().child(
                            Button::new("commit", commit_label)
                                .icon(IconName::Check)
                                .variant(ButtonVariant::Primary)
                                .disabled(!can_commit)
                                .on_click(cx.listener(|t, _, w, cx| t.commit(w, cx))),
                        ),
                    )
                    .when(all && busy.is_none(), |d| {
                        d.child(
                            div()
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child("Nothing staged. Commit all includes every change."),
                        )
                    })
                    .when(show_push, |d| {
                        d.child(
                            div().flex().flex_col().child(
                                Button::new("push", push_label)
                                    .icon(IconName::ArrowUp)
                                    .disabled(busy.is_some())
                                    .on_click(cx.listener(|t, _, _, cx| {
                                        t.scm.update(cx, |s, cx| s.push(cx))
                                    })),
                            ),
                        )
                    })
                    .when_some(error.clone(), |d, e| {
                        d.child(
                            div()
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.danger))
                                .child(SharedString::from(e)),
                        )
                    })
                    .when(error.is_none(), |d| {
                        d.when_some(done, |d, m| {
                            d.child(
                                div()
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .text_color(hsla(c.success))
                                    .child(SharedString::from(m)),
                            )
                        })
                    }),
            )
            .child(
                div()
                    .id("scm-lists")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pt(sp(1))
                    .pb(sp(2))
                    .border_t_1()
                    .border_color(hsla(c.border))
                    .when(!st.conflicts.is_empty(), |d| {
                        d.child(
                            div()
                                .mx(sp(3))
                                .my(sp(1))
                                .px(sp(2))
                                .py(sp(1))
                                .rounded(px(contextual_theme::radius::SM))
                                .border_1()
                                .border_color(hsla(c.warning_border))
                                .bg(hsla(c.warning_bg))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .child(SharedString::from(format!(
                                    "{} merge {}: resolve in your editor first.",
                                    st.conflicts.len(),
                                    if st.conflicts.len() == 1 {
                                        "conflict"
                                    } else {
                                        "conflicts"
                                    }
                                ))),
                        )
                    })
                    .child(self.section("Staged changes", true, &st.staged, cx))
                    .child(self.section("Changes", false, &st.changes, cx))
                    .when(empty, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(sp(2))
                                .px(sp(3))
                                .py(sp(3))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child(Icon::new(IconName::Check).size(sp(4)))
                                .child("No pending changes."),
                        )
                    }),
            )
            .into_any_element()
    }
}
