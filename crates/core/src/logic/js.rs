//! JavaScript string and number semantics that the port has to reproduce.
//!
//! JS `\s` and `trim()` use a different whitespace set than Rust (`\u{FEFF}` is whitespace in JS,
//! `\u{85}` is not), JS `.` excludes all line terminators, and JS lengths are UTF-16 code units.

use std::fmt::Write as _;

/// Body of a regex character class equal to JS `\s` (and the set `String.prototype.trim` strips).
pub(crate) const WS: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// `.` in JS (without the `s` flag): anything except line terminators.
const DOT: &str = r"[^\n\r\x{2028}\x{2029}]";

/// Expands JS placeholders in a regex pattern: `{s}` = `\s`, `{S}` = `\S`, `{dot}` = `.`, and
/// `{ws}` = the bare `\s` set for use inside a character class.
pub(crate) fn pattern(p: &str) -> String {
    p.replace("{s}", &format!("[{WS}]"))
        .replace("{S}", &format!("[^{WS}]"))
        .replace("{dot}", DOT)
        .replace("{ws}", WS)
}

pub(crate) fn is_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `\w` in JS (ASCII only).
pub(crate) fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub(crate) fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

pub(crate) fn trim(s: &str) -> &str {
    s.trim_matches(is_ws)
}

pub(crate) fn trim_start(s: &str) -> &str {
    s.trim_start_matches(is_ws)
}

pub(crate) fn trim_end(s: &str) -> &str {
    s.trim_end_matches(is_ws)
}

/// `s.length` in JS.
pub(crate) fn len16(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// Byte offset of UTF-16 index `idx` in `s`. An index inside a surrogate pair snaps to the start
/// of that character, or to its end when `up` is set.
pub(crate) fn byte_at_utf16(s: &str, idx: usize, up: bool) -> usize {
    let mut units = 0;
    for (byte, c) in s.char_indices() {
        if units >= idx {
            return byte;
        }
        let next = units + c.len_utf16();
        if next > idx {
            return if up { byte + c.len_utf8() } else { byte };
        }
        units = next;
    }
    s.len()
}

/// `Math.round`: nearest integer, ties towards +∞.
pub(crate) fn round(x: f64) -> f64 {
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// `Number.prototype.toFixed` for finite `x` with `0 <= x < 1e21`: rounds the exact binary value,
/// ties away from zero (Rust's `{:.N}` rounds ties to even).
pub(crate) fn to_fixed(x: f64, digits: usize) -> String {
    // 1100 fractional digits print any f64 exactly, so the tie check below is exact.
    let exact = format!("{x:.1100}");
    let (int, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let mut kept: Vec<u8> = int.bytes().chain(frac.bytes().take(digits)).collect();
    if frac.as_bytes().get(digits).is_some_and(|d| *d >= b'5') {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let split = kept.len() - digits;
    let mut out = String::from_utf8(kept[..split].to_vec()).unwrap_or_default();
    if digits > 0 {
        let _ = write!(out, ".{}", String::from_utf8_lossy(&kept[split..]));
    }
    out
}

/// `Number.prototype.toString(36)` for non-negative integers.
pub(crate) fn radix36(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_matches_js() {
        assert_eq!(trim("\u{FEFF} x \u{A0}"), "x");
        assert_eq!(trim("\u{85}x"), "\u{85}x");
        let re = regex::Regex::new(&pattern("^{s}+$")).unwrap();
        assert!(re.is_match("\u{FEFF}\u{3000}\t"));
        assert!(!re.is_match("\u{85}"));
    }

    #[test]
    fn utf16() {
        assert_eq!(len16("a😀"), 3);
        assert_eq!(byte_at_utf16("a😀b", 2, false), 1);
        assert_eq!(byte_at_utf16("a😀b", 2, true), 5);
        assert_eq!(byte_at_utf16("a😀b", 3, false), 5);
        assert_eq!(byte_at_utf16("a😀b", 9, false), 6);
    }

    #[test]
    fn rounding() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(2.4999), 2.0);
        assert_eq!(to_fixed(1.25, 1), "1.3");
        assert_eq!(to_fixed(1.15, 1), "1.1"); // 1.15 is 1.1499999… as a double
        assert_eq!(to_fixed(12.5, 0), "13");
        assert_eq!(to_fixed(9.999, 1), "10.0");
        assert_eq!(to_fixed(0.0, 2), "0.00");
        assert_eq!(radix36(0), "0");
        assert_eq!(radix36(35), "z");
        assert_eq!(radix36(36), "10");
    }
}
