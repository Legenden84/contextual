//! Line diff (LCS) with word-level ranges and hunk collapsing, GitHub-style (`lib/diff.ts`).

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::js;
use super::types::ChangeStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OpKind {
    #[serde(rename = " ")]
    Same,
    #[serde(rename = "+")]
    Add,
    #[serde(rename = "-")]
    Del,
}

impl OpKind {
    /// The unified-diff prefix: ` `, `+` or `-`.
    pub fn sign(self) -> char {
        match self {
            Self::Same => ' ',
            Self::Add => '+',
            Self::Del => '-',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffOp {
    pub k: OpKind,
    /// 1-based line number in the old file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a: Option<usize>,
    /// 1-based line number in the new file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<usize>,
    pub t: String,
    /// `[start, end)` of the changed characters when paired with a counterpart line.
    ///
    /// Unlike the TS (UTF-16 indices), these are **UTF-8 byte offsets into `t`**, always on
    /// character boundaries, so `&t[start..end]` is safe. When the TS range would split a
    /// surrogate pair (two emoji sharing a high surrogate), the range is widened to the whole
    /// character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub ops: Vec<DiffOp>,
    pub adds: usize,
    pub dels: usize,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
}

/// A visible hunk (changes plus context) or a collapsible run of unchanged lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DiffItem {
    Hunk { lines: Vec<DiffOp> },
    Gap { lines: Vec<DiffOp> },
}

impl DiffItem {
    pub fn lines(&self) -> &[DiffOp] {
        match self {
            Self::Hunk { lines } | Self::Gap { lines } => lines,
        }
    }
}

/// At most 4 MB for an exact LCS table. Large inputs first shed their identical edges;
/// an oversized remaining region is represented as removals followed by additions.
const MAX_LCS_CELLS: usize = 1_000_000;

fn fits_lcs_budget(n: usize, m: usize) -> bool {
    n.checked_add(1)
        .zip(m.checked_add(1))
        .and_then(|(n, m)| n.checked_mul(m))
        .is_some_and(|cells| cells <= MAX_LCS_CELLS)
}

fn same_line(a: &[String], ai: usize, bi: usize) -> DiffOp {
    DiffOp {
        k: OpKind::Same,
        a: Some(ai + 1),
        b: Some(bi + 1),
        t: a[ai].clone(),
        r: None,
    }
}

fn lcs_ops(a: &[String], b: &[String]) -> Vec<DiffOp> {
    // Preserve the original matching/tie-breaking for ordinary files and fixture parity.
    if fits_lcs_budget(a.len(), b.len()) {
        return exact_lcs_ops(a, b);
    }

    let prefix = a.iter().zip(b).take_while(|(a, b)| a == b).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let a_end = a.len() - suffix;
    let b_end = b.len() - suffix;
    let (middle_a, middle_b) = (&a[prefix..a_end], &b[prefix..b_end]);
    let mut out = Vec::new();
    out.extend((0..prefix).map(|i| same_line(a, i, i)));

    if fits_lcs_budget(middle_a.len(), middle_b.len()) {
        out.extend(exact_lcs_ops(middle_a, middle_b).into_iter().map(|mut op| {
            op.a = op.a.map(|n| n + prefix);
            op.b = op.b.map(|n| n + prefix);
            op
        }));
    } else {
        // Exact contents and line numbers, but deliberately not necessarily a minimal diff.
        // This bounds both table memory and comparison work for unrelated large files.
        out.extend((prefix..a_end).map(|i| DiffOp {
            k: OpKind::Del,
            a: Some(i + 1),
            b: None,
            t: a[i].clone(),
            r: None,
        }));
        out.extend((prefix..b_end).map(|i| DiffOp {
            k: OpKind::Add,
            a: None,
            b: Some(i + 1),
            t: b[i].clone(),
            r: None,
        }));
    }

    out.extend((0..suffix).map(|i| same_line(a, a_end + i, b_end + i)));
    out
}

fn exact_lcs_ops(a: &[String], b: &[String]) -> Vec<DiffOp> {
    let (n, m) = (a.len(), b.len());
    let w = m + 1;
    let mut dp = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if a[i] == b[j] {
                dp[(i + 1) * w + j + 1] + 1
            } else {
                dp[(i + 1) * w + j].max(dp[i * w + j + 1])
            };
        }
    }

