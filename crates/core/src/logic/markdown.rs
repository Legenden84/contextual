//! Markdown parser for assistant responses (`lib/markdown.ts`). Covers what models actually write:
//! headings, paragraphs, bullet and numbered lists (nested by indentation), block quotes, tables,
//! horizontal rules, fenced code blocks, and inline code, bold, italic and links. Plus the
//! `### FILE:` / `### EDIT:` / `### DELETE:` headers the output format asks for, which drive the
//! Changes panel. Anything unrecognised renders as a paragraph.
//!
//! The TS regexes use lookaround and backreferences, which the `regex` crate lacks; those rules are
//! rewritten as equivalent patterns or small hand-written matchers.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::edits::EditBlock;
use super::js;
use super::lang::{Lang, lang_from_fence, lang_from_path};
use super::types::ChangeStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Inline {
    Text { text: String },
    Code { text: String },
    Bold { children: Vec<Inline> },
    Italic { children: Vec<Inline> },
    Strike { children: Vec<Inline> },
    Link { href: String, children: Vec<Inline> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListItem {
    /// Nesting level, from the item's indentation.
    pub depth: usize,
    /// "1." for numbered items; `None` for bullets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// `[A]` / `[M]` / `[D]` at the start of an item, shown as a status letter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ChangeStatus>,
    /// `[ ]` / `[x]` task-list checkbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    pub inline: Vec<Inline>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Block {
    P {
        inline: Vec<Inline>,
    },
    Heading {
        level: u8,
        inline: Vec<Inline>,
    },
    List {
        items: Vec<ListItem>,
    },
    Quote {
        blocks: Vec<Block>,
    },
    Table {
        head: Vec<Vec<Inline>>,
        align: Vec<Option<Align>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Hr,
    File {
        path: String,
    },
    /// Changed regions of an existing file (`### EDIT:` + SEARCH/REPLACE blocks); `partial` while
    /// still streaming.
    Edit {
        path: String,
        edits: Vec<EditBlock>,
        partial: bool,
    },
    Delete {
        path: String,
    },
    Code {
        code: String,
        lang: Lang,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
}

/// Progress of a file section while receiving a reply. This describes receipt of a draft,
/// not validation of its edits or a write to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileProgress {
    Writing,
    Received,
    Unavailable,
}

/// Only the trailing file section can still be growing. Keep it active until another block
/// arrives or streaming ends: an EDIT section can contain several SEARCH/REPLACE pairs.
/// A stopped reply is not evidence of completion; callers use validated changes after streaming.
pub fn file_progress(blocks: &[Block], path: &str, streaming: bool) -> FileProgress {
    if !streaming {
        return FileProgress::Unavailable;
    }
    let trailing_path = blocks.last().and_then(|block| match block {
        Block::File { path }
        | Block::Edit { path, .. }
        | Block::Delete { path }
        | Block::Code {
            path: Some(path), ..
        } => Some(path.as_str()),
        _ => None,
    });
    if trailing_path == Some(path) {
        return FileProgress::Writing;
    }

    let mut received = false;
    for (index, block) in blocks.iter().enumerate() {
        match block {
            Block::Edit {
                path: candidate,
                edits,
                partial,
            } if candidate == path => {
                if *partial || edits.is_empty() {
                    return FileProgress::Unavailable;
                }
                received = true;
            }
            Block::Code {
                path: Some(candidate),
                ..
            } if candidate == path && index + 1 < blocks.len() => {
                // Another parsed block proves that this fence ended.
                received = true;
            }
            Block::Delete { path: candidate } if candidate == path => received = true,
            _ => {}
        }
    }
    if received {
        FileProgress::Received
    } else {
        FileProgress::Unavailable
    }
}

// ------------------------------------------------------------------ inline

/// Compiles a pattern written with the [`js::pattern`] placeholders.
fn re(pattern: &str) -> Regex {
    let p = js::pattern(pattern);
    Regex::new(&p).unwrap_or_else(|e| panic!("bad regex {p}: {e}"))
}

macro_rules! line_re {
    ($name:ident, $pattern:literal) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| re($pattern));
    };
}

line_re!(CODE_RE, r"`([^`\n]+)`");
line_re!(LINK_RE, r#"\[([^\]\n]+)\]\(([^){ws}]+)(?:{s}+"[^"]*")?\)"#);
// `(?=\S)([^\n]+?)` is the same as `(\S[^\n]*?)`.
line_re!(BOLD_RE, r"\*\*({S}[^\n]*?)\*\*|__({S}[^\n]*?)__");
line_re!(STRIKE_RE, r"~~({S}[^\n]*?)~~");
line_re!(
    URL_RE,
    r#"(?-u:\b)https?://[^{ws}<>()]+[^{ws}<>().,;:!?'"]"#
);

/// A matched inline construct before its children are parsed.
enum Hit<'a> {
    Code(&'a str),
    Link(&'a str, &'a str),
    Bold(&'a str),
    Strike(&'a str),
    Italic(&'a str),
    Url(&'a str),
}

impl Hit<'_> {
    fn into_inline(self) -> Inline {
        match self {
            Self::Code(t) => Inline::Code { text: t.to_owned() },
            Self::Link(text, href) => Inline::Link {
                href: href.to_owned(),
                children: parse_inline(text),
            },
            Self::Bold(t) => Inline::Bold {
                children: parse_inline(t),
            },
            Self::Strike(t) => Inline::Strike {
                children: parse_inline(t),
            },
            Self::Italic(t) => Inline::Italic {
                children: parse_inline(t),
            },
            Self::Url(u) => Inline::Link {
                href: u.to_owned(),
                children: vec![Inline::Text { text: u.to_owned() }],
            },
        }
    }
}

type Found<'a> = Option<(usize, usize, Hit<'a>)>;

fn find_regex<'a>(
    re: &Regex,
    text: &'a str,
    make: impl FnOnce(regex::Captures<'a>) -> Hit<'a>,
) -> Found<'a> {
    let c = re.captures(text)?;
    let m = c.get(0)?;
    Some((m.start(), m.end(), make(c)))
}

fn group<'a>(c: &regex::Captures<'a>, i: usize) -> &'a str {
    c.get(i).map_or("", |m| m.as_str())
}

/// Single `*` or `_` italics, but not inside words (snake_case, 2*3*4) or list markers. Mirrors
/// `(?<![\w*])\*(?=[^\s*])([^*\n]+?)\*(?![\w*])` and the same with `_`.
fn find_italic(text: &str) -> Found<'_> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (n, &(start, m)) in chars.iter().enumerate() {
        if m != '*' && m != '_' {
            continue;
        }
        let blocks = |c: char| js::is_word(c) || c == m;
        if n > 0 && blocks(chars[n - 1].1) {
            continue;
        }
        match chars.get(n + 1) {
            Some(&(_, c)) if !js::is_ws(c) && c != m => {}
            _ => continue,
        }
        // The body can't contain the marker, so the first marker after it is the only close.
        let Some(close) = chars[n + 1..]
            .iter()
            .position(|&(_, c)| c == m || c == '\n')
            .map(|p| n + 1 + p)
        else {
            continue;
        };
        if chars[close].1 != m || chars.get(close + 1).is_some_and(|&(_, c)| blocks(c)) {
            continue;
        }
        let (body_start, close_at) = (chars[n + 1].0, chars[close].0);
        return Some((
            start,
            close_at + 1,
            Hit::Italic(&text[body_start..close_at]),
        ));
    }
    None
}

