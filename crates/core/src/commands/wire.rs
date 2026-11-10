//! Bounded execution-message codecs, independent of RPC dispatch and process creation.
//!
//! Decoding produces data, never a LaunchPermit. An agent must still establish its trusted
//! connection binding, re-prepare on that host, enforce durable replay protection and acquire
//! admission before launch. These helpers do not enable an executable endpoint.
//!
//! Decode raw JSON directly, not through Value: duplicate fields must remain detectable.
//! The transport must also bound framing before accumulating a complete JSON-RPC line.
//! Limits here apply to command payloads, not arbitrary workspace scan/read responses.

use contextual_protocol::commands::{
    CancelCommand, CommandExecutionIdentity, CommandExecutionStatus, CommandLocation,
    CommandOutput, CommandProgress, CommandResult, CommandStreamOutput, DispatchCommand,
    EXECUTION_VERSION, PreparedCommand,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::execution::{
    CapturedOutput, ExecutionProgress, ExecutionResult, ExecutionStatus, OUTPUT_LIMIT,
    escape_controls,
};
use super::prepare::{BASE_VARIABLES, ENVIRONMENT_POLICY, MAX_ENVIRONMENT_BYTES};
use super::{
    CallId, CommandRequest, ExecutionBinding, InvocationId, LaunchPermit, MAX_REQUEST_BYTES,
    ReviewPlan, parse_arguments,
};
use crate::location::Location;
use crate::{Error, Result};

/// Limits are checked before deserialization. Unsupported sizes fail rather than truncate.
pub const MAX_DISPATCH_BYTES: usize = 1024 * 1024;
/// Includes escaped output, JSON escaping, binding paths and bounded diagnostic text.
pub const MAX_OBSERVATION_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CANCEL_BYTES: usize = 256 * 1024;

const MAX_ID_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_DETAIL_BYTES: usize = 4096;
const MAX_ENVIRONMENT_ENTRIES: usize = 16;

// One raw ASCII control byte can become six UTF-8 bytes, e.g. "\\u{7f}".
// Lossy UTF-8 replacement and escaping multi-byte formatting characters expand less.
const MAX_TEXT_BYTES_PER_RAW_BYTE: usize = 6;

fn invalid() -> Error {
    Error::invalid("Malformed, inconsistent or oversized command execution message.")
}

fn text(value: &str, limit: usize, empty_allowed: bool) -> Result<()> {
    if (!empty_allowed && value.is_empty()) || value.len() > limit || value.contains('\0') {
        return Err(invalid());
    }
    Ok(())
}

fn identifier(value: &str) -> Result<()> {
    text(value, MAX_ID_BYTES, false)?;
    if value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}

fn identity(value: &CommandExecutionIdentity) -> Result<InvocationId> {
    let invocation = InvocationId::parse(&value.invocation_id)?;
    text(&value.root, MAX_PATH_BYTES, false)?;
    identifier(&value.project_epoch)?;
    identifier(&value.host_epoch)?;
    Ok(invocation)
}

fn decode<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T> {
    if bytes.len() > limit {
        return Err(invalid());
    }
    // Do not expose serde diagnostics: they can contain private values from the payload.
    serde_json::from_slice(bytes).map_err(|_| invalid())
}

fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
    if bytes.len() > limit {
        return Err(invalid());
    }
    Ok(bytes)
}

fn same_location(wire: &CommandLocation, expected: &Location) -> bool {
    match (wire, expected) {
        (CommandLocation::Local, Location::Local) => true,
        (CommandLocation::Wsl { distro: left }, Location::Wsl { distro: right }) => left == right,
        _ => false,
    }
}

