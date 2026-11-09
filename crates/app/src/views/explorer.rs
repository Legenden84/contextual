//! The Files explorer (design spec: FileTree): filter, Select all / Clear, Respect .gitignore,
//! a virtualised tree with tri-state checkboxes, token counts, exclusions and badges.

use contextual_core::logic::format::fmt_k;
use contextual_core::logic::tree::is_overridable;
use contextual_core::logic::types::NodeKind;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle,
    Window, div, uniform_list,
};

use crate::state::{AppModel, Project, Tab};
use crate::theme::{ActiveTheme, TypeExt, hsla, px, row_height, sp};
use crate::ui::{Checkbox, IconButton, LinkButton, Switch, Tag, TagTone, file_glyph, folder_glyph};

const INDENT: f32 = 12.0;
const ROW_PAD: f32 = 6.0;
const CHEVRON: f32 = 14.0;
/// Files above this many tokens are flagged in the tree.
const BIG_FILE: u64 = 10_000;

pub struct Explorer {
    app: Entity<AppModel>,
    project: Entity<Project>,
    filter: Entity<InputState>,
    scroll: UniformListScrollHandle,
    _subs: Vec<Subscription>,
}

impl Explorer {
    pub fn new(
        app: Entity<AppModel>,
        project: Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter files"));
        let subs = vec![
            cx.observe(&project, |_, _, cx| cx.notify()),
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.subscribe(&filter, |this: &mut Self, input, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    let q = input.read(cx).value().to_string();
                    this.project.update(cx, |p, cx| {
                        p.set_filter(q);
                        cx.notify()
                    });
                }
            }),
        ];
        Self {
            app,
            project,
            filter,
            scroll: UniformListScrollHandle::new(),
            _subs: subs,
        }
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let p = self.project.read(cx);
        let c = &cx.tokens().colors;
        let Some(row) = p.rows().get(ix) else {
            return div().into_any_element();
        };
        let Some(node) = p.node(&row.path) else {
            return div().into_any_element();
        };
        let is_dir = node.kind == NodeKind::Dir;
        let open = is_dir
            && p.is_open(&node.path)
            && node.children.as_ref().is_some_and(|c| !c.is_empty());
        let opt_in = is_overridable(node);
        let blocked = node.excluded.is_some() && !opt_in;
        let file_selected = !is_dir && p.selected.contains(&node.path);
        let active = !is_dir && p.active == Tab::Preview(node.path.clone());
        let selected_tokens = if is_dir {
            p.folder_stats(&node.path).selected_tokens
        } else {
            0
        };
        let tokens = if blocked {
            None
        } else if is_dir {
            (selected_tokens > 0).then(|| fmt_k(selected_tokens))
        } else {
            Some(fmt_k(node.tokens.unwrap_or(0)))
        };
        let token_color = if (is_dir && selected_tokens > 0) || file_selected {
            if file_selected && node.tokens.unwrap_or(0) > BIG_FILE {
                c.warning
            } else {
                c.ink_muted
            }
        } else {
            c.ink_faint
        };
        let state = p.check_state(node);
        let path: SharedString = node.path.clone().into();
        let (path_a, path_b) = (path.clone(), path.clone());
        let name = SharedString::from(node.name.clone());
        let changed = node.changed_on_disk == Some(true);
        let image = node.image == Some(true) && !blocked;
        let condensed = node.condensed_from.is_some();
        let picked = !is_dir && p.model_picked.contains(&node.path) && file_selected;
        let depth = row.depth;

        div()
            .id(("file", ix))
            .w_full()
            .flex()
            .items_center()
            .gap(px(5.0))
            .h(row_height())
            .pl(px(ROW_PAD + depth as f32 * INDENT))
            .pr(sp(3))
            .type_style(contextual_theme::type_style::UI_BODY)
            .whitespace_nowrap()
            .text_color(hsla(if blocked || (opt_in && !file_selected) {
                c.ink_faint
            } else {
                c.ink
            }))
            .when(active, |d| {
                d.bg(hsla(c.surface_selected))
                    .border_1()
                    .border_color(hsla(c.focus_ring))
            })
            .when(!active, |d| d.hover(|s| s.bg(hsla(c.surface_hover))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.project.update(cx, |p, cx| {
                    if is_dir {
                        {
                            p.toggle_expand(&path_a);
                            cx.notify()
                        }
                    } else if !blocked {
                        {
                            p.open_preview(&path_a);
                            cx.notify()
                        }
                    }
                })
            }))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .w(px(CHEVRON))
                    .text_color(hsla(c.ink_muted))
                    .when(is_dir, |d| {
                        d.child(
                            Icon::new(if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(CHEVRON)),
                        )
                    }),
            )
            .child(
                Checkbox::new(("check", ix), state)
                    .disabled(blocked)
                    .on_toggle(cx.listener(move |this, _, _, cx| {
                        this.project.update(cx, |p, cx| {
                            p.toggle_check(&path_b);
                            cx.notify()
                        })
                    })),
            )
            .child(if is_dir {
                folder_glyph(open, cx).into_any_element()
            } else {
                file_glyph(&node.name, cx).into_any_element()
            })
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(name),
            )
            .when(changed, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_color(hsla(c.warning))
                        .child(Icon::new(IconName::Redo).size(px(12.0))),
                )
            })
            .when(image, |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_color(hsla(c.ink_muted))
                        .child(Icon::new(IconName::Eye).size(px(12.0))),
                )
            })
            .when(condensed, |d| d.child(Tag::new("condensed")))
            .when(picked, |d| d.child(Tag::new("model").tone(TagTone::Add)))
            .when(blocked, |d| {
                d.child(Tag::new("excluded").tone(TagTone::Excluded))
            })
            .when(opt_in, |d| {
                d.child(Tag::new("too large").tone(TagTone::Warn))
            })
            .child(div().flex_1())
            .when_some(tokens, |d, t| {
                d.child(
                    div()
                        .flex_none()
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .text_color(hsla(token_color))
                        .child(SharedString::from(t)),
                )
            })
            .into_any_element()
    }
}

