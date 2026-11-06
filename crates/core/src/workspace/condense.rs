//! Condenses large HTML / SVG files for the model (port of `fs/condense.ts`): embedded data the
//! model can't use (base64 images and fonts, minified scripts and styles, long SVG paths, JSON
//! blobs) is replaced by short placeholders. Structure, text, classes and readable code stay.
//! The file on disk is never changed; condensed copies are sent as read-only references.
//!
//! The regexes are the TypeScript ones rewritten for the `regex` crate (no backreferences,
//! ASCII `\w`/`\b`, JavaScript's `\s`, ASCII-only case folding), and lengths are counted in
//! UTF-16 code units like `string.length`, so placeholders read exactly as before.
//!
//! This module also holds the small JavaScript-compatibility helpers the port needs elsewhere.

use std::cmp::Ordering;
use std::sync::LazyLock;

use regex::{Captures, Regex};

/// Files at least this large are condensed (smaller ones are cheap enough as they are).
pub const CONDENSE_MIN_BYTES: u64 = 32 * 1024;

/// `/\.(html?|svg)$/i`.
pub fn is_condensable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".htm") || lower.ends_with(".html") || lower.ends_with(".svg")
}

// ---------------------------------------------------------------------------------------------
// JavaScript compatibility
// ---------------------------------------------------------------------------------------------

/// JavaScript's `\s` / `String.prototype.trim` whitespace (includes U+FEFF, excludes U+0085).
pub(crate) fn is_js_whitespace(c: char) -> bool {
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

/// `String.prototype.trim`.
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// `string.length` (UTF-16 code units).
pub(crate) fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Default `Array.prototype.sort` order (UTF-16 code units).
pub(crate) fn js_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `Math.round` for non-negative values.
pub(crate) fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

// ---------------------------------------------------------------------------------------------
// Condensing
// ---------------------------------------------------------------------------------------------

/// JavaScript's `\s` as a regex class body.
const JS_WS: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// `[sS][cC]…` — JavaScript's non-Unicode `i` flag only folds ASCII letters.
fn ascii_ci(word: &str) -> String {
    word.chars()
        .map(|c| format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase()))
        .collect()
}

static DATA_URI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"data:([0-9A-Za-z_/+.\-]+)?(;[0-9A-Za-z_=\-]+)*;base64,[A-Za-z0-9+/={JS_WS}]{{200,}}"
    ))
    .expect("data URI regex")
});

fn block_regex(tag: &str) -> Regex {
    let t = ascii_ci(tag);
    Regex::new(&format!(r"(<{t}(?-u:\b)[^>]*>)((?s:.)*?)(</{t}>)")).expect("block regex")
}