fn validate_prepared(prepared: &PreparedCommand, request: &CommandRequest) -> Result<()> {
    text(&prepared.request.root, MAX_PATH_BYTES, false)?;
    identifier(&prepared.request.project_epoch)?;
    identifier(&prepared.host_epoch)?;
    for path in [&prepared.root, &prepared.cwd, &prepared.program] {
        text(path, MAX_PATH_BYTES, false)?;
    }
    if prepared.environment_policy != ENVIRONMENT_POLICY || prepared.args.len() > 128 {
        return Err(invalid());
    }
    let mut argument_bytes = 0usize;
    for argument in &prepared.args {
        text(argument, MAX_REQUEST_BYTES, true)?;
        argument_bytes = argument_bytes
            .checked_add(argument.len())
            .ok_or_else(invalid)?;
    }
    if argument_bytes > MAX_REQUEST_BYTES
        || prepared.environment.len() > MAX_ENVIRONMENT_ENTRIES
        || !prepared.environment.contains_key("PATH")
    {
        return Err(invalid());
    }

    let mut environment_bytes = 0usize;
    for (name, value) in &prepared.environment {
        identifier(name)?;
        text(value, MAX_ENVIRONMENT_BYTES, true)?;
        let permitted = BASE_VARIABLES.contains(&name.as_str())
            || request
                .environment
                .iter()
                .any(|entry| entry.name == *name && entry.value == *value);
        if !permitted {
            return Err(invalid());
        }
        environment_bytes = environment_bytes
            .checked_add(name.len())
            .and_then(|bytes| bytes.checked_add(value.len()))
            .ok_or_else(invalid)?;
    }
    if environment_bytes > MAX_ENVIRONMENT_BYTES
        || request
            .environment
            .iter()
            .any(|entry| prepared.environment.get(&entry.name) != Some(&entry.value))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Parsed dispatch data. This is neither approval nor a host revalidation proof.
/// It intentionally has no public conversion into LaunchPermit.
pub struct DecodedDispatch {
    invocation: InvocationId,
    call: CallId,
    plan: ReviewPlan,
}

impl DecodedDispatch {
    pub fn invocation(&self) -> &InvocationId {
        &self.invocation
    }

    pub fn call(&self) -> &CallId {
        &self.call
    }

    pub fn plan(&self) -> &ReviewPlan {
        &self.plan
    }

    /// The execution service must obtain `current` from fresh preparation on the bound host.
    /// Equality does not grant authorization or acquire execution admission.
    pub fn check_preparation(&self, current: &ReviewPlan) -> Result<()> {
        if &self.plan != current {
            return Err(Error::conflict(
                "Command preparation changed. Review and approve again.",
            ));
        }
        Ok(())
    }
}

/// Decodes against an independently established app/project/host binding, not one taken
/// from the incoming envelope. Paths remain opaque host-native data until host preparation.
pub fn decode_dispatch(bytes: &[u8], expected: &ExecutionBinding) -> Result<DecodedDispatch> {
    let dispatch: DispatchCommand = decode(bytes, MAX_DISPATCH_BYTES)?;
    let invocation = identity(&dispatch.execution)?;
    identifier(&dispatch.turn_id)?;
    identifier(&dispatch.call_id)?;
    if let CommandLocation::Wsl { distro } = &dispatch.location {
        identifier(distro)?;
    }
    if dispatch.version != EXECUTION_VERSION
        || !same_location(&dispatch.location, &expected.project.location)
        || dispatch.execution.root != expected.project.path
        || dispatch.execution.project_epoch != expected.project_epoch
        || dispatch.execution.host_epoch != expected.host_epoch
        || dispatch.execution.root != dispatch.prepared.request.root
        || dispatch.execution.project_epoch != dispatch.prepared.request.project_epoch
        || dispatch.execution.host_epoch != dispatch.prepared.host_epoch
    {
        return Err(Error::conflict(
            "Command execution message belongs to a different version, project or host.",
        ));
    }

    let request = parse_arguments(&dispatch.prepared.request.command_json)?;
    validate_prepared(&dispatch.prepared, &request)?;
    let plan = ReviewPlan::from_prepared(expected.project.clone(), dispatch.prepared)?;
    Ok(DecodedDispatch {
        invocation,
        call: CallId {
            turn: dispatch.turn_id,
            call: dispatch.call_id,
        },
        plan,
    })
}

/// Consumes one permit to encode its exact dispatch parameters. This does not send or launch.
///
/// Encoded bytes remain ordinary, copyable transport data. The eventual receiver must enforce
/// durable invocation-ID protection; serializing an envelope is not reusable authorization.
/// On error the permit is consumed and the caller must finish the gate attempt as failed
/// to start. No automatic re-preparation or retry is performed.
pub fn encode_dispatch(permit: LaunchPermit) -> Result<Vec<u8>> {
    let plan = permit.plan();
    plan.request.validate()?;
    let os_text = |value: &std::ffi::OsStr| {
        value
            .to_str()
            .filter(|value| !value.contains('\0'))
            .map(str::to_owned)
            .ok_or_else(invalid)
    };
    let prepared = PreparedCommand {
        request: contextual_protocol::commands::PrepareCommand {
            root: plan.binding.project.path.clone(),
            project_epoch: plan.binding.project_epoch.clone(),
            command_json: serde_json::to_string(&plan.request).map_err(|_| invalid())?,
        },
        host_epoch: plan.binding.host_epoch.clone(),
        platform: plan.platform,
        environment_policy: plan.environment_policy.clone(),
        root: os_text(plan.directory.root().as_os_str())?,
        cwd: os_text(plan.directory.cwd().as_os_str())?,
        program: os_text(plan.program.as_os_str())?,
        args: plan
            .args
            .iter()
            .map(|argument| os_text(argument.as_os_str()))
            .collect::<Result<_>>()?,
        environment: plan
            .environment
            .iter()
            .map(|(name, value)| Ok((os_text(name)?, os_text(value)?)))
            .collect::<Result<_>>()?,
    };
    validate_prepared(&prepared, &plan.request)?;
    let execution = CommandExecutionIdentity {
        invocation_id: permit.invocation().as_str().to_owned(),
        root: plan.binding.project.path.clone(),
        project_epoch: plan.binding.project_epoch.clone(),
        host_epoch: plan.binding.host_epoch.clone(),
    };
    identity(&execution)?;
    identifier(&permit.call().turn)?;
    identifier(&permit.call().call)?;
    let location = match &plan.binding.project.location {
        Location::Local => CommandLocation::Local,
        Location::Wsl { distro } => {
            identifier(distro)?;
            CommandLocation::Wsl {
                distro: distro.clone(),
            }
        }
    };
    encode(
        &DispatchCommand {
            version: EXECUTION_VERSION,
            execution,
            turn_id: permit.call().turn.clone(),
            call_id: permit.call().call.clone(),
            location,
            prepared,
        },
        MAX_DISPATCH_BYTES,
    )
}

/// Validates Stop correlation only. It neither signals a process nor cancels an RPC task.
pub fn decode_cancel(bytes: &[u8], expected: &CommandExecutionIdentity) -> Result<()> {
    identity(expected)?;
    let request: CancelCommand = decode(bytes, MAX_CANCEL_BYTES)?;
    identity(&request.execution)?;
    if request.version != EXECUTION_VERSION || &request.execution != expected {
        return Err(Error::conflict(
            "Stop request does not match the active command.",
        ));
    }
    Ok(())
}

fn validate_stream(stream: &CommandStreamOutput) -> Result<()> {
    let retained = stream.bytes_seen.min(OUTPUT_LIMIT as u64);
    if stream.bytes_retained != retained
        || stream.truncated != (stream.bytes_seen > retained)
        || (stream.complete && stream.read_failed)
    {
        return Err(invalid());
    }
    let retained = retained as usize;
    if stream.text.len() < retained
        || stream.text.len() > retained * MAX_TEXT_BYTES_PER_RAW_BYTE
        || escape_controls(&stream.text) != stream.text
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_output(output: &CommandOutput) -> Result<()> {
    validate_stream(&output.stdout)?;
    validate_stream(&output.stderr)
}

fn advances(previous: &CommandStreamOutput, next: &CommandStreamOutput) -> bool {
    if previous.complete || previous.read_failed {
        return previous == next;
    }
    if next.bytes_seen < previous.bytes_seen || next.bytes_retained < previous.bytes_retained {
        return false;
    }
    if next.bytes_retained == previous.bytes_retained {
        return next.text == previous.text;
    }
    // A snapshot can end in a provisional replacement character when a UTF-8 code point
    // spans reads. Only that last character may change as the retained byte prefix grows.
    let stable_prefix = previous
        .text
        .strip_suffix('\u{fffd}')
        .unwrap_or(&previous.text);
    next.text.starts_with(stable_prefix)
}

fn core_stream(stream: &CommandStreamOutput) -> CapturedOutput {
    CapturedOutput {
        text: stream.text.clone(),
        bytes_seen: stream.bytes_seen,
        bytes_retained: stream.bytes_retained as usize,
        truncated: stream.truncated,
        complete: stream.complete,
        read_failed: stream.read_failed,
    }
}

fn core_output(output: &CommandOutput) -> ExecutionProgress {
    ExecutionProgress {
        stdout: core_stream(&output.stdout),
        stderr: core_stream(&output.stderr),
    }
}

fn validate_result(result: &CommandResult) -> Result<ExecutionStatus> {
    use CommandExecutionStatus as Status;

    validate_output(&result.output)?;
    for detail in [&result.detail, &result.history_error]
        .into_iter()
        .flatten()
    {
        text(detail, MAX_DETAIL_BYTES, false)?;
        if escape_controls(detail) != *detail {
            return Err(invalid());
        }
    }
    if result.cancelled && result.timed_out {
        return Err(invalid());
    }
    if !result.spawned {
        for stream in [&result.output.stdout, &result.output.stderr] {
            if stream.bytes_seen != 0 || stream.complete || stream.read_failed {
                return Err(invalid());
            }
        }
        if result.exit_code.is_some() {
            return Err(invalid());
        }
    }
    let status = match result.status {
        Status::Success
            if result.spawned
                && result.exit_code == Some(0)
                && !result.cancelled
                && !result.timed_out =>
        {
            ExecutionStatus::Success
        }
        Status::Failure
            if result.spawned
                && result.exit_code != Some(0)
                && !result.cancelled
                && !result.timed_out =>
        {
            ExecutionStatus::Failure
        }
        Status::FailedToStart if !result.spawned && !result.cancelled && !result.timed_out => {
            ExecutionStatus::FailedToStart
        }
        Status::Cancelled if result.cancelled && !result.timed_out => ExecutionStatus::Cancelled,
        Status::TimedOut if result.spawned && result.timed_out && !result.cancelled => {
            ExecutionStatus::TimedOut
        }
        Status::Unknown => ExecutionStatus::Unknown,
        _ => return Err(invalid()),
    };
    Ok(status)
}

/// Builds a bounded producer snapshot using the same validation as the receiving boundary.
pub fn encode_progress(
    execution: &CommandExecutionIdentity,
    sequence: u64,
    output: &ExecutionProgress,
) -> Result<String> {
    identity(execution)?;
    if sequence == 0 {
        return Err(invalid());
    }
    let output = wire_output(output);
    validate_output(&output)?;
    let bytes = encode(
        &CommandProgress {
            version: EXECUTION_VERSION,
            execution: execution.clone(),
            sequence,
            output,
        },
        MAX_OBSERVATION_BYTES,
    )?;
    String::from_utf8(bytes).map_err(|_| invalid())
}

/// Builds a final observation. Diagnostic text is escaped and bounded; output is not silently
/// changed or truncated here because its raw-byte counters must remain meaningful.
pub fn encode_result(
    execution: &CommandExecutionIdentity,
    sequence: u64,
    result: &ExecutionResult,
) -> Result<String> {
    identity(execution)?;
    if sequence == 0 {
        return Err(invalid());
    }
    let status = match result.status {
        ExecutionStatus::Success => CommandExecutionStatus::Success,
        ExecutionStatus::Failure => CommandExecutionStatus::Failure,
        ExecutionStatus::FailedToStart => CommandExecutionStatus::FailedToStart,
        ExecutionStatus::Cancelled => CommandExecutionStatus::Cancelled,
        ExecutionStatus::TimedOut => CommandExecutionStatus::TimedOut,
        ExecutionStatus::Unknown => CommandExecutionStatus::Unknown,
    };
    let message = CommandResult {
        version: EXECUTION_VERSION,
        execution: execution.clone(),
        final_sequence: sequence,
        status,
        exit_code: result.exit_code,
        spawned: result.spawned,
        cancelled: result.cancelled,
        timed_out: result.timed_out,
        output: wire_output(&result.output),
        detail: bounded_detail(result.detail.as_deref()),
        history_error: bounded_detail(result.history_error.as_deref()),
    };
    validate_result(&message)?;
    String::from_utf8(encode(&message, MAX_OBSERVATION_BYTES)?).map_err(|_| invalid())
}

fn bounded_detail(value: Option<&str>) -> Option<String> {
    let mut value = escape_controls(value?);
    if value.is_empty() {
        return None;
    }
    let mut end = value.len().min(MAX_DETAIL_BYTES);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    Some(value)
}

fn wire_output(output: &ExecutionProgress) -> CommandOutput {
    let stream = |output: &CapturedOutput| CommandStreamOutput {
        text: output.text.clone(),
        bytes_seen: output.bytes_seen,
        bytes_retained: output.bytes_retained as u64,
        truncated: output.truncated,
        complete: output.complete,
        read_failed: output.read_failed,
    };
    CommandOutput {
        stdout: stream(&output.stdout),
        stderr: stream(&output.stderr),
    }
}

/// Fail-closed observation stream for one independently bound invocation.
///
/// Retains at most one bounded cumulative snapshot. Malformed data permanently invalidates
/// this receiver: a later success cannot erase lost trust. The owner must report Unknown
/// and retain the project slot, not retry dispatch. A new receiver is not reconciliation.
pub struct ObservationReceiver {
    execution: CommandExecutionIdentity,
    latest: Option<(u64, CommandOutput)>,
    failed: bool,
    finished: bool,
}

impl ObservationReceiver {
    pub fn new(execution: CommandExecutionIdentity) -> Result<Self> {
        identity(&execution)?;
        Ok(Self {
            execution,
            latest: None,
            failed: false,
            finished: false,
        })
    }

    fn begin_message(&mut self) -> Result<()> {
        if self.failed || self.finished {
            return Err(Error::conflict(
                "Command observation stream is closed or unverified.",
            ));
        }
        // Every subsequent error leaves the receiver poisoned. Clear this only after the
        // entire message, including its cumulative output and terminal flags, is verified.
        self.failed = true;
        Ok(())
    }

    fn check_snapshot(
        &self,
        version: u32,
        execution: &CommandExecutionIdentity,
        sequence: u64,
        output: &CommandOutput,
    ) -> Result<()> {
        if version != EXECUTION_VERSION || execution != &self.execution || sequence == 0 {
            return Err(invalid());
        }
        validate_output(output)?;
        if let Some((previous_sequence, previous)) = &self.latest
            && (sequence <= *previous_sequence
                || !advances(&previous.stdout, &output.stdout)
                || !advances(&previous.stderr, &output.stderr))
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn progress(&mut self, bytes: &[u8]) -> Result<ExecutionProgress> {
        self.begin_message()?;
        let message: CommandProgress = decode(bytes, MAX_OBSERVATION_BYTES)?;
        self.check_snapshot(
            message.version,
            &message.execution,
            message.sequence,
            &message.output,
        )?;
        let output = core_output(&message.output);
        self.latest = Some((message.sequence, message.output));
        self.failed = false;
        Ok(output)
    }

    pub fn finish(&mut self, bytes: &[u8]) -> Result<ExecutionResult> {
        self.begin_message()?;
        let message: CommandResult = decode(bytes, MAX_OBSERVATION_BYTES)?;
        self.check_snapshot(
            message.version,
            &message.execution,
            message.final_sequence,
            &message.output,
        )?;
        let status = validate_result(&message)?;
        let result = ExecutionResult {
            status,
            exit_code: message.exit_code,
            spawned: message.spawned,
            cancelled: message.cancelled,
            timed_out: message.timed_out,
            output: core_output(&message.output),
            detail: message.detail,
            history_error: message.history_error,
        };
        self.latest = Some((message.final_sequence, message.output));
        self.failed = false;
        self.finished = true;
        Ok(result)
    }

    /// Reports lost or unverifiable completion using only the last accepted snapshot.
    /// It never guesses termination, retries dispatch, or replaces a verified final result.
    pub fn disconnect(&mut self) -> Result<ExecutionResult> {
        if self.finished {
            return Err(Error::conflict(
                "A final command observation was already received.",
            ));
        }
        self.failed = true;
        Ok(ExecutionResult {
            status: ExecutionStatus::Unknown,
            exit_code: None,
            spawned: true,
            cancelled: false,
            timed_out: false,
            output: self
                .latest
                .as_ref()
                .map(|(_, output)| core_output(output))
                .unwrap_or_default(),
            detail: Some(
                "Command completion could not be verified. Side effects may continue.".into(),
            ),
            history_error: Some("No verified remote completion was received.".into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{ApprovalGate, CommandState, Outcome};
    use crate::location::ProjectRoot;
    use contextual_protocol::commands::{CommandPlatform, PrepareCommand};

    fn proposal() -> (ApprovalGate, CallId, ReviewPlan) {
        let prepared = PreparedCommand {
            request: PrepareCommand {
                root: "/project".into(),
                project_epoch: "project-1".into(),
                command_json: serde_json::json!({
                    "version": 1,
                    "purpose": "Test wire data without executing a process.",
                    "command": {
                        "kind": "exec",
                        "program": "tool",
                        "args": ["a b", "", "$HOME", "&"]
                    },
                    "cwd": ".",
                    "timeoutSeconds": 30,
                    "environment": [{"name": "CI", "value": "true"}]
                })
                .to_string(),
            },
            host_epoch: "host-1".into(),
            platform: CommandPlatform::Linux,
            environment_policy: ENVIRONMENT_POLICY.into(),
            root: "/project".into(),
            cwd: "/project".into(),
            program: "/usr/bin/tool".into(),
            args: vec!["a b".into(), "".into(), "$HOME".into(), "&".into()],
            environment: [
                ("PATH".into(), "/usr/bin".into()),
                ("CI".into(), "true".into()),
            ]
            .into(),
        };
        let plan = ReviewPlan::from_prepared(
            ProjectRoot {
                location: Location::Wsl {
                    distro: "Ubuntu".into(),
                },
                path: "/project".into(),
            },
            prepared,
        )
        .unwrap();
        let mut gate = ApprovalGate::new(plan.binding.clone());
        let call = CallId {
            turn: "turn-1".into(),
            call: "call-1".into(),
        };
        gate.propose(call.clone(), plan.clone()).unwrap();
        (gate, call, plan)
    }

    fn dispatch_bytes() -> (ExecutionBinding, Vec<u8>) {
        let (mut gate, call, plan) = proposal();
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        (plan.binding, encode_dispatch(permit).unwrap())
    }

    fn execution() -> CommandExecutionIdentity {
        CommandExecutionIdentity {
            invocation_id: "0123456789abcdef0123456789abcdef".into(),
            root: "/project".into(),
            project_epoch: "project-1".into(),
            host_epoch: "host-1".into(),
        }
    }

    fn stream(value: &str) -> CommandStreamOutput {
        CommandStreamOutput {
            text: value.into(),
            bytes_seen: value.len() as u64,
            bytes_retained: value.len() as u64,
            truncated: false,
            complete: false,
            read_failed: false,
        }
    }

    fn progress(sequence: u64, value: &str) -> CommandProgress {
        CommandProgress {
            version: EXECUTION_VERSION,
            execution: execution(),
            sequence,
            output: CommandOutput {
                stdout: stream(value),
                stderr: stream(""),
            },
        }
    }

    fn result(sequence: u64, value: &str) -> CommandResult {
        let mut output = progress(sequence, value).output;
        output.stdout.complete = true;
        output.stderr.complete = true;
        CommandResult {
            version: EXECUTION_VERSION,
            execution: execution(),
            final_sequence: sequence,
            status: CommandExecutionStatus::Success,
            exit_code: Some(0),
            spawned: true,
            cancelled: false,
            timed_out: false,
            output,
            detail: None,
            history_error: None,
        }
    }

    fn json(value: &impl Serialize) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }

    #[test]
    fn encoding_consumes_a_permit_and_decoding_preserves_the_review_snapshot() {
        let (mut gate, call, plan) = proposal();
        assert_eq!(gate.state(&call), Some(CommandState::NeedsReview));
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let invocation = permit.invocation().clone();

        let bytes = encode_dispatch(permit).unwrap();
        let decoded = decode_dispatch(&bytes, &plan.binding).unwrap();
        assert_eq!(decoded.invocation(), &invocation);
        assert_eq!(decoded.call(), &call);
        assert!(decoded.plan() == &plan);
        assert!(decoded.check_preparation(&plan).is_ok());
        assert_eq!(gate.state(&call), Some(CommandState::Running));

        let mut changed = plan.clone();
        changed.args.push("--different".into());
        assert!(decoded.check_preparation(&changed).is_err());
    }

    #[test]
    fn mismatched_distro_host_project_and_root_are_refused() {
        let (expected, bytes) = dispatch_bytes();
        let mut local = expected.clone();
        local.project.location = Location::Local;
        let mut distro = expected.clone();
        distro.project.location = Location::Wsl {
            distro: "Debian".into(),
        };
        let mut host = expected.clone();
        host.host_epoch = "replacement-host".into();
        let mut project = expected.clone();
        project.project_epoch = "reopened-project".into();
        let mut root = expected;
        root.project.path = "/another-project".into();
        for binding in [local, distro, host, project, root] {
            assert!(decode_dispatch(&bytes, &binding).is_err());
        }
    }

    #[test]
    fn duplicate_model_fields_and_unpermitted_environment_are_refused() {
        let (binding, bytes) = dispatch_bytes();
        let mut dispatch: DispatchCommand = serde_json::from_slice(&bytes).unwrap();
        dispatch.prepared.request.command_json =
            dispatch
                .prepared
                .request
                .command_json
                .replacen('{', r#"{"version":1,"#, 1);
        assert!(decode_dispatch(&json(&dispatch), &binding).is_err());

        let mut dispatch: DispatchCommand = serde_json::from_slice(&bytes).unwrap();
        dispatch
            .prepared
            .environment
            .insert("AZURE_OPENAI_API_KEY".into(), "private-value".into());
        let error = decode_dispatch(&json(&dispatch), &binding).err().unwrap();
        assert!(!error.message.contains("private-value"));
    }

    #[test]
    fn oversized_dispatch_is_refused_before_json_parsing() {
        let (binding, _) = dispatch_bytes();
        let bytes = vec![b' '; MAX_DISPATCH_BYTES + 1];
        assert!(decode_dispatch(&bytes, &binding).is_err());
    }

    #[test]
    fn cancellation_requires_exact_identity_not_just_invocation_id() {
        let mut request = CancelCommand {
            version: EXECUTION_VERSION,
            execution: execution(),
        };
        assert!(decode_cancel(&json(&request), &execution()).is_ok());
        request.execution.host_epoch = "different-host".into();
        assert!(decode_cancel(&json(&request), &execution()).is_err());
    }

    #[test]
    fn slow_consumers_can_skip_sequences_without_losing_cumulative_output() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        receiver.progress(&json(&progress(1, "a"))).unwrap();
        let output = receiver.progress(&json(&progress(9, "abc"))).unwrap();
        assert_eq!(output.stdout.text, "abc");
        let completed = receiver.finish(&json(&result(10, "abcd"))).unwrap();
        assert_eq!(completed.status, ExecutionStatus::Success);
        assert_eq!(completed.output.stdout.text, "abcd");
        assert!(receiver.disconnect().is_err());
        assert!(receiver.finish(&json(&result(11, "abcd"))).is_err());
    }

    #[test]
    fn a_provisional_utf8_replacement_can_be_completed_in_the_next_snapshot() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        let mut partial = progress(1, "\u{fffd}");
        partial.output.stdout.bytes_seen = 1;
        partial.output.stdout.bytes_retained = 1;
        receiver.progress(&json(&partial)).unwrap();
        let complete = receiver.progress(&json(&progress(2, "雪"))).unwrap();
        assert_eq!(complete.stdout.text, "雪");
    }

    #[test]
    fn stale_sequence_poisoning_cannot_be_erased_by_a_later_success() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        receiver.progress(&json(&progress(4, "kept"))).unwrap();
        assert!(receiver.progress(&json(&progress(4, "kept"))).is_err());
        assert!(receiver.finish(&json(&result(5, "kept"))).is_err());
        let unknown = receiver.disconnect().unwrap();
        assert_eq!(unknown.outcome(), Outcome::Unknown);
        assert_eq!(unknown.output.stdout.text, "kept");
    }

    #[test]
    fn a_sequence_cannot_wrap_after_u64_max() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        receiver
            .progress(&json(&progress(u64::MAX, "kept")))
            .unwrap();
        assert!(receiver.finish(&json(&result(1, "kept"))).is_err());
    }

    #[test]
    fn unrelated_invocation_cannot_replace_the_last_verified_snapshot() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        receiver.progress(&json(&progress(1, "kept"))).unwrap();
        let mut other = progress(2, "replacement");
        other.execution.invocation_id = InvocationId::new().as_str().into();
        assert!(receiver.progress(&json(&other)).is_err());
        assert_eq!(receiver.disconnect().unwrap().output.stdout.text, "kept");
    }

    #[test]
    fn output_must_extend_its_verified_prefix() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        receiver.progress(&json(&progress(1, "abc"))).unwrap();
        assert!(
            receiver
                .progress(&json(&progress(2, "replacement")))
                .is_err()
        );
    }

    #[test]
    fn completed_streams_cannot_resume_producing_output() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        let mut completed = progress(1, "a");
        completed.output.stdout.complete = true;
        receiver.progress(&json(&completed)).unwrap();
        assert!(receiver.progress(&json(&progress(2, "ab"))).is_err());
    }

    #[test]
    fn unsafe_text_inconsistent_counters_and_oversized_text_are_refused() {
        let mut controls = progress(1, "\x1b[31m");
        let mut counters = progress(1, "abc");
        counters.output.stdout.bytes_retained = 2;
        let mut truncated = progress(1, "abc");
        truncated.output.stdout.truncated = true;
        let mut oversized = progress(1, "x");
        oversized.output.stdout.text = "x".repeat(OUTPUT_LIMIT * 6 + 1);
        let mut contradictory = progress(1, "abc");
        contradictory.output.stdout.complete = true;
        contradictory.output.stdout.read_failed = true;

        for message in [
            &mut controls,
            &mut counters,
            &mut truncated,
            &mut oversized,
            &mut contradictory,
        ] {
            let mut receiver = ObservationReceiver::new(execution()).unwrap();
            assert!(receiver.progress(&json(message)).is_err());
        }
    }

    #[test]
    fn contradictory_success_and_output_before_spawn_are_refused() {
        let mut cancelled_success = result(1, "a");
        cancelled_success.cancelled = true;
        let mut no_spawn = result(1, "a");
        no_spawn.status = CommandExecutionStatus::FailedToStart;
        no_spawn.spawned = false;
        no_spawn.exit_code = None;

        for message in [cancelled_success, no_spawn] {
            let mut receiver = ObservationReceiver::new(execution()).unwrap();
            assert!(receiver.finish(&json(&message)).is_err());
            assert_eq!(
                receiver.disconnect().unwrap().status,
                ExecutionStatus::Unknown
            );
        }
    }

    #[test]
    fn history_failure_does_not_fabricate_a_different_process_outcome() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        let mut message = result(1, "");
        message.history_error = Some("Completion was not durably recorded.".into());
        let observed = receiver.finish(&json(&message)).unwrap();
        assert_eq!(observed.status, ExecutionStatus::Success);
        assert!(observed.history_error.is_some());
    }

    #[test]
    fn oversized_observation_permanently_invalidates_the_receiver() {
        let mut receiver = ObservationReceiver::new(execution()).unwrap();
        assert!(
            receiver
                .progress(&vec![b' '; MAX_OBSERVATION_BYTES + 1])
                .is_err()
        );
        assert!(receiver.finish(&json(&result(1, ""))).is_err());
        assert_eq!(
            receiver.disconnect().unwrap().status,
            ExecutionStatus::Unknown
        );
    }
}
