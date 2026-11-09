//! Conservative Markdown typing helpers. All ranges and caret positions are UTF-8 bytes.
//! These functions propose local edits only; the native input owns selection and undo.

use std::ops::Range;

/// One replacement and the desired collapsed selection after it.
#[derive(Debug, PartialEq, Eq)]
pub struct ComposerEdit {
    pub range: Range<usize>,
    pub text: String,
    pub cursor: usize,
}

impl ComposerEdit {
    fn new(range: Range<usize>, text: String) -> Self {
        let cursor = range.start + text.len();
        Self {
            range,
            text,
            cursor,
        }
    }
}

struct Prefix {
    content_start: usize,
    next: String,
    list: bool,
}

/// Recognise an explicit Markdown prefix, never punctuation within ordinary prose.
fn prefix(line: &str) -> Option<Prefix> {
    let body = line.trim_start_matches([' ', '\t']);
    let indent_len = line.len() - body.len();
    let indent = &line[..indent_len];

    let (marker_len, next, list) = if body.starts_with("> ") {
        (2, "> ".to_string(), false)
    } else if matches!(body.as_bytes().first(), Some(b'*' | b'-' | b'+'))
        && body.as_bytes().get(1) == Some(&b' ')
    {
        (2, body[..2].to_string(), true)
    } else {
        let digits = body.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits)
            || !matches!(body.as_bytes().get(digits), Some(b'.' | b')'))
            || body.as_bytes().get(digits + 1) != Some(&b' ')
        {
            return None;
        }
        let number = body[..digits].parse::<u64>().ok()?.checked_add(1)?;
        (
            digits + 2,
            format!("{number}{} ", char::from(body.as_bytes()[digits])),
            true,
        )
    };

    let remaining = &body[marker_len..];
    let task = list
        && (remaining.starts_with("[ ] ")
            || remaining.starts_with("[x] ")
            || remaining.starts_with("[X] "));
    Some(Prefix {
        content_start: indent_len + marker_len + if task { 4 } else { 0 },
        next: format!("{indent}{next}{}", if task { "[ ] " } else { "" }),
        list,
    })
}

fn valid_range(text: &str, selection: &Range<usize>) -> bool {
    selection.start <= selection.end
        && selection.end <= text.len()
        && text.is_char_boundary(selection.start)
        && text.is_char_boundary(selection.end)
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |i| i + 1)
}

/// Backtick and tilde fences. Be conservative about indentation: code nested under
/// a list should also remain untouched.
fn in_fence(before_line: &str) -> bool {
    let mut open: Option<(u8, usize)> = None;
    for line in before_line.lines() {
        let body = line.trim_start_matches([' ', '\t']);
        let Some(&marker) = body.as_bytes().first() else {
            continue;
        };
        if marker != b'`' && marker != b'~' {
            continue;
        }
        let count = body.bytes().take_while(|b| *b == marker).count();
        if count < 3 {
            continue;
        }
        let tail = &body[count..];
        match open {
            Some((kind, length)) if marker == kind && count >= length && tail.trim().is_empty() => {
                open = None;
            }
            None if marker != b'`' || !tail.contains('`') => {
                open = Some((marker, count));
            }
            _ => {}
        }
    }
    open.is_some()
}

fn thematic_break(line: &str) -> bool {
    let mut chars = line.chars().filter(|c| !c.is_whitespace());
    let Some(marker) = chars.next() else {
        return false;
    };
    if !matches!(marker, '*' | '-' | '_') {
        return false;
    }
    let mut count = 1;
    for c in chars {
        if c != marker {
            return false;
        }
        count += 1;
    }
    count >= 3
}

