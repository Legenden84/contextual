//! Policy for assistant-proposed commands, independent of model transport and process I/O.
//!
//! Parsing never authorizes execution. The app must obtain a host-prepared [`ReviewPlan`],
//! show it to the user, and call [`ApprovalGate::approve`] only from an explicit user action.
//! The host must prepare it again immediately before [`ApprovalGate::begin`].
//!
//! Policy and non-executing host preparation are described in
//! `docs/plans/approved-command-execution.md`.
//! The execution supervisor consumes [`LaunchPermit`] through an explicitly supplied managed
//! process-tree backend. No native backend or executable host endpoint is enabled yet.

pub mod execution;
pub mod prepare;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::location::ProjectRoot;
use crate::{Error, Result};

/// Maximum encoded arguments for one command proposal.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
/// A command must have a finite timeout no greater than one hour.
pub const MAX_TIMEOUT_SECONDS: u32 = 3600;
/// Bound retained policy state; reaching this limit does not evict replay protection.
pub const MAX_PROPOSALS: usize = 256;

/// Explicit script interpreters. An unavailable interpreter must fail, not be substituted.
/// Other interpreters remain expressible as a direct program and argument vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Interpreter {
    Sh,
    Bash,
    Zsh,
    Pwsh,
    WindowsPowerShell,
    Cmd,
}

/// The model's requested invocation, before host-side program resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Command {
    /// No implicit shell, expansion, argument splitting or platform launcher.
    Exec { program: String, args: Vec<String> },
    /// The full script; interpreter flags and executable are shown in the prepared plan.
    Shell {
        interpreter: Interpreter,
        script: String,
    },
}

/// An explicitly permitted override, not an arbitrary inherited environment variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentOverride {
    pub name: String,
    pub value: String,
}

/// Versioned arguments for the proposed native tool. There are no execution defaults.
/// Call IDs, project identity, host identity and authorization are not model-controlled fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandRequest {
    pub version: u32,
    /// Model-provided explanation, not a safety assessment.
    pub purpose: String,
    pub command: Command,
    /// Project-relative, slash-separated directory; "." means the project root.
    pub cwd: String,
    pub timeout_seconds: u32,
    pub environment: Vec<EnvironmentOverride>,
}

impl CommandRequest {
    /// Checks syntax and version-one policy only. This does not inspect the host or authorize.
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(Error::invalid("Unsupported command request version."));
        }
        if self.purpose.trim().is_empty()
            || self.purpose.len() > 4096
            || self.purpose.contains('\0')
        {
            return Err(Error::invalid("A bounded command purpose is required."));
        }
        if !(1..=MAX_TIMEOUT_SECONDS).contains(&self.timeout_seconds) {
            return Err(Error::invalid(
                "Command timeout must be between 1 and 3600 seconds.",
            ));
        }
        validate_relative_directory(&self.cwd)?;
        match &self.command {
            Command::Exec { program, args } => {
                if program.trim().is_empty()
                    || program.contains('\0')
                    || args.len() > 128
                    || args.iter().any(|arg| arg.contains('\0'))
                {
                    return Err(Error::invalid("Invalid program or argument vector."));
                }
            }
            Command::Shell { script, .. } => {
                if script.trim().is_empty() || script.contains('\0') {
                    return Err(Error::invalid(
                        "A complete, non-empty shell script is required.",
                    ));
                }
            }
        }
        validate_overrides(&self.environment)?;
        if serde_json::to_vec(self)?.len() > MAX_REQUEST_BYTES {
            return Err(Error::invalid("Command request exceeds the size limit."));
        }
        Ok(())
    }
}