fn find_inline(text: &str) -> Found<'_> {
    let candidates = [
        find_regex(&CODE_RE, text, |c| Hit::Code(group(&c, 1))),
        find_regex(&LINK_RE, text, |c| Hit::Link(group(&c, 1), group(&c, 2))),
        find_regex(&BOLD_RE, text, |c| {
            Hit::Bold(c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str()))
        }),
        find_regex(&STRIKE_RE, text, |c| Hit::Strike(group(&c, 1))),
        find_italic(text),
        find_regex(&URL_RE, text, |c| Hit::Url(group(&c, 0))),
    ];
    // Earliest match wins; on a tie, the earlier rule.
    candidates
        .into_iter()
        .flatten()
        .fold(None, |best: Found<'_>, hit| match best {
            Some(b) if b.0 <= hit.0 => Some(b),
            _ => Some(hit),
        })
}

pub fn parse_inline(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let Some((start, end, hit)) = find_inline(rest) else {
            out.push(Inline::Text {
                text: rest.to_owned(),
            });
            break;
        };
        if start > 0 {
            out.push(Inline::Text {
                text: rest[..start].to_owned(),
            });
        }
        out.push(hit.into_inline());
        rest = &rest[end..];
    }
    out
}

/// Plain text of inline nodes (for titles and tests).
pub fn inline_text(nodes: &[Inline]) -> String {
    nodes
        .iter()
        .map(|n| match n {
            Inline::Text { text } | Inline::Code { text } => text.clone(),
            Inline::Bold { children }
            | Inline::Italic { children }
            | Inline::Strike { children }
            | Inline::Link { children, .. } => inline_text(children),
        })
        .collect()
}

