//! Non-executing command preparation shared by local and remote workspace hosts.
//!
//! A prepared command is a review snapshot, not authorization to execute it.

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