/// Decodes only dedicated tool arguments, never Markdown or extracted code fences.
/// The transport must additionally require successful completion of the entire model turn.
pub fn parse_arguments(arguments: &str) -> Result<CommandRequest> {
    if arguments.len() > MAX_REQUEST_BYTES {
        return Err(Error::invalid("Command request exceeds the size limit."));
    }
    // Deserialize directly, rather than through Value, so duplicate struct fields fail.
    let request: CommandRequest = serde_json::from_str(arguments)
        .map_err(|_| Error::invalid("Malformed command arguments."))?;
    request.validate()?;
    Ok(request)
}

fn validate_relative_directory(cwd: &str) -> Result<()> {
    if cwd.is_empty()
        || cwd.contains(['\0', '\\', ':'])
        || cwd.starts_with('/')
        || cwd.split('/').any(|part| part == "..")
    {
        return Err(Error::invalid(
            "Working directory must be project-relative, using '/' and no '..'.",
        ));
    }
    Ok(())
}

/// Version one accepts only a small, documented set of non-secret overrides.
/// PATH, credentials, loader options and tool configuration are not model-overridable.
pub fn validate_overrides(overrides: &[EnvironmentOverride]) -> Result<()> {
    let mut names = HashSet::new();
    for entry in overrides {
        let permitted = match entry.name.as_str() {
            "CI" => matches!(entry.value.as_str(), "true" | "false" | "1" | "0"),
            "NO_COLOR" => entry.value == "1",
            "TERM" => entry.value == "dumb",
            _ => false,
        };
        if !permitted || !names.insert(entry.name.as_str()) {
            return Err(Error::invalid(
                "Environment overrides must be unique: CI=true|false|1|0, NO_COLOR=1, TERM=dumb.",
            ));
        }
    }
    Ok(())
}

/// Existing, canonical directories on the execution host. This is not a sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingDirectory {
    root: PathBuf,
    cwd: PathBuf,
}

impl WorkingDirectory {
    /// Resolves on the project's host, without Windows/WSL path translation.
    /// The host must separately check its root allowlist.
    pub fn resolve(root: &Path, relative: &str) -> Result<Self> {
        validate_relative_directory(relative)?;
        if !root.is_absolute() {
            return Err(Error::invalid("Command project root must be absolute."));
        }
        let root = dunce::canonicalize(root)?;
        let cwd = dunce::canonicalize(root.join(relative))?;
        if !root.is_dir() || !cwd.is_dir() {
            return Err(Error::invalid(
                "Command working directory must exist and be a directory.",
            ));
        }
        // Compare canonical path components conservatively, including on case-sensitive
        // macOS volumes. A string-prefix or case-folded check could accept a sibling.
        if !cwd.starts_with(&root) {
            return Err(Error::forbidden(
                "Command working directory leaves the project.",
            ));
        }
        Ok(Self { root, cwd })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

/// App/host-owned identity. These epochs must change on project reopen and host restart.
/// A distro label alone does not identify the agent instance that prepared a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionBinding {
    pub project: ProjectRoot,
    pub project_epoch: String,
    pub host_epoch: String,
}

/// Everything that can affect the prepared invocation. Equality is deliberately exact.
///
/// Construction belongs to the host preparation layer, not the model parser or GPUI view.
/// `environment` is the complete environment after clearing inherited variables, not a delta.
/// It stays in memory and must not be logged or persisted as part of command history.
#[derive(Clone, PartialEq, Eq)]
pub struct ReviewPlan {
    pub request: CommandRequest,
    pub binding: ExecutionBinding,
    pub platform: contextual_protocol::commands::CommandPlatform,
    pub environment_policy: String,
    pub directory: WorkingDirectory,
    pub program: PathBuf,
    /// Exact OS arguments, including interpreter flags and the full script for shell calls.
    pub args: Vec<OsString>,
    pub environment: BTreeMap<OsString, OsString>,
}

impl ReviewPlan {
    /// Converts a response from the project's trusted host into a review snapshot.
    /// Remote paths are retained as data, not resolved against the app's filesystem.
    /// This conversion does not grant approval.
    pub fn from_prepared(
        project: ProjectRoot,
        prepared: contextual_protocol::commands::PreparedCommand,
    ) -> Result<Self> {
        if prepared.request.root != project.path
            || prepared.request.project_epoch.is_empty()
            || prepared.host_epoch.is_empty()
        {
            return Err(Error::conflict(
                "Command preparation has a different project identity.",
            ));
        }
        if prepared.environment_policy != prepare::ENVIRONMENT_POLICY {
            return Err(Error::invalid("Unsupported command environment policy."));
        }
        let request = parse_arguments(&prepared.request.command_json)?;
        Ok(Self {
            request,
            binding: ExecutionBinding {
                project,
                project_epoch: prepared.request.project_epoch,
                host_epoch: prepared.host_epoch,
            },
            platform: prepared.platform,
            environment_policy: prepared.environment_policy,
            directory: WorkingDirectory {
                root: PathBuf::from(prepared.root),
                cwd: PathBuf::from(prepared.cwd),
            },
            program: PathBuf::from(prepared.program),
            args: prepared.args.into_iter().map(OsString::from).collect(),
            environment: prepared
                .environment
                .into_iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect(),
        })
    }

