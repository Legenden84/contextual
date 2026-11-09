//! Assistant replies rendered natively from `contextual_core::logic::markdown` blocks (plan §9.2:
//! model output is never interpreted as HTML). Prose uses styled text runs; code blocks and
//! whole files use the kit's highlighted code block. `### EDIT:` blocks show the rebuilt file,
//! never the edit format (CLAUDE.md).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;

use contextual_core::logic::diff::Diff;
use contextual_core::logic::markdown::{Align, Block, Inline, ListItem, parse_markdown};
use contextual_core::logic::types::{Change, ChangeStatus};
use gpui_kit::base::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ElementId, FontStyle, FontWeight, HighlightStyle, InteractiveElement,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, StrikethroughStyle,
    Styled, StyledText, UnderlineStyle, div,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, px, sp};
use crate::ui::{Tag, TagTone};

/// Lines shown before "Show N more lines" (modified files are shown shorter).
const MAX_LINES: usize = 28;
const MAX_LINES_MODIFIED: usize = 9;
const LIST_INDENT: f32 = 18.0;

const MAX_CACHED_REPLIES: usize = 16;
const MAX_CACHED_SOURCE_BYTES: usize = 4 * 1024 * 1024;

struct CachedReply {
    id: String,
    source: String,
    blocks: Rc<Vec<Block>>,
}

/// Cache parsed data, not themed GPUI elements. Changed text (including equal-length
/// replacements) invalidates it. One oversized reply may occupy the cache alone.
#[derive(Default)]
pub struct ReplyCache(RefCell<VecDeque<CachedReply>>);

impl ReplyCache {
    pub fn get(&self, id: &str, source: &str) -> Rc<Vec<Block>> {
        let mut entries = self.0.borrow_mut();
        if let Some(index) = entries.iter().position(|entry| entry.id == id) {
            let entry = entries.remove(index).expect("located cache entry");
            if entry.source == source {
                let blocks = entry.blocks.clone();
                entries.push_back(entry);
                return blocks;
            }
        }
        let blocks = Rc::new(parse_markdown(source));
        entries.push_back(CachedReply {
            id: id.into(),
            source: source.into(),
            blocks: blocks.clone(),
        });
        while entries.len() > 1
            && (entries.len() > MAX_CACHED_REPLIES
                || entries
                    .iter()
                    .map(|entry| entry.source.len())
                    .sum::<usize>()
                    > MAX_CACHED_SOURCE_BYTES)
        {
            entries.pop_front();
        }
        blocks
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
}

type ChangeEntry<'a> = (&'a Change, Option<&'a Diff>);

/// Flattens inline runs into one string with highlight ranges.
fn runs(inline: &[Inline], cx: &App) -> (String, Vec<(Range<usize>, HighlightStyle)>) {
    let c = &cx.tokens().colors;
    let mut text = String::new();
    let mut hl = Vec::new();
    fn walk(
        nodes: &[Inline],
        style: HighlightStyle,
        text: &mut String,
        hl: &mut Vec<(Range<usize>, HighlightStyle)>,
        c: &contextual_theme::Colors,
    ) {
        for n in nodes {
            match n {
                Inline::Text { text: t } => {
                    let start = text.len();
                    text.push_str(t);
                    if style != HighlightStyle::default() {
                        hl.push((start..text.len(), style));
                    }
                }
                Inline::Code { text: t } => {
                    let start = text.len();
                    text.push_str(t);
                    hl.push((
                        start..text.len(),
                        HighlightStyle {
                            background_color: Some(hsla(c.pill_bg)),
                            color: Some(hsla(c.ink_strong)),
                            ..style
                        },
                    ));
                }
                Inline::Bold { children } => walk(
                    children,
                    HighlightStyle {
                        font_weight: Some(FontWeight::SEMIBOLD),
                        color: Some(hsla(c.ink_strong)),
                        ..style
                    },
                    text,
                    hl,
                    c,
                ),
                Inline::Italic { children } => walk(
                    children,
                    HighlightStyle {
                        font_style: Some(FontStyle::Italic),
                        ..style
                    },
                    text,
                    hl,
                    c,
                ),
                Inline::Strike { children } => walk(
                    children,
                    HighlightStyle {
                        strikethrough: Some(StrikethroughStyle {
                            thickness: px(1.0),
                            color: None,
                        }),
                        color: Some(hsla(c.ink_muted)),
                        ..style
                    },
                    text,
                    hl,
                    c,
                ),
                // Links are shown, not followed, until click handling lands (plan §9.2: http(s) only).
                Inline::Link { children, .. } => walk(
                    children,
                    HighlightStyle {
                        color: Some(hsla(c.accent_ink)),
                        underline: Some(UnderlineStyle {
                            thickness: px(1.0),
                            color: None,
                            wavy: false,
                        }),
                        ..style
                    },
                    text,
                    hl,
                    c,
                ),
            }
        }
    }
    walk(inline, HighlightStyle::default(), &mut text, &mut hl, c);
    // Nested styles produce overlapping ranges; StyledText wants them sorted and disjoint.
    hl.sort_by_key(|(r, _)| r.start);
    let mut flat: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for (r, s) in hl {
        match flat.last_mut() {
            Some((last, _)) if r.start < last.end => {
                let start = last.end.min(r.end);
                if start < r.end {
                    flat.push((start..r.end, s));
                }
            }
            _ => flat.push((r, s)),
        }
    }
    (text, flat)
}

fn inline_text(inline: &[Inline], cx: &App) -> StyledText {
    let (text, hl) = runs(inline, cx);
    StyledText::new(text).with_highlights(hl)
}

fn list(items: &[ListItem], cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    div()
        .flex()
        .flex_col()
        .mb(sp(3))
        .children(items.iter().map(|it| {
            let marker = match (&it.checked, &it.number) {
                (Some(true), _) => "☑".to_string(),
                (Some(false), _) => "☐".to_string(),
                (None, Some(n)) => n.clone(),
                (None, None) => {
                    if it.depth % 2 == 1 {
                        "◦".into()
                    } else {
                        "•".into()
                    }
                }
            };
            div()
                .flex()
                .gap(sp(2))
                .my(px(3.0))
                .pl(px(it.depth as f32 * LIST_INDENT))
                .child(
                    div()
                        .w(px(16.0))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(marker)),
                )
                .child(div().flex_1().min_w_0().child(inline_text(&it.inline, cx)))
        }))
}

