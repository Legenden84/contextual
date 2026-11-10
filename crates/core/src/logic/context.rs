//! Context budget for the next request: what the context bar shows (ported from the totals in
//! `AppProvider.tsx`).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::defaults::{EST_OUTPUT_TOKENS, USER_MESSAGE_TOKENS};
use super::format::{TextTokenEstimate, estimate_tokens, fmt_k};
use super::prompt::{ApiContent, content_images, content_text, system_prompt};
use super::tree::{Selection, biggest_folder, node_stats};
use super::types::{Deployment, NodeKind, Rule, TreeNode};

/// Tokens counted for a picture already in the conversation.
pub const PICTURE_TOKENS: u64 = 765;
/// Context limit when the deployment's isn't known.
pub const FALLBACK_CONTEXT_LIMIT: u64 = 128_000;

/// Admission budget for an actual request, separate from the context bar's input estimate.
///
/// Until the catalog distinguishes input-only limits from combined context windows, reserve
/// the requested output conservatively in both cases. Ten percent of the limit is also left
/// unused for estimation uncertainty. This is a heuristic guard, not a provider token count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestBudget {
    limit: u64,
    output_reserve: u64,
}

impl RequestBudget {
    pub fn new(limit: u64, output_reserve: u64) -> Self {
        Self {
            limit,
            output_reserve,
        }
    }

    pub fn safety_margin(self) -> u64 {
        self.limit.div_ceil(10)
    }

    /// Maximum estimated input, including instructions, history and message framing.
    pub fn input_capacity(self) -> u64 {
        self.limit
            .saturating_sub(self.safety_margin())
            .saturating_sub(self.output_reserve)
    }

    /// Rejects locally without truncating input or changing any conversation state.
    pub fn check(self, input_tokens: u64) -> Result<(), BudgetExceeded> {
        let fits = self
            .output_reserve
            .checked_add(self.safety_margin())
            .and_then(|reserved| reserved.checked_add(input_tokens))
            .is_some_and(|total| self.limit > 0 && total <= self.limit);
        if fits {
            Ok(())
        } else {
            Err(BudgetExceeded {
                input_tokens,
                input_capacity: self.input_capacity(),
            })
        }
    }
}

/// A locally refused request, not an HTTP or network failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "Not sent: estimated input ({input_tokens} tokens) does not fit the usable input budget \
     ({input_capacity} tokens), after reserving output and an estimation margin. \
     Reduce the request or use a larger-context model. Earlier conversation still counts \
     even when files are deselected."
)]
pub struct BudgetExceeded {
    pub input_tokens: u64,
    pub input_capacity: u64,
}

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

impl Totals {
    /// Reprices and reclassifies a raw input estimate without recounting files or history.
    /// Used by the live gauge; callers supply the full estimate, not a rounded delta.
    pub fn with_input_tokens(mut self, raw: u64, deployment: &Deployment) -> Self {
        self.tokens = round_100(raw);
        self.share = self.tokens as f64 / deployment.limit.max(1) as f64;
        self.level = if self.share >= 0.95 {
            Level::Danger
        } else if self.share >= 0.75 {
            Level::Warn
        } else {
            Level::Ok
        };
        self.cost = deployment.cost(self.tokens, 0, EST_OUTPUT_TOKENS);
        self.long_context = deployment.rates(self.tokens).long_context;
        if self.level == Level::Ok {
            self.hint = None;
        }
        self
    }
}

/// Provisional context contributed by the current round. Never persisted: a completed round
/// replaces it in the transcript, while a stopped or failed round is discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveContextTokens {
    input: u64,
    reply: TextTokenEstimate,
}

impl LiveContextTokens {
    pub fn new(system: &str, input: &ApiContent) -> Self {
        Self {
            input: estimate_tokens(system) + content_tokens(input),
            reply: TextTokenEstimate::default(),
        }
    }

    pub fn push_reply(&mut self, text: &str) {
        self.reply.push(text);
    }