    fn same_invocation(&self, other: &Self) -> bool {
        // A different model explanation does not turn a duplicate invocation into a new one.
        self.binding == other.binding
            && self.platform == other.platform
            && self.environment_policy == other.environment_policy
            && self.directory == other.directory
            && self.program == other.program
            && self.args == other.args
            && self.environment == other.environment
            && self.request.command == other.request.command
            && self.request.cwd == other.request.cwd
            && self.request.timeout_seconds == other.request.timeout_seconds
            && self.request.environment == other.request.environment
    }
}

/// Provider call ID scoped by an app-generated, globally unique logical turn ID.
/// Retries of a model request must retain that turn ID; follow-up turns receive a new one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallId {
    pub turn: String,
    pub call: String,
}

/// A terminal execution observation, not an assertion that side effects were undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Outcome {
    Success,
    Failure { code: Option<i32> },
    Cancelled,
    TimedOut,
    FailedToStart,
    Unknown,
}

/// Observable policy state. Authorization itself is held separately and never serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandState {
    NeedsReview,
    Approved,
    Running,
    Rejected,
    Finished(Outcome),
}

struct Entry {
    plan: ReviewPlan,
    revision: Uuid,
    state: CommandState,
}

/// Single-use, in-memory authorization. It intentionally implements neither Clone nor serde.
pub struct Approval {
    gate: Uuid,
    call: CallId,
    revision: Uuid,
}

/// Permission to dispatch exactly one prepared invocation. The future execution service must
/// consume this value, revalidate the host-side preparation, and never retry dispatch.
///
/// A permit is not a completed execution. Once issued, the gate remains busy until a terminal
/// observation is recorded, even if a caller loses or drops this value.
pub struct LaunchPermit {
    call: CallId,
    plan: ReviewPlan,
}

impl LaunchPermit {
    pub fn call(&self) -> &CallId {
        &self.call
    }

    pub fn plan(&self) -> &ReviewPlan {
        &self.plan
    }
}

/// One gate per open project, shared across its conversations. No implicit queue exists.
///
/// This is trusted application policy, not authentication against malicious in-process code.
/// The UI must not call `approve` from parsing, restoration, timers or model continuations.
pub struct ApprovalGate {
    id: Uuid,
    binding: ExecutionBinding,
    entries: HashMap<CallId, Entry>,
}

impl ApprovalGate {
    pub fn new(binding: ExecutionBinding) -> Self {
        Self {
            id: Uuid::new_v4(),
            binding,
            entries: HashMap::new(),
        }
    }

