//! Replays the Phase 0 logic fixtures (recorded from `src/web/lib` by
//! `tests/fixtures/logic/generate.mjs`) against `contextual_core::logic`.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use contextual_core::logic::{
    changes::{ChangeContext, parse_changes},
    defaults, diff,
    edits::{self, EditBlock},
    format, foundry_status, git_tabs, hash, images, markdown, prompt,
    tree::{self, Selection},
    types::{ChangeStatus, Rule, TreeNode},
};
use pretty_assertions::assert_eq;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Case {
    name: String,
    #[serde(default)]
    r#fn: Option<String>,
    input: Value,
    output: Value,
}

fn load(name: &str) -> Vec<Case> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/logic")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let cases: Vec<Case> = serde_json::from_str(&text).expect("fixture JSON");
    assert!(!cases.is_empty());
    cases
}

/// JSON with every number as f64, so `1` and `1.0` compare equal.
fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().expect("number")),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, norm(v))).collect()),
        other => other,
    }
}

fn check(case: &Case, actual: impl serde::Serialize) {
    let actual = norm(serde_json::to_value(actual).expect("serialise"));
    assert_eq!(
        actual,
        norm(case.output.clone()),
        "case: {} {:?}",
        case.name,
        case.r#fn
    );
}

fn from<T: serde::de::DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).expect("fixture input")
}

fn str_of(v: &Value) -> &str {
    v.as_str().expect("string input")
}

// ------------------------------------------------------------------ diff

/// Byte offset → UTF-16 index (the unit the TS fixtures use).
fn utf16_at(s: &str, byte: usize) -> u64 {
    s[..byte].encode_utf16().count() as u64
}

/// Snaps a TS UTF-16 range outwards to whole characters, as the port does.
fn snap(s: &str, start: u64, end: u64) -> (u64, u64) {
    let mut bounds = vec![0u64];
    let mut u = 0;
    for c in s.chars() {
        u += c.len_utf16() as u64;
        bounds.push(u);
    }
    let lo = *bounds.iter().rev().find(|&&b| b <= start).unwrap_or(&0);
    let hi = *bounds.iter().find(|&&b| b >= end).unwrap_or(&u);
    (lo, hi)
}

/// The port's ops as TS JSON: `r` converted from bytes to UTF-16.
fn ops_json(ops: &[diff::DiffOp]) -> Value {
    Value::Array(
        ops.iter()
            .map(|o| {
                let mut v = serde_json::to_value(o).unwrap();
                if let Some((s, e)) = o.r {
                    v["r"] = json!([utf16_at(&o.t, s), utf16_at(&o.t, e)]);
                }
                v
            })
            .collect(),
    )
}

/// The TS ops with `r` snapped to character boundaries.
fn snap_ops(ops: &Value) -> Value {
    let mut ops = ops.clone();
    for o in ops.as_array_mut().unwrap() {
        if let Some(r) = o.get("r").cloned() {
            let t = o["t"].as_str().unwrap().to_owned();
            let (s, e) = snap(&t, r[0].as_u64().unwrap(), r[1].as_u64().unwrap());
            o["r"] = json!([s, e]);
        }
    }
    ops
}

fn items_json(items: &[diff::DiffItem]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| {
                let kind = if matches!(i, diff::DiffItem::Hunk { .. }) {
                    "hunk"
                } else {
                    "gap"
                };
                json!({ "kind": kind, "lines": ops_json(i.lines()) })
            })
            .collect(),
    )
}

fn snap_items(items: &Value) -> Value {
    let mut items = items.clone();
    for i in items.as_array_mut().unwrap() {
        i["lines"] = snap_ops(&i["lines"]);
    }
    items
}

#[test]
fn diff_fixtures() {
    for case in load("diff") {
        let (old, new) = (str_of(&case.input["old"]), str_of(&case.input["new"]));
        let d = diff::compute_diff(old, new);
        let out = &case.output;
        let ctx = |what: &str| format!("{}: {what}", case.name);

        assert_eq!(
            ops_json(&d.ops),
            snap_ops(&out["diff"]["ops"]),
            "{}",
            ctx("ops")
        );
        assert_eq!(json!(d.adds), out["diff"]["adds"], "{}", ctx("adds"));
        assert_eq!(json!(d.dels), out["diff"]["dels"], "{}", ctx("dels"));
        assert_eq!(
            json!(d.old_lines),
            out["diff"]["oldLines"],
            "{}",
            ctx("oldLines")
        );
        assert_eq!(
            json!(d.new_lines),
            out["diff"]["newLines"],
            "{}",
            ctx("newLines")
        );

        let hunks = diff::make_hunks(&d.ops, 3);
        assert_eq!(
            items_json(&hunks),
            snap_items(&out["hunks"]),
            "{}",
            ctx("hunks")
        );
        assert_eq!(
            items_json(&diff::make_hunks(&d.ops, 1)),
            snap_items(&out["hunks1"]),
            "{}",
            ctx("hunks1")
        );
        assert_eq!(
            items_json(&diff::make_hunks(&d.ops, 0)),
            snap_items(&out["hunks0"]),
            "{}",
            ctx("hunks0")
        );

        let headers: Vec<String> = hunks
            .iter()
            .filter_map(|h| match h {
                diff::DiffItem::Hunk { lines } => Some(diff::hunk_header(lines, &d.new_lines)),
                diff::DiffItem::Gap { .. } => None,
            })
            .collect();
        assert_eq!(json!(headers), out["headers"], "{}", ctx("headers"));
        for (key, status) in [
            ("patchM", ChangeStatus::M),
            ("patchA", ChangeStatus::A),
            ("patchD", ChangeStatus::D),
        ] {
            assert_eq!(
                diff::unified_patch("src/File.cs", &d, status),
                str_of(&out[key]),
                "{}",
                ctx(key)
            );
        }
    }
}

