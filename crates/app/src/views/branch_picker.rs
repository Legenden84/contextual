//! Branch quick pick (VS Code-style), opened from the branch name in the status bar or Source
 //! Control: filter, switch, create, or explicitly delete a local branch.

use contextual_protocol::git::BranchInfo;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div,
};

use crate::state::AppModel;
use crate::state::scm::ScmModel;
use crate::theme::{ActiveTheme, TypeExt, hsla, px, shadow, sp};
use crate::ui::IconButton;

const W: f32 = 620.0;
const ROW_H: f32 = 26.0;
const MAX_H: f32 = 420.0;

#[derive(Clone)]
enum Item {
    Create(String),
    Section(&'static str),
    Branch(BranchInfo, bool),
}

pub struct BranchPicker {
    app: Entity<AppModel>,
    scm: Entity<ScmModel>,
    input: Entity<InputState>,
    creating: bool,
    confirm_delete: Option<String>,
    _subs: Vec<Subscription>,
}

impl BranchPicker {
    pub fn new(
        app: Entity<AppModel>,
        scm: Entity<ScmModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Select a branch to switch to"));
        input.update(cx, |i, cx| i.focus(window, cx));
        let subs = vec![
            cx.observe(&scm, |_, _, cx| cx.notify()),
            cx.subscribe(
                &input,
                |this: &mut Self, input, e: &InputEvent, cx| match e {
                    InputEvent::Change => {
                        this.confirm_delete = None;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        if this.confirm_delete.is_some() || this.scm.read(cx).busy.is_some() {
                            return;
                        }
                        let q = input.read(cx).value().trim().to_string();
                        if this.creating {
                            if !q.is_empty() {
                                this.create(q, cx);
                            }
                        } else if let Some(item) = this
                            .items(cx)
                            .into_iter()
                            .find(|i| !matches!(i, Item::Section(_)))
                        {
                            this.choose(item, cx);
                        }
                    }
                    _ => {}
                },
            ),
        ];
        Self {
            app,
            scm,
            input,
            creating: false,
            confirm_delete: None,
            _subs: subs,
        }
    }

    fn items(&self, cx: &App) -> Vec<Item> {
        let q = self.input.read(cx).value().trim().to_string();
        let ql = q.to_lowercase();
        let s = self.scm.read(cx);
        let matches = |b: &&BranchInfo| ql.is_empty() || b.name.to_lowercase().contains(&ql);
        let mut out = vec![Item::Create(q.clone())];
        if let Some(list) = &s.branches {
            let local: Vec<_> = list.local.iter().filter(matches).cloned().collect();
            let remote: Vec<_> = list.remote.iter().filter(matches).cloned().collect();
            if !local.is_empty() {
                out.push(Item::Section("branches"));
                out.extend(local.into_iter().map(|b| Item::Branch(b, false)));
            }
            if !remote.is_empty() {
                out.push(Item::Section("remote branches"));
                out.extend(remote.into_iter().map(|b| Item::Branch(b, true)));
            }
        }
        out
    }

    fn rescan(app: &Entity<AppModel>) -> impl FnOnce(&mut App) + 'static {
        let app = app.clone();
        move |cx| app.update(cx, |a, cx| a.rescan(cx))
    }

    fn create(&mut self, name: String, cx: &mut Context<Self>) {
        let done = Self::rescan(&self.app);
        self.scm.update(cx, |s, cx| s.create_branch(name, done, cx));
    }