    /// Adds a prepared proposal without approving it. Duplicate calls and identical invocations
    /// within one logical turn are refused, including when their explanation differs.
    pub fn propose(&mut self, call: CallId, plan: ReviewPlan) -> Result<()> {
        plan.request.validate()?;
        if call.turn.is_empty() || call.call.is_empty() {
            return Err(Error::invalid("Command call identity is required."));
        }
        if plan.binding != self.binding {
            return Err(Error::conflict(
                "Command belongs to a different project or host instance.",
            ));
        }
        if self.entries.contains_key(&call) {
            return Err(Error::conflict("Command call was already received."));
        }
        if self
            .entries
            .iter()
            .any(|(id, entry)| id.turn == call.turn && entry.plan.same_invocation(&plan))
        {
            return Err(Error::conflict("Duplicate invocation in this model turn."));
        }
        if self.entries.len() >= MAX_PROPOSALS {
            return Err(Error::invalid(
                "Command proposal limit reached for this project session.",
            ));
        }
        self.entries.insert(
            call,
            Entry {
                plan,
                revision: Uuid::new_v4(),
                state: CommandState::NeedsReview,
            },
        );
        Ok(())
    }

    pub fn state(&self, call: &CallId) -> Option<CommandState> {
        self.entries.get(call).map(|entry| entry.state)
    }

    /// Running and disconnected/unknown executions both retain the project slot.
    pub fn busy(&self) -> bool {
        self.entries.values().any(|entry| {
            matches!(
                entry.state,
                CommandState::Running | CommandState::Finished(Outcome::Unknown)
            )
        })
    }

    /// Invalidates authorization immediately when editing starts, even if the editor is later
    /// cancelled or the new draft is malformed.
    pub fn invalidate(&mut self, call: &CallId) -> Result<()> {
        let entry = self.editable(call)?;
        entry.revision = Uuid::new_v4();
        entry.state = CommandState::NeedsReview;
        Ok(())
    }

    /// Replaces a pending plan. Even a failed replacement invalidates the old approval.
    pub fn edit(&mut self, call: &CallId, plan: ReviewPlan) -> Result<()> {
        self.invalidate(call)?;
        plan.request.validate()?;
        if plan.binding != self.binding {
            return Err(Error::conflict(
                "Command belongs to a different project or host instance.",
            ));
        }
        self.editable(call)?.plan = plan;
        Ok(())
    }

    fn editable(&mut self, call: &CallId) -> Result<&mut Entry> {
        let entry = self
            .entries
            .get_mut(call)
            .ok_or_else(|| Error::not_found("Command proposal is no longer available."))?;
        if !matches!(
            entry.state,
            CommandState::NeedsReview | CommandState::Approved
        ) {
            return Err(Error::conflict(
                "This command is no longer awaiting review.",
            ));
        }
        Ok(entry)
    }

    /// Explicit user rejection. It cannot be reversed into approval for this call.
    pub fn reject(&mut self, call: &CallId) -> Result<()> {
        let entry = self.editable(call)?;
        entry.revision = Uuid::new_v4();
        entry.state = CommandState::Rejected;
        Ok(())
    }

    /// Called only after the user reviews this exact snapshot and presses Run.
    /// Output sharing is a separate action and is not authorized by this method.
    pub fn approve(&mut self, call: &CallId, displayed: &ReviewPlan) -> Result<Approval> {
        if self.busy() {
            return Err(Error::conflict(
                "A command is running or its outcome is unknown.",
            ));
        }
        let gate = self.id;
        let entry = self.editable(call)?;
        if &entry.plan != displayed {
            entry.revision = Uuid::new_v4();
            entry.state = CommandState::NeedsReview;
            return Err(Error::conflict(
                "Command changed. Review it again before running.",
            ));
        }
        // Repeated clicks replace, rather than duplicate, the one pending approval.
        entry.revision = Uuid::new_v4();
        entry.state = CommandState::Approved;
        Ok(Approval {
            gate,
            call: call.clone(),
            revision: entry.revision,
        })
    }

