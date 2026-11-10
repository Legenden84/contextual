//! One-shot approved dispatch over an already reviewed RemoteHost connection.
//!
//! The process-wide client reservation deliberately excludes host/project epochs: replacing
//! an agent, reopening a project or constructing another RemoteHost cannot clear Unknown.
//! This is not cross-process ownership or persisted app execution history.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use contextual_protocol::commands::{
    CancelCommand, CommandExecutionIdentity, CommandProgress, EXECUTION_CAPABILITY,
    EXECUTION_VERSION, PreparedCommand,
};
use serde::Serialize;
use serde_json::value::RawValue;
use tokio::sync::{oneshot, watch};

use super::{RemoteHost, agent_gone, method, raw};
use crate::commands::execution::{
    ExecutionHandle, ExecutionProgress, ExecutionResult, ExecutionStatus,
};
use crate::commands::wire::{MAX_OBSERVATION_BYTES, ObservationReceiver, encode_dispatch};
use crate::commands::{InvocationId, LaunchPermit, ReviewPlan};
use crate::location::{Location, ProjectRoot};
use crate::{Error, Result};
use tokio_util::sync::CancellationToken;

const MAX_PROJECTS: usize = 32;
const MAX_RESERVATIONS: usize = 256;

type ProjectKey = (Location, String);
static RESERVED: LazyLock<Mutex<HashMap<ProjectKey, InvocationId>>> =
    LazyLock::new(Mutex::default);

struct Observation {
    receiver: ObservationReceiver,
    progress: watch::Sender<ExecutionProgress>,
}

#[derive(Default)]
pub(super) struct CommandConnection {
    location: Mutex<Option<Location>>,
    prepared: Mutex<HashMap<String, ReviewPlan>>,
    active: Mutex<HashMap<String, Arc<Mutex<Observation>>>>,
}

impl CommandConnection {
    pub(super) fn configure(&self, location: Location) -> Result<()> {
        let mut current = self
            .location
            .lock()
            .map_err(|_| unavailable())?;
        if current.is_some() {
            return Err(Error::conflict("Command connection location is already bound."));
        }
        *current = Some(location);
        Ok(())
    }

    pub(super) fn remember(&self, prepared: &PreparedCommand) -> Result<()> {
        let location = self.location.lock().map_err(|_| unavailable())?.clone();
        let Some(location) = location else {
            return Ok(());
        };
        let plan = ReviewPlan::from_prepared(
            ProjectRoot {
                location,
                path: prepared.request.root.clone(),
            },
            prepared.clone(),
        )?;
        let mut snapshots = self.prepared.lock().map_err(|_| unavailable())?;
        if snapshots.len() >= MAX_PROJECTS && !snapshots.contains_key(&prepared.request.root) {
            return Err(Error::conflict("Remote command preparation limit reached."));
        }
        snapshots.insert(prepared.request.root.clone(), plan);
        Ok(())
    }

    pub(super) fn progress(&self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > MAX_OBSERVATION_BYTES {
            return Err(Error::invalid("Remote command progress exceeds the size limit."));
        }
        let message: CommandProgress = serde_json::from_slice(bytes)
            .map_err(|_| Error::invalid("Malformed remote command progress."))?;
        let observation = self
            .active
            .lock()
            .map_err(|_| unavailable())?
            .get(&message.execution.invocation_id)
            .cloned()
            .ok_or_else(|| Error::conflict("Unbound remote command progress."))?;
        let mut observation = observation.lock().map_err(|_| unavailable())?;
        let output = observation.receiver.progress(bytes)?;
        observation.progress.send_replace(output);
        Ok(())
    }
}

fn unavailable() -> Error {
    Error::internal("Remote command state is unavailable.")
}

#[derive(Serialize)]
struct Request<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: &'a RawValue,
}

/// Synchronously registers and enqueues exactly one frame. No CancelOnDrop, retry, host
/// replacement, or generic request future can turn this into a second dispatch.
fn begin(
    host: &RemoteHost,
    method: &str,
    params: &RawValue,
) -> Result<(u64, oneshot::Receiver<raw::Response>)> {
    let id = host
        .next_id
        .fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |id| id.checked_add(1),
        )
        .map_err(|_| Error::internal("Workspace request identities exhausted."))?;
    let (sender, receiver) = oneshot::channel();
    {
        let mut pending = host.pending.lock().map_err(|_| unavailable())?;
        if !host.is_alive() {
            return Err(agent_gone());
        }
        pending.insert(id, sender);
    }
    if let Err(error) = host.send(&Request {
        jsonrpc: contextual_protocol::rpc::JSONRPC,
        id,
        method,
        params,
    }) {
        host.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
        return Err(error);
    }
    Ok((id, receiver))
}

