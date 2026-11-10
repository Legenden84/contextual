//! Source Control: branch with ahead/behind, commit message (drafted by the model on request),
//! Commit / Commit all, Push / Publish, and comparison before opening a pull request.
//! Clicking a staged or unstaged file opens its diff in a tab.

use std::collections::{HashMap, HashSet};

use contextual_core::logic::git_tabs::{git_tab_id, workspace_git_path};
use contextual_protocol::fs::Repository;
use contextual_protocol::git::{ChangeLetter, StatusEntry};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
};

use crate::state::scm::{ScmModel, status_change_count};
use crate::state::{AppModel, Project, Tab};
use crate::theme::{ActiveTheme, TypeExt, hsla, px, row_height, sp};
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
    message_root: Option<String>,
    messages: HashMap<String, String>,
    /// Repository-qualified section ids; collapsing one group leaves the others alone.
    closed_sections: HashSet<String>,
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
        let message_root = scm.read(cx).repository_root().map(str::to_owned);
        Self {
            app,
            project,
            scm,
            message,
            message_root,
            messages: HashMap::new(),
            closed_sections: HashSet::new(),
            _subs: subs,
        }
    }

    fn sync_message_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.scm.read(cx).repository_root().map(str::to_owned);
        if self.message_root == root {
            return;
        }
        if let Some(previous) = self.message_root.take() {
            self.messages
                .insert(previous, self.message.read(cx).value().to_string());
        }
        let text = root
            .as_ref()
            .and_then(|root| self.messages.get(root))
            .cloned()
            .unwrap_or_default();
        self.message_root = root;
        self.message
            .update(cx, |message, cx| message.set_value(text, window, cx));
    }

    fn repositories(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        let scm = self.scm.read(cx);
        let visible = scm.repositories.len() > 1
            || scm.repositories.first().is_some_and(|r| !r.path.is_empty());
        let disabled = scm.busy.is_some() || scm.branch_picker_open;
        let rows: Vec<_> = scm
            .repositories
            .iter()
            .enumerate()
            .map(|(index, repository)| {
                let name = if repository.path.is_empty() {
                    self.project.read(cx).name.clone()
                } else {
                    repository.path.clone()
                };
                let status = scm.repository_statuses.get(&repository.root);
                let label = match status {
                    Some(status) => format!(
                        "{} · {} · {}",
                        name,
                        status.branch.as_deref().unwrap_or("detached"),
                        status_change_count(status),
                    ),
                    None if scm.repository_errors.contains_key(&repository.root) => {
                        format!("{name} · unavailable")
                    }
                    None => format!("{name} · loading"),
                };
                let root = repository.root.clone();
                Button::new(("scm-repository", index), label)
                    .variant(ButtonVariant::Ghost)
                    .selected(scm.repository_root() == Some(repository.root.as_str()))
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.scm
                            .update(cx, |scm, cx| scm.select_repository(&root, cx));
                        this.sync_message_scope(window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        div().when(visible, |d| {
            d.flex()
                .flex_col()
                .flex_none()
                .gap(sp(1))
                .px(sp(3))
                .pb(sp(2))
                .child(
                    div()
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child("Repositories · pending files"),
                )
                .child(
                    div()
                        .id("scm-repositories")
                        .flex()
                        .flex_col()
                        .gap(sp(1))
                        .max_h(sp(8) * 4.0)
                        .overflow_y_scroll()
                        .children(rows),
                )
        })
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_message_scope(window, cx);
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
        repository: &Repository,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        let busy = self.scm.read(cx).busy.is_some();
        let workspace_path = workspace_git_path(repository, &e.path);
        let tab = git_tab_id(&workspace_path, staged);
        let active = self.project.read(cx).active == Tab::Preview(tab);
        let repository_root = repository.root.clone();
        let (dir, name) = match e.path.rsplit_once('/') {
            Some((d, n)) => (d.to_string(), n.to_string()),
            None => (String::new(), e.path.clone()),
        };
        let (open_path, act_path) = (workspace_path, e.path.clone());
        let full_path = SharedString::from(open_path.clone());
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
                            s.unstage_in(&repository_root, p, cx)
                        } else {
                            s.stage_in(&repository_root, p, cx)
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
        repository: &Repository,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        let section_id = format!("{}:{title}", repository.root);
        let open = !self.closed_sections.contains(&section_id);
        let busy = self.scm.read(cx).busy.is_some();
        let repository_root = repository.root.clone();
        let paths: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
        let rows: Vec<_> = if open {
            entries
                .iter()
                .enumerate()
                .map(|(i, e)| self.row(i, e, staged, repository, cx).into_any_element())
                .collect()
        } else {
            Vec::new()
        };
        div()
            .id(SharedString::from(section_id.clone()))
            .when(!entries.is_empty(), |d| {
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
                            if !t.closed_sections.remove(&section_id) {
                                t.closed_sections.insert(section_id.clone());
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
                            .on_click(cx.listener(
                                move |t, _, _, cx| {
                                    cx.stop_propagation();
                                    let p = paths.clone();
                                    t.scm.update(cx, |s, cx| {
                                        if staged {
                                            s.unstage_in(&repository_root, p, cx)
                                        } else {
                                            s.stage_in(&repository_root, p, cx)
                                        }
                                    })
                                },
                            )),
                        )
                        .child(count(entries.len(), cx)),
                )
                .children(rows)
            })
    }

    fn grouped_changes(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        let scm = self.scm.read(cx);
        let repositories = scm.repositories.clone();
        let grouped = repositories.len() > 1
            || repositories
                .first()
                .is_some_and(|repository| !repository.path.is_empty());
        let active_root = scm.repository_root().map(str::to_owned);
        let statuses = scm.repository_statuses.clone();
        let errors = scm.repository_errors.clone();
        let mut groups = Vec::with_capacity(repositories.len());

        for repository in &repositories {
            let status = statuses.get(&repository.root);
            let pending = status.map(status_change_count).unwrap_or_default();
            let name = if repository.path.is_empty() {
                self.project.read(cx).name.clone()
            } else {
                repository.path.clone()
            };
            let selected = active_root.as_deref() == Some(repository.root.as_str());
            let group = div()
                .id(SharedString::from(format!("scm-group:{}", repository.root)))
                .flex()
                .flex_col()
                .when(grouped, |d| {
                    d.mb(sp(2)).child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(2))
                            .px(sp(3))
                            .py(sp(2))
                            .border_b_1()
                            .border_color(hsla(c.border))
                            .bg(hsla(c.surface_100))
                            .when(selected, |d| d.bg(hsla(c.surface_selected)))
                            .child(Icon::new(gpui_kit::assets::IconName::GitBranch).size(sp(4)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .type_style(contextual_theme::type_style::CODE_SMALL)
                                    .text_color(hsla(c.ink))
                                    .child(SharedString::from(name)),
                            )
                            .child(count(pending, cx)),
                    )
                })
                .map(|d| {
                    let Some(status) = status else {
                        return d.child(
                            div()
                                .px(sp(3))
                                .py(sp(2))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(if errors.contains_key(&repository.root) {
                                    c.danger
                                } else {
                                    c.ink_muted
                                }))
                                .child(SharedString::from(
                                    errors
                                        .get(&repository.root)
                                        .cloned()
                                        .unwrap_or_else(|| "Loading changes…".into()),
                                )),
                        );
                    };
                    d.when(!status.conflicts.is_empty(), |d| {
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
                                    status.conflicts.len(),
                                    if status.conflicts.len() == 1 {
                                        "conflict"
                                    } else {
                                        "conflicts"
                                    }
                                ))),
                        )
                    })
                    .child(self.section("Staged changes", true, &status.staged, repository, cx))
                    .child(self.section("Changes", false, &status.changes, repository, cx))
                    .when(pending == 0, |d| {
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
                    })
                });
            groups.push(group.into_any_element());
        }

        div()
            .id("scm-lists")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pt(sp(1))
            .pb(sp(2))
            .border_t_1()
            .border_color(hsla(c.border))
            .children(groups)
    }
}

