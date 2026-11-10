//! Open a pull request, like GitHub's compare view: base ← compare, the commits the base doesn't
//! have and their file diffs, a title and description (drafted by the model from the commits,
//! the diff and the chat), then push and `gh pr create`. Committing happens in Source Control.

use std::collections::HashSet;
use std::sync::Arc;

use contextual_core::host::WorkspaceHost;
use contextual_core::llm::types::PrDraftInput;
use contextual_core::llm::{FoundryClient, FoundryConfig};
use contextual_core::logic::diff::{Diff, compute_diff};
use contextual_core::logic::pr::{default_title, notes, suggest_branch};
use contextual_protocol::git::{
    BranchesRequest, ChangeLetter, CompareRequest, Comparison, CreatePrRequest, PrDiffRequest,
    PrRequest, PrResult,
};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
    ListAlignment, ListState, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Window, div, list, relative,
};
use tokio_util::sync::CancellationToken;

use crate::runtime;
use crate::state::AppModel;
use crate::state::chat::ChatModel;
use crate::theme::{ActiveTheme, ListZoom, TypeExt, hsla, px, shadow, sp};
use crate::ui::{Button, ButtonVariant, Checkbox, IconButton, LinkButton};
use crate::views::diff_rows::{self, FileRows, Row};

const W: f32 = 960.0;
const SELECT_W: f32 = 220.0;

type BranchSelect = SelectState<SearchableVec<SharedString>>;

pub struct PrDialog {
    app: Entity<AppModel>,
    /// Fixed for the lifetime of this review, independently of Source Control selection.
    repository_root: String,
    chat: Entity<ChatModel>,
    host: Arc<dyn WorkspaceHost>,
    cmp: Option<Comparison>,
    diffs: Vec<Diff>,
    loading: bool,
    load_error: Option<String>,
    base_select: Entity<BranchSelect>,
    head_select: Entity<BranchSelect>,
    /// Branch names and selections to apply on the next render (they need the window).
    pending: Option<(Vec<SharedString>, String, String)>,
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    new_branch: Entity<InputState>,
    pending_text: Option<(String, String)>,
    pending_branch: Option<String>,
    /// The user typed a title or description: a new comparison doesn't overwrite it.
    edited: bool,
    applying: bool,
    draft: bool,
    drafting: bool,
    draft_error: Option<String>,
    creating: bool,
    error: Option<String>,
    open_gaps: HashSet<(usize, usize)>,
    rows: Vec<Row>,
    list: ListState,
    list_zoom: ListZoom,
    _subs: Vec<Subscription>,
}

