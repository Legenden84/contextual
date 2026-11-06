//! Settings → Exclusions → Frameworks: the framework presets (Rust, Node, .NET…). Each can be
//! turned on or off and its markers and globs edited; custom ones can be added and removed.
//! A preset applies to a project that contains one of its markers near the root.

use contextual_core::logic::defaults::builtin_preset;
use contextual_protocol::fs::ExclusionPreset;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div, px,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, Switch, Tag, TagTone};

struct Editor {
    label: Entity<InputState>,
    markers: Entity<InputState>,
    globs: Entity<TextareaState>,
}

struct Row {
    preset: ExclusionPreset,
    editor: Option<Editor>,
}

pub struct PresetsEditor {
    rows: Vec<Row>,
    /// Labels the open project's scan detected.
    detected: Vec<String>,
}

/// Markers as typed: separated by commas or whitespace.
fn parse_markers(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// One glob per line.
fn parse_globs(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

impl PresetsEditor {
    pub fn new(presets: Vec<ExclusionPreset>, detected: Vec<String>) -> Self {
        Self {
            rows: rows(presets),
            detected,
        }
    }

    /// Replaces every preset (Reset to defaults).
    pub fn set(&mut self, presets: Vec<ExclusionPreset>, cx: &mut Context<Self>) {
        self.rows = rows(presets);
        cx.notify();
    }

    /// The presets with what is being edited applied.
    pub fn presets(&self, cx: &gpui_kit::App) -> Vec<ExclusionPreset> {
        self.rows
            .iter()
            .map(|r| match &r.editor {
                Some(e) => edited(&r.preset, e, cx),
                None => r.preset.clone(),
            })
            .collect()
    }

    fn edit(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let p = &self.rows[i].preset;
        let (label, markers, globs) = (
            p.label.clone(),
            p.markers.join(", "),
            p.exclusions.join("\n"),
        );
        let editor = Editor {
            label: cx.new(|cx| InputState::new(window, cx).default_value(label)),
            markers: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Empty: every project")
                    .default_value(markers)
            }),
            globs: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(6)
                    .placeholder("**/build/**")
                    .default_value(globs)
            }),
        };
        self.rows[i].editor = Some(editor);
        cx.notify();
    }

    fn done(&mut self, i: usize, cx: &mut Context<Self>) {
        let row = &mut self.rows[i];
        if let Some(e) = row.editor.take() {
            row.preset = edited(&row.preset, &e, cx);
        }
        cx.notify();
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let n = (1..)
            .find(|n| {
                !self
                    .rows
                    .iter()
                    .any(|r| r.preset.id == format!("custom-{n}"))
            })
            .unwrap_or(1);
        self.rows.push(Row {
            preset: ExclusionPreset {
                id: format!("custom-{n}"),
                label: "Custom".into(),
                enabled: true,
                markers: Vec::new(),
                exclusions: Vec::new(),
            },
            editor: None,
        });
        self.edit(self.rows.len() - 1, window, cx);
    }

    fn row(&self, i: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        let r = &self.rows[i];
        let p = &r.preset;
        let builtin = builtin_preset(&p.id).is_some();
        let detected = !p.markers.is_empty() && self.detected.contains(&p.label);
        let markers = if p.markers.is_empty() {
            "every project".to_string()
        } else {
            p.markers.join(", ")
        };
        let header = div()
            .flex()
            .items_center()
            .gap(sp(2))
            .child(
                Switch::new(("preset-on", i), p.enabled).on_toggle(cx.listener(
                    move |t, _, _, cx| {
                        let p = &mut t.rows[i].preset;
                        p.enabled = !p.enabled;
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .text_color(hsla(if p.enabled { c.ink } else { c.ink_muted }))
                    .child(SharedString::from(p.label.clone())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(SharedString::from(markers)),
            )
            .when(detected, |d| {
                d.child(Tag::new("in this project").tone(TagTone::Add))
            })
            .child(match r.editor {
                Some(_) => Button::new(("preset-done", i), "Done")
                    .size(ButtonSize::Xs)
                    .on_click(cx.listener(move |t, _, _, cx| t.done(i, cx))),
                None => Button::new(("preset-edit", i), "Edit")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Xs)
                    .on_click(cx.listener(move |t, _, w, cx| t.edit(i, w, cx))),
            })
            .when(!builtin, |d| {
                d.child(
                    Button::new(("preset-rm", i), "Remove")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Xs)
                        .on_click(cx.listener(move |t, _, _, cx| {
                            t.rows.remove(i);
                            cx.notify();
                        })),
                )
            });

        let body = match &r.editor {
            Some(e) => div()
                .flex()
                .flex_col()
                .gap(sp(2))
                .pt(sp(2))
                .pl(sp(8))
                .child(labelled("Name", None, Input::new(&e.label), cx))
                .child(labelled(
                    "Markers",
                    Some("File or folder names at the project root or up to two folders down, comma-separated; globs like *.csproj work."),
                    Input::new(&e.markers),
                    cx,
                ))
                .child(labelled(
                    "Exclusions",
                    Some("One glob per line."),
                    div()
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .child(Textarea::new(&e.globs)),
                    cx,
                ))
                .into_any_element(),
            None => div()
                .flex()
                .flex_wrap()
                .gap(sp(1))
                .pt(sp(1))
                .pl(sp(8))
                .children(p.exclusions.iter().map(|g| {
                    div()
                        .px(sp(2))
                        .rounded(px(contextual_theme::radius::SM))
                        .bg(hsla(c.surface_200))
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .text_color(hsla(if p.enabled { c.ink } else { c.ink_faint }))
                        .child(SharedString::from(g.clone()))
                }))
                .into_any_element(),
        };

        div()
            .py(sp(2))
            .border_b_1()
            .border_color(hsla(c.border))
            .child(header)
            .child(body)
    }
}

fn rows(presets: Vec<ExclusionPreset>) -> Vec<Row> {
    presets
        .into_iter()
        .map(|preset| Row {
            preset,
            editor: None,
        })
        .collect()
}

fn edited(p: &ExclusionPreset, e: &Editor, cx: &gpui_kit::App) -> ExclusionPreset {
    let label = e.label.read(cx).value().trim().to_string();
    ExclusionPreset {
        label: if label.is_empty() {
            p.label.clone()
        } else {
            label
        },
        markers: parse_markers(&e.markers.read(cx).value()),
        exclusions: parse_globs(&e.globs.read(cx).value()),
        ..p.clone()
    }
}

fn labelled(
    label: &'static str,
    hint: Option<&'static str>,
    field: impl IntoElement,
    cx: &gpui_kit::App,
) -> impl IntoElement {
    let c = &cx.tokens().colors;
    div()
        .flex()
        .flex_col()
        .gap(sp(1))
        .child(
            div()
                .type_style(contextual_theme::type_style::UI_SMALL)
                .text_color(hsla(c.ink))
                .child(label),
        )
        .child(field)
        .when_some(hint, |d, h| {
            d.child(
                div()
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(h),
            )
        })
}

impl Render for PresetsEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = (0..self.rows.len())
            .map(|i| self.row(i, cx))
            .collect::<Vec<_>>();
        div().flex().flex_col().children(rows).child(
            div().pt(sp(2)).child(
                Button::new("preset-add", "Add framework")
                    .size(ButtonSize::Xs)
                    .on_click(cx.listener(|t, _, w, cx| t.add(w, cx))),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_and_globs_parse_like_typed() {
        assert_eq!(
            parse_markers("Cargo.toml, *.csproj  go.mod,"),
            vec!["Cargo.toml", "*.csproj", "go.mod"]
        );
        assert_eq!(
            parse_globs("**/target/**\n\n  **/out/**  \r\n"),
            vec!["**/target/**", "**/out/**"]
        );
    }
}
