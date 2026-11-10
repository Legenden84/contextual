//! Command preparation and execution wire contracts for workspace hosts.
//!
//! A prepared command is a review snapshot, not authorization. Execution messages are
//! ephemeral host-transport data, not saved-chat records or model-facing tool results.
//! Declaring these types does not enable an endpoint or advertise execution support.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Request from the app to the project's host. The model supplies only `command_json`;
/// the app supplies the current project root and project-instance identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareCommand {
    pub root: String,
    pub project_epoch: String,
    /// Raw dedicated tool arguments. Keeping these as JSON text lets core reject duplicate
    /// fields rather than losing them through an intermediate serde_json::Value.
    pub command_json: String,
}

/// The execution host's operating system, not necessarily the app's operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandPlatform {
    Windows,
    Macos,
    Linux,
}

/// Exact host-side preparation. This contains the complete restricted child environment,
/// so callers must not log it or include it in saved chat history.
///
/// It does not carry reusable authorization. Execution must independently require approval
/// and revalidate the preparation against the same host instance.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedCommand {
    pub request: PrepareCommand,
    pub host_epoch: String,
    pub platform: CommandPlatform,
    pub environment_policy: String,
    /// Canonical, existing project and working directories on the execution host.
    pub root: String,
    pub cwd: String,
    /// Canonical executable path and exact argument boundaries.
    pub program: String,
    pub args: Vec<String>,
    /// Complete environment after filtering and permitted overrides, not an inherited delta.
    pub environment: BTreeMap<String, String>,
}

impl std::fmt::Debug for PreparedCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Scripts, arguments and even allowlisted environment values may contain private data.
        f.debug_struct("PreparedCommand")
            .field("host_epoch", &self.host_epoch)
            .field("platform", &self.platform)
            .field("environment_policy", &self.environment_policy)
            .finish_non_exhaustive()
    }
}

/// Execution envelopes are versioned independently of model arguments and environment policy.
pub const EXECUTION_VERSION: u32 = 1;

/// Advertise only when dispatch, progress, cooperative cancellation and durable ownership
/// are implemented by the serving host. Preparation support alone is insufficient.
pub const EXECUTION_CAPABILITY: &str = "commandExecutionV1";

/// Correlation and exact host/project binding, never reusable authorization.
///
/// Paths retain the execution host's syntax. A receiver must validate identity lengths,
/// invocation-ID syntax and the binding before accepting dispatch or cancellation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandExecutionIdentity {
    pub invocation_id: String,
    pub root: String,
    pub project_epoch: String,
    pub host_epoch: String,
}

/// App-selected execution location. This is not supplied by the model.
/// The receiving connection must independently establish that it serves this location.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum CommandLocation {
    Local,
    Wsl { distro: String },
}

/// One approved attempt, sent only by the core-owned, permit-consuming dispatch path.
///
/// The envelope itself is not evidence of a UI approval. The trusted app/agent connection
/// supplies that boundary; the receiver must additionally enforce durable invocation-ID
/// replay protection, project admission and exact re-preparation before any launch.
/// Debug is intentionally omitted because the snapshot contains private environment values.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DispatchCommand {
    pub version: u32,
    pub execution: CommandExecutionIdentity,
    pub turn_id: String,
    pub call_id: String,
    pub location: CommandLocation,
    pub prepared: PreparedCommand,
}

/// Cooperative Stop for an exact invocation, not cancellation of an RPC task.
///
/// A successful RPC response must contain the final CommandResult after cleanup, not an
/// acknowledgement that a cancellation signal was merely sent. Stop does not undo effects.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelCommand {
    pub version: u32,
    pub execution: CommandExecutionIdentity,
}

/// Escaped plain text from one stream, never terminal instructions or Markdown.
///
/// The producer must retain only the supervisor's bounded prefix and continue draining.
/// Receivers must enforce encoded-size and counter bounds before accepting a snapshot.
/// This transport type must not be used to automatically persist or share unshared output.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandStreamOutput {
    pub text: String,
    pub bytes_seen: u64,
    pub bytes_retained: u64,
    pub truncated: bool,
    pub complete: bool,
    pub read_failed: bool,
}

/// Independently bounded, cumulative stdout and stderr.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandOutput {
    pub stdout: CommandStreamOutput,
    pub stderr: CommandStreamOutput,
}

/// A cumulative snapshot. Sequence numbers increase per invocation without wrapping.
/// Receivers may skip intermediate snapshots, but must reject mismatched or stale identities.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandProgress {
    pub version: u32,
    pub execution: CommandExecutionIdentity,
    pub sequence: u64,
    pub output: CommandOutput,
}

/// An observation, never a statement that side effects were reversed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandExecutionStatus {
    Success,
    Failure,
    FailedToStart,
    Cancelled,
    TimedOut,
    Unknown,
}