/// Indent a newly typed top-level list marker by two spaces. This is text
/// indentation, not UI spacing. Only a single inserted space qualifies; pasted
/// paragraphs, deletions and undoing the indentation must not be reformatted.
pub fn indent_on_space(
    previous: &str,
    text: &str,
    selection: Range<usize>,
) -> Option<ComposerEdit> {
    if !valid_range(text, &selection)
        || !selection.is_empty()
        || text.len().checked_sub(previous.len()) != Some(1)
    {
        return None;
    }
    let cursor = selection.start;
    let inserted = cursor.checked_sub(1)?;
    if text.as_bytes().get(inserted) != Some(&b' ')
        || previous.get(..inserted) != text.get(..inserted)
        || previous.get(inserted..) != text.get(cursor..)
    {
        return None;
    }

    let start = line_start(text, cursor);
    let typed = &text[start..cursor];
    if typed.starts_with([' ', '\t']) || in_fence(&text[..start]) {
        return None;
    }
    let marker = prefix(typed)?;
    if !marker.list || marker.content_start != typed.len() {
        return None;
    }

    // Do not change an existing line merely because a space was inserted into
    // its prefix. This shortcut is for starting a new item while typing.
    let end = text[cursor..].find('\n').map_or(text.len(), |i| cursor + i);
    if !text[cursor..end].trim().is_empty() {
        return None;
    }

    Some(ComposerEdit::new(start..cursor, format!("  {typed}")))
}