#[test]
fn diff_random_fixtures() {
    for case in load("diff_random") {
        let d = diff::compute_diff(str_of(&case.input["old"]), str_of(&case.input["new"]));
        let out = &case.output;
        assert_eq!(
            ops_json(&d.ops),
            snap_ops(&out["diff"]["ops"]),
            "{}: ops",
            case.name
        );
        assert_eq!(
            items_json(&diff::make_hunks(&d.ops, 2)),
            snap_items(&out["hunks"]),
            "{}: hunks",
            case.name
        );
        assert_eq!(
            diff::unified_patch("f", &d, ChangeStatus::M),
            str_of(&out["patch"]),
            "{}: patch",
            case.name
        );
    }
}

// ------------------------------------------------------------------ markdown

#[test]
fn markdown_random_fixtures() {
    for case in load("markdown_random") {
        check(&case, markdown::parse_markdown(str_of(&case.input)));
    }
}

#[test]
fn edits_random_fixtures() {
    for case in load("edits_random") {
        let blocks: Vec<EditBlock> = from(&case.input["edits"]);
        check(
            &case,
            edits::apply_edits(str_of(&case.input["original"]), &blocks),
        );
    }
}

#[test]
fn markdown_fixtures() {
    for case in load("markdown") {
        let src = str_of(&case.input);
        match case.r#fn.as_deref() {
            Some("parseMarkdown") => check(&case, markdown::parse_markdown(src)),
            Some("parseInline") => {
                let nodes = markdown::parse_inline(src);
                let text = markdown::inline_text(&nodes);
                check(&case, json!({ "nodes": nodes, "text": text }));
            }
            other => panic!("unknown fn {other:?}"),
        }
    }
}

// ------------------------------------------------------------------ edits

#[test]
fn edits_fixtures() {
    for case in load("edits") {
        let blocks: Vec<EditBlock> = from(&case.input["edits"]);
        match case.r#fn.as_deref() {
            Some("applyEdits") => check(
                &case,
                edits::apply_edits(str_of(&case.input["original"]), &blocks),
            ),
            Some("formatEdits") => check(&case, edits::format_edits(&blocks)),
            other => panic!("unknown fn {other:?}"),
        }
    }
}

// ------------------------------------------------------------------ changes

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureCtx {
    exists: Vec<String>,
    originals: BTreeMap<String, String>,
    changed_on_disk: Vec<String>,
    read_only: Vec<String>,
}

impl ChangeContext for FixtureCtx {
    fn exists(&self, path: &str) -> bool {
        self.exists.iter().any(|p| p == path)
    }
    fn original(&self, path: &str) -> Option<String> {
        self.originals.get(path).cloned()
    }
    fn changed_on_disk(&self, path: &str) -> bool {
        self.changed_on_disk.iter().any(|p| p == path)
    }
    fn read_only(&self, path: &str) -> bool {
        self.read_only.iter().any(|p| p == path)
    }
}

#[test]
fn changes_fixtures() {
    for case in load("changes") {
        let ctx: FixtureCtx = from(&case.input);
        check(&case, parse_changes(str_of(&case.input["response"]), &ctx));
    }
}

// ------------------------------------------------------------------ prompt

#[test]
fn prompt_fixtures() {
    for case in load("prompt") {
        let i = &case.input;
        match case.r#fn.as_deref() {
            Some("systemPrompt") => {
                let rules: Vec<Rule> = from(&i["rules"]);
                check(
                    &case,
                    prompt::system_prompt(&rules, str_of(&i["outputFormat"])),
                );
            }
            Some("contextBlock") => {
                let files: Vec<prompt::ContextFile> = from(&i["files"]);
                check(&case, prompt::context_block(&files));
            }
            Some("withImages") => {
                let images: Vec<prompt::ContextImage> = from(&i["images"]);
                check(
                    &case,
                    prompt::with_images(str_of(&i["text"]).to_owned(), &images),
                );
            }
            Some("buildTurn") => {
                let turn: prompt::TurnInput = from(i);
                let content = prompt::build_turn(&turn);
                let text = prompt::content_text(&content);
                let images = prompt::content_images(&content);
                check(
                    &case,
                    json!({ "content": content, "text": text, "images": images }),
                );
            }
            Some("const") => check(&case, prompt::REFERENCE_NOTE),
            other => panic!("unknown fn {other:?}"),
        }
    }
}

