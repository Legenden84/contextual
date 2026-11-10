//! The conversation round by round, exactly as sent and received (the Inspect view): what went
//! in (system prompt, project tree, files, read answers, the request) and what came back (read
//! requests, prose, edit blocks), raw.
//!
//! Command proposals appear as what they are on the wire: the model's calls on the reply
//! side, and on the input side exactly what the model was told about each earlier call —
//! a rejection, "not run", a withheld result, or the outcome with whatever output the user
//! chose to share. Each is labelled from the chat's command history.

use crate::chat::{ApiMessage, ThreadState, Usage, touched_paths};
use crate::commands::CallId;
use crate::commands::execution::ExecutionStatus;
use crate::commands::history::{Answer, CommandRecord, RecordStatus};
use crate::llm::types::ChatRole;
use crate::logic::format::estimate_tokens;
use crate::logic::prompt::{ApiContent, content_images, content_text};
use crate::logic::reads::parse_read_request;

/// A round still streaming: sent, reply not complete.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveRound {
    pub input: ApiContent,
    pub system: String,
    pub followup: bool,
    pub reply: String,
}

/// One part of an input message ("# Context files (3)"), with its approximate size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub tokens: u64,
}

/// One command call the model issued in a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundCall {
    pub call_id: String,
    pub tool: String,
    /// The arguments exactly as the model wrote them.
    pub arguments: String,
    /// Why it could never become a command, if so.
    pub problem: Option<String>,
    /// Where the proposal stands now: "Needs review", "Rejected", "Exit code 3"…
    pub status: String,
}

/// What the model was told about one earlier call, as part of a round's input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundResult {
    pub call_id: String,
    /// Exactly the text that was sent.
    pub output: String,
    /// "Rejected", "Not run", "Result not shared", "Outcome shared, stdout omitted"…
    pub label: String,
}

fn status_label(record: &CommandRecord) -> String {
    match record.status() {
        RecordStatus::Unusable => "Could not be read as a command".into(),
        RecordStatus::NeedsReview => "Needs review".into(),
        RecordStatus::Rejected => "Rejected".into(),
        RecordStatus::Running => "Running".into(),
        RecordStatus::NotRun => "Not run".into(),
        RecordStatus::Finished(status) => {
            let observed = record.attempts.last().and_then(|attempt| attempt.observed);
            match (status, observed) {
                (_, Some(observed)) if observed.interrupted => {
                    "Outcome unknown (the app stopped while it ran)".into()
                }
                (ExecutionStatus::Success, _) => "Succeeded".into(),
                (ExecutionStatus::Failure, Some(observed)) => match observed.exit_code {
                    Some(code) => format!("Exit code {code}"),
                    None => "Failed".into(),
                },
                (ExecutionStatus::Failure, None) => "Failed".into(),
                (ExecutionStatus::FailedToStart, _) => "Did not start".into(),
                (ExecutionStatus::Cancelled, _) => "Stopped".into(),
                (ExecutionStatus::TimedOut, _) => "Timed out".into(),
                (ExecutionStatus::Unknown, _) => "Outcome unknown".into(),
            }
        }
    }
}

