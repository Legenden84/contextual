//! Explicitly configured agent execution service. Ordinary agent startup does not construct it.
//!
//! Location comes from the trusted connection owner, project/host epochs from preparation on
//! this service's LocalHost, and invocation replay protection from an agent-owned file journal.
//! There is no native backend selection and no app-side fallback.
//!
//! The process worker outlives RPC waiters. Disconnect requests Stop and waits for that worker;
//! dropping or aborting an RPC waiter is never evidence of process-tree termination.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use contextual_protocol::commands::{
    CancelCommand, CommandExecutionIdentity, DispatchCommand, PrepareCommand, PreparedCommand,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::execution::{ExecutionEngine, ExecutionHandle, ManagedCommandRunner};
use super::journal::FileExecutionJournal;
use super::wire::{
    MAX_CANCEL_BYTES, MAX_DISPATCH_BYTES, decode_cancel, decode_dispatch, encode_progress,
    encode_result,
};
use super::{ExecutionBinding, InvocationId, ReviewPlan};
use crate::host::{LocalHost, WorkspaceHost};
use crate::location::{Location, ProjectRoot};
use crate::{Error, Result};

const MAX_PROJECTS: usize = 32;
const MAX_ATTEMPTS: usize = 1024;

struct PreparedProject {
    binding: ExecutionBinding,
    canonical_root: String,
}

#[derive(Default)]
struct State {
    projects: HashMap<String, PreparedProject>,
    operations: HashMap<String, Arc<AgentOperation>>,
    // Includes synchronous refusals. Never evicted within this service lifetime. Accepted
    // workers additionally publish immutable intent before cancellation/revalidation/spawn.
    seen: HashSet<InvocationId>,
}

/// One bounded cumulative observation. None on Finished means encoding/worker observation
/// failed; it must become Unknown at the client, not a fabricated terminal process status.
#[derive(Clone)]
pub enum CommandObservation {
    Progress(Arc<str>),
    Finished(Option<Arc<str>>),
}

/// A subscription and cooperative Stop handle, not authorization.
pub struct AgentOperation {
    identity: CommandExecutionIdentity,
    cancel: CancellationToken,
    observations: watch::Receiver<Option<CommandObservation>>,
}

impl AgentOperation {
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub fn observations(&self) -> watch::Receiver<Option<CommandObservation>> {
        self.observations.clone()
    }

    /// Waits for the supervisor, including process-tree cleanup and completion recording.
    pub async fn wait(&self) -> Result<Arc<str>> {
        let mut observations = self.observations();
        loop {
            let current = observations.borrow_and_update().clone();
            if let Some(CommandObservation::Finished(result)) = current {
                return result.ok_or_else(observation_lost);
            }
            observations.changed().await.map_err(|_| observation_lost())?;
        }
    }
}

fn observation_lost() -> Error {
    Error::tool("Command completion could not be verified.")
}

/// One execution service on the agent, shared by its connections.
///
/// The journal must live outside projects and chat pruning. Reusing the service across
/// connections preserves in-memory admission; reopening its journal preserves durable replay
/// and unknown-outcome checks. Cross-process ownership is still a separate prerequisite for
/// selecting any native backend.
pub struct AgentExecutionService {
    host: Arc<LocalHost>,
    location: Location,
    engine: ExecutionEngine,
    state: Mutex<State>,
}

impl AgentExecutionService {
    pub fn new(
        host: Arc<LocalHost>,
        location: Location,
        runner: Arc<dyn ManagedCommandRunner>,
        journal: Arc<FileExecutionJournal>,
    ) -> Self {
        Self {
            engine: ExecutionEngine::new(host.clone(), runner, journal),
            host,
            location,
            state: Mutex::default(),
        }
    }

    pub fn host(&self) -> Arc<LocalHost> {
        self.host.clone()
    }

