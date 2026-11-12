//! Assistant replies rendered natively from `contextual_core::logic::markdown` blocks (plan §9.2:
//! model output is never interpreted as HTML). Prose and code use the kit's selectable text
//! views, including native keyboard copying. `### EDIT:` blocks show the rebuilt file,
//! never the edit format (CLAUDE.md).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use contextual_core::logic::diff::Diff;
use contextual_core::logic::markdown::{
    Align, Block, FileProgress, Inline, ListItem, file_progress, parse_markdown,
};
use contextual_core::logic::types::{Change, ChangeStatus};
use gpui_kit::base::text::TextView;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardItem, ElementId, Entity, FontWeight, IntoElement,
    ParentElement, Pixels, SharedString, Styled, Window, div,
};

use crate::theme::{ActiveTheme, TypeExt, control_height, hsla, px, sp};
use crate::ui::{
    Button, ButtonSize, ButtonVariant, Tag, TagTone, activity_mark, completed_mark, file_glyph,
};

/// Lines shown before "Show N more lines" (modified files are shown shorter).
const MAX_LINES: usize = 28;
const MAX_LINES_MODIFIED: usize = 9;
const LIST_INDENT: f32 = 18.0;

const MAX_CACHED_REPLIES: usize = 16;
const MAX_CACHED_SOURCE_BYTES: usize = 4 * 1024 * 1024;

struct CachedReply {
    id: String,
    source: String,
    blocks: Arc<Vec<Block>>,
}

/// Cache parsed data, not themed GPUI elements. Streamed replies supply blocks prepared
/// on the blocking pool; historical cache misses still parse locally. Changed text (including
/// equal-length replacements) invalidates it. One oversized reply may occupy the cache alone.
#[derive(Default)]
pub struct ReplyCache(RefCell<VecDeque<CachedReply>>);

