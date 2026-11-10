//! One-off requests outside any conversation: connection test, commit message and PR drafts,
//! and rebuilding a file whose edit blocks couldn't be placed. Prompts are identical to the
//! old `foundry.ts`.

use super::client::{FoundryClient, FoundryError};
use super::types::{ChatMessage, MESSAGE_OVERHEAD_TOKENS, PrDraftInput, RepairInput, RepairResult};
use contextual_protocol::git::PrDraft;
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

pub const PR_DRAFT_SYSTEM: &str = "You write GitHub pull request titles and descriptions.\n\
Reply in exactly this format:\n\
TITLE: <imperative summary, at most 72 characters, no trailing period>\n\
---\n\
<markdown description: a short paragraph on what changed and why, then a \"Changes\" list of the notable edits per file. No headings above level 2. Do not invent testing that was not mentioned.>";

pub const COMMIT_DRAFT_SYSTEM: &str = "You write git commit messages. Reply with the message only: an imperative subject line of at most 72 characters without a trailing period, then (only if useful) a blank line and a few short \"- \" bullet lines on the notable changes. No code fences, no quotes.";

pub const REPAIR_SYSTEM: &str = "You apply edits to a file. The SEARCH parts of the edit blocks may not match the file exactly; work out where each edit belongs and apply its intent. Reply with the complete updated file only: no explanation, no code fence.";

static TITLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*TITLE:\s*(.+)\n+-{3,}\n?([\s\S]*)$").unwrap());
static TITLE_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^#+\s*|^TITLE:\s*").unwrap());
static FENCE_EDGES_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^```\w*\n?|```$").unwrap());
static FENCED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*```[^\n]*\n([\s\S]*?)\n```\s*$").unwrap());

/// First `n` UTF-16 units, like JavaScript's `slice(0, n)`.
fn js_slice(s: &str, n: usize) -> String {
    crate::workspace::git::js_truncate(s, n).to_string()
}

fn message_content(res: &Value) -> String {
    res["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// Parses a `TITLE: …\n---\n…` reply (or falls back to first line = title).
pub fn parse_pr_draft(text: &str) -> PrDraft {
    if let Some(m) = TITLE_RE.captures(text) {
        return PrDraft {
            title: js_slice(m[1].trim(), 120),
            body: m[2].trim().to_string(),
        };
    }
    let trimmed = text.trim();
    let mut lines = trimmed.split('\n');
    let first = lines.next().unwrap_or("");
    let rest: Vec<&str> = lines.collect();
    PrDraft {
        title: js_slice(TITLE_PREFIX_RE.replace_all(first, "").trim(), 120),
        body: rest.join("\n").trim().to_string(),
    }
}

/// Strips a fence the model may have wrapped a commit message in.
pub fn clean_commit_message(text: &str) -> String {
    FENCE_EDGES_RE.replace_all(text, "").trim().to_string()
}

impl FoundryClient {
    /// One-off requests use the same admission policy as conversations, with their own
    /// completion limit. Build the HTTP body only after admission succeeds.
    fn task_body(
        &self,
        messages: &[ChatMessage],
        output_tokens: u32,
    ) -> Result<Value, FoundryError> {
        let input_tokens = messages.iter().fold(MESSAGE_OVERHEAD_TOKENS, |total, m| {
            total.saturating_add(m.estimated_tokens())
        });
        self.check_input_budget(input_tokens, output_tokens)?;
        Ok(json!({
            "model": self.cfg.deployment,
            "messages": messages,
            "max_completion_tokens": output_tokens,
        }))
    }

    /// Round-trip check for Settings → Test connection (16-token chat completion). Returns ms.
    pub async fn test_connection(&self, cancel: &CancellationToken) -> crate::Result<u64> {
        let started = Instant::now();
        let body = self.task_body(&[ChatMessage::user("ping")], 16)?;
        self.chat_json(&body, cancel).await?;
        Ok(started.elapsed().as_millis() as u64)
    }

    /// Writes a commit message for a diff, in the usual git style.
    pub async fn draft_commit_message(
        &self,
        diff: &str,
        cancel: &CancellationToken,
    ) -> crate::Result<String> {
        let body = self.task_body(
            &[
                ChatMessage::system(COMMIT_DRAFT_SYSTEM),
                ChatMessage::user(format!("```diff\n{diff}\n```")),
            ],
            1000,
        )?;
        let res = self.chat_json(&body, cancel).await?;
        Ok(clean_commit_message(&message_content(&res)))
    }

    /// Writes a PR title and description from the chat notes, commits and diff.
    pub async fn draft_pull_request(
        &self,
        input: &PrDraftInput,
        cancel: &CancellationToken,
    ) -> crate::Result<PrDraft> {
        let commits = if input.commits.is_empty() {
            "(none)"
        } else {
            &input.commits
        };
        let notes = if input.notes.is_empty() {
            "(no chat notes)"
        } else {
            &input.notes
        };
        let user = format!(
            "Branch: {}\n\n# Commits\n\n{commits}\n\n# What was asked and explained in the chat\n\n{notes}\n\n# Diff\n\n```diff\n{}\n```",
            input.branch, input.diff
        );
        let body = self.task_body(
            &[
                ChatMessage::system(PR_DRAFT_SYSTEM),
                ChatMessage::user(user),
            ],
            2000,
        )?;
        let res = self.chat_json(&body, cancel).await?;
        Ok(parse_pr_draft(&message_content(&res)))
    }

    /// Asks for the whole file with the intended edits applied, when a reply's edit blocks
    /// couldn't be placed locally.
    pub async fn repair_file(
        &self,
        input: &RepairInput,
        cancel: &CancellationToken,
    ) -> crate::Result<RepairResult> {
        let body = self.task_body(
            &[
                ChatMessage::system(REPAIR_SYSTEM),
                ChatMessage::user(format!(
                    "File: {}\n\n# Current file\n\n{}\n\n# Edits to apply\n\n{}",
                    input.path, input.original, input.edits
                )),
            ],
            self.cfg.max_completion_tokens,
        )?;
        let res = self.chat_json(&body, cancel).await?;
        let mut content = message_content(&res);
        // Strip a code fence if the model added one anyway.
        if let Some(m) = FENCED_RE.captures(&content) {
            content = format!("{}\n", &m[1]);
        }
        if res["choices"][0]["finish_reason"] == "length" {
            return Err(FoundryError::Failed("The rebuilt file was cut off.".into()).into());
        }
        let n = |v: &Value| v.as_u64().unwrap_or(0);
        Ok(RepairResult {
            content,
            input_tokens: n(&res["usage"]["prompt_tokens"]),
            output_tokens: n(&res["usage"]["completion_tokens"]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr_draft_parsing() {
        let d = parse_pr_draft("TITLE: Add thing\n---\nBody here\n\n- a\n");
        assert_eq!(
            d,
            PrDraft {
                title: "Add thing".into(),
                body: "Body here\n\n- a".into()
            }
        );
        let d = parse_pr_draft("## Fix bug\nMore text");
        assert_eq!(
            d,
            PrDraft {
                title: "Fix bug".into(),
                body: "More text".into()
            }
        );
        let d = parse_pr_draft("title: lower\n\n-----\nx");
        assert_eq!(d.title, "lower");
    }

    #[test]
    fn commit_message_cleaning() {
        assert_eq!(clean_commit_message("```text\nFix it\n```"), "Fix it");
        assert_eq!(clean_commit_message("Fix it\n"), "Fix it");
    }
}