    /// Establishes the project binding independently of subsequent dispatch envelopes.
    /// No environment or command contents are retained in this binding registry.
    pub async fn prepare(&self, request: PrepareCommand) -> Result<PreparedCommand> {
        let prepared = self.host.prepare_command(request).await?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::internal("Agent execution state is unavailable."))?;
        if !state.projects.contains_key(&prepared.request.root)
            && state.projects.len() >= MAX_PROJECTS
        {
            return Err(Error::conflict("Agent command project limit reached."));
        }
        state.projects.insert(
            prepared.request.root.clone(),
            PreparedProject {
                binding: ExecutionBinding {
                    project: ProjectRoot {
                        location: self.location.clone(),
                        path: prepared.request.root.clone(),
                    },
                    project_epoch: prepared.request.project_epoch.clone(),
                    host_epoch: prepared.host_epoch.clone(),
                },
                canonical_root: prepared.root.clone(),
            },
        );
        Ok(prepared)
    }

    /// Trusted agent dispatch boundary, not a model-argument execution API.
    ///
    /// Admission never queues behind another invocation. A successful return means a worker
    /// owns this attempt; its final observation, not an RPC cancellation, determines cleanup.
    pub fn dispatch(
        &self,
        bytes: &[u8],
        scope: CancellationToken,
    ) -> Result<Arc<AgentOperation>> {
        if bytes.len() > MAX_DISPATCH_BYTES {
            return Err(Error::invalid("Command dispatch exceeds the size limit."));
        }
        // This first raw decode only selects an independently stored binding. The full codec
        // below validates it again without a Value intermediary or lost duplicate fields.
        let envelope: DispatchCommand = serde_json::from_slice(bytes)
            .map_err(|_| Error::invalid("Malformed command dispatch."))?;
        let invocation = InvocationId::parse(&envelope.execution.invocation_id)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::internal("Agent execution state is unavailable."))?;
        let project = state
            .projects
            .get(&envelope.execution.root)
            .ok_or_else(|| Error::conflict("Prepare this project on this agent before dispatch."))?;
        let dispatch = decode_dispatch(bytes, &project.binding)?;
        if dispatch.plan().directory.root().to_str() != Some(project.canonical_root.as_str()) {
            return Err(Error::conflict("Command admission root differs from preparation."));
        }
        let canonical_root = project.canonical_root.clone();
        if state.seen.contains(&invocation) {
            return Err(Error::conflict("Command invocation was already received."));
        }
        if state.seen.len() >= MAX_ATTEMPTS {
            return Err(Error::conflict("Agent command attempt limit reached."));
        }
        state.seen.insert(invocation);
        if !state.operations.contains_key(&canonical_root)
            && state.operations.len() >= MAX_PROJECTS
        {
            return Err(Error::conflict("Agent command operation limit reached."));
        }

        // This reserves the process-wide canonical project slot synchronously. The engine
        // re-prepares the exact snapshot after durable intent, immediately before spawn.
        let cancel = scope.child_token();
        let handle = self.engine.start_agent(dispatch, cancel.clone())?;
        let (sender, observations) = watch::channel(None);
        let operation = Arc::new(AgentOperation {
            identity: envelope.execution,
            cancel,
            observations,
        });
        state
            .operations
            .insert(canonical_root, operation.clone());
        let identity = operation.identity.clone();
        tokio::spawn(relay(handle, identity, sender));
        Ok(operation)
    }

    /// Validates the full Stop binding and returns the original operation. Its eventual result
    /// is exactly the same envelope as dispatch's final result, including final sequence.
    pub fn stop(&self, bytes: &[u8]) -> Result<Arc<AgentOperation>> {
        if bytes.len() > MAX_CANCEL_BYTES {
            return Err(Error::invalid("Command Stop exceeds the size limit."));
        }
        let request: CancelCommand = serde_json::from_slice(bytes)
            .map_err(|_| Error::invalid("Malformed command Stop."))?;
        let state = self
            .state
            .lock()
            .map_err(|_| Error::internal("Agent execution state is unavailable."))?;
        let operation = state
            .operations
            .values()
            .find(|operation| operation.identity == request.execution)
            .ok_or_else(|| Error::conflict("Stop does not match a retained command."))?;
        decode_cancel(bytes, &operation.identity)?;
        operation.stop();
        Ok(operation.clone())
    }

    /// Cancels and awaits retained workers. Call after stopping connection admission.
    /// A failed observation does not clear either the engine slot or durable unknown intent.
    pub async fn stop_and_wait(&self) {
        let operations: Vec<_> = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .operations
            .values()
            .cloned()
            .collect();
        for operation in &operations {
            operation.stop();
        }
        for operation in operations {
            let _ = operation.wait().await;
        }
    }
}

async fn relay(
    handle: ExecutionHandle,
    identity: CommandExecutionIdentity,
    sender: watch::Sender<Option<CommandObservation>>,
) {
    let mut progress = handle.progress();
    let wait = handle.wait();
    tokio::pin!(wait);
    let mut sequence = 0u64;
    let mut progress_open = true;
    loop {
        tokio::select! {
            biased;
            result = &mut wait => {
                let result = sequence.checked_add(1)
                    .and_then(|sequence| encode_result(&identity, sequence, &result).ok())
                    .map(Arc::from);
                sender.send_replace(Some(CommandObservation::Finished(result)));
                return;
            }
            changed = progress.changed(), if progress_open => {
                if changed.is_err() {
                    progress_open = false;
                    continue;
                }
                let Some(next) = sequence.checked_add(1) else {
                    // Sequence exhaustion must not publish a wrapped observation. Dropping
                    // the wait future requests Stop; the engine still owns cleanup.
                    return;
                };
                sequence = next;
                let output = progress.borrow_and_update().clone();
                if let Ok(message) = encode_progress(&identity, sequence, &output) {
                    sender.send_replace(Some(CommandObservation::Progress(Arc::from(message))));
                } else {
                    return;
                }
            }
        }
    }
}