impl ReplyCache {
    /// `prepared`, when supplied, must describe exactly `source`.
    pub fn get(
        &self,
        id: &str,
        source: &str,
        prepared: Option<&Arc<Vec<Block>>>,
    ) -> Arc<Vec<Block>> {
        let mut entries = self.0.borrow_mut();
        if let Some(index) = entries.iter().position(|entry| entry.id == id) {
            let entry = entries.remove(index).expect("located cache entry");
            if entry.source == source {
                let blocks = entry.blocks.clone();
                entries.push_back(entry);
                return blocks;
            }
        }
        let blocks = match prepared {
            Some(blocks) => blocks.clone(),
            None => {
                // Opt-in measurements only: record sizes and time, never reply/file contents.
                let started = tracing::enabled!(
                    target: "contextual::performance",
                    tracing::Level::DEBUG
                )
                .then(std::time::Instant::now);
                let blocks = Arc::new(parse_markdown(source));
                if let Some(started) = started {
                    tracing::debug!(
                        target: "contextual::performance",
                        source_bytes = source.len(),
                        blocks = blocks.len(),
                        parse_us = started.elapsed().as_micros() as u64,
                        "reply Markdown parsed on UI thread"
                    );
                }
                blocks
            }
        };
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

const MAX_CACHED_CODE_VIEWS: usize = 32;

struct CachedCodeView {
    id: ElementId,
    language: String,
    source: String,
    editor: Entity<EditorState>,
}

/// Editor entities survive conversation renders, preserving selection and scrollbar position.
/// Like the parsed-reply cache, this is bounded rather than retaining the entire thread.
#[derive(Default)]
pub struct CodeViews {
    entries: RefCell<VecDeque<CachedCodeView>>,
    viewport_height: Cell<Pixels>,
}

impl CodeViews {
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
    }

    pub fn set_viewport_height(&self, height: Pixels) {
        self.viewport_height.set(height);
    }

    fn editor(
        &self,
        id: ElementId,
        source: &str,
        language: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<EditorState> {
        let mut entries = self.entries.borrow_mut();
        if let Some(index) = entries
            .iter()
            .position(|entry| entry.id == id && entry.language == language)
            && let Some(mut entry) = entries.remove(index)
        {
            if entry.source != source {
                // Streaming appends should not move an existing selection or replace the
                // whole buffer. Collapse and validated replacements do replace the buffer.
                if let Some(suffix) = source.strip_prefix(&entry.source) {
                    entry.editor.update(cx, |editor, cx| {
                        let selection = editor.selected_range();
                        let end = editor.value().len();
                        editor.set_readonly(false, cx);
                        editor.set_selected_range(end..end, cx);
                        editor.replace(suffix.to_owned(), window, cx);
                        editor.set_selected_range(selection, cx);
                        editor.set_readonly(true, cx);
                    });
                } else {
                    entry.editor.update(cx, |editor, cx| {
                        editor.set_value(source.to_owned(), window, cx);
                    });
                }
                entry.source = source.to_owned();
            }
            let editor = entry.editor.clone();
            entries.push_back(entry);
            return editor;
        }

        let editor = cx.new(|cx| {
            let mut editor = EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .soft_wrap(false)
                .default_value(source.to_owned());
            editor.set_readonly(true, cx);
            editor
        });
        entries.push_back(CachedCodeView {
            id,
            language: language.to_owned(),
            source: source.to_owned(),
            editor: editor.clone(),
        });
        while entries.len() > MAX_CACHED_CODE_VIEWS {
            entries.pop_front();
        }
        editor
    }
}

/// Reserve both controls, borders and the block's bottom spacing within the thread viewport.
/// Measurements are already logical pixels: do not apply UI zoom to them a second time.
fn code_body_budget(viewport: Pixels) -> Pixels {
    let chrome = (control_height() + sp(1)) * 2.0 + sp(3) + gpui_kit::px(2.0);
    (viewport - chrome).max(gpui_kit::px(0.0))
}

fn preview_line_limit(max: usize, budget: Pixels) -> usize {
    let rows = ((budget - sp(4)).max(gpui_kit::px(0.0))
        / px(contextual_theme::type_style::CODE.line_height))
    .floor() as usize;
    max.min(rows.max(1))
}

type ChangeEntry<'a> = (&'a Change, Option<&'a Diff>);

/// Encode only our parsed inline formatting for the selectable Markdown view. Literal
/// punctuation is escaped so text cannot become HTML, an image, a link or a block construct.
/// Link destinations are deliberately omitted: replies do not gain URL-opening behaviour.
fn selectable_markdown(inline: &[Inline]) -> String {
    fn walk(nodes: &[Inline], out: &mut String) {
        for node in nodes {
            match node {
                Inline::Text { text } => {
                    for ch in text.chars() {
                        if ch.is_ascii_punctuation() {
                            out.push('\\');
                        }
                        out.push(ch);
                    }
                }
                Inline::Code { text } => {
                    let longest = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
                    let fence = "`".repeat(longest + 1);
                    // CommonMark removes one surrounding space when the content is not
                    // entirely spaces. Padding protects backticks and significant spaces.
                    let pad = !text.chars().all(|ch| ch == ' ')
                        && (text.starts_with(['`', ' ']) || text.ends_with(['`', ' ']));
                    out.push_str(&fence);
                    if pad {
                        out.push(' ');
                    }
                    out.push_str(text);
                    if pad {
                        out.push(' ');
                    }
                    out.push_str(&fence);
                }
                Inline::Bold { children }
                | Inline::Italic { children }
                | Inline::Strike { children } => {
                    let marker = match node {
                        Inline::Bold { .. } => "**",
                        Inline::Italic { .. } => "*",
                        _ => "~~",
                    };
                    out.push_str(marker);
                    walk(children, out);
                    out.push_str(marker);
                }
                Inline::Link { children, .. } => walk(children, out),
            }
        }
    }

    let mut out = String::new();
    walk(inline, &mut out);
    out
}

/// Stable, message-scoped ids let the toolkit own focus, selection and native Copy handling.
/// This is read-only display, not an input, and never intercepts the composer's shortcuts.
fn inline_text(id: String, inline: &[Inline]) -> impl IntoElement + use<> {
    TextView::markdown(ElementId::Name(id.into()), selectable_markdown(inline)).selectable(true)
}

/// Display literal text without allowing it to introduce Markdown formatting or links.
/// Explicit breaks keep multiline messages and command output on separate lines.
pub(crate) fn selectable_plain(id: String, text: &str) -> impl IntoElement + use<> {
    let source = selectable_markdown(&[Inline::Text {
        text: text.to_owned(),
    }])
    .replace('\n', "  \n");
    TextView::markdown(ElementId::Name(id.into()), source).selectable(true)
}

fn list(id: &str, items: &[ListItem], cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    div()
        .flex()
        .flex_col()
        .mb(sp(3))
        .children(items.iter().enumerate().map(|(index, it)| {
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
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(inline_text(format!("{id}-item-{index}"), &it.inline)),
                )
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

/// Borrow the visible prefix without collecting every line or allocating a joined preview.
/// Match split('\n') semantics, including the empty line after a trailing newline.
fn code_excerpt(code: &str, max: usize, expanded: bool) -> (&str, usize) {
    if expanded {
        return (code, 0);
    }
    let mut lines = 1;
    let mut end = code.len();
    for (offset, _) in code.match_indices('\n') {
        if lines == max {
            end = offset;
        }
        lines += 1;
    }
    let hidden = lines.saturating_sub(max);
    let shown = if max == 0 { "" } else { &code[..end] };
    (shown, hidden)
}

/// A numbered, read-only editor with fixed chrome outside its scrollable body.
/// Copy always includes hidden lines; wheel routing belongs to the enclosing Chat panel.
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
    code_views: &CodeViews,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<F> {
    let c = &cx.tokens().colors;
    let budget = code_body_budget(code_views.viewport_height.get());
    let max = preview_line_limit(max, budget);
    let (shown, hidden) = code_excerpt(code, max, expanded);
    let lines = shown.split('\n').count();
    let height =
        (px(contextual_theme::type_style::CODE.line_height) * lines as f32 + sp(4)).min(budget);
    let editor = code_views.editor(id.clone(), shown, lang, window, cx);
    // Copy the source, never the numbered gutter or the truncated preview.
    let copy_text = code.to_string();
    div()
        .w_full()
        .min_w_0()
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
                .h(control_height() + sp(1))
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
                .when_some(badge, |d, b| d.child(b))
                .child(
                    Button::new(ElementId::Name(format!("{id}-copy").into()), "Copy")
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        }),
                ),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .h(height)
                .overflow_hidden()
                .type_style(contextual_theme::type_style::CODE)
                .child(Editor::new(&editor).readonly(true).bordered(false).h_full()),
        )
        .when(hidden > 0 || expanded, |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(sp(2))
                    .h(control_height() + sp(1))
                    .px(sp(2))
                    .border_t_1()
                    .border_color(hsla(c.border))
                    .bg(hsla(c.surface_200))
                    .child(
                        Button::new(
                            ElementId::Name(format!("{id}-more").into()),
                            if expanded {
                                "Show fewer lines".to_owned()
                            } else {
                                format!("Show {hidden} more lines")
                            },
                        )
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            on_expand(cx);
                        }),
                    )
                    .when(expanded, |d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child("Scroll within code to see more lines"),
                        )
                    }),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileState {
    Writing,
    Checking,
    Received,
    Ready,
    Attention,
    Unavailable,
}