impl Render for Explorer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let p = self.project.read(cx);
        let (files, tokens) = p.selection_totals();
        let filtering = !p.filter.trim().is_empty();
        let empty_filter = filtering && p.rows().is_empty();
        let filter_text = p.filter.clone();
        let truncated = p.truncated;
        let name = SharedString::from(p.name.clone());
        let row_count = p.rows().len();
        let respect = self.app.read(cx).settings.workspace.respect_gitignore;
        let app = self.app.clone();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(sp(8))
                    .pl(sp(3))
                    .pr(sp(2))
                    .child(div().type_style(contextual_theme::type_style::UI_CAPTION).text_color(hsla(c.ink_muted)).child("EXPLORER"))
                    .child(div().flex_1())
                    .child(IconButton::new("rescan", IconName::Redo, "Rescan folder").on_click({
                        let app = app.clone();
                        move |_, _, cx| app.update(cx, |a, cx| a.rescan(cx))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(sp(2))
                    .px(sp(3))
                    .pb(sp(2))
                    .child(Input::new(&self.filter).small().cleanable(true).prefix(Icon::new(IconName::Search).size(px(14.0))))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(3))
                            .child(LinkButton::new("select-all", "Select all", cx.listener(|t, _, _, cx| t.project.update(cx, |p, cx| {
 p.select_all();
 cx.notify()
 }))))
                            .child(LinkButton::new("clear", "Clear", cx.listener(|t, _, _, cx| t.project.update(cx, |p, cx| {
 p.clear_selection();
 cx.notify()
 }))))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(sp(1))
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .child(Switch::new("gitignore", respect).on_toggle(move |_, _, cx| {
                                        app.update(cx, |a, cx| {
                                            let mut s = a.settings.clone();
                                            s.workspace.respect_gitignore = !s.workspace.respect_gitignore;
                                            a.save_settings(s, None, cx);
                                        })
                                    }))
                                    .child("Respect .gitignore"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(sp(1))
                    .h(row_height())
                    .pl(px(ROW_PAD))
                    .pr(sp(3))
                    .border_t_1()
                    .border_color(hsla(c.border))
                    .type_style(contextual_theme::type_style::UI_CAPTION)
                    .text_color(hsla(c.ink))
                    .child(div().w(px(CHEVRON)).text_color(hsla(c.ink_muted)).child(Icon::new(IconName::ChevronDown).size(px(CHEVRON))))
                    .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).overflow_hidden().text_ellipsis().child(SharedString::from(name.to_uppercase())))
                    .child(div().flex_1())
                    .child(
                        div()
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(format!("{files} files · {}", fmt_k(tokens)))),
                    ),
            )
            .when(truncated, |d| {
                d.child(
                    div()
                        .mx(sp(2))
                        .mb(sp(1))
                        .px(sp(2))
                        .py(px(3.0))
                        .rounded(px(contextual_theme::radius::SM))
                        .bg(hsla(c.warning_bg))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .child("Large folder: only the first 20,000 entries are listed. Add exclusions in Settings."),
                )
            })
            .child(
                uniform_list("explorer", row_count, cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
                }))
                .track_scroll(&self.scroll)
                .flex_1()
                .pb(sp(2)),
            )
            .when(empty_filter, |d| {
                d.child(
                    div()
                        .px(sp(4))
                        .py(sp(2))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(format!("No files match “{}”.", filter_text.trim()))),
                )
            })
    }
}