fn answer_label(answer: &Answer) -> String {
    match answer {
        Answer::Rejected => "Rejected".into(),
        Answer::NotRun { .. } => "Not run".into(),
        Answer::Withheld { .. } => "Ran; result not shared".into(),
        Answer::Outcome {
            stdout_shared,
            stderr_shared,
            redacted,
            ..
        } => {
            let mut label = match (stdout_shared, stderr_shared) {
                (true, true) => "Outcome and output shared".to_owned(),
                (true, false) => "Outcome shared, stderr omitted".to_owned(),
                (false, true) => "Outcome shared, stdout omitted".to_owned(),
                (false, false) => "Outcome shared, output omitted".to_owned(),
            };
            if *redacted {
                label.push_str(", edited before sharing");
            }
            label
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    /// 1-based.
    pub number: usize,
    /// Written by the app (answering a read request), not typed by the user.
    pub followup: bool,
    pub input: String,
    pub images: usize,
    pub sections: Vec<Section>,
    /// The system prompt in effect; `None` when the chat was saved before it was recorded.
    pub system: Option<String>,
    /// The system prompt differs from the previous round's (always true for the first).
    pub system_changed: bool,
    pub reply: String,
    pub usage: Option<Usage>,
    /// Still streaming.
    pub live: bool,
    /// What the model was told about earlier calls in this round's input, in order.
    pub results: Vec<RoundResult>,
    /// The command calls in this round's reply, in order.
    pub calls: Vec<RoundCall>,
}

/// Top-level sections the app writes into a turn (see `logic::prompt::build_turn`).
const SECTIONS: [&str; 6] = [
    "Project files",
    "Context files",
    "Unchanged files",
    "Removed from context",
    "Requested context",
    "Request",
];

fn section_title(line: &str) -> Option<&str> {
    let title = line.strip_prefix("# ")?;
    SECTIONS
        .iter()
        .any(|s| title == *s || title.starts_with(&format!("{s} (")))
        .then_some(title)
}

/// Splits an input message into the sections the app wrote; text before the first one (a
/// plain message) counts as the request.
pub fn sections(text: &str) -> Vec<Section> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut fence: Option<usize> = None;
    for line in text.lines() {
        let t = line.trim_start();
        let ticks = t.chars().take_while(|c| *c == '`').count();
        match fence {
            Some(open) if ticks >= open && t[ticks..].trim().is_empty() => fence = None,
            None if ticks >= 3 => fence = Some(ticks),
            _ => {}
        }
        let heading = fence.is_none().then(|| section_title(line)).flatten();
        match (heading, out.last_mut()) {
            (Some(title), _) => out.push((title.to_string(), String::new())),
            (None, Some((_, body))) => {
                body.push_str(line);
                body.push('\n');
            }
            (None, None) => out.push(("Request".into(), format!("{line}\n"))),
        }
    }
    out.into_iter()
        .map(|(title, body)| Section {
            tokens: estimate_tokens(body.trim()),
            title,
        })
        .collect()
}

/// A round with its input filled in; the caller sets the rest.
fn round(number: usize, input: &ApiContent, followup: bool) -> Round {
    let text = content_text(input);
    Round {
        number,
        followup,
        sections: sections(&text),
        images: content_images(input),
        input: text,
        system: None,
        system_changed: false,
        reply: String::new(),
        usage: None,
        live: false,
        results: Vec::new(),
        calls: Vec::new(),
    }
}

/// Every round of the conversation, plus the one streaming now.
pub fn rounds(thread: &ThreadState, live: Option<&LiveRound>) -> Vec<Round> {
    let mut out = Vec::new();
    let mut system: Option<String> = None;
    // The turn whose calls the next results answer.
    let mut asked_turn: Option<String> = None;
    let mut msgs = thread.transcript.iter().peekable();
    while let Some(first) = msgs.next() {
        // A round's input is the answers to earlier calls, then what the user sent. Either
        // part can be missing, but not both.
        let mut results = Vec::new();
        let mut changed = out.is_empty();
        let mut message = Some(first);
        while let Some(m) = message.filter(|m| m.role == ChatRole::Tool) {
            if let Some(s) = &m.system {
                system = Some(s.clone());
                changed = true;
            }
            let call_id = m.tool_call_id.clone().unwrap_or_default();
            let record = asked_turn.as_ref().and_then(|turn| {
                thread.commands.get(&CallId {
                    turn: turn.clone(),
                    call: call_id.clone(),
                })
            });
            results.push(RoundResult {
                label: record
                    .and_then(|record| record.answered.as_ref())
                    .map_or_else(|| "Sent".into(), |answered| answer_label(&answered.answer)),
                output: content_text(&m.content),
                call_id,
            });
            message = msgs.next_if(|next| matches!(next.role, ChatRole::Tool | ChatRole::User));
        }
        let user = message.filter(|m| m.role == ChatRole::User);
        if user.is_none() && results.is_empty() {
            // A reply without an input: nothing to show as a round.
            continue;
        }
        if let Some(s) = user.and_then(|m| m.system.as_ref()) {
            system = Some(s.clone());
            changed = true;
        }
        let empty = ApiContent::Text(String::new());
        let reply: Option<&ApiMessage> = msgs.next_if(|r| r.role == ChatRole::Assistant);
        asked_turn = reply.and_then(|r| r.command_turn.clone());
        let calls = reply
            .map(|r| {
                r.tool_calls
                    .iter()
                    .map(|call| RoundCall {
                        status: asked_turn
                            .as_ref()
                            .and_then(|turn| {
                                thread.commands.get(&CallId {
                                    turn: turn.clone(),
                                    call: call.call_id.clone(),
                                })
                            })
                            .map_or_else(|| "No record".into(), status_label),
                        call_id: call.call_id.clone(),
                        tool: call.name.clone(),
                        arguments: call.arguments.clone(),
                        problem: call.problem.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.push(Round {
            system: system.clone(),
            system_changed: changed,
            reply: reply.map(|r| content_text(&r.content)).unwrap_or_default(),
            usage: reply.and_then(|r| r.usage),
            results,
            calls,
            ..round(
                out.len() + 1,
                user.map_or(&empty, |m| &m.content),
                user.is_some_and(|m| m.followup),
            )
        });
    }
    if let Some(l) = live {
        let changed = out.is_empty() || system.as_deref() != Some(l.system.as_str());
        out.push(Round {
            system: Some(l.system.clone()),
            system_changed: changed,
            reply: l.reply.clone(),
            live: true,
            ..round(out.len() + 1, &l.input, l.followup)
        });
    }
    out
}

impl Round {
    /// What the input was: the user's request (first line), or the app answering a read.
    pub fn input_label(&self) -> String {
        if self.followup {
            return "Requested context".into();
        }
        if self.input.trim().is_empty() && !self.results.is_empty() {
            let n = self.results.len();
            return format!("Command {}", if n == 1 { "result" } else { "results" });
        }
        let request = self
            .input
            .rsplit_once("\n# Request\n")
            .map(|(_, request)| request)
            .or_else(|| self.input.strip_prefix("# Request\n"))
            .unwrap_or(self.input.as_str());
        let first = request
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let mut chars = first.chars();
        let mut label: String = chars.by_ref().take(60).collect();
        if chars.next().is_some() {
            label.push('…');
        }
        label
    }

    /// What the reply did: asked for context, proposed changes, or answered.
    pub fn reply_label(&self) -> String {
        if self.live {
            return "Streaming…".into();
        }
        if !self.calls.is_empty() {
            let n = self.calls.len();
            return format!(
                "Proposed {n} {}",
                if n == 1 { "command" } else { "commands" }
            );
        }
        if self.reply.is_empty() {
            return "No reply".into();
        }
        if let Some(req) = parse_read_request(&self.reply) {
            let n = req.paths.len() + req.folders.len();
            return format!("Asked for {n} {}", if n == 1 { "path" } else { "paths" });
        }
        let n = touched_paths(&self.reply).len();
        if n > 0 {
            return format!("Changed {n} {}", if n == 1 { "file" } else { "files" });
        }
        "Answered".into()
    }

    pub fn input_tokens(&self) -> u64 {
        self.sections.iter().map(|s| s.tokens).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::prompt::{TurnInput, build_turn};

    #[test]
    fn splits_sections_outside_code_fences() {
        let input = build_turn(&TurnInput {
            text: "Fix the bug\nin main".into(),
            files: vec![crate::logic::prompt::ContextFile {
                path: "notes.md".into(),
                content: Some("# Request\nnot a section".into()),
                ..Default::default()
            }],
            tree: Some("src/ (1k)".into()),
            ..Default::default()
        });
        let titles: Vec<String> = sections(&content_text(&input))
            .into_iter()
            .map(|s| s.title)
            .collect();
        assert_eq!(titles, ["Project files", "Context files (1)", "Request"]);
        assert_eq!(sections("just a question")[0].title, "Request");
    }

    #[test]
    fn rounds_carry_the_system_prompt_forward() {
        let mut t = ThreadState::fresh();
        t.record_round(
            ApiContent::Text("# Request\n\nAdd a test".into()),
            "sys A",
            false,
            "```read\nsrc/a.rs\nsrc/\n```".into(),
            Some(Usage {
                input_tokens: 10,
                output_tokens: 2,
                cached_tokens: 0,
            }),
        );
        t.record_round(
            ApiContent::Text("# Requested context\n\nSent above: src/a.rs.".into()),
            "sys A",
            true,
            "Done.\n\n### DELETE: src/a.rs\n".into(),
            None,
        );
        // Recorded once, while unchanged.
        assert!(t.transcript[0].system.is_some() && t.transcript[2].system.is_none());

        let live = LiveRound {
            input: ApiContent::Text("again".into()),
            system: "sys B".into(),
            followup: false,
            reply: "Wor".into(),
        };
        let r = rounds(&t, Some(&live));
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].input_label(), "Add a test");
        assert_eq!(r[0].reply_label(), "Asked for 2 paths");
        assert_eq!(r[0].usage.map(|u| u.input_tokens), Some(10));
        assert_eq!(r[1].input_label(), "Requested context");
        assert_eq!(r[1].reply_label(), "Changed 1 file");
        assert_eq!(
            (r[1].system.as_deref(), r[1].system_changed),
            (Some("sys A"), false)
        );
        assert!(r[2].live && r[2].system_changed);
        assert_eq!(r[2].reply_label(), "Streaming…");
    }

    #[test]
    fn input_labels_use_the_last_request_without_copying_the_input() {
        let input = ApiContent::Text(
            "# Request\nfirst\n# Context files (1)\ncontent\n# Request\n\nlast".into(),
        );
        assert_eq!(round(1, &input, false).input_label(), "last");
    }

    #[test]
    fn input_labels_accept_a_request_at_the_start() {
        let input = ApiContent::Text("# Request\n\nFix this".into());
        assert_eq!(round(1, &input, false).input_label(), "Fix this");
    }

    #[test]
    fn input_labels_truncate_at_character_boundaries() {
        let input = ApiContent::Text("😀".repeat(61));
        assert_eq!(
            round(1, &input, false).input_label(),
            format!("{}…", "😀".repeat(60)),
        );
    }

    #[test]
    fn old_chats_have_no_system_prompt() {
        let mut t = ThreadState::fresh();
        t.transcript = vec![
            ApiMessage::new(ChatRole::User, ApiContent::Text("hi".into())),
            ApiMessage::new(ChatRole::Assistant, ApiContent::Text("hello".into())),
        ];
        let r = rounds(&t, None);
        assert_eq!(r[0].system, None);
        assert_eq!(r[0].reply, "hello");
    }

    #[test]
    fn command_calls_and_what_the_model_was_told_appear_round_by_round() {
        use crate::commands::InvocationId;
        use crate::commands::execution::{ExecutionProgress, ExecutionResult, ExecutionStatus};
        use crate::llm::tools::{PROPOSE_COMMAND, ToolCall};
        use crate::location::Location;
        use std::collections::BTreeMap;

        const NOW: &str = "2026-11-10T12:00:00Z";
        let call = |id: &str| ToolCall {
            call_id: id.into(),
            name: PROPOSE_COMMAND.into(),
            arguments: serde_json::json!({
                "version": 1, "purpose": "Check.", "cwd": ".", "timeoutSeconds": 30,
                "command": { "kind": "exec", "program": id, "args": [] }, "environment": []
            })
            .to_string(),
            problem: None,
        };
        let mut t = ThreadState::fresh();
        t.record_command_round(
            Vec::new(),
            Some(ApiContent::Text("Check it".into())),
            "sys",
            false,
            "Two checks.".into(),
            vec![call("call_run"), call("call_no")],
            None,
            &Location::Local,
            NOW,
        )
        .unwrap();

        let r = rounds(&t, None);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].reply_label(), "Proposed 2 commands");
        assert_eq!(r[0].calls[0].status, "Needs review");
        assert!(r[0].calls[0].arguments.contains("call_run"));

        // One is run and fails, the other is rejected; the user continues without sharing.
        let turn = t.unanswered_calls().0.unwrap().to_owned();
        let id = |call: &str| CallId {
            turn: turn.clone(),
            call: call.into(),
        };
        let invocation = InvocationId::new();
        let request = t
            .command(&id("call_run"))
            .unwrap()
            .request()
            .unwrap()
            .clone();
        t.commands
            .begin(&id("call_run"), invocation.clone(), request, NOW)
            .unwrap();
        let failed = ExecutionResult {
            status: ExecutionStatus::Failure,
            exit_code: Some(3),
            spawned: true,
            cancelled: false,
            timed_out: false,
            output: ExecutionProgress::default(),
            detail: None,
            history_error: None,
        };
        t.commands
            .finish(&id("call_run"), &invocation, &failed, NOW)
            .unwrap();
        t.commands.reject(&id("call_no"), NOW).unwrap();
        let answers = t.plan_answers(&BTreeMap::new()).unwrap();
        t.record_command_round(
            answers,
            None,
            "sys",
            false,
            "Understood.".into(),
            Vec::new(),
            None,
            &Location::Local,
            NOW,
        )
        .unwrap();

        let r = rounds(&t, None);
        assert_eq!(r.len(), 2);
        // The first round's calls now show how each ended.
        assert_eq!(r[0].calls[0].status, "Exit code 3");
        assert_eq!(r[0].calls[1].status, "Rejected");
        // The second round's input is exactly what the model was told, correlated by call.
        assert_eq!(r[1].input_label(), "Command results");
        assert!(!r[1].system_changed);
        assert_eq!(r[1].results.len(), 2);
        assert_eq!(r[1].results[0].call_id, "call_run");
        assert_eq!(r[1].results[0].label, "Ran; result not shared");
        assert!(r[1].results[0].output.contains("notShared"));
        assert_eq!(r[1].results[1].label, "Rejected");
        assert_eq!(r[1].reply, "Understood.");
        assert_eq!(r[1].reply_label(), "Answered");
    }
}