impl Render for SourceControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_message_scope(window, cx);
        if let Some(text) = self.scm.update(cx, |s, _| s.drafted.take()) {
            self.message
                .update(cx, |m, cx| m.set_value(text, window, cx));
        }
        let repositories = self.repositories(cx);
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
                .child(repositories)
                .child(
                    div()
                        .px(sp(3))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(
                            s.error.clone()
                                .or_else(|| s.repository_root().and_then(|root| s.repository_errors.get(root)).cloned())
                                .unwrap_or_else(|| if s.not_repo {
                                    if self.project.read(cx).truncated {
                                        "No repositories found in the scanned portion. Narrow the opened folder or adjust exclusions."
                                    } else {
                                        "No repositories found in this workspace."
                                    }
                                } else {
                                    "Loading…"
                                }.to_string()),
                        )),
                )
                .child(self.grouped_changes(cx))
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
        let can_compare = st.branch.is_some() && busy.is_none();
        let pr_hint = if st.branch.is_none() {
            "Switch to a branch to compare changes and open a pull request."
        } else if !empty {
            "Review committed changes for a pull request. Uncommitted changes are not included."
        } else {
            "Compare your branch with a base branch, then create or update a pull request."
        };
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
            .child(repositories)
            .when(self.project.read(cx).truncated, |d| {
                d.child(
                    div()
                        .px(sp(3))
                        .pb(sp(2))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.warning))
                        .child("Scan limit reached. More repositories may exist."),
                )
            })
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
                                .variant(if has_changes {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Secondary
                                })
                                .disabled(!can_commit)
                                .on_click(cx.listener(|t, _, w, cx| t.commit(w, cx))),
                        ),
                    )
                    .when(all && busy.is_none(), |d| {
                        d.child(
                            div()
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child("Nothing staged. Commit all includes every change in this repository."),
                        )
                    })
                    .when(show_push, |d| {
                        d.child(
                            div().flex().flex_col().child(
                                Button::new("push", push_label)
                                    .icon(IconName::ArrowUp)
                                    .variant(if empty {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Secondary
                                    })
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
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(sp(2))
                    .px(sp(3))
                    .py(sp(3))
                    .border_t_1()
                    .border_color(hsla(c.border))
                    .child(
                        div()
                            .type_style(contextual_theme::type_style::UI_CAPTION)
                            .text_color(hsla(c.ink_muted))
                            .child("PULL REQUEST"),
                    )
                    .child(
                        div()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(pr_hint),
                    )
                    .child(
                        Button::new("scm-pull-request", "Compare & pull request")
                            .icon(IconName::Github)
                            .variant(if empty && !show_push {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            })
                            .disabled(!can_compare)
                            .on_click(cx.listener(|t, _, _, cx| {
                                t.scm.update(cx, |s, cx| s.open_pull_request(cx))
                            })),
                    ),
            )
            .child(self.grouped_changes(cx))
            .into_any_element()
    }
}