    /// Includes the system prompt once, the prepared input and the reply so far.
    pub fn tokens(self) -> u64 {
        self.input + self.reply.tokens()
    }
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
    /// Tokens of the project tree and skill index sent when the model picks its files.
    pub auto_tokens: u64,
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

/// Estimate a stored message without copying ordinary text or image data URLs.
pub fn content_tokens(content: &ApiContent) -> u64 {
    match content {
        ApiContent::Text(text) => estimate_tokens(text),
        ApiContent::Parts(_) => {
            estimate_tokens(&content_text(content))
                + content_images(content) as u64 * PICTURE_TOKENS
        }
    }
}

pub fn totals(input: &ContextInput<'_>) -> Totals {
    totals_with_conversation_tokens(input, input.transcript.iter().map(content_tokens).sum())
}

/// As `totals`, but with an already counted conversation. `input.transcript` is not read.
pub fn totals_with_conversation_tokens(
    input: &ContextInput<'_>,
    conversation_tokens: u64,
) -> Totals {
    let (mut files, mut file_tok) = (0, 0);
    for n in input.tree {
        let s = node_stats(n, input.selected);
        files += s.selected;
        file_tok += s.selected_tokens;
    }
    let rule_tokens = input.rules_on.iter().map(|r| r.tokens).sum();
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
    let new_tokens =
        new_file_tokens + input.picture_tokens + input.auto_tokens + USER_MESSAGE_TOKENS;
    let raw = estimate_tokens(&system_prompt(input.rules_on, input.output_format))
        + conversation_tokens
        + new_tokens;
    let hint = if conversation_tokens > file_tok && conversation_tokens > rule_tokens {
        Some(format!(
            "Earlier conversation: {}",
            fmt_k(conversation_tokens)
        ))
    } else {
        biggest_folder(input.tree, input.selected)
            .map(|b| format!("{}: {}", b.name, fmt_k(b.tokens)))
    };
    Totals {
        files,
        rules: input.rules_on.len(),
        file_tokens: file_tok,
        rule_tokens,
        tokens: 0,
        new_tokens: round_100(new_tokens),
        conversation_tokens,
        share: 0.0,
        level: Level::Ok,
        cost: 0.0,
        long_context: false,
        hint,
    }
    .with_input_tokens(raw, input.deployment)
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
            auto_tokens: 0,
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
    fn pre_counted_conversation_matches_totals() {
        use crate::logic::prompt::{ContextImage, with_images};

        let transcript = [
            ApiContent::Text("An earlier reply with Unicode: 😀".into()),
            with_images(
                "Look at this".into(),
                &[ContextImage {
                    name: "image.png".into(),
                    url: "data:image/png;base64,AAAA".into(),
                }],
            ),
        ];
        let selected = Selection::new();
        let versions = HashMap::new();
        let stale = HashSet::new();
        let deployment = Deployment {
            limit: 128_000,
            ..Deployment::default()
        };
        let input = ContextInput {
            tree: &[],
            selected: &selected,
            rules_on: &[],
            output_format: "",
            transcript: &transcript,
            file_versions: &versions,
            stale: &stale,
            resend_all: false,
            picture_tokens: 0,
            auto_tokens: 0,
            deployment: &deployment,
        };
        let counted = transcript.iter().map(content_tokens).sum();
        assert_eq!(
            totals(&input),
            totals_with_conversation_tokens(
                &ContextInput {
                    transcript: &[],
                    ..input
                },
                counted,
            )
        );
        for content in &transcript {
            assert_eq!(
                content_tokens(content),
                estimate_tokens(&content_text(content))
                    + content_images(content) as u64 * PICTURE_TOKENS
            );
        }
    }

    #[test]
    fn live_round_matches_completed_content_without_counting_files_twice() {
        use crate::logic::prompt::{ContextImage, with_images};

        let system = "System instructions";
        let input = with_images(
            "Prepared input already contains the selected files".into(),
            &[ContextImage {
                name: "screenshot.png".into(),
                url: "data:image/png;base64,AAAA".into(),
            }],
        );
        let mut live = LiveContextTokens::new(system, &input);
        live.push_reply("A reply ");
        live.push_reply("with Unicode 😀");
        assert_eq!(
            live.tokens(),
            estimate_tokens(system)
                + content_tokens(&input)
                + estimate_tokens("A reply with Unicode 😀")
        );
    }

    #[test]
    fn deselecting_files_does_not_remove_earlier_conversation() {
        let selected = Selection::new();
        let versions = HashMap::new();
        let stale = HashSet::new();
        let deployment = Deployment {
            limit: 10_000,
            ..Deployment::default()
        };
        let t = totals_with_conversation_tokens(
            &ContextInput {
                tree: &[],
                selected: &selected,
                rules_on: &[],
                output_format: "",
                transcript: &[],
                file_versions: &versions,
                stale: &stale,
                resend_all: false,
                picture_tokens: 0,
                auto_tokens: 0,
                deployment: &deployment,
            },
            8_000,
        );
        assert_eq!(t.files, 0);
        assert!(t.tokens >= 8_000);
        assert_eq!(t.hint.as_deref(), Some("Earlier conversation: 8k"));
    }

    #[test]
    fn request_budget_reserves_output_and_estimation_margin() {
        let budget = RequestBudget::new(128_000, 16_384);
        assert_eq!(budget.input_capacity(), 98_816);
    }

    #[test]
    fn request_budget_accepts_the_exact_boundary() {
        assert_eq!(RequestBudget::new(10_000, 2_000).check(7_000), Ok(()));
    }

    #[test]
    fn request_budget_rejects_one_token_over_without_rounding() {
        assert_eq!(
            RequestBudget::new(10_000, 2_000).check(7_001),
            Err(BudgetExceeded {
                input_tokens: 7_001,
                input_capacity: 7_000,
            })
        );
    }

    #[test]
    fn request_budget_rejects_output_that_exhausts_the_window() {
        assert!(RequestBudget::new(1_000, 901).check(0).is_err());
    }

    #[test]
    fn request_budget_rejects_a_zero_limit() {
        assert!(RequestBudget::new(0, 0).check(0).is_err());
    }

    #[test]
    fn request_budget_rejects_arithmetic_overflow() {
        assert!(RequestBudget::new(u64::MAX, u64::MAX).check(1).is_err());
    }

    #[test]
    fn rounding_matches_js() {
        assert_eq!(round_100(149), 100);
        assert_eq!(round_100(150), 200);
        assert_eq!(round_100(0), 0);
    }
}