fn badge(change: &Change, diff: Option<&Diff>) -> AnyElement {
    if change.status == ChangeStatus::A {
        return Tag::new("new file").tone(TagTone::Add).into_any_element();
    }
    let letter = if change.status == ChangeStatus::D {
        "D"
    } else {
        "M"
    };
    let label = match diff {
        Some(d) => format!("{letter} +{} −{}", d.adds, d.dels),
        None => letter.to_string(),
    };
    Tag::new(label).into_any_element()
}

/// A code block: header with path or language and an optional badge, then highlighted code,
/// cut after `max` lines with "Show N more lines".
#[allow(clippy::too_many_arguments)]
fn code_block<F: Fn(&mut App) + 'static>(
    id: ElementId,
    code: &str,
    lang: &str,
    label: &str,
    badge: Option<AnyElement>,
    max: usize,
    expanded: bool,
    on_expand: F,
    cx: &App,
) -> impl IntoElement + use<F> {
    let c = &cx.tokens().colors;
    let lines: Vec<&str> = code.split('\n').collect();
    let hidden = if expanded {
        0
    } else {
        lines.len().saturating_sub(max)
    };
    let shown = if hidden > 0 {
        lines[..max].join("\n")
    } else {
        code.to_string()
    };
    let fence = format!("```{lang}\n{shown}\n```");
    div()
        .mb(sp(3))
        .rounded(px(contextual_theme::radius::MD))
        .border_1()
        .border_color(hsla(c.border))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap(sp(2))
                .h(px(28.0))
                .px(sp(3))
                .bg(hsla(c.surface_200))
                .border_b_1()
                .border_color(hsla(c.border))
                .type_style(contextual_theme::type_style::CODE_SMALL)
                .text_color(hsla(c.ink))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(SharedString::from(label.to_string())),
                )
                .when_some(badge, |d, b| d.child(b)),
        )
        .child(
            div().type_style(contextual_theme::type_style::CODE).child(
                TextView::markdown(id.clone(), fence)
                    .selectable(true)
                    .shared_code_block_highlighter(
                        cx.global::<crate::theme::CodeHighlighter>().0.clone(),
                    ),
            ),
        )
        .when(hidden > 0, |d| {
            d.child(
                div()
                    .id(ElementId::Name(format!("{id}-more").into()))
                    .px(sp(3))
                    .py(sp(1))
                    .border_t_1()
                    .border_color(hsla(c.border))
                    .bg(hsla(c.surface_200))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.accent_ink))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| on_expand(cx))
                    .child(SharedString::from(format!("Show {hidden} more lines"))),
            )
        })
}

