//! Diff of one changed file from Source Control (VS Code-style): staged = HEAD → index,
//! changes = index → working tree.

use std::collections::HashSet;
use std::sync::Arc;

use contextual_core::host::WorkspaceHost;
use contextual_core::logic::diff::{Diff, compute_diff};
use contextual_core::logic::git_tabs::repository_file;
use contextual_protocol::git::{DiffRequest, FileDiff};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Entity, IntoElement, ListAlignment, ListState, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, list,
};

use crate::runtime;
use crate::state::Project;
use crate::state::scm::ScmModel;
use crate::theme::{ActiveTheme, ListZoom, TypeExt, hsla, px, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant};
use crate::views::diff_rows::{self, FileRows, Row};

pub struct GitDiffView {
    path: String,
    staged: bool,
    /// Captured when the tab opens; switching the Source Control selection cannot retarget it.
    target: Option<(String, String)>,
    load_generation: u64,
    host: Arc<dyn WorkspaceHost>,
    file: Option<FileDiff>,
    diff: Option<Diff>,
    error: Option<String>,
    split: bool,
    open_gaps: HashSet<(usize, usize)>,
    rows: Vec<Row>,
    list: ListState,
    list_zoom: ListZoom,
    _sub: Subscription,
}

impl GitDiffView {
    pub fn new(
        path: String,
        staged: bool,
        project: Entity<Project>,
        scm: Entity<ScmModel>,
        host: Arc<dyn WorkspaceHost>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Reload whenever git status changes (stage, commit, save…).
        let sub = cx.observe(&scm, |this: &mut Self, _, cx| this.load(cx));
        let target = repository_file(&project.read(cx).repositories, &path)
            .map(|(repository, path)| (repository.root.clone(), path.to_string()));
        let mut this = Self {
            path,
            staged,
            target,
            load_generation: 0,
            host,
            file: None,
            diff: None,
            error: None,
            split: false,
            open_gaps: HashSet::new(),
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(600.0)),
            list_zoom: ListZoom::default(),
            _sub: sub,
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some((root, path)) = &self.target else {
            self.error =
                Some("No discovered repository owns this file. Rescan the workspace.".into());
            cx.notify();
            return;
        };
        self.load_generation += 1;
        let generation = self.load_generation;
        let req = DiffRequest {
            root: root.clone(),
            path: path.clone(),
            staged: self.staged,
        };
        let host = self.host.clone();
        let job = runtime::spawn(cx, async move { host.git_diff(req).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                if this.load_generation != generation {
                    return;
                }
                match result {
                    Ok(f) => {
                        let changed = this.file.as_ref().is_none_or(|old| {
                            old.old != f.old || old.new != f.new || old.skipped != f.skipped
                        });
                        if changed {
                            this.diff = Some(compute_diff(&f.old, &f.new));
                            this.file = Some(f);
                            this.rebuild();
                        }
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rebuild(&mut self) {
        let Some(diff) = &self.diff else { return };
        let skipped = self.file.as_ref().is_some_and(|f| f.skipped.is_some());
        let files = [FileRows {
            diff,
            banner: skipped,
            hide_lines: skipped,
        }];
        let (rows, _) = diff_rows::build(&files, self.split, false, &self.open_gaps);
        self.list.reset(rows.len());
        self.rows = rows;
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let skipped = self
            .file
            .as_ref()
            .and_then(|f| f.skipped.clone())
            .unwrap_or_default();
        let weak = cx.entity().downgrade();
        match &self.rows[ix] {
            // The breadcrumb bar above serves as this view's file header.
            Row::File(_) => div().into_any_element(),
            row => diff_rows::render(
                row,
                ix,
                move |_| format!("Not shown: {skipped}."),
                move |key, cx| {
                    weak.update(cx, |t, cx| {
                        t.open_gaps.insert(key);
                        t.rebuild();
                        cx.notify();
                    })
                    .ok();
                },
                cx,
            ),
        }
    }
}

impl Render for GitDiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.list_zoom.refresh(&self.list, self.rows.len());
        let c = &cx.tokens().colors;
        let (adds, dels) = self
            .diff
            .as_ref()
            .map(|d| (d.adds, d.dels))
            .unwrap_or((0, 0));
        let unchanged = self.diff.is_some()
            && adds == 0
            && dels == 0
            && self.file.as_ref().is_some_and(|f| f.skipped.is_none());
        let weak = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(sp(2))
                    .h(px(28.0))
                    .pl(sp(4))
                    .pr(sp(3))
                    .border_b_1()
                    .border_color(hsla(c.border))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(
                        div()
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .text_color(hsla(c.ink))
                            .child(SharedString::from(self.path.clone())),
                    )
                    .child(if self.staged {
                        "· staged (HEAD → index)"
                    } else {
                        "· working tree (index → file)"
                    })
                    .when(self.diff.is_some(), |d| {
                        d.child(
                            div()
                                .text_color(hsla(c.success))
                                .child(SharedString::from(format!("+{adds}"))),
                        )
                        .child(
                            div()
                                .text_color(hsla(c.danger))
                                .child(SharedString::from(format!("−{dels}"))),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("unified", "Unified")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .selected(!self.split)
                            .on_click(cx.listener(|t, _, _, cx| {
                                t.split = false;
                                t.rebuild();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("split", "Split")
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .selected(self.split)
                            .on_click(cx.listener(|t, _, _, cx| {
                                t.split = true;
                                t.rebuild();
                                cx.notify();
                            })),
                    ),
            )
            .map(|d| {
                if let Some(e) = self.error.clone() {
                    d.child(
                        div()
                            .p(sp(4))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.danger))
                            .child(SharedString::from(e)),
                    )
                } else if self.diff.is_none() {
                    d.child(
                        div()
                            .p(sp(4))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("Loading…"),
                    )
                } else if unchanged {
                    d.child(
                        div()
                            .p(sp(4))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("No changes."),
                    )
                } else {
                    d.child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(
                                list(self.list.clone(), move |ix, _, cx| {
                                    weak.upgrade()
                                        .map(|e| e.update(cx, |t, cx| t.render_row(ix, cx)))
                                        .unwrap_or_else(|| div().into_any_element())
                                })
                                .flex_1()
                                .w_full()
                                .min_w_0()
                                .min_h_0(),
                            )
                            .child(Scrollbar::vertical(&self.list)),
                    )
                }
            })
    }
}