/// Plain Enter: continue a list/quote, exit an empty item, or insert a normal
/// newline. Replacing a selection is deliberately ordinary text editing.
pub fn newline(text: &str, selection: Range<usize>) -> Option<ComposerEdit> {
    if !valid_range(text, &selection) {
        return None;
    }
    let ordinary = || ComposerEdit::new(selection.clone(), "\n".to_string());
    if !selection.is_empty() {
        return Some(ordinary());
    }

    let cursor = selection.start;
    let start = line_start(text, cursor);
    let end = text[cursor..].find('\n').map_or(text.len(), |i| cursor + i);
    let line = &text[start..end];
    if in_fence(&text[..start]) || thematic_break(line) {
        return Some(ordinary());
    }
    let Some(marker) = prefix(line) else {
        return Some(ordinary());
    };
    if cursor < start + marker.content_start {
        return Some(ordinary());
    }

    if line[marker.content_start..].trim().is_empty() {
        // Remove indentation and the empty marker, then make a regular break.
        return Some(ComposerEdit::new(start..end, "\n".to_string()));
    }

    // The continued marker already ends in a space. When splitting after
    // content, absorb one existing word separator rather than duplicating it.
    // Preserve additional spaces, tabs, and all text after that separator.
    let replace_end =
        if cursor > start + marker.content_start && text.as_bytes().get(cursor) == Some(&b' ') {
            cursor + 1
        } else {
            cursor
        };
    Some(ComposerEdit::new(
        cursor..replace_end,
        format!("\n{}", marker.next),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, edit: ComposerEdit) -> (String, usize) {
        let mut result = text.to_string();
        result.replace_range(edit.range, &edit.text);
        assert!(result.is_char_boundary(edit.cursor));
        (result, edit.cursor)
    }

    fn enter(text: &str) -> String {
        apply(text, newline(text, text.len()..text.len()).unwrap()).0
    }

    #[test]
    fn newly_typed_markers_indent() {
        for (before, after, expected) in [
            ("*", "* ", "  * "),
            ("-", "- ", "  - "),
            ("+", "+ ", "  + "),
            ("1.", "1. ", "  1. "),
            ("9)", "9) ", "  9) "),
            ("Hello 🦀\n*", "Hello 🦀\n* ", "Hello 🦀\n  * "),
        ] {
            let edit = indent_on_space(before, after, after.len()..after.len()).unwrap();
            assert_eq!(apply(after, edit), (expected.into(), expected.len()));
        }
    }

    #[test]
    fn ordinary_typing_paste_and_undo_are_not_reformatted() {
        for (before, after) in [
            ("", "* pasted item"),
            ("", "* "),
            ("  * ", "* "),
            ("  *", "  * "),
            ("a *", "a * "),
            ("\\*", "\\* "),
            (">", "> "),
            ("**", "** "),
        ] {
            assert!(indent_on_space(before, after, after.len()..after.len()).is_none());
        }
        assert!(indent_on_space("*hello", "* hello", 2..2).is_none());
    }

    #[test]
    fn lists_continue_and_empty_items_exit() {
        assert_eq!(enter("  * first"), "  * first\n  * ");
        assert_eq!(enter("  * first\n  * "), "  * first\n\n");
        assert_eq!(enter("  - "), "\n");
        assert_eq!(enter("    + nested"), "    + nested\n    + ");
        assert_eq!(enter("    + "), "\n");
        assert_eq!(enter("plain text"), "plain text\n");
    }

    #[test]
    fn numbered_lists_checklists_and_quotes_continue() {
        assert_eq!(enter("  9. ninth"), "  9. ninth\n  10. ");
        assert_eq!(enter("2) second"), "2) second\n3) ");
        assert_eq!(enter("  - [x] done"), "  - [x] done\n  - [ ] ");
        assert_eq!(enter("* [X] done"), "* [X] done\n* [ ] ");
        assert_eq!(enter("  - [ ] "), "\n");
        assert_eq!(enter("> quoted"), "> quoted\n> ");
        assert_eq!(enter("> "), "\n");
    }

    #[test]
    fn splits_at_the_caret_without_losing_the_tail() {
        let text = "  * hello world\nnext";
        let cursor = "  * hello".len();
        let edit = newline(text, cursor..cursor).unwrap();
        assert_eq!(
            apply(text, edit),
            ("  * hello\n  * world\nnext".into(), "  * hello\n  * ".len())
        );

        let text = "* hello";
        assert_eq!(apply(text, newline(text, 2..2).unwrap()).0, "* \n* hello");
    }

    #[test]
    fn splitting_absorbs_only_one_space_and_keeps_other_whitespace() {
        for (before, tail, expected) in [
            ("  * hello", " world", "  * hello\n  * world"),
            ("  * hello", "   world", "  * hello\n  *   world"),
            ("  * hello", "\tworld", "  * hello\n  * \tworld"),
            ("  * hel", "lo", "  * hel\n  * lo"),
            ("  * café", " 🦀", "  * café\n  * 🦀"),
            ("9. hello", " world", "9. hello\n10. world"),
            ("- [x] done", " next", "- [x] done\n- [ ] next"),
            ("> hello", " world", "> hello\n> world"),
        ] {
            let text = format!("{before}{tail}\nnext");
            let cursor = before.len();
            let edit = newline(&text, cursor..cursor).unwrap();
            let expected_cursor = expected.rfind('\n').unwrap()
                + 1
                + prefix(expected.rsplit('\n').next().unwrap())
                    .unwrap()
                    .content_start;
            assert_eq!(
                apply(&text, edit),
                (format!("{expected}\nnext"), expected_cursor)
            );
        }
    }

    #[test]
    fn selections_and_unicode_use_byte_offsets() {
        let text = "🦀 café";
        let start = "🦀 ".len();
        let edit = newline(text, start..text.len()).unwrap();
        assert_eq!(apply(text, edit), ("🦀 \n".into(), start + 1));
        assert!(newline(text, 1..1).is_none());
        assert!(newline(text, 0..text.len() + 1).is_none());
    }

    #[test]
    fn fences_and_thematic_breaks_remain_literal() {
        for text in [
            "```rust\n* code",
            "~~~text\n1. code",
            "````\n```\n* still code",
            "  * * *",
            "---",
        ] {
            assert_eq!(enter(text), format!("{text}\n"));
        }
        assert_eq!(
            enter("```text\nliteral\n```\n* item"),
            "```text\nliteral\n```\n* item\n* "
        );
        assert_eq!(
            enter("~~~\nliteral\n~~~\n- item"),
            "~~~\nliteral\n~~~\n- item\n- "
        );
        let before = "```\n*";
        let after = "```\n* ";
        assert!(indent_on_space(before, after, after.len()..after.len()).is_none());
    }
}