// ------------------------------------------------------------------ blocks

line_re!(FENCE, r"^{s}*(`{3,}|~{3,})({dot}*)$");
line_re!(SEARCH_MARK, r"^{s}*<{5,9}{s}*SEARCH{s}*$");
line_re!(DIVIDER_MARK, r"^{s}*={5,9}{s}*$");
line_re!(REPLACE_MARK, r"^{s}*>{5,9}{s}*REPLACE{s}*$");
line_re!(FILE_HEADER, r"^{s}*#{2,4} FILE:{s}*({dot}+)$");
line_re!(DELETE_HEADER, r"^{s}*#{2,4} DELETE:{s}*({dot}+)$");
line_re!(EDIT_HEADER, r"^{s}*#{2,4} EDIT:{s}*({dot}+)$");
line_re!(HEADING, r"^{s}{0,3}(#{1,6}){s}+({dot}*?){s}*#*{s}*$");
// `([-*_])(\s*\1){2,}` without the backreference.
line_re!(
    HR,
    r"^{s}{0,3}(?:-(?:{s}*-){2,}|\*(?:{s}*\*){2,}|_(?:{s}*_){2,}){s}*$"
);
line_re!(LIST_ITEM, r"^({s}*)([-*+]|[0-9]{1,9}[.)]){s}+({dot}*)$");
line_re!(QUOTE, r"^{s}{0,3}>{s}?({dot}*)$");
line_re!(
    TABLE_DIVIDER,
    r"^{s}*\|?{s}*:?-{2,}:?{s}*(?:\|{s}*:?-{2,}:?{s}*)*\|?{s}*$"
);
line_re!(NOTE_SUFFIX, r"{s}+\({dot}*\)$");
line_re!(STATUS, r"^\[([AMD])\]{s}*");
line_re!(TASK, r"^\[( |x|X)\]{s}+");
line_re!(BACKTICK_EDGES, r"^`|`$");
line_re!(PIPE_EDGES, r"^\||\|$");

/// A header path without backticks or a trailing note such as "(condensed reference, …)".
fn clean_path(raw: &str) -> String {
    let p = NOTE_SUFFIX.replace(js::trim(raw), "");
    BACKTICK_EDGES.replace_all(&p, "").into_owned()
}

/// `^\s*X{n,}\s*$` for the fence character `X`: the line that closes a fence.
fn closes_fence(line: &str, marker: char, len: usize) -> bool {
    let t = js::trim(line);
    !t.is_empty() && t.chars().all(|c| c == marker) && t.chars().count() >= len
}

/// `^\s{2,}\S`: an indented line continuing a list item.
fn is_continuation(line: &str) -> bool {
    let lead = line.chars().take_while(|&c| js::is_ws(c)).count();
    lead >= 2 && line.chars().nth(lead).is_some()
}

