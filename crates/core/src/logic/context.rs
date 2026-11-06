//! Context budget for the next request: what the context bar shows (ported from the totals in
//! `AppProvider.tsx`).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::defaults::{EST_OUTPUT_TOKENS, USER_MESSAGE_TOKENS};
use super::format::{estimate_tokens, fmt_k};
use super::prompt::{ApiContent, content_images, content_text, system_prompt};
use super::tree::{Selection, biggest_folder, node_stats};
use super::types::{Deployment, NodeKind, Rule, TreeNode};

/// Tokens counted for a picture already in the conversation.
pub const PICTURE_TOKENS: u64 = 765;
/// Context limit when the deployment's isn't known.
pub const FALLBACK_CONTEXT_LIMIT: u64 = 128_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub files: usize,
    pub rules: usize,
    pub file_tokens: u64,
    pub rule_tokens: u64,
    /// Estimated input tokens of the next request (everything the model reads), rounded to 100.
    pub tokens: u64,
    /// Of those, the part that is new: files not yet sent, pictures, the message.
    pub new_tokens: u64,
    /// Of those, the earlier conversation (re-read by the model, often from cache).
    pub conversation_tokens: u64,
    /// Share of the deployment's context limit, 0–1+.
    pub share: f64,
    pub level: Level,
    /// Estimated cost of the next message in USD.
    pub cost: f64,
    /// The next message would be billed at the model's long-context rates.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub long_context: bool,
    /// Where the bulk comes from, when near or over the limit (`"Infrastructure: 31k"`).
    pub hint: Option<String>,
}

/// Everything the totals depend on.
pub struct ContextInput<'a> {
    pub tree: &'a [TreeNode],
    pub selected: &'a Selection,
    /// The rules that are switched on.
    pub rules_on: &'a [Rule],
    pub output_format: &'a str,
    /// The conversation as sent so far.
    pub transcript: &'a [ApiContent],
    /// Version (hash) of each file the model has, by path.
    pub file_versions: &'a HashMap<String, String>,
    /// Files changed on disk since they were sent.
    pub stale: &'a HashSet<String>,
    /// "Resend full context": count every selected file as new.
    pub resend_all: bool,
    /// Tokens of the pictures attached to the next message.
    pub picture_tokens: u64,
    pub deployment: &'a Deployment,
}

fn round_100(n: u64) -> u64 {
    // Math.round(n / 100) * 100 for non-negative n.
    (n + 50) / 100 * 100
}

fn file_tokens(nodes: &[TreeNode], out: &mut HashMap<String, u64>) {
    for n in nodes {
        match n.kind {
            NodeKind::File => {
                out.insert(n.path.clone(), n.tokens.unwrap_or(0));
            }
            NodeKind::Dir => file_tokens(n.children.as_deref().unwrap_or_default(), out),
        }
    }
}

pub fn totals(input: &ContextInput<'_>) -> Totals {
    let (mut files, mut file_tok) = (0, 0);
    for n in input.tree {
        let s = node_stats(n, input.selected);
        files += s.selected;
        file_tok += s.selected_tokens;
    }
    let rule_tokens = input.rules_on.iter().map(|r| r.tokens).sum();
    let conversation_tokens: u64 = input
        .transcript
        .iter()
        .map(|c| estimate_tokens(&content_text(c)) + content_images(c) as u64 * PICTURE_TOKENS)
        .sum();
    let mut sizes = HashMap::new();
    file_tokens(input.tree, &mut sizes);
    let new_file_tokens: u64 = input
        .selected
        .iter()
        .filter(|p| {
            input.resend_all || !input.file_versions.contains_key(*p) || input.stale.contains(*p)
        })
        .map(|p| sizes.get(p).copied().unwrap_or(0))
        .sum();
    let new_tokens = new_file_tokens + input.picture_tokens + USER_MESSAGE_TOKENS;
    let raw = estimate_tokens(&system_prompt(input.rules_on, input.output_format))
        + conversation_tokens
        + new_tokens;
    let tokens = round_100(raw);
    let limit = input.deployment.limit.max(1);
    let share = tokens as f64 / limit as f64;
    let level = if share >= 0.95 {
        Level::Danger
    } else if share >= 0.75 {
        Level::Warn
    } else {
        Level::Ok
    };
    let cost = input.deployment.cost(tokens, 0, EST_OUTPUT_TOKENS);
    let long_context = input.deployment.rates(tokens).long_context;
    let hint = (level != Level::Ok)
        .then(|| biggest_folder(input.tree, input.selected))
        .flatten()
        .map(|b| format!("{}: {}", b.name, fmt_k(b.tokens)));
    Totals {
        files,
        rules: input.rules_on.len(),
        file_tokens: file_tok,
        rule_tokens,
        tokens,
        new_tokens: round_100(new_tokens),
        conversation_tokens,
        share,
        level,
        cost,
        long_context,
        hint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::types::RuleType;

    fn file(path: &str, tokens: u64) -> TreeNode {
        serde_json::from_value(
            serde_json::json!({"name": path, "path": path, "kind": "file", "tokens": tokens}),
        )
        .unwrap()
    }

    #[test]
    fn counts_levels_and_cost() {
        let tree = vec![file("a.rs", 60_000), file("b.rs", 1_000)];
        let selected: Selection = ["a.rs".to_string()].into();
        let rules = vec![Rule {
            path: "AGENTS.md".into(),
            kind: RuleType::Rule,
            tokens: 50,
            content: "Be nice.".into(),
        }];
        let dep = Deployment {
            name: "d".into(),
            limit: 64_000,
            in_price: 1.0,
            out_price: 4.0,
            ..Deployment::default()
        };
        let (fv, stale) = (HashMap::new(), HashSet::new());
        let input = ContextInput {
            tree: &tree,
            selected: &selected,
            rules_on: &rules,
            output_format: "",
            transcript: &[],
            file_versions: &fv,
            stale: &stale,
            resend_all: false,
            picture_tokens: 0,
            deployment: &dep,
        };
        let t = totals(&input);
        assert_eq!((t.files, t.rules, t.file_tokens), (1, 1, 60_000));
        assert_eq!(t.tokens % 100, 0);
        assert!(t.tokens > 60_000);
        assert_eq!(t.level, Level::Warn);
        assert!(
            (t.cost - (t.tokens as f64 / 1e6 + EST_OUTPUT_TOKENS as f64 / 1e6 * 4.0)).abs() < 1e-9
        );

        // Already sent and unchanged: not new.
        let fv: HashMap<_, _> = [("a.rs".to_string(), "h".to_string())].into();
        let t2 = totals(&ContextInput {
            file_versions: &fv,
            ..input
        });
        assert!(t2.new_tokens < 1_000);
    }

    #[test]
    fn rounding_matches_js() {
        assert_eq!(round_100(149), 100);
        assert_eq!(round_100(150), 200);
        assert_eq!(round_100(0), 0);
    }
}
