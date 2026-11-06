//! Applies the model's SEARCH/REPLACE edit blocks to the local copy of a file (`lib/edits.ts`).
//!
//! To save output tokens, the model returns only the changed region of an existing file: a few
//! unchanged lines that locate it (SEARCH) and what they become (REPLACE). The full file is
//! rebuilt here, so everything downstream (Changes panel, diffs, Save) works on whole files.

use serde::{Deserialize, Serialize};

use super::js;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EditBlock {
    pub search: String,
    pub replace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditOutcome {
    /// The file with every block that could be placed applied.
    pub content: String,
    /// Blocks whose SEARCH text wasn't found.
    pub failed: Vec<EditBlock>,
}

fn to_lines(s: &str) -> Vec<String> {
    let mut lines: Vec<String> = s
        .replace("\r\n", "\n")
        .split('\n')
        .map(str::to_owned)
        .collect();
    // A block ends with a newline before the marker; drop that empty tail.
    if lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - js::trim_start(line).len()]
}

fn is_blank(line: &str) -> bool {
    js::trim(line).is_empty()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Compare {
    Exact,
    Trailing,
    Indent,
}

impl Compare {
    fn eq(self, a: &str, b: &str) -> bool {
        match self {
            Self::Exact => a == b,
            Self::Trailing => js::trim_end(a) == js::trim_end(b),
            Self::Indent => js::trim(a) == js::trim(b),
        }
    }
}

/// Start indexes where `needle` occurs in `hay` under `eq`.
fn find_all(hay: &[String], needle: &[String], eq: Compare) -> Vec<usize> {
    if needle.len() > hay.len() {
        return Vec::new();
    }
    (0..=hay.len() - needle.len())
        .filter(|&i| needle.iter().zip(&hay[i..]).all(|(n, h)| eq.eq(h, n)))
        .collect()
}

/// Shifts replacement lines by the indentation difference between the file and the SEARCH text.
fn reindent(replace: &[String], file_line: &str, search_line: &str) -> Vec<String> {
    let have = indent_of(file_line);
    let wrote = indent_of(search_line);
    if have == wrote {
        return replace.to_vec();
    }
    replace
        .iter()
        .map(|l| {
            if is_blank(l) {
                return l.clone();
            }
            if let Some(extra) = have.strip_prefix(wrote) {
                return format!("{extra}{l}");
            }
            match wrote
                .strip_prefix(have)
                .and_then(|less| l.strip_prefix(less))
            {
                Some(rest) => rest.to_owned(),
                None => l.clone(),
            }
        })
        .collect()
}

fn trim_blank_edges(lines: &[String]) -> &[String] {
    let a = lines
        .iter()
        .position(|l| !is_blank(l))
        .unwrap_or(lines.len());
    let b = lines
        .iter()
        .rposition(|l| !is_blank(l))
        .map_or(a, |b| b + 1);
    &lines[a..b]
}

/// Applies blocks in order. Each SEARCH is matched exactly, then ignoring trailing whitespace,
/// then ignoring indentation (re-indenting the replacement), then with blank edge lines dropped.
/// When a SEARCH occurs more than once, the first occurrence after the previous edit wins.
pub fn apply_edits(original: &str, edits: &[EditBlock]) -> EditOutcome {
    let mut lines = to_lines(original);
    let mut failed = Vec::new();
    let mut cursor = 0;

    for edit in edits {
        let search = to_lines(&edit.search);
        let replace = if edit.replace.is_empty() {
            Vec::new()
        } else {
            to_lines(&edit.replace)
        };

        // An empty SEARCH only makes sense for an empty file: the replacement is the whole file.
        if is_blank(&search.concat()) {
            if is_blank(&lines.concat()) {
                lines = replace;
                cursor = lines.len();
            } else {
                failed.push(edit.clone());
            }
            continue;
        }

        let mut placed = false;
        'attempts: for (needle, repl) in [
            (&search[..], &replace[..]),
            (trim_blank_edges(&search), trim_blank_edges(&replace)),
        ] {
            if needle.is_empty() {
                continue;
            }
            for eq in [Compare::Exact, Compare::Trailing, Compare::Indent] {
                let hits = find_all(&lines, needle, eq);
                let Some(&first) = hits.first() else { continue };
                let at = hits.iter().copied().find(|&h| h >= cursor).unwrap_or(first);
                let first_real = needle.iter().position(|l| !is_blank(l));
                let fixed = match first_real {
                    Some(fr) if eq == Compare::Indent => {
                        reindent(repl, &lines[at + fr], &needle[fr])
                    }
                    _ => repl.to_vec(),
                };
                cursor = at + fixed.len();
                lines.splice(at..at + needle.len(), fixed);
                placed = true;
                break 'attempts;
            }
        }
        if !placed {
            failed.push(edit.clone());
        }
    }

    let mut content = lines.join("\n");
    if original.ends_with('\n') && !lines.is_empty() {
        content.push('\n');
    }
    EditOutcome { content, failed }
}

/// The blocks as the model wrote them, for asking it to redo a file in full.
pub fn format_edits(edits: &[EditBlock]) -> String {
    edits
        .iter()
        .map(|e| {
            format!(
                "<<<<<<< SEARCH\n{}\n=======\n{}\n>>>>>>> REPLACE",
                e.search, e.replace
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(search: &str, replace: &str) -> EditBlock {
        EditBlock {
            search: search.into(),
            replace: replace.into(),
        }
    }

    #[test]
    fn exact_and_newline_kept() {
        let r = apply_edits("a\nb\nc\n", &[block("b\n", "B\n")]);
        assert_eq!(r.content, "a\nB\nc\n");
        assert!(r.failed.is_empty());
    }

    #[test]
    fn reindents_on_indent_match() {
        let r = apply_edits("    foo()\n", &[block("foo()", "bar()\n  baz()")]);
        assert_eq!(r.content, "    bar()\n      baz()\n");
    }

    #[test]
    fn failure_is_reported() {
        let e = block("nope", "x");
        let r = apply_edits("a\n", std::slice::from_ref(&e));
        assert_eq!(r.content, "a\n");
        assert_eq!(r.failed, vec![e]);
    }

    #[test]
    fn empty_search_fills_empty_file() {
        assert_eq!(apply_edits("", &[block("", "x\n")]).content, "x");
    }

    #[test]
    fn format() {
        assert_eq!(
            format_edits(&[block("a", "b")]),
            "<<<<<<< SEARCH\na\n=======\nb\n>>>>>>> REPLACE"
        );
    }
}