    fn choose(&mut self, item: Item, cx: &mut Context<Self>) {
        if self.scm.read(cx).busy.is_some() {
            return;
        }
        self.confirm_delete = None;
        let current = self
            .scm
            .read(cx)
            .branches
            .as_ref()
            .and_then(|l| l.current.clone());
        match item {
            Item::Create(name) if name.is_empty() => {
                self.creating = true;
                cx.notify();
            }
            Item::Create(name) => self.create(name, cx),
            Item::Branch(b, remote) if !remote && Some(&b.name) == current.as_ref() => {
                self.close(cx)
            }
            Item::Branch(b, _) => {
                let done = Self::rescan(&self.app);
                self.scm
                    .update(cx, |s, cx| s.switch_branch(b.name, done, cx));
            }
            Item::Section(_) => {}
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.scm.update(cx, |s, cx| s.close_branch_picker(cx));
    }

    fn delete_controls(
        &self,
        branch: &BranchInfo,
        current: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &cx.tokens().colors;
        let is_current = current == Some(branch.name.as_str());
        let busy = self.scm.read(cx).busy.is_some();
        let confirming = self.confirm_delete.as_deref() == Some(branch.name.as_str());
        let name = branch.name.clone();
        let label = if is_current {
            "Switch to another branch before deleting this branch".to_string()
        } else if confirming {
            format!("Confirm deletion of local branch {name}")
        } else {
            format!("Delete local branch {name}")
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(sp(1))
            .when(confirming, |d| {
                d.child(
                    div()
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.danger))
                        .child("Delete locally?"),
                )
                .child(
                    IconButton::new(
                        SharedString::from(format!("cancel-delete:{}", branch.name)),
                        IconName::Close,
                        "Cancel branch deletion",
                    )
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_delete = None;
                        cx.notify();
                    })),
                )
            })
            .child(
                IconButton::with_glyph(
                    SharedString::from(format!(
                        "{}:{}",
                        if confirming { "confirm-delete" } else { "delete-branch" },
                        branch.name
                    )),
                    div()
                        .when(confirming && !busy, |d| d.text_color(hsla(c.danger)))
                        .child(Icon::new(IconName::Delete).size(sp(4))),
                    label,
                )
                .disabled(busy || is_current)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if confirming {
                        // A stale click must not confirm a different or cancelled review.
                        if this.confirm_delete.as_deref() != Some(name.as_str()) {
                            return;
                        }
                        this.confirm_delete = None;
                        this.scm.update(cx, |scm, cx| scm.delete_branch(name.clone(), cx));
                    } else {
                        this.confirm_delete = Some(name.clone());
                    }
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    fn row(
        &self,
        i: usize,
        item: &Item,
        current: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &cx.tokens().colors;
        let base = |id: usize| {
            div()
                .id(("branch-row", id))
                .flex()
                .items_center()
                .gap(sp(2))
                .h(px(ROW_H))
                .px(sp(3))
                .cursor_pointer()
                .hover(|s| s.bg(hsla(c.surface_selected)))
        };
        let pick = item.clone();
        let click = cx.listener(move |t, _, _, cx| t.choose(pick.clone(), cx));
        match item {
            Item::Section(label) => div()
                .mt(sp(1))
                .pt(sp(1))
                .px(sp(3))
                .flex()
                .justify_end()
                .border_t_1()
                .border_color(hsla(c.border))
                .type_style(contextual_theme::type_style::UI_CAPTION)
                .text_color(hsla(c.ink_muted))
                .child(SharedString::from(*label))
                .into_any_element(),
            Item::Create(name) => base(i)
                .on_click(click)
                .child(
                    div()
                        .text_color(hsla(c.ink_muted))
                        .child(Icon::new(IconName::Plus).size(px(14.0))),
                )
                .child(SharedString::from(if name.is_empty() {
                    "Create new branch…".to_string()
                } else {
                    format!("Create new branch “{name}”")
                }))
                .child(div().flex_1())
                .when(!name.is_empty(), |d| {
                    d.child(
                        div()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(format!(
                                "from {}",
                                current.unwrap_or("HEAD")
                            ))),
                    )
                })
                .into_any_element(),
            Item::Branch(b, remote) => base(i)
                .on_click(click)
                .child(div().text_color(hsla(c.ink_muted)).child(if *remote {
                    Icon::new(IconName::Globe).size(px(14.0))
                } else {
                    Icon::new(gpui_kit::assets::IconName::GitBranch).size(px(14.0))
                }))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .text_color(hsla(c.ink))
                        .child(SharedString::from(b.name.clone())),
                )
                .when(!remote && Some(b.name.as_str()) == current, |d| {
                    d.child(
                        div()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.accent_ink))
                            .child("current"),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .flex()
                        .justify_end()
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(
                            if self.confirm_delete.as_deref() == Some(b.name.as_str()) && !remote {
                                String::new()
                            } else {
                                format!("{} · {} · {}", b.sha, b.subject, b.when)
                            },
                        )),
                )
                .when(!remote, |d| d.child(self.delete_controls(b, current, cx)))
                .into_any_element(),
        }
    }
}

