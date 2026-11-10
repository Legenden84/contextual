//! What a command review card shows and which actions it offers, decided without any UI.
//!
//! The card is built from what is recorded for a proposal ([`CommandRecord`]) and from the
//! in-memory review state of the open project. The view renders it and wires each offered
//! action to the chat model; it does not decide anything itself. In particular Run is offered
//! only when a prepared plan is on screen to be approved, and never for a proposal whose call
//! was already answered.

use contextual_protocol::commands::CommandPlatform;

use super::execution::ExecutionStatus;
use super::history::{Answer, CommandRecord, Proposal, RecordStatus, SharedResult};
use super::{Command, CommandRequest, CommandState, Interpreter, ReviewPlan};
use crate::location::Location;

/// Shown on every card that can still run something.
pub const EFFECTS_NOTE: &str = "Commands may modify files or access the network. \
                                Save/Undo does not undo command effects.";
/// Shown before any output is shared.
pub const SHARE_WARNING: &str =
    "Output may contain secrets or contents of files you did not select.";
/// Shown with whatever will be shared.
pub const SAVED_NOTE: &str =
    "What you share is sent to the model and becomes part of the saved chat.";
/// Shown next to Stop.
pub const STOP_NOTE: &str = "Stopping ends the command. It does not undo what it already did.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardTone {
    Neutral,
    Ok,
    Warn,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPhase {
    /// The call could never be a command.
    Unusable,
    /// Waiting for the host to prepare it, or the preparation failed.
    Unprepared,
    /// A prepared plan is shown and can be approved.
    Review,
    /// Run was pressed; the launch is being set up.
    Starting,
    Running,
    Finished,
    Rejected,
    /// The conversation moved on without it.
    NotRun,
}

/// One labelled value. Command text and paths are shown in the monospace face.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardLine {
    pub label: String,
    pub value: String,
    pub mono: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CardActions {
    /// Approve exactly what is shown and start it.
    pub run: bool,
    /// The proposal ran before; Run starts another attempt.
    pub again: bool,
    pub edit: bool,
    pub reject: bool,
    pub stop: bool,
    /// Ask the host to prepare the proposal (again).
    pub prepare: bool,
    /// Ask the host whether anything from an unknown outcome still runs.
    pub reconcile: bool,
    /// Open the preview of what would be shared with the model.
    pub share: bool,
    /// Withdraw a result that was chosen for sharing and has not been sent.
    pub unshare: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub phase: CardPhase,
    pub status: String,
    pub tone: CardTone,
    /// "Purpose (model-provided)".
    pub purpose: Option<String>,
    /// The exact program and argument boundaries, or the interpreter and complete script.
    pub command: Vec<CardLine>,
    /// Directory, executable, timeout, overrides and where it runs.
    pub details: Vec<CardLine>,
    /// Why an action is unavailable, or what went wrong.
    pub notice: Option<String>,
    /// The request shown differs from what the model proposed.
    pub edited: bool,
    /// What will be shared when the conversation continues, if the user chose a result.
    pub staged: Option<String>,
    pub actions: CardActions,
}

/// Everything the card depends on.
pub struct CardInput<'a> {
    pub record: &'a CommandRecord,
    /// The plan prepared in this session, if any.
    pub plan: Option<&'a ReviewPlan>,
    pub review: Option<CommandState>,
    /// Another command runs in this project, or one ended unknown and is unreconciled.
    pub project_busy: bool,
    /// The model is replying, or the chat is saving.
    pub chat_busy: bool,
    pub preparing: bool,
    pub starting: bool,
    pub error: Option<&'a str>,
    /// Command execution is configured for this project's host.
    pub enabled: bool,
    /// The latest attempt's output is still held in memory, so it can be shared.
    pub output_held: bool,
    /// The result the user chose to share when the conversation continues.
    pub staged: Option<&'a SharedResult>,
}

fn line(label: &str, value: impl Into<String>, mono: bool) -> CardLine {
    CardLine {
        label: label.to_owned(),
        value: value.into(),
        mono,
    }
}