pub(super) fn start(
    host: &RemoteHost,
    permit: LaunchPermit,
    scope: CancellationToken,
) -> Result<ExecutionHandle> {
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| Error::internal("Command execution requires a Tokio runtime."))?;
    if !host
        .info
        .capabilities
        .iter()
        .any(|capability| capability == EXECUTION_CAPABILITY)
    {
        return Err(Error::tool("Command execution is unavailable on this workspace host."));
    }
    if !host.is_alive() {
        return Err(agent_gone());
    }
    if scope.is_cancelled() {
        return Err(Error::conflict("Command scope was cancelled before dispatch."));
    }
    let plan = permit.plan();
    let location = host.commands.location.lock().map_err(|_| unavailable())?;
    if location.as_ref() != Some(&plan.binding.project.location) {
        return Err(Error::forbidden("Command belongs to a different execution location."));
    }
    drop(location);
    let snapshots = host.commands.prepared.lock().map_err(|_| unavailable())?;
    if snapshots.get(&plan.binding.project.path) != Some(plan) {
        return Err(Error::conflict(
            "Command was not reviewed on this exact connection and project instance.",
        ));
    }
    let key = (
        plan.binding.project.location.clone(),
        plan.directory
            .root()
            .to_str()
            .ok_or_else(|| Error::invalid("Remote command root is not UTF-8."))?
            .to_owned(),
    );
    let invocation = permit.invocation().clone();
    let identity = CommandExecutionIdentity {
        invocation_id: invocation.as_str().into(),
        root: plan.binding.project.path.clone(),
        project_epoch: plan.binding.project_epoch.clone(),
        host_epoch: plan.binding.host_epoch.clone(),
    };
    let timeout = Duration::from_secs(u64::from(plan.request.timeout_seconds) + 60);
    // Hold snapshot ownership through encoding and enqueue. Another preparation cannot
    // replace this project's reviewed snapshot in the middle of synchronous admission.
    let bytes = encode_dispatch(permit)?;
    let params = RawValue::from_string(
        String::from_utf8(bytes).map_err(|_| Error::invalid("Invalid dispatch encoding."))?,
    )
    .map_err(|_| Error::invalid("Invalid dispatch encoding."))?;
    let (progress_tx, progress) = watch::channel(ExecutionProgress::default());
    let observation = Arc::new(Mutex::new(Observation {
        receiver: ObservationReceiver::new(identity.clone())?,
        progress: progress_tx,
    }));
    {
        let mut reserved = RESERVED.lock().map_err(|_| unavailable())?;
        if reserved.contains_key(&key) {
            return Err(Error::conflict(
                "A command is running or its remote outcome/history remains unresolved.",
            ));
        }
        if reserved.len() >= MAX_RESERVATIONS {
            return Err(Error::conflict("Remote command admission limit reached."));
        }
        reserved.insert(key.clone(), invocation.clone());
    }
    {
        let mut active = match host.commands.active.lock() {
            Ok(active) => active,
            Err(_) => {
                release(&key, &invocation);
                return Err(unavailable());
            }
        };
        active.insert(invocation.as_str().into(), observation.clone());
    }
    let (id, receiver) = match begin(host, method::COMMAND_DISPATCH, &params) {
        Ok(pending) => pending,
        Err(error) => {
            host.commands
                .active
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(invocation.as_str());
            release(&key, &invocation);
            return Err(error);
        }
    };
    drop(snapshots);
    let cancel = scope.child_token();
    let worker_cancel = cancel.clone();
    let worker_host = host.clone();
    let worker_invocation = invocation.clone();
    let task = runtime.spawn(async move {
        let result = observe(
            &worker_host,
            id,
            receiver,
            identity,
            worker_cancel,
            timeout,
            &observation,
        )
        .await;
        worker_host
            .commands
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(worker_invocation.as_str());
        if result.status != ExecutionStatus::Unknown && result.history_error.is_none() {
            release(&key, &worker_invocation);
        }
        // Unknown, history failure, panic and dropped workers all leave RESERVED occupied.
        result
    });
    Ok(ExecutionHandle::from_worker(invocation, task, cancel, progress))
}

fn release(key: &ProjectKey, invocation: &InvocationId) {
    if let Ok(mut reserved) = RESERVED.lock()
        && reserved.get(key) == Some(invocation)
    {
        reserved.remove(key);
    }
}

async fn observe(
    host: &RemoteHost,
    id: u64,
    mut dispatch: oneshot::Receiver<raw::Response>,
    identity: CommandExecutionIdentity,
    cancel: CancellationToken,
    timeout: Duration,
    observation: &Mutex<Observation>,
) -> ExecutionResult {
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut stop: Option<(u64, oneshot::Receiver<raw::Response>)> = None;
    let response = loop {
        tokio::select! {
            biased;
            response = &mut dispatch => break response.ok(),
            response = async {
                match stop.as_mut() {
                    Some((_, receiver)) => receiver.await,
                    None => std::future::pending().await,
                }
            } => break response.ok(),
            _ = cancel.cancelled(), if stop.is_none() => {
                let params = serde_json::to_string(&CancelCommand {
                    version: EXECUTION_VERSION,
                    execution: identity.clone(),
                })
                .ok()
                .and_then(|json| RawValue::from_string(json).ok());
                stop = params
                    .as_deref()
                    .and_then(|params| begin(host, method::COMMAND_CANCEL, params).ok());
                if stop.is_none() {
                    break None;
                }
            }
            _ = &mut deadline => break None,
        }
    };
    {
        let mut pending = host.pending.lock().unwrap_or_else(|error| error.into_inner());
        pending.remove(&id);
        if let Some((id, _)) = stop {
            pending.remove(&id);
        }
    }
    let mut observation = observation.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(response) = response
        && response.error.is_none()
        && let Some(result) = response.result
        && let Ok(result) = observation.receiver.finish(result.get().as_bytes())
    {
        observation.progress.send_replace(result.output.clone());
        return result;
    }
    // RPC errors, failed delivery and malformed completion cannot prove non-execution.
    // Closing also asks the agent to stop; we never report its signal as confirmed cleanup.
    host.close();
    match observation.receiver.disconnect() {
        Ok(result) => result,
        Err(_) => ExecutionResult {
            status: ExecutionStatus::Unknown,
            exit_code: None,
            spawned: true,
            cancelled: false,
            timed_out: false,
            output: observation.progress.borrow().clone(),
            detail: Some("Remote command completion could not be verified.".into()),
            history_error: Some("No verified remote completion was received.".into()),
        },
    }
}