fn split_row(line: &str) -> Vec<String> {
    let s = PIPE_EDGES.replace_all(js::trim(line), "");
    let mut cells = Vec::new();
    let mut start = 0;
    let mut prev = None;
    for (i, c) in s.char_indices() {
        if c == '|' && prev != Some('\\') {
            cells.push(&s[start..i]);
            start = i + 1;
        }
        prev = Some(c);
    }
    cells.push(&s[start..]);
    cells
        .into_iter()
        .map(|c| js::trim(c).replace("\\|", "|"))
        .collect()
}

fn list_item(c: &regex::Captures<'_>) -> ListItem {
    let indent: usize = group(c, 1)
        .chars()
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum();
    let marker = group(c, 2);
    let mut body = group(c, 3);
    let mut item = ListItem {
        depth: indent / 2,
        number: None,
        status: None,
        checked: None,
        inline: Vec::new(),
    };
    if marker.starts_with(|ch: char| ch.is_ascii_digit()) {
        item.number = Some(marker.replacen(')', ".", 1));
    }
    if let Some(st) = STATUS.captures(body) {
        item.status = match group(&st, 1) {
            "A" => Some(ChangeStatus::A),
            "M" => Some(ChangeStatus::M),
            _ => Some(ChangeStatus::D),
        };
        body = &body[st.get(0).map_or(0, |m| m.end())..];
    }
    if let Some(task) = TASK.captures(body) {
        item.checked = Some(group(&task, 1) != " ");
        body = &body[task.get(0).map_or(0, |m| m.end())..];
    }
    item.inline = parse_inline(body);
    item
}

fn align_of(cell: &str) -> Option<Align> {
    match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Some(Align::Center),
        (_, true) => Some(Align::Right),
        (true, _) => Some(Align::Left),
        _ => None,
    }
}

#[derive(PartialEq, Eq)]
enum EditState {
    Idle,
    Search,
    Replace,
}

/// Parses the edit section after an `### EDIT:` header at `lines[header]`. Returns the block and
/// the index of the first line after the section.
fn parse_edit(
    lines: &[&str],
    header: usize,
    path: String,
    file_path: &mut Option<String>,
) -> (Block, usize) {
    let mut edits = Vec::new();
    let mut state = EditState::Idle;
    let (mut search, mut replace): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let mut as_file = false;
    let mut i = header + 1;
    while i < lines.len() {
        let l = lines[i];
        match state {
            EditState::Idle => {
                if SEARCH_MARK.is_match(l) {
                    state = EditState::Search;
                    search.clear();
                    replace.clear();
                } else if js::trim(l).is_empty() {
                } else if FENCE.is_match(l) {
                    // A fence that doesn't wrap SEARCH/REPLACE blocks is a whole file under an EDIT header.
                    let mut k = i + 1;
                    while k < lines.len() && js::trim(lines[k]).is_empty() {
                        k += 1;
                    }
                    if edits.is_empty() && k < lines.len() && !SEARCH_MARK.is_match(lines[k]) {
                        as_file = true;
                        break;
                    }
                } else {
                    break; // Anything else ends the edit section.
                }
            }
            EditState::Search => {
                if DIVIDER_MARK.is_match(l) {
                    state = EditState::Replace;
                } else {
                    search.push(l);
                }
            }
            EditState::Replace => {
                if REPLACE_MARK.is_match(l) {
                    edits.push(EditBlock {
                        search: search.join("\n"),
                        replace: replace.join("\n"),
                    });
                    state = EditState::Idle;
                } else {
                    replace.push(l);
                }
            }
        }
        i += 1;
    }
    let partial = state != EditState::Idle;
    if as_file {
        *file_path = Some(path.clone());
        (Block::File { path }, i)
    } else {
        (
            Block::Edit {
                path,
                edits,
                partial,
            },
            i,
        )
    }
}