fn interpreter_name(interpreter: Interpreter) -> &'static str {
    match interpreter {
        Interpreter::Sh => "sh",
        Interpreter::Bash => "bash",
        Interpreter::Zsh => "zsh",
        Interpreter::Pwsh => "pwsh (PowerShell)",
        Interpreter::WindowsPowerShell => "Windows PowerShell",
        Interpreter::Cmd => "cmd",
    }
}

/// The request as the model, or the user's edit, wrote it: boundaries made explicit.
fn requested(request: &CommandRequest) -> Vec<CardLine> {
    match &request.command {
        Command::Exec { program, args } => {
            let mut lines = vec![line("Program", program, true)];
            lines.extend(
                args.iter().enumerate().map(|(index, argument)| {
                    line(&format!("Argument {}", index + 1), argument, true)
                }),
            );
            if args.is_empty() {
                lines.push(line("Arguments", "None", false));
            }
            lines
        }
        Command::Shell {
            interpreter,
            script,
        } => vec![
            line("Interpreter", interpreter_name(*interpreter), false),
            line("Script", script, true),
        ],
    }
}

fn where_it_runs(location: &Location, platform: Option<CommandPlatform>) -> String {
    match location {
        Location::Wsl { distro } => format!("WSL: {distro}"),
        Location::Local => {
            let platform = platform.unwrap_or(if cfg!(windows) {
                CommandPlatform::Windows
            } else if cfg!(target_os = "macos") {
                CommandPlatform::Macos
            } else {
                CommandPlatform::Linux
            });
            format!(
                "This computer ({})",
                match platform {
                    CommandPlatform::Windows => "Windows",
                    CommandPlatform::Macos => "macOS",
                    CommandPlatform::Linux => "Linux",
                }
            )
        }
    }
}

fn details(
    record: &CommandRecord,
    request: &CommandRequest,
    plan: Option<&ReviewPlan>,
) -> Vec<CardLine> {
    let mut lines = Vec::new();
    match plan {
        Some(plan) => {
            lines.push(line("Executable", plan.program.to_string_lossy(), true));
            lines.push(line(
                "Directory",
                format!(
                    "{}  (requested: {})",
                    plan.directory.cwd().to_string_lossy(),
                    request.cwd
                ),
                true,
            ));
        }
        None => lines.push(line(
            "Directory",
            format!("requested: {}", request.cwd),
            true,
        )),
    }
    lines.push(line(
        "Timeout",
        format!("{} seconds", request.timeout_seconds),
        false,
    ));
    lines.push(line(
        "Environment",
        if request.environment.is_empty() {
            "No overrides".to_owned()
        } else {
            request
                .environment
                .iter()
                .map(|variable| format!("{}={}", variable.name, variable.value))
                .collect::<Vec<_>>()
                .join("\n")
        },
        !request.environment.is_empty(),
    ));
    lines.push(line(
        "Runs on",
        where_it_runs(&record.location, plan.map(|plan| plan.platform)),
        false,
    ));
    lines
}

fn finished_status(record: &CommandRecord, status: ExecutionStatus) -> (String, CardTone) {
    let observed = record.attempts.last().and_then(|attempt| attempt.observed);
    if observed.is_some_and(|observed| observed.interrupted) {
        return (
            "Outcome unknown: the app stopped while it ran".into(),
            CardTone::Warn,
        );
    }
    match status {
        ExecutionStatus::Success => ("Finished: exit code 0".into(), CardTone::Ok),
        ExecutionStatus::Failure => (
            match observed.and_then(|observed| observed.exit_code) {
                Some(code) => format!("Finished: exit code {code}"),
                None => "Ended without an exit code".into(),
            },
            CardTone::Danger,
        ),
        ExecutionStatus::FailedToStart => ("Did not start".into(), CardTone::Danger),
        ExecutionStatus::Cancelled => ("Stopped".into(), CardTone::Warn),
        ExecutionStatus::TimedOut => ("Stopped: timed out".into(), CardTone::Warn),
        ExecutionStatus::Unknown => (
            "Outcome unknown: it may still be running".into(),
            CardTone::Warn,
        ),
    }
}