// ------------------------------------------------------------------ tree

fn all_nodes(nodes: &[TreeNode], out: &mut Vec<TreeNode>) {
    for n in nodes {
        out.push(n.clone());
        if let Some(kids) = &n.children {
            all_nodes(kids, out);
        }
    }
}

#[test]
fn tree_fixtures() {
    for case in load("tree") {
        let nodes: Vec<TreeNode> = from(&case.input["nodes"]);
        match case.r#fn.as_deref() {
            Some("pickableFiles") => {
                let mut p: Vec<String> = tree::pickable_files(&nodes).into_iter().collect();
                // JS sorts by UTF-16 code units; these paths are ASCII.
                p.sort();
                check(&case, p);
            }
            Some("selectableFiles") => check(&case, tree::selectable_files(&nodes)),
            Some("sortNodes") => check(
                &case,
                tree::sort_nodes(&nodes)
                    .iter()
                    .map(|n| &n.path)
                    .collect::<Vec<_>>(),
            ),
            Some("findNode") => check(
                &case,
                tree::find_node(&nodes, str_of(&case.input["path"])).map(|n| &n.path),
            ),
            Some("filterTree") => check(
                &case,
                tree::filter_tree(&nodes, str_of(&case.input["query"])),
            ),
            Some("selection") => {
                let selected: Selection = from::<HashSet<String>>(&case.input["selected"]);
                let mut every = Vec::new();
                all_nodes(&nodes, &mut every);
                let stats: BTreeMap<_, _> = every
                    .iter()
                    .map(|n| (n.path.clone(), tree::node_stats(n, &selected)))
                    .collect();
                let checks: BTreeMap<_, _> = every
                    .iter()
                    .map(|n| (n.path.clone(), tree::check_state(n, &selected)))
                    .collect();
                let overridable: Vec<_> = every
                    .iter()
                    .filter(|n| tree::is_overridable(n))
                    .map(|n| &n.path)
                    .collect();
                check(
                    &case,
                    json!({
                        "stats": stats,
                        "check": checks,
                        "overridable": overridable,
                        "biggest": tree::biggest_folder(&nodes, &selected),
                    }),
                );
            }
            other => panic!("unknown fn {other:?}"),
        }
    }
}

// ------------------------------------------------------------------ images

#[test]
fn images_fixtures() {
    for case in load("images") {
        let w = case.input["width"].as_u64().unwrap() as u32;
        let h = case.input["height"].as_u64().unwrap() as u32;
        let (sw, sh) = images::scaled_size(w, h);
        check(
            &case,
            json!({ "tokens": images::vision_tokens(w, h), "scaled": [sw, sh], "scaledTokens": images::vision_tokens(sw, sh) }),
        );
    }
}

// ------------------------------------------------------------------ small modules

#[test]
fn format_fixtures() {
    for case in load("format") {
        let i = &case.input;
        match case.r#fn.as_deref() {
            Some("hashText") => {
                let seed = i["seed"].as_i64().unwrap() as i32;
                check(&case, hash::hash_text_seeded(str_of(&i["text"]), seed));
                if seed == 0 {
                    check(&case, hash::hash_text(str_of(&i["text"])));
                }
            }
            Some("fmtK") => check(&case, format::fmt_k(i.as_u64().unwrap())),
            Some("fmtN") => check(&case, format::fmt_n(i.as_u64().unwrap())),
            Some("dedent") => check(&case, format::dedent(str_of(i))),
            Some("splitPath") => {
                let s = format::split_path(str_of(i));
                check(&case, json!({ "dir": s.dir, "name": s.name }));
            }
            Some("estimateTokens") => check(&case, format::estimate_tokens(str_of(i))),
            Some("gitTabId") => check(
                &case,
                git_tabs::git_tab_id(str_of(&i["path"]), i["staged"].as_bool().unwrap()),
            ),
            Some("parseGitTab") => check(&case, git_tabs::parse_git_tab(str_of(i))),
            Some("describeFoundry") => {
                let status: foundry_status::FoundryStatus = from(i);
                check(&case, foundry_status::describe_foundry(&status));
            }
            other => panic!("unknown fn {other:?}"),
        }
    }
}

#[test]
fn defaults_fixtures() {
    for case in load("defaults") {
        match case.name.as_str() {
            "outputPrompt" => check(&case, defaults::OUTPUT_PROMPT),
            "LEGACY_OUTPUT_PROMPTS" => check(&case, defaults::LEGACY_OUTPUT_PROMPTS),
            "defaultExclusions" => check(&case, defaults::DEFAULT_EXCLUSIONS),
            "PLACEHOLDER_DEPLOYMENT" => check(&case, defaults::placeholder_deployment()),
            "USER_MESSAGE_TOKENS" => check(&case, defaults::USER_MESSAGE_TOKENS),
            "EST_OUTPUT_TOKENS" => check(&case, defaults::EST_OUTPUT_TOKENS),
            "systemPrompt default" => {
                check(&case, prompt::system_prompt(&[], defaults::OUTPUT_PROMPT))
            }
            other => panic!("unknown case {other}"),
        }
    }
}