/// Editor language for a fence or path.
fn lang_name(path: Option<&str>, fence_lang: &str) -> String {
    match path {
        Some(p) => crate::views::code_preview::editor_language(p).to_string(),
        None => fence_lang.to_string(),
    }
}

/// Renders cached reply data. `expanded` holds the ids of expanded blocks.
#[allow(clippy::too_many_arguments)]
pub fn render_reply(
    message_id: &str,
    parsed: &[Block],
    changes: &[Change],
    diffs: Option<&[Diff]>,
    streaming: bool,
    expanded: &HashSet<String>,
    on_expand: impl Fn(String, &mut App) + Clone + 'static,
    cx: &App,
) -> Vec<AnyElement> {
    let by_path: HashMap<&str, ChangeEntry<'_>> = changes
        .iter()
        .enumerate()
        .map(|(i, change)| {
            (
                change.path.as_str(),
                (change, diffs.and_then(|diffs| diffs.get(i))),
            )
        })
        .collect();
    let mut shown_edits = HashSet::new();
    let mut out = Vec::new();
    blocks(
        parsed,
        message_id,
        &by_path,
        &mut shown_edits,
        streaming,
        expanded,
        &on_expand,
        &mut out,
        cx,
    );
    out
}

#[allow(clippy::too_many_arguments)]
fn blocks(
    blocks_in: &[Block],
    message_id: &str,
    by_path: &HashMap<&str, ChangeEntry<'_>>,
    shown_edits: &mut HashSet<String>,
    streaming: bool,
    expanded: &HashSet<String>,
    on_expand: &(impl Fn(String, &mut App) + Clone + 'static),
    out: &mut Vec<AnyElement>,
    cx: &App,
) {
    let c = &cx.tokens().colors;
    let header = |text: String| {
        div()
            .mt(sp(4))
            .type_style(contextual_theme::type_style::CODE_SMALL)
            .text_color(hsla(c.ink_muted))
            .child(SharedString::from(text))
            .into_any_element()
    };
    for (i, b) in blocks_in.iter().enumerate() {
        let key = format!("{message_id}-{}-{i}", out.len());
        let expand = {
            let f = on_expand.clone();
            let k = key.clone();
            move |cx: &mut App| f(k.clone(), cx)
        };
        let el = match b {
            Block::P { inline } => div()
                .mb(sp(3))
                .child(inline_text(inline, cx))
                .into_any_element(),
            Block::Heading { level, inline } => {
                let style = match level {
                    1 | 2 => contextual_theme::type_style::UI_TITLE,
                    3 => contextual_theme::type_style::UI_PROSE,
                    _ => contextual_theme::type_style::UI_BODY,
                };
                div()
                    .mt(sp(4))
                    .mb(sp(2))
                    .type_style(style)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(hsla(c.ink_strong))
                    .when(*level == 1, |d| {
                        d.pb(sp(1)).border_b_1().border_color(hsla(c.border))
                    })
                    .child(inline_text(inline, cx))
                    .into_any_element()
            }
            Block::List { items } => list(items, cx).into_any_element(),
            Block::Quote { blocks: inner } => {
                let mut kids = Vec::new();
                blocks(
                    inner,
                    &key,
                    by_path,
                    shown_edits,
                    streaming,
                    expanded,
                    on_expand,
                    &mut kids,
                    cx,
                );
                div()
                    .mb(sp(3))
                    .pl(sp(3))
                    .border_l_2()
                    .border_color(hsla(c.border_strong))
                    .text_color(hsla(c.ink_muted))
                    .children(kids)
                    .into_any_element()
            }
            Block::Table { head, align, rows } => {
                let cell = |content: &Vec<Inline>, j: usize, head: bool| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .px(sp(2))
                        .py(sp(1))
                        .border_1()
                        .border_color(hsla(c.border))
                        .map(|d| match align.get(j).copied().flatten() {
                            Some(Align::Right) => d.flex().justify_end(),
                            Some(Align::Center) => d.flex().justify_center(),
                            _ => d,
                        })
                        .when(head, |d| {
                            d.bg(hsla(c.surface_200))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(hsla(c.ink_strong))
                        })
                        .child(inline_text(content, cx))
                };
                div()
                    .mb(sp(3))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .children(head.iter().enumerate().map(|(j, h)| cell(h, j, true))),
                    )
                    .children(rows.iter().map(|r| {
                        div().flex().children(
                            (0..head.len())
                                .map(|j| cell(r.get(j).unwrap_or(&Vec::new()), j, false)),
                        )
                    }))
                    .into_any_element()
            }
            Block::Hr => div()
                .my(sp(4))
                .h(px(1.0))
                .bg(hsla(c.border))
                .into_any_element(),
            Block::File { path } => header(format!("### FILE: {path}")),
            Block::Delete { path } => header(format!("### DELETE: {path}")),
            Block::Edit { path, partial, .. } => {
                if !shown_edits.insert(path.clone()) {
                    continue;
                }
                let entry = by_path.get(path.as_str()).copied();
                let change = entry.map(|(change, _)| change);
                out.push(header(format!("### FILE: {path}")));
                match change {
                    Some(ch)
                        if ch.status != ChangeStatus::D
                            && ch.unplaced.as_ref().is_none_or(|u| u.is_empty()) =>
                    {
                        code_block(
                            ElementId::Name(key.clone().into()),
                            &ch.new,
                            &lang_name(Some(path), ""),
                            path,
                            Some(badge(ch, entry.and_then(|(_, diff)| diff))),
                            MAX_LINES_MODIFIED,
                            expanded.contains(&key),
                            expand,
                            cx,
                        )
                        .into_any_element()
                    }
                    Some(ch) if ch.conflict.is_some() && !partial => div()
                        .mt(sp(2))
                        .mb(sp(3))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(ch.conflict.clone().unwrap_or_default()))
                        .into_any_element(),
                    _ => div()
                        .mt(sp(2))
                        .mb(sp(3))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink_muted))
                        .child(SharedString::from(format!(
                            "{} changes to {}…",
                            if streaming { "Writing" } else { "Preparing" },
                            path.rsplit('/').next().unwrap_or(path)
                        )))
                        .into_any_element(),
                }
            }
            Block::Code { code, lang, path } => {
                let entry = path.as_deref().and_then(|p| by_path.get(p).copied());
                let change = entry.map(|(change, _)| change);
                let max = if change.is_some_and(|c| c.status == ChangeStatus::M) {
                    MAX_LINES_MODIFIED
                } else {
                    MAX_LINES
                };
                let label = path.clone().unwrap_or_else(|| lang.label().to_string());
                code_block(
                    ElementId::Name(key.clone().into()),
                    code,
                    &lang_name(path.as_deref(), &format!("{lang:?}").to_lowercase()),
                    &label,
                    entry.map(|(change, diff)| badge(change, diff)),
                    max,
                    expanded.contains(&key),
                    expand,
                    cx,
                )
                .into_any_element()
            }
        };
        out.push(el);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_reuse_parsing_but_not_stale_text() {
        let cache = ReplyCache::default();
        let first = cache.get("a", "**one**");
        assert!(Rc::ptr_eq(&first, &cache.get("a", "**one**")));
        let changed = cache.get("a", "**two**");
        assert!(!Rc::ptr_eq(&first, &changed));
        assert_eq!(*changed, parse_markdown("**two**"));
        cache.clear();
        assert!(!Rc::ptr_eq(&changed, &cache.get("a", "**two**")));
    }

    #[test]
    fn reply_cache_does_not_grow_with_the_whole_thread() {
        let cache = ReplyCache::default();
        let first = cache.get("first", "hello");
        for i in 0..MAX_CACHED_REPLIES {
            cache.get(&i.to_string(), "hello");
        }
        assert_eq!(cache.0.borrow().len(), MAX_CACHED_REPLIES);
        assert!(!Rc::ptr_eq(&first, &cache.get("first", "hello")));
    }
}