pub fn parse_markdown(src: &str) -> Vec<Block> {
    let normalised = src.replace("\r\n", "\n");
    let lines: Vec<&str> = normalised.split('\n').collect();
    let mut blocks = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut file_path: Option<String> = None;

    let flush = |para: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::P {
                inline: parse_inline(&para.join(" ")),
            });
            para.clear();
        }
    };

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(fence) = FENCE.captures(line) {
            flush(&mut para, &mut blocks);
            let marker = group(&fence, 1);
            let (ch, len) = (marker.chars().next().unwrap_or('`'), marker.len());
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !closes_fence(lines[i], ch, len) {
                code.push(lines[i]);
                i += 1;
            }
            let info = js::trim(group(&fence, 2))
                .split(js::is_ws)
                .next()
                .unwrap_or("");
            let lang = if info.is_empty() {
                lang_from_path(file_path.as_deref())
            } else {
                lang_from_fence(info)
            };
            blocks.push(Block::Code {
                code: code.join("\n"),
                lang,
                path: file_path.take(),
            });
        } else if let Some(edit) = EDIT_HEADER.captures(line) {
            flush(&mut para, &mut blocks);
            let (block, next) = parse_edit(&lines, i, clean_path(group(&edit, 1)), &mut file_path);
            blocks.push(block);
            i = next - 1;
        } else if let Some(file) = FILE_HEADER.captures(line) {
            flush(&mut para, &mut blocks);
            let path = clean_path(group(&file, 1));
            file_path = Some(path.clone());
            blocks.push(Block::File { path });
        } else if let Some(del) = DELETE_HEADER.captures(line) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Delete {
                path: clean_path(group(&del, 1)),
            });
        } else if let Some(h) = HEADING.captures(line) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading {
                level: group(&h, 1).len() as u8,
                inline: parse_inline(group(&h, 2)),
            });
        } else if HR.is_match(line) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Hr);
        } else if LIST_ITEM.is_match(line) {
            flush(&mut para, &mut blocks);
            let mut items: Vec<ListItem> = Vec::new();
            while i < lines.len() {
                let l = lines[i];
                if let Some(m) = LIST_ITEM.captures(l) {
                    items.push(list_item(&m));
                } else if is_continuation(l) && !FENCE.is_match(l) && !items.is_empty() {
                    // An indented line continues the previous item.
                    if let Some(last) = items.last_mut() {
                        last.inline.push(Inline::Text { text: " ".into() });
                        last.inline.extend(parse_inline(js::trim(l)));
                    }
                } else if js::trim(l).is_empty()
                    && lines.get(i + 1).is_some_and(|n| LIST_ITEM.is_match(n))
                {
                    // A blank line followed by another item keeps the list going.
                } else {
                    break;
                }
                i += 1;
            }
            i -= 1;
            blocks.push(Block::List { items });
        } else if QUOTE.is_match(line) {
            flush(&mut para, &mut blocks);
            let mut inner = Vec::new();
            while let Some(q) = lines.get(i).and_then(|l| QUOTE.captures(l)) {
                inner.push(group(&q, 1));
                i += 1;
            }
            i -= 1;
            blocks.push(Block::Quote {
                blocks: parse_markdown(&inner.join("\n")),
            });
        } else if line.contains('|')
            && lines.get(i + 1).is_some_and(|n| TABLE_DIVIDER.is_match(n))
            && para.is_empty()
        {
            let head = split_row(line).iter().map(|c| parse_inline(c)).collect();
            let align = split_row(lines[i + 1])
                .iter()
                .map(|c| align_of(c))
                .collect();
            let mut rows = Vec::new();
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !js::trim(lines[i]).is_empty() {
                rows.push(
                    split_row(lines[i])
                        .iter()
                        .map(|c| parse_inline(c))
                        .collect(),
                );
                i += 1;
            }
            i -= 1;
            blocks.push(Block::Table { head, align, rows });
        } else if js::trim(line).is_empty() {
            flush(&mut para, &mut blocks);
        } else {
            para.push(js::trim(line));
        }
        i += 1;
    }
    flush(&mut para, &mut blocks);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text { text: s.into() }
    }

    #[test]
    fn italic_rules() {
        assert_eq!(
            parse_inline("snake_case_name"),
            vec![text("snake_case_name")]
        );
        assert_eq!(parse_inline("2*3*4"), vec![text("2*3*4")]);
        assert_eq!(
            parse_inline("a *b* c"),
            vec![
                text("a "),
                Inline::Italic {
                    children: vec![text("b")]
                },
                text(" c")
            ]
        );
        assert_eq!(parse_inline("* x *"), vec![text("* x *")]);
    }

    #[test]
    fn earliest_rule_wins() {
        let n = parse_inline("**b** `c`");
        assert_eq!(
            n[0],
            Inline::Bold {
                children: vec![text("b")]
            }
        );
        assert_eq!(n[2], Inline::Code { text: "c".into() });
        assert_eq!(inline_text(&n), "b c");
    }

    #[test]
    fn file_header_tags_next_fence() {
        let b = parse_markdown("### FILE: `a.ts` (note)\n```\nx\n```");
        assert_eq!(
            b[0],
            Block::File {
                path: "a.ts".into()
            }
        );
        assert_eq!(
            b[1],
            Block::Code {
                code: "x".into(),
                lang: Lang::Typescript,
                path: Some("a.ts".into())
            }
        );
    }

    #[test]
    fn edit_blocks() {
        let b =
            parse_markdown("### EDIT: a\n<<<<<<< SEARCH\nx\n=======\ny\n>>>>>>> REPLACE\nafter");
        assert_eq!(
            b[0],
            Block::Edit {
                path: "a".into(),
                edits: vec![EditBlock {
                    search: "x".into(),
                    replace: "y".into()
                }],
                partial: false
            }
        );
        assert_eq!(
            b[1],
            Block::P {
                inline: vec![text("after")]
            }
        );
    }

    #[test]
    fn only_the_trailing_file_action_is_writing() {
        let source = "### EDIT: a.rs\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE\n\n### EDIT: b.rs\n<<<<<<< SEARCH\nold";
        let blocks = parse_markdown(source);
        assert_eq!(file_progress(&blocks, "a.rs", true), FileProgress::Received);
        assert_eq!(file_progress(&blocks, "b.rs", true), FileProgress::Writing);
        assert_eq!(
            file_progress(&blocks, "b.rs", false),
            FileProgress::Unavailable
        );
    }

    #[test]
    fn a_file_header_alone_is_not_a_received_draft() {
        let blocks = parse_markdown("### FILE: a.rs\n\nExplanation.");
        assert_eq!(
            file_progress(&blocks, "a.rs", true),
            FileProgress::Unavailable
        );
    }

    #[test]
    fn file_fences_and_deletions_advance_to_received() {
        let blocks = parse_markdown(
            "### FILE: a.rs\n```rust\nfn a() {}\n```\n\n### DELETE: old.rs\n\nDone.",
        );
        assert_eq!(file_progress(&blocks, "a.rs", true), FileProgress::Received);
        assert_eq!(
            file_progress(&blocks, "old.rs", true),
            FileProgress::Received
        );
        // No successful proposal should be inferred just because streaming stopped.
        assert_eq!(
            file_progress(&blocks, "a.rs", false),
            FileProgress::Unavailable
        );
    }

    #[test]
    fn a_complete_edit_pair_can_still_be_followed_by_more_pairs() {
        let blocks =
            parse_markdown("### EDIT: a.rs\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE");
        assert_eq!(file_progress(&blocks, "a.rs", true), FileProgress::Writing);
        let blocks = parse_markdown(
            "### EDIT: a.rs\n<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE\n\nDone.",
        );
        assert_eq!(file_progress(&blocks, "a.rs", true), FileProgress::Received);
    }

    #[test]
    fn hr_needs_same_char() {
        assert_eq!(parse_markdown("- * -").len(), 1);
        assert_eq!(parse_markdown("- - -"), vec![Block::Hr]);
    }

    #[test]
    fn table_escaped_pipe() {
        assert_eq!(split_row("| a \\| b | c |"), vec!["a | b", "c"]);
    }
}