impl PrDialog {
    pub fn new(
        app: Entity<AppModel>,
        repository_root: String,
        chat: Entity<ChatModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = app.update(cx, |a, _| a.host());
        let base_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SharedString>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let head_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SharedString>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(8)
                .placeholder("Description (Markdown)")
        });
        let new_branch = cx.new(|cx| InputState::new(window, cx));
        let mark_edited = |this: &mut Self, e: &InputEvent| {
            if matches!(e, InputEvent::Change) && !this.applying {
                this.edited = true;
            }
        };
        let subs = vec![
            cx.subscribe(
                &base_select,
                |this: &mut Self, _, e: &SelectEvent<SearchableVec<SharedString>>, cx| {
                    let SelectEvent::Confirm(Some(b)) = e else {
                        return;
                    };
                    let head = this.cmp.as_ref().and_then(|c| c.branch.clone());
                    this.compare(Some(b.to_string()), head, cx);
                },
            ),
            cx.subscribe(
                &head_select,
                |this: &mut Self, _, e: &SelectEvent<SearchableVec<SharedString>>, cx| {
                    let SelectEvent::Confirm(Some(b)) = e else {
                        return;
                    };
                    let base = this.cmp.as_ref().map(|c| c.base.clone());
                    this.compare(base, Some(b.to_string()), cx);
                },
            ),
            cx.subscribe(&title, move |this: &mut Self, _, e: &InputEvent, _| {
                mark_edited(this, e)
            }),
            cx.subscribe(&body, move |this: &mut Self, _, e: &InputEvent, _| {
                mark_edited(this, e)
            }),
        ];
        let mut this = Self {
            app,
            repository_root,
            chat,
            host,
            cmp: None,
            diffs: Vec::new(),
            loading: true,
            load_error: None,
            base_select,
            head_select,
            pending: None,
            title,
            body,
            new_branch,
            pending_text: None,
            pending_branch: None,
            edited: false,
            applying: false,
            draft: false,
            drafting: false,
            draft_error: None,
            creating: false,
            error: None,
            open_gaps: HashSet::new(),
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(600.0)),
            list_zoom: ListZoom::default(),
            _subs: subs,
        };
        this.compare(None, None, cx);
        this
    }

    fn root(&self, _cx: &Context<Self>) -> String {
        self.repository_root.clone()
    }

    fn config(&self, cx: &Context<Self>) -> Option<FoundryConfig> {
        self.app.read(cx).config.clone()
    }

    fn first_ask(&self, cx: &Context<Self>) -> String {
        self.chat
            .read(cx)
            .messages
            .iter()
            .find(|m| m.role == contextual_core::chat::Role::User)
            .map(|m| m.text.clone())
            .unwrap_or_default()
    }

    /// Loads the comparison (and the branch names) for `base` ← `head` (defaults: the server's).
    fn compare(&mut self, base: Option<String>, head: Option<String>, cx: &mut Context<Self>) {
        self.loading = true;
        self.load_error = None;
        self.error = None;
        cx.notify();
        let (host, root) = (self.host.clone(), self.root(cx));
        let job = runtime::spawn(cx, async move {
            let cmp = host
                .pr_compare(CompareRequest {
                    root: root.clone(),
                    base,
                    head,
                })
                .await?;
            let branches = host
                .git_branches(BranchesRequest { root, fetch: false })
                .await
                .ok();
            Ok((cmp, branches))
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok((cmp, branches)) => {
                        let mut names: Vec<SharedString> = Vec::new();
                        if let Some(l) = branches {
                            for n in l.local.iter().map(|b| b.name.clone()).chain(
                                l.remote
                                    .iter()
                                    .map(|b| b.name.trim_start_matches("origin/").to_string()),
                            ) {
                                let n = SharedString::from(n);
                                if !names.contains(&n) {
                                    names.push(n);
                                }
                            }
                        }
                        this.pending = Some((
                            names,
                            cmp.base.clone(),
                            cmp.branch.clone().unwrap_or_default(),
                        ));
                        this.diffs = cmp
                            .files
                            .iter()
                            .map(|f| compute_diff(&f.old, &f.new))
                            .collect();
                        let first_ask = this.first_ask(cx);
                        let subjects: Vec<String> =
                            cmp.commits.iter().map(|c| c.subject.clone()).collect();
                        let on_default = cmp.branch.is_some()
                            && cmp.branch == cmp.default_branch
                            && cmp.branch == cmp.current;
                        if on_default {
                            let seed = subjects.last().cloned().unwrap_or(first_ask.clone());
                            this.pending_branch = Some(suggest_branch(&seed));
                        }
                        if !this.edited {
                            this.pending_text =
                                Some((default_title(&subjects, &first_ask), String::new()));
                        }
                        let draft_now = !this.edited
                            && !cmp.commits.is_empty()
                            && cmp.existing_pr.is_none()
                            && this.config(cx).is_some();
                        this.cmp = Some(cmp);
                        this.rebuild();
                        if draft_now {
                            this.run_draft(cx);
                        }
                    }
                    Err(e) => this.load_error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rebuild(&mut self) {
        let Some(cmp) = &self.cmp else { return };
        let files: Vec<FileRows> = cmp
            .files
            .iter()
            .zip(&self.diffs)
            .map(|(f, d)| FileRows {
                diff: d,
                banner: f.skipped.is_some(),
                hide_lines: f.skipped.is_some(),
            })
            .collect();
        let (rows, _) = diff_rows::build(&files, false, false, &self.open_gaps);
        self.list.reset(rows.len());
        self.rows = rows;
    }

    /// Asks the model for a title and description from the commits, the diff and the chat.
    fn run_draft(&mut self, cx: &mut Context<Self>) {
        let (Some(cmp), Some(config)) = (self.cmp.clone(), self.config(cx)) else {
            return;
        };
        self.drafting = true;
        self.draft_error = None;
        cx.notify();
        let chat = self.chat.read(cx);
        let transcript: Vec<(bool, contextual_core::logic::prompt::ApiContent)> = chat
            .thread
            .transcript
            .iter()
            .map(|m| {
                (
                    m.role == contextual_core::llm::types::ChatRole::User,
                    m.content.clone(),
                )
            })
            .collect();
        let refs: Vec<(bool, &contextual_core::logic::prompt::ApiContent)> =
            transcript.iter().map(|(u, c)| (*u, c)).collect();
        let notes = notes(&refs);
        let (host, root) = (self.host.clone(), self.root(cx));
        let job = runtime::spawn(cx, async move {
            let d = host
                .pr_diff(PrDiffRequest {
                    root,
                    base: cmp.base.clone(),
                    head: cmp.branch.clone(),
                })
                .await?;
            let input = PrDraftInput {
                notes,
                diff: d.diff,
                branch: d.branch,
                commits: d.commits,
            };
            FoundryClient::new(config)?
                .draft_pull_request(&input, &CancellationToken::new())
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.drafting = false;
                match result {
                    Ok(d) => {
                        this.edited = false;
                        this.pending_text = Some((d.title, d.body));
                    }
                    Err(e) => this.draft_error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let Some(cmp) = self.cmp.clone() else { return };
        let on_default =
            cmp.branch.is_some() && cmp.branch == cmp.default_branch && cmp.branch == cmp.current;
        let req = CreatePrRequest {
            root: self.root(cx),
            pr: PrRequest {
                title: self.title.read(cx).value().trim().to_string(),
                body: self.body.read(cx).value().to_string(),
                base: cmp.base.clone(),
                head: cmp
                    .branch
                    .clone()
                    .filter(|b| Some(b) != cmp.current.as_ref()),
                draft: self.draft,
                new_branch: on_default.then(|| self.new_branch.read(cx).value().trim().to_string()),
            },
        };
        self.creating = true;
        self.error = None;
        cx.notify();
        let host = self.host.clone();
        let job = runtime::spawn(cx, async move { host.pr_create(req).await });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            this.update(cx, |this, cx| {
                this.creating = false;
                match result {
                    Ok(pr) => this.record(pr, cx),
                    Err(e) => this.error = Some(e.message),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Notes the PR in the chat (and the thread) and closes.
    fn record(&mut self, pr: PrResult, cx: &mut Context<Self>) {
        let label = pr.number.map(|n| format!("#{n}")).unwrap_or_default();
        let text = if pr.updated_existing {
            format!(
                "Pushed {}; pull request {label} is up to date: {}",
                pr.branch, pr.url
            )
        } else {
            format!("Pull request {label} opened from {}: {}", pr.branch, pr.url)
        };
        self.chat.update(cx, |c, cx| {
            c.record_pr(pr, text, cx);
        });
        let app = self.app.clone();
        app.update(cx, |a, cx| a.rescan(cx));
        self.close(cx);
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.creating {
            return;
        }
        self.chat.update(cx, |c, cx| {
            c.pr_open = false;
            cx.notify();
        });
    }

    fn file_header(&self, f: usize, cx: &Context<Self>) -> AnyElement {
        let c = &cx.tokens().colors;
        let Some(cmp) = &self.cmp else {
            return div().into_any_element();
        };
        let file = &cmp.files[f];
        let d = &self.diffs[f];
        let (letter, color) = match file.letter {
            ChangeLetter::A | ChangeLetter::U => ("A", c.success),
            ChangeLetter::D => ("D", c.danger),
            _ => ("M", c.modified),
        };
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(sp(2))
            .h(px(32.0))
            .px(sp(3))
            .mt(sp(2))
            .bg(hsla(c.surface_200))
            .border_1()
            .border_color(hsla(c.border))
            .type_style(contextual_theme::type_style::CODE_SMALL)
            .child(
                div()
                    .text_color(hsla(color))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(letter),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(SharedString::from(file.path.clone())),
            )
            .child(
                div()
                    .text_color(hsla(c.success))
                    .child(SharedString::from(format!("+{}", d.adds))),
            )
            .child(
                div()
                    .text_color(hsla(c.danger))
                    .child(SharedString::from(format!("−{}", d.dels))),
            )
            .into_any_element()
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match &self.rows[ix] {
            Row::File(f) => self.file_header(*f, cx),
            row => {
                let skipped: Vec<String> = self
                    .cmp
                    .as_ref()
                    .map(|c| {
                        c.files
                            .iter()
                            .map(|f| f.skipped.clone().unwrap_or_default())
                            .collect()
                    })
                    .unwrap_or_default();
                let weak = cx.entity().downgrade();
                diff_rows::render(
                    row,
                    ix,
                    move |f| {
                        format!(
                            "Not shown: {}.",
                            skipped.get(f).cloned().unwrap_or_default()
                        )
                    },
                    move |key, cx| {
                        weak.update(cx, |t, cx| {
                            t.open_gaps.insert(key);
                            t.rebuild();
                            cx.notify();
                        })
                        .ok();
                    },
                    cx,
                )
            }
        }
    }
}

impl Render for PrDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.list_zoom.refresh(&self.list, self.rows.len());
        // Apply values computed off-window.
        if let Some((names, base, head)) = self.pending.take() {
            for (sel, value) in [(&self.base_select, base), (&self.head_select, head)] {
                let items = SearchableVec::new(names.clone());
                let v = SharedString::from(value);
                sel.update(cx, |s, cx| {
                    s.set_items(items, window, cx);
                    s.set_selected_value(&v, window, cx);
                });
            }
        }
        if let Some((title, body)) = self.pending_text.take() {
            self.applying = true;
            self.title
                .update(cx, |i, cx| i.set_value(title, window, cx));
            self.body.update(cx, |i, cx| i.set_value(body, window, cx));
            self.applying = false;
        }
        if let Some(b) = self.pending_branch.take() {
            self.new_branch.update(cx, |i, cx| {
                if i.value().is_empty() {
                    i.set_value(b, window, cx)
                }
            });
        }

        let c = &cx.tokens().colors;
        let cmp = self.cmp.clone();
        let on_default = cmp.as_ref().is_some_and(|c| {
            c.branch.is_some() && c.branch == c.default_branch && c.branch == c.current
        });
        let existing = cmp
            .as_ref()
            .filter(|_| !on_default)
            .and_then(|c| c.existing_pr.clone());
        let nothing = cmp.as_ref().is_some_and(|c| {
            c.problem.is_none()
                && c.commits.is_empty()
                && c.branch.as_deref() != Some(c.base.as_str())
        });
        let ready = cmp
            .as_ref()
            .is_some_and(|c| c.github && !c.commits.is_empty());
        let same = cmp
            .as_ref()
            .is_some_and(|c| c.branch.as_deref() == Some(c.base.as_str()));
        let title_ok = !self.title.read(cx).value().trim().is_empty();
        let branch_ok = !on_default || !self.new_branch.read(cx).value().trim().is_empty();
        let unpushed = cmp.as_ref().map(|c| c.unpushed).unwrap_or(0);
        let can_create = ready
            && !same
            && !self.creating
            && !self.loading
            && (if existing.is_some() {
                unpushed > 0
            } else {
                title_ok
            })
            && branch_ok;
        let (adds, dels) = self
            .diffs
            .iter()
            .fold((0, 0), |(a, d), x| (a + x.adds, d + x.dels));
        let can_draft = self.config(cx).is_some();
        let primary = match (&existing, self.creating) {
            (_, true) => "Working…".to_string(),
            (Some(e), _) if unpushed > 0 => format!("Push to #{}", e.number),
            (Some(_), _) => "Up to date".into(),
            (None, _) if self.draft && unpushed > 0 => "Push & create draft pull request".into(),
            (None, _) if self.draft => "Create draft pull request".into(),
            (None, _) if unpushed > 0 => "Push & create pull request".into(),
            (None, _) => "Create pull request".into(),
        };
        let weak = cx.entity().downgrade();
        let small = |t: String, color| {
            div()
                .type_style(contextual_theme::type_style::UI_SMALL)
                .text_color(hsla(color))
                .child(SharedString::from(t))
        };

        div()
            .id("pr-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(c.surface_scrim))
            .on_key_down(cx.listener(|t, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    t.close(cx)
                }
            }))
            .on_click(cx.listener(|t, _, _, cx| t.close(cx)))
            .child(
                div()
                    .id("pr-dialog")
                    .flex()
                    .flex_col()
                    .w(px(W))
                    .max_w(relative(0.95))
                    .h(relative(0.9))
                    .overflow_hidden()
                    .rounded(px(contextual_theme::radius::LG))
                    .border_1()
                    .border_color(hsla(c.border_strong))
                    .bg(hsla(c.surface_100))
                    .shadow(vec![shadow(cx.tokens().shadows.modal)])
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .type_style(contextual_theme::type_style::UI_BODY)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .pt(sp(3))
                            .pr(sp(3))
                            .pb(sp(2))
                            .pl(sp(4))
                            .child(
                                div().flex_1().type_style(contextual_theme::type_style::UI_TITLE).text_color(hsla(c.ink_strong)).child(SharedString::from(
                                    existing.as_ref().map(|e| format!("Pull request #{}", e.number)).unwrap_or_else(|| "Compare changes".into()),
                                )),
                            )
                            .child(IconButton::new("pr-close", gpui_kit::component::IconName::Close, "Close").on_click(cx.listener(|t, _, _, cx| t.close(cx)))),
                    )
                    .child(
                        div()
                            .px(sp(4))
                            .pb(sp(2))
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(self.repository_root.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(sp(2))
                            .px(sp(4))
                            .py(sp(2))
                            .border_b_1()
                            .border_color(hsla(c.border))
                            .bg(hsla(c.surface_000))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("base")
                            .child(div().w(px(SELECT_W)).child(Select::new(&self.base_select).menu_width(px(SELECT_W))))
                            .child("←")
                            .child("compare")
                            .child(div().w(px(SELECT_W)).child(Select::new(&self.head_select).menu_width(px(SELECT_W))))
                            .when(on_default, |d| d.child("as new branch:").child(div().w(px(260.0)).child(Input::new(&self.new_branch))))
                            .child(div().flex_1())
                            .when(self.loading, |d| d.child("Comparing…"))
                            .when_some(cmp.as_ref().filter(|c| !self.loading && !c.commits.is_empty()), |d, c| {
                                d.child(SharedString::from(format!(
                                    "{} {} · {} {}",
                                    c.commits.len(),
                                    if c.commits.len() == 1 { "commit" } else { "commits" },
                                    c.files.len(),
                                    if c.files.len() == 1 { "file" } else { "files" }
                                )))
                                .child(div().text_color(hsla(cx.tokens().colors.success)).child(SharedString::from(format!("+{adds}"))))
                                .child(div().text_color(hsla(cx.tokens().colors.danger)).child(SharedString::from(format!("−{dels}"))))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .gap(sp(3))
                            .px(sp(4))
                            .py(sp(3))
                            .when_some(self.load_error.clone(), |d, e| d.child(small(e, c.danger)))
                            .when_some(cmp.as_ref().and_then(|c| c.problem.clone()), |d, p| {
                                d.child(
                                    div()
                                        .px(sp(3))
                                        .py(sp(2))
                                        .rounded(px(contextual_theme::radius::MD))
                                        .border_1()
                                        .border_color(hsla(c.warning_border))
                                        .bg(hsla(c.warning_bg))
                                        .type_style(contextual_theme::type_style::UI_SMALL)
                                        .child(SharedString::from(p)),
                                )
                            })
                            .when(same, |d| d.child(small("Choose different branches: base and compare are the same.".into(), c.ink_muted)))
                            .when(on_default && ready, |d| {
                                let n = cmp.as_ref().map(|c| c.commits.len()).unwrap_or(0);
                                d.child(small(
                                    format!(
                                        "You're on the default branch. Your {n} {} to the new branch, which is pushed and used for the pull request.",
                                        if n == 1 { "commit moves" } else { "commits move" }
                                    ),
                                    c.ink_muted,
                                ))
                            })
                            .when(nothing, |d| {
                                let (base, branch, uncommitted) =
                                    cmp.as_ref().map(|c| (c.base.clone(), c.branch.clone().unwrap_or_default(), c.uncommitted)).unwrap_or_default();
                                d.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_center()
                                        .gap(sp(1))
                                        .px(sp(6))
                                        .py(sp(8))
                                        .rounded(px(contextual_theme::radius::MD))
                                        .border_1()
                                        .border_color(hsla(c.border))
                                        .bg(hsla(c.surface_200))
                                        .child(div().type_style(contextual_theme::type_style::UI_TITLE).text_color(hsla(c.ink_strong)).child("There isn't anything to compare."))
                                        .child(small(format!("{base} is up to date with all commits from {branch}."), c.ink_muted))
                                        .when(uncommitted > 0, |d| {
                                            d.child(small(
                                                format!(
                                                    "You have {uncommitted} uncommitted {}. Commit them in Source Control first.",
                                                    if uncommitted == 1 { "change" } else { "changes" }
                                                ),
                                                c.ink_muted,
                                            ))
                                        }),
                                )
                            })
                            .when_some(existing.clone().filter(|_| !self.loading), |d, e| {
                                d.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(sp(2))
                                        .child(small(
                                            format!(
                                                "This branch already has an open pull request: #{}. {}",
                                                e.number,
                                                if unpushed > 0 {
                                                    format!("Push {unpushed} new {} to update it.", if unpushed == 1 { "commit" } else { "commits" })
                                                } else {
                                                    "It is up to date.".into()
                                                }
                                            ),
                                            c.ink,
                                        ))
                                        .child(
                                            div().flex().child(
                                                Button::new("pr-open-github", "Open on GitHub")
                                                    .icon(gpui_kit::component::IconName::Github)
                                                    .on_click(move |_, _, cx| cx.open_url(&e.url)),
                                            ),
                                        ),
                                )
                            })
                            .when(ready && existing.is_none(), |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(sp(1))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .type_style(contextual_theme::type_style::UI_SMALL)
                                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                                .child(div().flex_1().child("Title"))
                                                .when(can_draft, |d| {
                                                    d.child(LinkButton::new(
                                                        "draft-pr",
                                                        if self.drafting { "Drafting…" } else { "Draft with AI" },
                                                        cx.listener(|t, _, _, cx| {
                                                            if !t.drafting {
                                                                t.run_draft(cx)
                                                            }
                                                        }),
                                                    ))
                                                }),
                                        )
                                        .child(Input::new(&self.title))
                                        .child(div().mt(sp(2)).type_style(contextual_theme::type_style::UI_SMALL).font_weight(gpui_kit::FontWeight::MEDIUM).child("Description"))
                                        .child(div().type_style(contextual_theme::type_style::CODE_SMALL).child(Textarea::new(&self.body)))
                                        .when_some(self.draft_error.clone(), |d, e| d.child(small(format!("Couldn't draft the text: {e}"), c.warning))),
                                )
                            })
                            .when(ready && cmp.as_ref().is_some_and(|c| c.uncommitted > 0), |d| {
                                let n = cmp.as_ref().map(|c| c.uncommitted).unwrap_or(0);
                                d.child(small(
                                    format!(
                                        "{n} uncommitted {} not part of this pull request. Commit in Source Control.",
                                        if n == 1 { "change is" } else { "changes are" }
                                    ),
                                    c.ink_muted,
                                ))
                            })
                            .when_some(cmp.as_ref().filter(|_| ready), |d, cmp| {
                                d.child(div().type_style(contextual_theme::type_style::UI_CAPTION).text_color(hsla(c.ink_muted)).child("COMMITS"))
                                    .child(
                                        div().flex().flex_col().rounded(px(contextual_theme::radius::MD)).border_1().border_color(hsla(c.border)).children(
                                            cmp.commits.iter().map(|k| {
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(sp(3))
                                                    .h(px(28.0))
                                                    .px(sp(3))
                                                    .border_b_1()
                                                    .border_color(hsla(c.border))
                                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                                    .child(div().type_style(contextual_theme::type_style::CODE_SMALL).text_color(hsla(c.accent_ink)).child(SharedString::from(k.sha.clone())))
                                                    .child(div().flex_1().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(SharedString::from(k.subject.clone())))
                                                    .child(div().text_color(hsla(c.ink_muted)).child(SharedString::from(format!("{} · {}", k.author, k.when))))
                                            }),
                                        ),
                                    )
                                    .child(div().type_style(contextual_theme::type_style::UI_CAPTION).text_color(hsla(c.ink_muted)).child("FILES CHANGED"))
                                    .child(
                                        list(self.list.clone(), move |ix, _, cx| {
                                            weak.upgrade().map(|e| e.update(cx, |t, cx| t.render_row(ix, cx))).unwrap_or_else(|| div().into_any_element())
                                        })
                                        .flex_1()
                                        .min_h(px(160.0)),
                                    )
                            })
                            .when_some(self.error.clone(), |d, e| d.child(small(e, c.danger))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(3))
                            .px(sp(4))
                            .py(sp(3))
                            .border_t_1()
                            .border_color(hsla(c.border))
                            .bg(hsla(c.surface_000))
                            .when(ready && existing.is_none(), |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(sp(1))
                                        .type_style(contextual_theme::type_style::UI_SMALL)
                                        .child(
                                            Checkbox::new("draft", if self.draft { crate::state::project::CheckState::On } else { crate::state::project::CheckState::Off })
                                                .on_toggle(cx.listener(|t, _, _, cx| {
                                                    t.draft = !t.draft;
                                                    cx.notify();
                                                })),
                                        )
                                        .child("Open as draft"),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .when_some(cmp.as_ref().and_then(|c| c.remote.clone()), |d, r| d.child(small(format!("Pushes to {r}"), c.ink_muted))),
                            )
                            .child(Button::new("pr-cancel", if nothing || !ready { "Close" } else { "Cancel" }).disabled(self.creating).on_click(cx.listener(|t, _, _, cx| t.close(cx))))
                            .when(ready, |d| {
                                d.child(Button::new("pr-create", primary).variant(ButtonVariant::Primary).disabled(!can_create).on_click(cx.listener(|t, _, _, cx| t.create(cx))))
                            }),
                    ),
            )
    }
}