impl FileState {
    fn label(self) -> &'static str {
        match self {
            Self::Writing => "Writing proposal…",
            Self::Checking => "Checking proposal…",
            Self::Received => "Draft received",
            Self::Ready => "Ready to review",
            Self::Attention => "Needs review",
            Self::Unavailable => "No reviewable change",
        }
    }
}

fn file_state(progress: FileProgress, change: Option<&Change>, writing: bool) -> FileState {
    if let Some(change) = change {
        return if change.conflict.is_some()
            || change
                .unplaced
                .as_ref()
                .is_some_and(|edits| !edits.is_empty())
        {
            FileState::Attention
        } else {
            FileState::Ready
        };
    }
    match progress {
        FileProgress::Writing if writing => FileState::Writing,
        FileProgress::Writing => FileState::Checking,
        FileProgress::Received => FileState::Received,
        FileProgress::Unavailable => FileState::Unavailable,
    }
}

/// The output-format marker stays in the source; the chat shows a file-oriented header.
/// Status copy makes it explicit that the green check is about a proposal, never a disk write.
fn file_header(
    id: &str,
    path: &str,
    deleting: bool,
    state: FileState,
    reduce_motion: bool,
    cx: &App,
) -> AnyElement {
    let c = &cx.tokens().colors;
    let (directory, name) = path
        .rsplit_once('/')
        .map_or(("", path), |(directory, name)| (directory, name));
    let status_color = match state {
        FileState::Writing | FileState::Checking => c.accent_ink,
        FileState::Received | FileState::Ready => c.success,
        FileState::Attention => c.warning,
        FileState::Unavailable => c.ink_muted,
    };
    let mark = match state {
        FileState::Writing | FileState::Checking => activity_mark(
            ElementId::Name(format!("{id}-activity").into()),
            state == FileState::Writing && !reduce_motion,
            cx,
        ),
        FileState::Received | FileState::Ready => completed_mark(cx).into_any_element(),
        _ => div()
            .flex_none()
            .size(sp(4))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(contextual_theme::radius::FULL))
            .border_1()
            .border_color(hsla(status_color))
            .type_style(contextual_theme::type_style::UI_SMALL)
            .text_color(hsla(status_color))
            .child(if state == FileState::Attention {
                "!"
            } else {
                "−"
            })
            .into_any_element(),
    };
    div()
        .mt(sp(4))
        .mb(sp(2))
        .p(sp(3))
        .flex()
        .items_center()
        .gap(sp(2))
        .rounded(px(contextual_theme::radius::MD))
        .border_1()
        .border_color(hsla(c.border))
        .bg(hsla(c.surface_200))
        .child(file_glyph(path, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(sp(1))
                .child(
                    div()
                        .type_style(contextual_theme::type_style::CODE_SMALL)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hsla(c.ink_strong))
                        .whitespace_normal()
                        .child(SharedString::from(name.to_string())),
                )
                .when(!directory.is_empty(), |d| {
                    d.child(
                        div()
                            .type_style(contextual_theme::type_style::CODE_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .whitespace_normal()
                            .child(SharedString::from(format!("{directory}/"))),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(sp(2))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(status_color))
                        .child(mark)
                        .child(SharedString::from(if deleting {
                            format!("Delete proposal · {}", state.label())
                        } else {
                            state.label().to_string()
                        })),
                ),
        )
        .into_any_element()
}

/// Renders cached reply data. `expanded` holds the ids of expanded blocks.
#[allow(clippy::too_many_arguments)]
pub fn render_reply(
    message_id: &str,
    parsed: &[Block],
    changes: &[Change],
    diffs: Option<&[Diff]>,
    streaming: bool,
    writing: bool,
    reduce_motion: bool,
    expanded: &HashSet<String>,
    on_expand: impl Fn(String, &mut App) + Clone + 'static,
    code_views: &CodeViews,
    window: &mut Window,
    cx: &mut App,
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
        writing,
        reduce_motion,
        expanded,
        &on_expand,
        &mut out,
        code_views,
        window,
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
    writing: bool,
    reduce_motion: bool,
    expanded: &HashSet<String>,
    on_expand: &(impl Fn(String, &mut App) + Clone + 'static),
    out: &mut Vec<AnyElement>,
    code_views: &CodeViews,
    window: &mut Window,
    cx: &mut App,
) {
    let c = &cx.tokens().colors;
    let header = |id: &str, path: &str, deleting: bool, cx: &App| {
        let change = by_path.get(path).map(|(change, _)| *change);
        let state = file_state(file_progress(blocks_in, path, streaming), change, writing);
        file_header(id, path, deleting, state, reduce_motion, cx)
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
                .child(inline_text(format!("{key}-text"), inline))
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
                    .child(inline_text(format!("{key}-text"), inline))
                    .into_any_element()
            }
            Block::List { items } => list(&key, items, cx).into_any_element(),
            Block::Quote { blocks: inner } => {
                let mut kids = Vec::new();
                blocks(
                    inner,
                    &key,
                    by_path,
                    shown_edits,
                    streaming && i + 1 == blocks_in.len(),
                    writing && i + 1 == blocks_in.len(),
                    reduce_motion,
                    expanded,
                    on_expand,
                    &mut kids,
                    code_views,
                    window,
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
                let cell = |content: &[Inline], row: usize, j: usize, head: bool| {
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
                        .child(inline_text(format!("{key}-cell-{row}-{j}"), content))
                };
                div()
                    .mb(sp(3))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .children(head.iter().enumerate().map(|(j, h)| cell(h, 0, j, true))),
                    )
                    .children(rows.iter().enumerate().map(|(row, r)| {
                        div().flex().children((0..head.len()).map(|j| {
                            cell(
                                r.get(j).map(Vec::as_slice).unwrap_or(&[]),
                                row + 1,
                                j,
                                false,
                            )
                        }))
                    }))
                    .into_any_element()
            }
            Block::Hr => div()
                .my(sp(4))
                .h(px(1.0))
                .bg(hsla(c.border))
                .into_any_element(),
            Block::File { path } | Block::Append { path } => header(&key, path, false, cx),
            Block::Delete { path } => header(&key, path, true, cx),
            Block::Edit { path, partial, .. } => {
                if !shown_edits.insert(path.clone()) {
                    continue;
                }
                let entry = by_path.get(path.as_str()).copied();
                let change = entry.map(|(change, _)| change);
                out.push(header(&key, path, false, cx));
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
                            code_views,
                            window,
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
                    // The header owns progress while the proposal is being received.
                    // Do not leave a stale "Preparing…" label on stopped or failed replies.
                    _ => div().into_any_element(),
                }
            }
            Block::Code {
                code, lang, path, ..
            } => {
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
                    code_views,
                    window,
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
    fn code_body_and_chrome_fit_the_measured_viewport() {
        let viewport = gpui_kit::px(600.0);
        let chrome = (control_height() + sp(1)) * 2.0 + sp(3) + gpui_kit::px(2.0);
        assert_eq!(code_body_budget(viewport) + chrome, viewport);
    }

    #[test]
    fn tiny_viewports_never_produce_negative_editor_heights() {
        assert_eq!(code_body_budget(gpui_kit::px(1.0)), gpui_kit::px(0.0));
    }

    #[test]
    fn collapsed_previews_respect_the_available_code_rows() {
        let budget = px(contextual_theme::type_style::CODE.line_height) * 5.0 + sp(4);
        assert_eq!(preview_line_limit(MAX_LINES, budget), 5);
    }

    #[test]
    fn modified_previews_keep_their_shorter_default() {
        let budget = px(contextual_theme::type_style::CODE.line_height) * 40.0 + sp(4);
        assert_eq!(
            preview_line_limit(MAX_LINES_MODIFIED, budget),
            MAX_LINES_MODIFIED
        );
    }

    #[test]
    fn selectable_prose_escapes_literal_markup_and_preserves_unicode() {
        let inline = vec![Inline::Text {
            text: "<img> [x](file:///secret) ![image](https://example.com) café ☕".into(),
        }];
        assert_eq!(
            selectable_markdown(&inline),
            r"\<img\> \[x\]\(file\:\/\/\/secret\) \!\[image\]\(https\:\/\/example\.com\) café ☕"
        );
    }

    #[test]
    fn selectable_prose_retains_formatting_without_enabling_links() {
        let inline = vec![
            Inline::Bold {
                children: vec![Inline::Text {
                    text: "Important".into(),
                }],
            },
            Inline::Text { text: " ".into() },
            Inline::Link {
                href: "file:///secret".into(),
                children: vec![Inline::Text {
                    text: "label".into(),
                }],
            },
        ];
        assert_eq!(selectable_markdown(&inline), "**Important** label");
    }

    #[test]
    fn selectable_inline_code_protects_backticks_and_spaces() {
        for (text, expected) in [
            ("a*b", "`a*b`"),
            ("a`b", "``a`b``"),
            ("`x`", "`` `x` ``"),
            (" x ", "`  x  `"),
            ("   ", "`   `"),
        ] {
            assert_eq!(
                selectable_markdown(&[Inline::Code { text: text.into() }]),
                expected
            );
        }
    }

    #[test]
    fn file_status_distinguishes_drafting_checking_and_review() {
        assert_eq!(
            file_state(FileProgress::Writing, None, true),
            FileState::Writing
        );
        assert_eq!(
            file_state(FileProgress::Writing, None, false),
            FileState::Checking
        );
        assert_eq!(
            file_state(FileProgress::Unavailable, None, false),
            FileState::Unavailable
        );
        let mut change = Change {
            path: "a.rs".into(),
            status: ChangeStatus::M,
            old: "old".into(),
            new: "new".into(),
            conflict: None,
            edits: None,
            unplaced: None,
        };
        assert_eq!(
            file_state(FileProgress::Unavailable, Some(&change), false),
            FileState::Ready
        );
        change.conflict = Some("Changed on disk".into());
        assert_eq!(
            file_state(FileProgress::Unavailable, Some(&change), false),
            FileState::Attention
        );
    }

    #[test]
    fn replies_reuse_parsing_but_not_stale_text() {
        let cache = ReplyCache::default();
        let first = cache.get("a", "**one**", None);
        assert!(Arc::ptr_eq(&first, &cache.get("a", "**one**", None)));
        let changed = cache.get("a", "**two**", None);
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(*changed, parse_markdown("**two**"));
        cache.clear();
        assert!(!Arc::ptr_eq(&changed, &cache.get("a", "**two**", None)));
    }

    #[test]
    fn replies_share_background_parsing_and_keep_it_after_streaming() {
        let cache = ReplyCache::default();
        let prepared = Arc::new(parse_markdown("**one**"));
        let first = cache.get("a", "**one**", Some(&prepared));
        assert!(Arc::ptr_eq(&prepared, &first));
        assert!(Arc::ptr_eq(&prepared, &cache.get("a", "**one**", None)));

        // Equal-length replacements must use the new snapshot, not the old blocks.
        let replacement = Arc::new(parse_markdown("**two**"));
        let changed = cache.get("a", "**two**", Some(&replacement));
        assert!(Arc::ptr_eq(&replacement, &changed));
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(*changed, parse_markdown("**two**"));
    }

    #[test]
    fn reply_cache_does_not_grow_with_the_whole_thread() {
        let cache = ReplyCache::default();
        let first = cache.get("first", "hello", None);
        for i in 0..MAX_CACHED_REPLIES {
            cache.get(&i.to_string(), "hello", None);
        }
        assert_eq!(cache.0.borrow().len(), MAX_CACHED_REPLIES);
        assert!(!Arc::ptr_eq(&first, &cache.get("first", "hello", None)));
    }

    #[test]
    fn code_excerpts_preserve_line_boundaries_without_copying() {
        for source in ["", "one", "one\n", "one\n\nthree", "café\n☕\n", "a\r\nb"] {
            let lines: Vec<_> = source.split('\n').collect();
            for max in 0..=lines.len() + 1 {
                let (shown, hidden) = code_excerpt(source, max, false);
                assert_eq!(shown, lines[..max.min(lines.len())].join("\n"));
                assert_eq!(hidden, lines.len().saturating_sub(max));
                assert_eq!(code_excerpt(source, max, true), (source, 0));
            }
        }
    }
}