    let mut ops = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            ops.push(DiffOp {
                k: OpKind::Same,
                a: Some(i + 1),
                b: Some(j + 1),
                t: a[i].clone(),
                r: None,
            });
            i += 1;
            j += 1;
        } else if j < m && (i >= n || dp[i * w + j + 1] >= dp[(i + 1) * w + j]) {
            ops.push(DiffOp {
                k: OpKind::Add,
                a: None,
                b: Some(j + 1),
                t: b[j].clone(),
                r: None,
            });
            j += 1;
        } else {
            ops.push(DiffOp {
                k: OpKind::Del,
                a: Some(i + 1),
                b: None,
                t: a[i].clone(),
                r: None,
            });
            i += 1;
        }
    }

    // Inside each change run, put removals before additions.
    let mut out = Vec::with_capacity(ops.len());
    let mut run: Vec<DiffOp> = Vec::new();
    let flush = |run: &mut Vec<DiffOp>, out: &mut Vec<DiffOp>| {
        let (dels, adds): (Vec<_>, Vec<_>) = run.drain(..).partition(|o| o.k == OpKind::Del);
        out.extend(dels);
        out.extend(adds);
    };
    for o in ops {
        if o.k == OpKind::Same {
            flush(&mut run, &mut out);
            out.push(o);
        } else {
            run.push(o);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Pairs removed/added lines and marks the changed span when the lines are similar enough.
fn word_ranges(ops: &mut [DiffOp]) {
    let mut i = 0;
    while i < ops.len() {
        if ops[i].k != OpKind::Del {
            i += 1;
            continue;
        }
        let mut d = i;
        while d < ops.len() && ops[d].k == OpKind::Del {
            d += 1;
        }
        let mut p = d;
        while p < ops.len() && ops[p].k == OpKind::Add {
            p += 1;
        }
        for q in 0..(d - i).min(p - d) {
            let (del, add) = (i + q, d + q);
            if let Some((rx, ry)) = changed_span(&ops[del].t, &ops[add].t) {
                ops[del].r = Some(rx);
                ops[add].r = Some(ry);
            }
        }
        i = p;
    }
}

/// The changed spans of a removed/added pair (byte offsets), or `None` when too dissimilar.
/// Compared in UTF-16 code units so the similarity decision is the same as in the TS.
fn changed_span(x: &str, y: &str) -> Option<((usize, usize), (usize, usize))> {
    let xs: Vec<u16> = x.encode_utf16().collect();
    let ys: Vec<u16> = y.encode_utf16().collect();
    let mut pre = 0;
    while pre < xs.len() && pre < ys.len() && xs[pre] == ys[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < xs.len() - pre
        && suf < ys.len() - pre
        && xs[xs.len() - 1 - suf] == ys[ys.len() - 1 - suf]
    {
        suf += 1;
    }
    let common = (pre + suf) as f64;
    let longest = js::len16(js::trim(x)).max(js::len16(js::trim(y))) as f64;
    let leading = (xs.len() - js::len16(js::trim_start(x))) as f64;
    if common - leading <= longest * 0.35 {
        return None;
    }
    let span = |s: &str, end: usize| {
        (
            js::byte_at_utf16(s, pre, false),
            js::byte_at_utf16(s, end, true),
        )
    };
    Some((span(x, xs.len() - suf), span(y, ys.len() - suf)))
}

/// Splits text into lines; a final newline ends the last line, it isn't an extra empty line.
fn split_lines(t: &str) -> Vec<String> {
    if t.is_empty() {
        return Vec::new();
    }
    t.strip_suffix('\n')
        .unwrap_or(t)
        .split('\n')
        .map(str::to_owned)
        .collect()
}

pub fn compute_diff(old_text: &str, new_text: &str) -> Diff {
    let a = split_lines(old_text);
    let b = split_lines(new_text);
    let mut ops = lcs_ops(&a, &b);
    word_ranges(&mut ops);
    Diff {
        adds: ops.iter().filter(|o| o.k == OpKind::Add).count(),
        dels: ops.iter().filter(|o| o.k == OpKind::Del).count(),
        ops,
        old_lines: a,
        new_lines: b,
    }
}

/// Groups ops into visible hunks (changes plus `context` lines around them) and collapsible gaps.
/// The TS default for `context` is 3.
pub fn make_hunks(ops: &[DiffOp], context: usize) -> Vec<DiffItem> {
    let mut keep = vec![false; ops.len()];
    for (i, o) in ops.iter().enumerate() {
        if o.k == OpKind::Same {
            continue;
        }
        let hi = (i + context).min(ops.len() - 1);
        for k in keep.iter_mut().take(hi + 1).skip(i.saturating_sub(context)) {
            *k = true;
        }
    }
    let mut items = Vec::new();
    let mut i = 0;
    while i < ops.len() {
        let hunk = keep[i];
        let start = i;
        while i < ops.len() && keep[i] == hunk {
            i += 1;
        }
        let lines = ops[start..i].to_vec();
        items.push(if hunk {
            DiffItem::Hunk { lines }
        } else {
            DiffItem::Gap { lines }
        });
    }
    items
}

static MEMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&js::pattern(
        r"^{s}*(?:public|internal|private|protected)(?-u:\b){dot}*(?-u:\b)(?:class|record|struct|interface|void|Task|static)(?-u:\b)",
    ))
    .expect("MEMBER regex")
});