static SCRIPT: LazyLock<Regex> = LazyLock::new(|| block_regex("script"));
static STYLE: LazyLock<Regex> = LazyLock::new(|| block_regex("style"));
static JSON_TYPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"type=["']?application/(ld\+)?json"#).expect("json type regex"));
static SVG_GEOMETRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?-u:\b)(d|points)=(?:"([^"']{300,})"|'([^"']{300,})')"#)
        .expect("svg geometry regex")
});

fn kb(n: usize) -> String {
    format!("{} KB", js_round(n as f64 / 1024.0).max(1.0))
}

/// Minified code: few line breaks relative to its length.
fn looks_minified(code: &str) -> bool {
    let lines = code
        .split('\n')
        .filter(|l| !js_trim(l).is_empty())
        .count()
        .max(1);
    let len = js_len(code);
    len > 2000 && len as f64 / lines as f64 > 250.0
}

pub fn condense(text: &str) -> String {
    // data: URIs (images, fonts) in attributes and CSS url(...).
    let out = DATA_URI.replace_all(text, |c: &Captures<'_>| {
        let mime = c.get(1).map_or("application/octet-stream", |m| m.as_str());
        format!("data:{mime};base64,[{} omitted]", kb(js_len(&c[0])))
    });

    // <script> blocks: minified code and large JSON payloads.
    let out = SCRIPT.replace_all(&out, |c: &Captures<'_>| {
        let (open, body, close) = (&c[1], &c[2], &c[3]);
        let json = JSON_TYPE.is_match(&open.to_ascii_lowercase());
        if (json && js_len(body) > 5000) || looks_minified(body) {
            let what = if json { "JSON data" } else { "minified script" };
            format!("{open}/* {} of {what} omitted */{close}", kb(js_len(body)))
        } else {
            c[0].to_string()
        }
    });

    // <style> blocks with minified CSS.
    let out = STYLE.replace_all(&out, |c: &Captures<'_>| {
        let (open, body, close) = (&c[1], &c[2], &c[3]);
        if looks_minified(body) {
            format!(
                "{open}/* {} of minified CSS omitted */{close}",
                kb(js_len(body))
            )
        } else {
            c[0].to_string()
        }
    });

    // Long SVG geometry.
    let out = SVG_GEOMETRY.replace_all(&out, |c: &Captures<'_>| {
        let attr = &c[1];
        let (q, v) = match c.get(2) {
            Some(v) => ('"', v.as_str()),
            None => ('\'', c.get(3).map_or("", |m| m.as_str())),
        };
        format!("{attr}={q}[{} chars of path data omitted]{q}", js_len(v))
    });

    out.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_helpers() {
        assert_eq!(js_trim("\u{FEFF} a \u{3000}"), "a");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
        assert_eq!(js_len("a😀"), 3);
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(0.49), 0.0);
        assert_eq!(js_cmp("\u{FF5E}", "😀"), Ordering::Greater);
    }

    #[test]
    fn condensable_names() {
        assert!(
            is_condensable("a/Index.HTML") && is_condensable("x.htm") && is_condensable("i.svg")
        );
        assert!(!is_condensable("x.svgz") && !is_condensable("html"));
    }

    #[test]
    fn data_uris() {
        let b64 = "A".repeat(3000);
        let src =
            format!(r#"<img src="data:image/png;base64,{b64}"> url(data:;charset=x;base64,{b64})"#);
        assert_eq!(
            condense(&src),
            r#"<img src="data:image/png;base64,[3 KB omitted]"> url(data:application/octet-stream;base64,[3 KB omitted])"#
        );
        // Short data URIs stay.
        let short = format!("data:image/png;base64,{}", "A".repeat(199));
        assert_eq!(condense(&short), short);
    }

    #[test]
    fn scripts_and_styles() {
        let min = "a=1;".repeat(1000);
        let src = format!(
            "<SCRIPT type=module>{min}</script><style>{min}</style><script>\nlet a = 1\n</script>"
        );
        assert_eq!(
            condense(&src),
            "<SCRIPT type=module>/* 4 KB of minified script omitted */</script><style>/* 4 KB of minified CSS omitted */</style><script>\nlet a = 1\n</script>"
        );
        let json = format!(
            "<script type=\"application/ld+json\">{}</script>",
            "{\"a\": 1}\n".repeat(700)
        );
        assert_eq!(
            condense(&json),
            "<script type=\"application/ld+json\">/* 6 KB of JSON data omitted */</script>"
        );
        // `<scripts>` is not a script tag.
        let not = format!("<scripts>{min}</script>");
        assert_eq!(condense(&not), not);
    }

    #[test]
    fn svg_geometry() {
        let d = "M0 0L1 1".repeat(50);
        assert_eq!(
            condense(&format!("<path d=\"{d}\"/>")),
            "<path d=\"[400 chars of path data omitted]\"/>"
        );
        assert_eq!(
            condense(&format!("<p points='{d}'/>")),
            "<p points='[400 chars of path data omitted]'/>"
        );
        let mixed = format!("<path d=\"{d}'/>");
        assert_eq!(condense(&mixed), mixed);
        let short = format!("<path d=\"{}\"/>", "M".repeat(299));
        assert_eq!(condense(&short), short);
        let word = format!("<path xd=\"{d}\"/>");
        assert_eq!(condense(&word), word);
    }
}