/// Final ephemeral host result, separate from persistable execution history.
///
/// Lost or unverifiable completion must become Unknown at the caller, never an automatic
/// retry. Unknown outcomes retain admission. A history error must also prevent admission
/// even when the observed process status is known.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandResult {
    pub version: u32,
    pub execution: CommandExecutionIdentity,
    pub final_sequence: u64,
    pub status: CommandExecutionStatus,
    pub exit_code: Option<i32>,
    /// True also when a failed transport cannot rule out launch.
    pub spawned: bool,
    pub cancelled: bool,
    pub timed_out: bool,
    pub output: CommandOutput,
    pub detail: Option<String>,
    pub history_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatch() -> DispatchCommand {
        DispatchCommand {
            version: EXECUTION_VERSION,
            execution: CommandExecutionIdentity {
                invocation_id: "0123456789abcdef0123456789abcdef".into(),
                root: "/project".into(),
                project_epoch: "project-1".into(),
                host_epoch: "host-1".into(),
            },
            turn_id: "turn-1".into(),
            call_id: "call-1".into(),
            location: CommandLocation::Wsl {
                distro: "Ubuntu".into(),
            },
            prepared: PreparedCommand {
                request: PrepareCommand {
                    root: "/project".into(),
                    project_epoch: "project-1".into(),
                    command_json: r#"{"version":1,"version":2}"#.into(),
                },
                host_epoch: "host-1".into(),
                platform: CommandPlatform::Linux,
                environment_policy: "minimal-v1".into(),
                root: "/project".into(),
                cwd: "/project".into(),
                program: "/usr/bin/tool".into(),
                args: vec!["a b".into(), "".into(), "$HOME".into(), "&".into()],
                environment: BTreeMap::from([("PATH".into(), "/usr/bin".into())]),
            },
        }
    }

    #[test]
    fn dispatch_roundtrip_preserves_binding_arguments_and_raw_model_json() {
        let original = dispatch();
        let encoded = serde_json::to_value(&original).unwrap();
        assert_eq!(encoded["version"], EXECUTION_VERSION);
        assert_eq!(encoded["location"]["kind"], "wsl");
        assert_eq!(encoded["location"]["distro"], "Ubuntu");
        assert_eq!(encoded["execution"]["hostEpoch"], "host-1");
        assert_eq!(encoded["prepared"]["args"][0], "a b");
        assert_eq!(encoded["prepared"]["args"][1], "");
        let restored: DispatchCommand = serde_json::from_value(encoded).unwrap();
        assert!(restored == original);
        // The wire layer preserves duplicate model fields so core validation can reject them.
        assert_eq!(
            restored.prepared.request.command_json,
            r#"{"version":1,"version":2}"#
        );
    }

    #[test]
    fn dispatch_requires_a_version_and_does_not_accept_an_approval_flag() {
        let mut encoded = serde_json::to_value(dispatch()).unwrap();
        encoded["approved"] = serde_json::json!(true);
        assert!(serde_json::from_value::<DispatchCommand>(encoded.clone()).is_err());
        encoded.as_object_mut().unwrap().remove("approved");
        encoded.as_object_mut().unwrap().remove("version");
        assert!(serde_json::from_value::<DispatchCommand>(encoded).is_err());
    }

    #[test]
    fn cancellation_requires_the_complete_execution_binding() {
        let request = CancelCommand {
            version: EXECUTION_VERSION,
            execution: dispatch().execution,
        };
        let mut encoded = serde_json::to_value(&request).unwrap();
        let restored: CancelCommand = serde_json::from_value(encoded.clone()).unwrap();
        assert!(restored == request);
        encoded["execution"]
            .as_object_mut()
            .unwrap()
            .remove("hostEpoch");
        assert!(serde_json::from_value::<CancelCommand>(encoded).is_err());
    }

    #[test]
    fn unknown_result_preserves_cancellation_and_output_observations() {
        let stream = CommandStreamOutput {
            text: "untrusted \\u{1b}[31m".into(),
            bytes_seen: 100_000,
            bytes_retained: 65_536,
            truncated: true,
            complete: false,
            read_failed: true,
        };
        let result = CommandResult {
            version: EXECUTION_VERSION,
            execution: dispatch().execution,
            final_sequence: 12,
            status: CommandExecutionStatus::Unknown,
            exit_code: None,
            spawned: true,
            cancelled: true,
            timed_out: false,
            output: CommandOutput {
                stdout: stream.clone(),
                stderr: stream,
            },
            detail: Some("Termination was not established.".into()),
            history_error: None,
        };
        let encoded = serde_json::to_value(&result).unwrap();
        assert_eq!(encoded["status"], "unknown");
        assert_eq!(encoded["finalSequence"], 12);
        assert_eq!(encoded["cancelled"], true);
        let restored: CommandResult = serde_json::from_value(encoded).unwrap();
        assert!(restored == result);
    }
}