/// `@@ -a,n +b,m @@ enclosing member` for a hunk.
pub fn hunk_header(lines: &[DiffOp], new_lines: &[String]) -> String {
    let (mut a_start, mut b_start, mut a_count, mut b_count) = (0, 0, 0, 0);
    for o in lines {
        if let Some(a) = o.a.filter(|&a| a != 0) {
            if a_start == 0 {
                a_start = a;
            }
            a_count += 1;
        }
        if let Some(b) = o.b.filter(|&b| b != 0) {
            if b_start == 0 {
                b_start = b;
            }
            b_count += 1;
        }
    }
    let from = lines
        .first()
        .and_then(|o| o.b)
        .filter(|&b| b != 0)
        .unwrap_or(if b_start != 0 { b_start } else { 1 });
    let member = (0..from.saturating_sub(1))
        .rev()
        .filter_map(|k| new_lines.get(k))
        .find(|l| MEMBER.is_match(l))
        .map(|l| format!(" {}", js::trim(l)))
        .unwrap_or_default();
    format!("@@ -{a_start},{a_count} +{b_start},{b_count} @@{member}")
}

/// A plain-text unified diff, for "Copy diff".
pub fn unified_patch(path: &str, diff: &Diff, status: ChangeStatus) -> String {
    let from = if status == ChangeStatus::A {
        "/dev/null".to_owned()
    } else {
        format!("a/{path}")
    };
    let to = if status == ChangeStatus::D {
        "/dev/null".to_owned()
    } else {
        format!("b/{path}")
    };
    let mut out = vec![format!("--- {from}"), format!("+++ {to}")];
    for item in make_hunks(&diff.ops, 3) {
        let DiffItem::Hunk { lines } = item else {
            continue;
        };
        out.push(hunk_header(&lines, &diff.new_lines));
        out.extend(lines.iter().map(|o| format!("{}{}", o.k.sign(), o.t)));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_range_is_bytes() {
        let d = compute_diff("let s = \"héllo\";\n", "let s = \"héllo wörld\";\n");
        let add = &d.ops[1];
        let (s, e) = add.r.unwrap();
        assert_eq!(&add.t[s..e], " wörld");
    }

    #[test]
    fn shared_surrogate_widens_to_char() {
        let d = compute_diff("face 😀 end\n", "face 😁 end\n");
        let (s, e) = d.ops[0].r.unwrap();
        assert_eq!(&d.ops[0].t[s..e], "😀");
    }

    #[test]
    fn dels_before_adds_and_counts() {
        let d = compute_diff("a\nb\nc\n", "a\nx\ny\nc\n");
        let kinds: Vec<char> = d.ops.iter().map(|o| o.k.sign()).collect();
        assert_eq!(kinds, [' ', '-', '+', '+', ' ']);
        assert_eq!((d.adds, d.dels), (2, 1));
    }

    #[test]
    fn header_finds_member() {
        let new = ["public class A", "{", "  x", "  y", "  z", "  w"].map(String::from);
        let d = compute_diff(
            "public class A\n{\n  x\n  y\n  z\n  v\n",
            &(new.join("\n") + "\n"),
        );
        let hunks = make_hunks(&d.ops, 1);
        let DiffItem::Hunk { lines } = hunks.last().unwrap() else {
            panic!()
        };
        assert_eq!(hunk_header(lines, &new), "@@ -5,2 +5,2 @@ public class A");
    }

    fn assert_reconstructs(diff: &Diff) {
        let old: Vec<_> = diff.ops.iter().filter(|op| op.k != OpKind::Add).collect();
        let new: Vec<_> = diff.ops.iter().filter(|op| op.k != OpKind::Del).collect();
        assert_eq!(old.len(), diff.old_lines.len());
        assert_eq!(new.len(), diff.new_lines.len());
        for (i, op) in old.iter().enumerate() {
            assert_eq!(op.a, Some(i + 1));
            assert_eq!(op.t, diff.old_lines[i]);
        }
        for (i, op) in new.iter().enumerate() {
            assert_eq!(op.b, Some(i + 1));
            assert_eq!(op.t, diff.new_lines[i]);
        }
    }

    #[test]
    fn large_local_edit_keeps_unchanged_edges() {
        let old: Vec<_> = (0..10_000).map(|i| format!("line {i}")).collect();
        let mut new = old.clone();
        new[5_000] = "changed".into();
        let diff = compute_diff(&old.join("\n"), &new.join("\n"));
        assert_eq!((diff.adds, diff.dels), (1, 1));
        assert_reconstructs(&diff);
    }

    #[test]
    fn oversized_replacement_is_complete_and_numbered() {
        let old = format!("prefix\n{}suffix\n", "old\n".repeat(1_500));
        let new = format!("prefix\n{}suffix\n", "new\n".repeat(1_600));
        let diff = compute_diff(&old, &new);
        assert_eq!((diff.adds, diff.dels), (1_600, 1_500));
        assert_eq!(diff.ops.first().unwrap().k, OpKind::Same);
        assert_eq!(diff.ops.last().unwrap().k, OpKind::Same);
        assert_reconstructs(&diff);
    }

    #[test]
    fn large_identical_and_one_sided_inputs() {
        let text = "same\n".repeat(10_000);
        let identical = compute_diff(&text, &text);
        assert_eq!((identical.adds, identical.dels), (0, 0));
        assert_reconstructs(&identical);
        assert_reconstructs(&compute_diff("", &text));
        assert_reconstructs(&compute_diff(&text, ""));
        assert!(!fits_lcs_budget(usize::MAX, 1));
        assert!(!fits_lcs_budget(10_000, 10_000));
    }

    #[test]
    fn empty_inputs() {
        let d = compute_diff("", "");
        assert!(d.ops.is_empty());
        assert!(make_hunks(&d.ops, 3).is_empty());
        assert_eq!(unified_patch("x", &d, ChangeStatus::M), "--- a/x\n+++ b/x");
    }
}