impl Render for BranchPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let s = self.scm.read(cx);
        let current = s
            .branches
            .as_ref()
            .and_then(|l| l.current.clone())
            .or_else(|| s.status.as_ref().and_then(|st| st.branch.clone()));
        let loaded = s.branches.is_some();
        let busy = s.busy == Some("Branch");
        let error = s.branch_error.clone();
        let items = self.items(cx);
        let rows: Vec<_> = items
            .iter()
            .enumerate()
            .map(|(i, it)| self.row(i, it, current.as_deref(), cx))
            .collect();

        div()
            .id("branch-scrim")
            .absolute()
            .inset_0()
            .on_mouse_down(gpui_kit::MouseButton::Left, cx.listener(|t, _, _, cx| t.close(cx)))
            .on_key_down(cx.listener(|t, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    if t.confirm_delete.take().is_some() {
                        cx.notify();
                    } else if t.creating {
                        t.creating = false;
                        cx.notify();
                    } else {
                        t.close(cx);
                    }
                }
            }))
            .child(
                div()
                    .id("branch-picker")
                    .absolute()
                    .top(px(6.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w(px(W))
                            .max_w_full()
                            .overflow_hidden()
                            .rounded(px(contextual_theme::radius::MD))
                            .border_1()
                            .border_color(hsla(c.border_strong))
                            .bg(hsla(c.surface_200))
                            .shadow(vec![shadow(cx.tokens().shadows.modal)])
                            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .type_style(contextual_theme::type_style::UI_BODY)
                            .child(
                                div()
                                    .px(sp(3))
                                    .pt(sp(2))
                                    .type_style(contextual_theme::type_style::CODE_SMALL)
                                    .text_color(hsla(c.ink_muted))
                                    .child(SharedString::from(self.scm.read(cx).repository_name(cx))),
                            )
                            .child(div().p(px(6.0)).child(Input::new(&self.input)))
                            .map(|d| {
                                if self.creating {
                                    d.child(
                                        div()
                                            .px(sp(3))
                                            .pb(sp(2))
                                            .type_style(contextual_theme::type_style::UI_SMALL)
                                            .text_color(hsla(c.ink_muted))
                                            .child(SharedString::from(format!(
                                                "Type the new branch name and press Enter. It starts from {} and your uncommitted changes come along.",
                                                current.clone().unwrap_or_else(|| "HEAD".into())
                                            ))),
                                    )
                                } else {
                                    d.child(div().id("branch-rows").max_h(px(MAX_H)).overflow_y_scroll().pb(sp(1)).children(rows).when(!loaded, |d| {
                                        d.child(div().px(sp(3)).py(sp(1)).type_style(contextual_theme::type_style::UI_SMALL).text_color(hsla(c.ink_muted)).child("Loading branches…"))
                                    }))
                                }
                            })
                            .when(self.confirm_delete.is_some(), |d| {
                                d.child(
                                    div()
                                        .px(sp(3))
                                        .py(sp(2))
                                        .border_t_1()
                                        .border_color(hsla(c.border))
                                        .type_style(contextual_theme::type_style::UI_SMALL)
                                        .text_color(hsla(c.ink_muted))
                                        .child("Local branch only; the remote is unchanged. Git refuses unmerged branches. Not covered by Save/Undo."),
                                )
                            })
                            .when(busy || error.is_some(), |d| {
                                d.child(
                                    div()
                                        .px(sp(3))
                                        .py(px(6.0))
                                        .border_t_1()
                                        .border_color(hsla(c.border))
                                        .type_style(contextual_theme::type_style::UI_SMALL)
                                        .text_color(hsla(if error.is_some() { c.danger } else { c.ink_muted }))
                                        .child(SharedString::from(error.clone().unwrap_or_else(|| "Working…".into()))),
                                )
                            }),
                    ),
            )
    }
}
