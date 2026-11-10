//! Number and text formatting helpers (`lib/format.ts`).

use super::js;

/// 1.5k, 12k, 940 — for trees and lists.
pub fn fmt_k(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let digits = if n >= 10_000 { 0 } else { 1 };
    let s = js::to_fixed(n as f64 / 1000.0, digits);
    format!("{}k", s.strip_suffix(".0").unwrap_or(&s))
}

/// 18,400 — for the context bar (`toLocaleString('en-US')`).
pub fn fmt_n(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Strips the common leading indentation (spaces) of a multi-line literal, dropping one leading
/// newline and all trailing whitespace.
pub fn dedent(s: &str) -> String {
    let body = js::trim_end(s.strip_prefix('\n').unwrap_or(s));
    let lines: Vec<&str> = body.split('\n').collect();
    let min = lines
        .iter()
        .filter(|l| !js::trim(l).is_empty())
        .map(|l| l.len() - l.trim_start_matches(' ').len())
        .min();
    lines
        .iter()
        .map(|l| match min {
            // Lines shorter than `min` are whitespace-only (BMP), so chars == UTF-16 units.
            Some(min) => l.chars().skip(min).collect::<String>(),
            None => String::new(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Directory (with trailing `/`) and file name of a `/`-separated path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitPath<'a> {
    pub dir: &'a str,
    pub name: &'a str,
}

pub fn split_path(path: &str) -> SplitPath<'_> {
    let i = path.rfind('/').map_or(0, |i| i + 1);
    SplitPath {
        dir: &path[..i],
        name: &path[i..],
    }
}

/// Incremental equivalent of `estimate_tokens`, without retaining or rescanning the text.
/// Count characters before rounding so the estimate does not depend on stream chunk sizes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextTokenEstimate {
    utf16_units: u64,
}

impl TextTokenEstimate {
    pub fn push(&mut self, text: &str) {
        self.utf16_units += js::len16(text) as u64;
    }

    pub fn tokens(self) -> u64 {
        // Math.round(len / 4)
        ((self.utf16_units + 2) / 4).max(1)
    }
}

/// Rough token estimate for text the app hasn't counted yet (about 4 UTF-16 characters per token).
pub fn estimate_tokens(text: &str) -> u64 {
    let mut estimate = TextTokenEstimate::default();
    estimate.push(text);
    estimate.tokens()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k() {
        assert_eq!(fmt_k(999), "999");
        assert_eq!(fmt_k(1000), "1k");
        assert_eq!(fmt_k(1250), "1.3k");
        assert_eq!(fmt_k(12_500), "13k");
    }

    #[test]
    fn n() {
        assert_eq!(fmt_n(0), "0");
        assert_eq!(fmt_n(1000), "1,000");
        assert_eq!(fmt_n(1_234_567), "1,234,567");
    }

    #[test]
    fn dedents() {
        assert_eq!(dedent("\n  a\n    b\n\n  c\n  "), "a\n  b\n\nc");
        assert_eq!(dedent("\n \n"), "");
    }

    #[test]
    fn streaming_estimate_matches_whole_text_at_every_character_boundary() {
        let mut estimate = TextTokenEstimate::default();
        let mut text = String::new();
        for character in "Small chunks, Unicode 😀, and code: fn main() {}".chars() {
            let mut buffer = [0; 4];
            estimate.push(character.encode_utf8(&mut buffer));
            text.push(character);
            assert_eq!(estimate.tokens(), estimate_tokens(&text));
        }
    }

    #[test]
    fn paths_and_tokens() {
        assert_eq!(
            split_path("a/b/c.ts"),
            SplitPath {
                dir: "a/b/",
                name: "c.ts"
            }
        );
        assert_eq!(
            split_path("c.ts"),
            SplitPath {
                dir: "",
                name: "c.ts"
            }
        );
        assert_eq!(estimate_tokens(""), 1);
        assert_eq!(estimate_tokens("abcdef"), 2);
        assert_eq!(estimate_tokens("abcde"), 1);
    }
}