    /// Consumes approval after fresh host preparation. Any parameter or host/project change
    /// refuses launch and requires another explicit approval. This method never starts I/O.
    pub fn begin(&mut self, approval: Approval, current: &ReviewPlan) -> Result<LaunchPermit> {
        if approval.gate != self.id {
            return Err(Error::conflict(
                "Approval belongs to an earlier project session.",
            ));
        }
        let busy = self.busy();
        let entry = self
            .entries
            .get_mut(&approval.call)
            .ok_or_else(|| Error::not_found("Command proposal is no longer available."))?;
        if entry.state != CommandState::Approved || entry.revision != approval.revision {
            return Err(Error::conflict("Command approval is no longer valid."));
        }
        if busy || &entry.plan != current {
            entry.revision = Uuid::new_v4();
            entry.state = CommandState::NeedsReview;
            return Err(Error::conflict(
                "Command or execution state changed. Review it again before running.",
            ));
        }
        entry.state = CommandState::Running;
        Ok(LaunchPermit {
            call: approval.call,
            plan: entry.plan.clone(),
        })
    }

    /// Records an observation; completion never starts a pending command.
    /// Unknown outcomes retain the slot until a later execution-lifecycle implementation can
    /// establish termination. They must not be converted to success or cancellation by guess.
    pub fn finish(&mut self, call: &CallId, outcome: Outcome) -> Result<()> {
        let entry = self
            .entries
            .get_mut(call)
            .ok_or_else(|| Error::not_found("Command proposal is no longer available."))?;
        if entry.state != CommandState::Running {
            return Err(Error::conflict("Command has no active execution."));
        }
        entry.state = CommandState::Finished(outcome);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Location;

    fn request() -> CommandRequest {
        CommandRequest {
            version: 1,
            purpose: "Run the project's tests.".into(),
            command: Command::Exec {
                program: "cargo".into(),
                args: vec!["test".into()],
            },
            cwd: ".".into(),
            timeout_seconds: 60,
            environment: Vec::new(),
        }
    }

    fn plan() -> ReviewPlan {
        let root = PathBuf::from("project");
        ReviewPlan {
            request: request(),
            binding: ExecutionBinding {
                project: ProjectRoot {
                    location: Location::Local,
                    path: "project".into(),
                },
                project_epoch: "project-1".into(),
                host_epoch: "host-1".into(),
            },
            platform: contextual_protocol::commands::CommandPlatform::Linux,
            environment_policy: prepare::ENVIRONMENT_POLICY.into(),
            directory: WorkingDirectory {
                root: root.clone(),
                cwd: root,
            },
            program: PathBuf::from("cargo"),
            args: vec![OsString::from("test")],
            environment: BTreeMap::new(),
        }
    }

    fn call(turn: &str, id: &str) -> CallId {
        CallId {
            turn: turn.into(),
            call: id.into(),
        }
    }

    fn proposed() -> (ApprovalGate, CallId, ReviewPlan) {
        let plan = plan();
        let mut gate = ApprovalGate::new(plan.binding.clone());
        let id = call("turn-1", "call-1");
        gate.propose(id.clone(), plan.clone()).unwrap();
        (gate, id, plan)
    }

    #[test]
    fn parsing_and_proposal_do_not_authorize() {
        let parsed = parse_arguments(&serde_json::to_string(&request()).unwrap()).unwrap();
        let (gate, id, plan) = proposed();
        assert_eq!(parsed, plan.request);
        assert_eq!(gate.state(&id), Some(CommandState::NeedsReview));
        assert!(!gate.busy());
    }

    #[test]
    fn malformed_partial_and_ordinary_markdown_are_not_arguments() {
        for input in [
            r#"{"version":1"#,
            "```sh\ncargo test\n```",
            "Run cargo test",
            r#"{"version":1,"approved":true}"#,
        ] {
            assert!(parse_arguments(input).is_err(), "{input}");
        }
    }

    #[test]
    fn unknown_and_duplicate_fields_are_refused() {
        let json = serde_json::to_string(&request()).unwrap();
        let unknown = json.replacen('{', r#"{"approved":true,"#, 1);
        let duplicate = json.replacen('{', r#"{"version":1,"#, 1);
        assert!(parse_arguments(&unknown).is_err());
        assert!(parse_arguments(&duplicate).is_err());
    }

    #[test]
    fn shell_script_is_preserved_without_translation() {
        let mut request = request();
        request.command = Command::Shell {
            interpreter: Interpreter::Bash,
            script: "printf '%s\\n' \"$HOME\" | head -1 > result.txt\n".into(),
        };
        let parsed = parse_arguments(&serde_json::to_string(&request).unwrap()).unwrap();
        assert_eq!(parsed, request);
    }

    #[test]
    fn unpermitted_environment_is_not_silently_filtered() {
        let mut request = request();
        request.environment.push(EnvironmentOverride {
            name: "AZURE_OPENAI_API_KEY".into(),
            value: "not-a-real-key".into(),
        });
        assert!(request.validate().is_err());
    }

    #[test]
    fn duplicate_overrides_are_refused() {
        let override_value = EnvironmentOverride {
            name: "CI".into(),
            value: "true".into(),
        };
        assert!(validate_overrides(&[override_value.clone(), override_value]).is_err());
    }

    #[test]
    fn approved_exact_plan_produces_one_permit() {
        let (mut gate, id, plan) = proposed();
        let approval = gate.approve(&id, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        assert_eq!(permit.call(), &id);
        assert!(permit.plan() == &plan);
        assert!(gate.approve(&id, &plan).is_err());
        assert!(gate.busy());
    }

    #[test]
    fn rejection_invalidates_an_outstanding_approval() {
        let (mut gate, id, plan) = proposed();
        let approval = gate.approve(&id, &plan).unwrap();
        gate.reject(&id).unwrap();
        assert!(gate.begin(approval, &plan).is_err());
        assert_eq!(gate.state(&id), Some(CommandState::Rejected));
    }

    #[test]
    fn opening_an_editor_invalidates_approval_even_without_a_change() {
        let (mut gate, id, plan) = proposed();
        let approval = gate.approve(&id, &plan).unwrap();
        gate.invalidate(&id).unwrap();
        assert!(gate.begin(approval, &plan).is_err());
    }

    #[test]
    fn malformed_edit_also_invalidates_approval() {
        let (mut gate, id, plan) = proposed();
        let approval = gate.approve(&id, &plan).unwrap();
        let mut changed = plan.clone();
        changed.request.timeout_seconds = 0;
        assert!(gate.edit(&id, changed).is_err());
        assert!(gate.begin(approval, &plan).is_err());
    }

    #[test]
    fn duplicate_clicks_cannot_produce_two_launch_permits() {
        let (mut gate, id, plan) = proposed();
        let earlier = gate.approve(&id, &plan).unwrap();
        let latest = gate.approve(&id, &plan).unwrap();
        assert!(gate.begin(earlier, &plan).is_err());
        assert!(gate.begin(latest, &plan).is_ok());
        assert!(gate.approve(&id, &plan).is_err());
    }

    #[test]
    fn duplicate_requests_do_not_create_another_action() {
        let (mut gate, id, plan) = proposed();
        assert!(gate.propose(id, plan.clone()).is_err());
        let mut duplicate = plan;
        duplicate.request.purpose = "A different explanation.".into();
        assert!(gate.propose(call("turn-1", "call-2"), duplicate).is_err());
    }

    #[test]
    fn every_followup_needs_its_own_approval() {
        let (mut gate, id, plan) = proposed();
        let approval = gate.approve(&id, &plan).unwrap();
        let _permit = gate.begin(approval, &plan).unwrap();
        gate.finish(&id, Outcome::Success).unwrap();
        let followup = call("turn-2", "call-2");
        gate.propose(followup.clone(), plan).unwrap();
        assert_eq!(gate.state(&followup), Some(CommandState::NeedsReview));
    }

    #[test]
    fn completion_does_not_launch_or_approve_the_next_command() {
        let (mut gate, id, plan) = proposed();
        let second = call("turn-2", "call-2");
        gate.propose(second.clone(), plan.clone()).unwrap();
        let approval = gate.approve(&id, &plan).unwrap();
        let _permit = gate.begin(approval, &plan).unwrap();
        assert!(gate.approve(&second, &plan).is_err());
        gate.finish(&id, Outcome::TimedOut).unwrap();
        assert_eq!(gate.state(&second), Some(CommandState::NeedsReview));
    }

    #[test]
    fn reopening_cannot_reuse_an_approval_even_with_identical_parameters() {
        let (mut old, id, plan) = proposed();
        let approval = old.approve(&id, &plan).unwrap();
        let mut reopened = ApprovalGate::new(plan.binding.clone());
        reopened.propose(id.clone(), plan.clone()).unwrap();
        assert!(reopened.begin(approval, &plan).is_err());
        assert_eq!(reopened.state(&id), Some(CommandState::NeedsReview));
    }

    #[test]
    fn changed_execution_parameters_require_review_again() {
        let (mut gate, id, plan) = proposed();
        let approval = gate.approve(&id, &plan).unwrap();
        let mut current = plan;
        current.args.push(OsString::from("--release"));
        assert!(gate.begin(approval, &current).is_err());
        assert_eq!(gate.state(&id), Some(CommandState::NeedsReview));
    }

    #[test]
    fn changed_host_project_or_environment_invalidates_approval() {
        let base = plan();
        let mut changed_host = base.clone();
        changed_host.binding.host_epoch = "host-2".into();
        let mut changed_project = base.clone();
        changed_project.binding.project_epoch = "project-2".into();
        let mut changed_environment = base.clone();
        changed_environment
            .environment
            .insert(OsString::from("PATH"), OsString::from("different"));
        for current in [changed_host, changed_project, changed_environment] {
            let (mut gate, id, plan) = proposed();
            let approval = gate.approve(&id, &plan).unwrap();
            assert!(gate.begin(approval, &current).is_err());
        }
    }

    #[test]
    fn wsl_approval_cannot_be_used_for_a_local_plan() {
        let mut plan = plan();
        plan.binding.project.location = Location::Wsl {
            distro: "Ubuntu".into(),
        };
        let mut gate = ApprovalGate::new(plan.binding.clone());
        let id = call("turn-1", "call-1");
        gate.propose(id.clone(), plan.clone()).unwrap();
        let approval = gate.approve(&id, &plan).unwrap();
        plan.binding.project.location = Location::Local;
        assert!(gate.begin(approval, &plan).is_err());
    }

    #[test]
    fn unknown_outcomes_block_new_execution_and_cannot_be_retried() {
        let (mut gate, id, plan) = proposed();
        let next = call("turn-2", "call-2");
        gate.propose(next.clone(), plan.clone()).unwrap();
        let approval = gate.approve(&id, &plan).unwrap();
        let _permit = gate.begin(approval, &plan).unwrap();
        gate.finish(&id, Outcome::Unknown).unwrap();
        assert!(gate.busy());
        assert!(gate.approve(&id, &plan).is_err());
        assert!(gate.approve(&next, &plan).is_err());
    }

    #[test]
    fn working_directory_must_exist_inside_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("file"), "not a directory").unwrap();
        let directory = WorkingDirectory::resolve(tmp.path(), "src").unwrap();
        assert_eq!(
            directory.cwd(),
            dunce::canonicalize(tmp.path().join("src")).unwrap()
        );
        for path in ["../outside", "/outside", r"C:\outside", "missing", "file"] {
            assert!(
                WorkingDirectory::resolve(tmp.path(), path).is_err(),
                "{path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn working_directory_cannot_escape_through_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        assert!(WorkingDirectory::resolve(&root, "escape").is_err());
    }
}