/// What the model was told, as a short phrase for a closed card.
fn answered_note(answer: &Answer) -> &'static str {
    match answer {
        Answer::Rejected => "The model was told it was rejected.",
        Answer::NotRun { .. } => "The model was told it was not run.",
        Answer::Withheld { .. } => "The model was told it ran. Its result was not shared.",
        Answer::Outcome { .. } => "Its result was shared with the model.",
    }
}

pub fn review_card(input: &CardInput<'_>) -> Card {
    let record = input.record;
    let answered = record.answered.as_ref();
    let Proposal::Command { request: proposed } = &record.proposal else {
        let Proposal::Unusable { tool, problem } = &record.proposal else {
            unreachable!("a proposal is a command or unusable");
        };
        return Card {
            phase: CardPhase::Unusable,
            status: "Could not be read as a command".into(),
            tone: CardTone::Warn,
            purpose: None,
            command: vec![line("Tool", tool, true)],
            details: Vec::new(),
            notice: Some(problem.clone()),
            edited: false,
            staged: None,
            actions: CardActions::default(),
        };
    };

    // What is on screen: the plan under review, else what last ran, else the proposal.
    let shown = input
        .plan
        .map(|plan| &plan.request)
        .or_else(|| record.attempts.last().map(|attempt| &attempt.request))
        .unwrap_or(proposed);
    let open = answered.is_none();
    let reviewing = matches!(
        input.review,
        Some(CommandState::NeedsReview | CommandState::Approved)
    ) && input.plan.is_some();
    let recorded = record.status();

    let (phase, status, tone) = match recorded {
        RecordStatus::Running => (CardPhase::Running, "Running".to_owned(), CardTone::Neutral),
        RecordStatus::Rejected => (
            CardPhase::Rejected,
            "Rejected".to_owned(),
            CardTone::Neutral,
        ),
        RecordStatus::NotRun => (CardPhase::NotRun, "Not run".to_owned(), CardTone::Neutral),
        _ if input.starting => (
            CardPhase::Starting,
            "Starting".to_owned(),
            CardTone::Neutral,
        ),
        // A fresh review of a proposal that ran before: a re-run awaiting approval.
        RecordStatus::Finished(_) if reviewing && open => (
            CardPhase::Review,
            "Needs review to run again".to_owned(),
            CardTone::Warn,
        ),
        RecordStatus::Finished(status) => {
            let (status, tone) = finished_status(record, status);
            (CardPhase::Finished, status, tone)
        }
        RecordStatus::NeedsReview if reviewing => {
            (CardPhase::Review, "Needs review".to_owned(), CardTone::Warn)
        }
        RecordStatus::NeedsReview => (
            CardPhase::Unprepared,
            if input.preparing {
                "Preparing".to_owned()
            } else if !input.enabled {
                "Commands cannot be run here".to_owned()
            } else {
                "Not prepared".to_owned()
            },
            CardTone::Neutral,
        ),
        RecordStatus::Unusable => unreachable!("handled above"),
    };

    let blocked = if !open || !input.enabled {
        None
    } else if input.project_busy && !matches!(phase, CardPhase::Running | CardPhase::Starting) {
        Some("Another command is running in this project, or one ended with an unknown outcome.")
    } else if input.chat_busy {
        Some("Wait for the reply to finish.")
    } else {
        None
    };
    let free = open && input.enabled && blocked.is_none();
    let unknown = matches!(recorded, RecordStatus::Finished(ExecutionStatus::Unknown));
    let actions = CardActions {
        run: phase == CardPhase::Review && free,
        again: phase == CardPhase::Review && !record.attempts.is_empty(),
        edit: phase == CardPhase::Review && free,
        reject: open
            && recorded == RecordStatus::NeedsReview
            && matches!(phase, CardPhase::Review | CardPhase::Unprepared),
        stop: phase == CardPhase::Running,
        prepare: open
            && input.enabled
            && !input.preparing
            && !input.chat_busy
            && match phase {
                CardPhase::Unprepared => true,
                // Offer another attempt after a finished one.
                CardPhase::Finished => !unknown && !input.project_busy,
                _ => false,
            },
        reconcile: unknown && input.enabled && phase == CardPhase::Finished,
        // Sharing is a separate decision from running, taken on a finished attempt whose
        // output this session still holds.
        share: open
            && phase == CardPhase::Finished
            && input.output_held
            && input.staged.is_none()
            && !input.chat_busy,
        unshare: open && input.staged.is_some() && !input.chat_busy,
    };

    let notice = input
        .error
        .map(str::to_owned)
        .or_else(|| blocked.map(str::to_owned))
        .or_else(|| answered.map(|answered| answered_note(&answered.answer).to_owned()));

    Card {
        phase,
        status,
        tone,
        purpose: Some(shown.purpose.clone()),
        command: requested(shown),
        details: details(record, shown, input.plan),
        notice,
        edited: shown != proposed,
        staged: input.staged.filter(|_| open).map(|result| {
            format!(
                "When you continue, the model receives {}.",
                result.summary()
            )
        }),
        actions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::execution::{ExecutionProgress, ExecutionResult};
    use crate::commands::history::CommandHistory;
    use crate::commands::{CallId, InvocationId, parse_arguments};

    const NOW: &str = "2026-11-10T12:00:00Z";

    fn request(command: serde_json::Value) -> CommandRequest {
        parse_arguments(
            &serde_json::json!({
                "version": 1,
                "purpose": "Run the tests.",
                "command": command,
                "cwd": "crates/core",
                "timeoutSeconds": 120,
                "environment": []
            })
            .to_string(),
        )
        .unwrap()
    }

    fn exec() -> CommandRequest {
        request(serde_json::json!({
            "kind": "exec", "program": "cargo", "args": ["test", "--", "a b", ""]
        }))
    }

    fn call() -> CallId {
        CallId {
            turn: "turn-1".into(),
            call: "call_1".into(),
        }
    }

    fn proposed(request: CommandRequest) -> CommandHistory {
        let mut history = CommandHistory::default();
        history
            .propose(call(), "propose_command", Ok(request), Location::Local, NOW)
            .unwrap();
        history
    }

    fn input(record: &CommandRecord) -> CardInput<'_> {
        CardInput {
            record,
            plan: None,
            review: None,
            project_busy: false,
            chat_busy: false,
            preparing: false,
            starting: false,
            error: None,
            enabled: true,
            output_held: false,
            staged: None,
        }
    }

    fn result(status: ExecutionStatus, code: Option<i32>) -> ExecutionResult {
        ExecutionResult {
            status,
            exit_code: code,
            spawned: true,
            cancelled: false,
            timed_out: false,
            output: ExecutionProgress::default(),
            detail: None,
            history_error: None,
        }
    }

    fn finished(status: ExecutionStatus, code: Option<i32>) -> CommandHistory {
        let mut history = proposed(exec());
        let invocation = InvocationId::new();
        history
            .begin(&call(), invocation.clone(), exec(), NOW)
            .unwrap();
        history
            .finish(&call(), &invocation, &result(status, code), NOW)
            .unwrap();
        history
    }

    #[test]
    fn argument_boundaries_and_the_whole_script_are_shown_exactly() {
        let history = proposed(exec());
        let card = review_card(&input(history.get(&call()).unwrap()));
        let shown: Vec<(&str, &str)> = card
            .command
            .iter()
            .map(|line| (line.label.as_str(), line.value.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Program", "cargo"),
                ("Argument 1", "test"),
                ("Argument 2", "--"),
                ("Argument 3", "a b"),
                ("Argument 4", ""),
            ]
        );
        assert_eq!(card.purpose.as_deref(), Some("Run the tests."));
        let details: Vec<&str> = card
            .details
            .iter()
            .map(|line| line.value.as_str())
            .collect();
        assert_eq!(details[0], "requested: crates/core");
        assert_eq!(details[1], "120 seconds");
        assert_eq!(details[2], "No overrides");
        assert!(details[3].starts_with("This computer ("));

        let script = "set -e\ncargo test 2>&1 | tail -5";
        let history = proposed(request(serde_json::json!({
            "kind": "shell", "interpreter": "bash", "script": script
        })));
        let scripted = review_card(&input(history.get(&call()).unwrap()));
        assert_eq!(scripted.command[0].value, "bash");
        assert_eq!(scripted.command[1].value, script);
    }

    #[test]
    fn nothing_can_be_run_until_a_prepared_plan_is_on_screen() {
        let history = proposed(exec());
        let record = history.get(&call()).unwrap();

        let unprepared = review_card(&input(record));
        assert_eq!(unprepared.phase, CardPhase::Unprepared);
        assert!(!unprepared.actions.run && !unprepared.actions.edit);
        assert!(unprepared.actions.prepare && unprepared.actions.reject);

        let preparing = review_card(&CardInput {
            preparing: true,
            ..input(record)
        });
        assert_eq!(preparing.status, "Preparing");
        assert!(!preparing.actions.run && !preparing.actions.prepare);

        // A review state without a plan to show is still not runnable.
        let stateless = review_card(&CardInput {
            review: Some(CommandState::NeedsReview),
            ..input(record)
        });
        assert!(!stateless.actions.run);

        let disabled = review_card(&CardInput {
            enabled: false,
            ..input(record)
        });
        assert_eq!(disabled.status, "Commands cannot be run here");
        assert!(!disabled.actions.run && !disabled.actions.prepare);
        // It can still be rejected, so that the conversation can continue.
        assert!(disabled.actions.reject);
    }

    #[test]
    fn a_running_or_unknown_command_elsewhere_blocks_run_and_says_why() {
        let history = proposed(exec());
        let record = history.get(&call()).unwrap();
        for (project_busy, chat_busy) in [(true, false), (false, true)] {
            let card = review_card(&CardInput {
                project_busy,
                chat_busy,
                ..input(record)
            });
            assert!(!card.actions.run && !card.actions.edit);
            assert!(card.notice.is_some());
        }
    }

    #[test]
    fn a_running_command_offers_only_stop() {
        let mut history = proposed(exec());
        history
            .begin(&call(), InvocationId::new(), exec(), NOW)
            .unwrap();
        let card = review_card(&CardInput {
            review: Some(CommandState::Running),
            project_busy: true,
            ..input(history.get(&call()).unwrap())
        });
        assert_eq!(card.phase, CardPhase::Running);
        assert_eq!(
            card.actions,
            CardActions {
                stop: true,
                ..CardActions::default()
            }
        );
        assert!(card.notice.is_none());
    }

    #[test]
    fn finished_cards_state_the_outcome_and_offer_only_a_fresh_preparation() {
        let history = finished(ExecutionStatus::Failure, Some(3));
        let card_failed = review_card(&input(history.get(&call()).unwrap()));
        assert_eq!(card_failed.status, "Finished: exit code 3");
        assert_eq!(card_failed.tone, CardTone::Danger);
        assert!(card_failed.actions.prepare);
        assert!(!card_failed.actions.run && !card_failed.actions.reject);

        let history = finished(ExecutionStatus::Unknown, None);
        let unknown = review_card(&CardInput {
            project_busy: true,
            ..input(history.get(&call()).unwrap())
        });
        assert_eq!(unknown.tone, CardTone::Warn);
        // An unknown outcome is checked, never simply run again.
        assert!(unknown.actions.reconcile && !unknown.actions.prepare && !unknown.actions.run);
    }

    #[test]
    fn sharing_is_offered_only_for_a_finished_attempt_whose_output_is_held() {
        use crate::commands::history::ShareChoice;

        let history = finished(ExecutionStatus::Failure, Some(3));
        let record = history.get(&call()).unwrap();
        // Finished, but this session no longer holds the output (for example after a restart).
        assert!(!review_card(&input(record)).actions.share);
        let held = review_card(&CardInput {
            output_held: true,
            ..input(record)
        });
        assert!(held.actions.share && !held.actions.unshare && held.staged.is_none());
        // Not while the model is replying.
        assert!(
            !review_card(&CardInput {
                output_held: true,
                chat_busy: true,
                ..input(record)
            })
            .actions
            .share
        );

        // A chosen result is described, can be withdrawn, and is not offered twice.
        let observed = result(ExecutionStatus::Failure, Some(3));
        let choice = ShareChoice::from_edit(&observed.output, None, None).unwrap();
        let staged = SharedResult::observed(&call(), &record.attempts[0], &observed, &choice);
        let chosen = review_card(&CardInput {
            output_held: true,
            staged: Some(&staged),
            ..input(record)
        });
        assert!(!chosen.actions.share && chosen.actions.unshare);
        assert_eq!(
            chosen.staged.as_deref(),
            Some("When you continue, the model receives the outcome only, no output.")
        );

        // A running command has nothing to share yet.
        let mut running = proposed(exec());
        running
            .begin(&call(), InvocationId::new(), exec(), NOW)
            .unwrap();
        assert!(
            !review_card(&CardInput {
                output_held: true,
                ..input(running.get(&call()).unwrap())
            })
            .actions
            .share
        );
    }

    #[test]
    fn a_rerun_is_reviewed_like_a_first_run_and_an_answered_call_is_closed() {
        let mut history = finished(ExecutionStatus::Success, Some(0));
        // No plan: a finished card. With a freshly prepared plan: a review again.
        let record = history.get(&call()).unwrap();
        assert_eq!(review_card(&input(record)).phase, CardPhase::Finished);

        // After the conversation continued, nothing is offered any more.
        let invocation = history.get(&call()).unwrap().attempts[0].invocation.clone();
        history
            .answer(&call(), Answer::Withheld { invocation }, NOW)
            .unwrap();
        let closed = review_card(&CardInput {
            review: Some(CommandState::NeedsReview),
            ..input(history.get(&call()).unwrap())
        });
        assert_eq!(closed.actions, CardActions::default());
        assert_eq!(
            closed.notice.as_deref(),
            Some("The model was told it ran. Its result was not shared.")
        );
    }

    #[test]
    fn rejected_unusable_and_restored_cards_offer_nothing_to_run() {
        let mut history = proposed(exec());
        history.reject(&call(), NOW).unwrap();
        let rejected = review_card(&input(history.get(&call()).unwrap()));
        assert_eq!(rejected.phase, CardPhase::Rejected);
        assert_eq!(rejected.actions, CardActions::default());

        let mut history = CommandHistory::default();
        history
            .propose(
                call(),
                "run_now",
                Err("Unknown tool.".into()),
                Location::Local,
                NOW,
            )
            .unwrap();
        let unusable = review_card(&input(history.get(&call()).unwrap()));
        assert_eq!(unusable.phase, CardPhase::Unusable);
        assert_eq!(unusable.actions, CardActions::default());
        assert_eq!(unusable.notice.as_deref(), Some("Unknown tool."));

        // Restored after the app stopped mid-run.
        let mut history = proposed(exec());
        history
            .begin(&call(), InvocationId::new(), exec(), NOW)
            .unwrap();
        history.restore(NOW);
        let restored = review_card(&input(history.get(&call()).unwrap()));
        assert_eq!(
            restored.status,
            "Outcome unknown: the app stopped while it ran"
        );
        assert!(!restored.actions.run && !restored.actions.prepare);
        assert!(restored.actions.reconcile);
    }

    #[test]
    fn wsl_projects_name_their_distro() {
        let mut history = CommandHistory::default();
        history
            .propose(
                call(),
                "propose_command",
                Ok(exec()),
                Location::Wsl {
                    distro: "Ubuntu".into(),
                },
                NOW,
            )
            .unwrap();
        let card = review_card(&input(history.get(&call()).unwrap()));
        assert_eq!(card.details.last().unwrap().value, "WSL: Ubuntu");
    }
}
