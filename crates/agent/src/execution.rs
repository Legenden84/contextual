//! Opt-in executable connection around an agent-owned service.
//!
//! Kept separate from ordinary serve: no binary startup, environment flag or capability
//! advertisement enables execution accidentally. Queue saturation fails final delivery closed;
//! intermediate cumulative snapshots may be skipped. Process workers never wait on this queue.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use contextual_core::commands::service::{AgentExecutionService, CommandObservation};
use contextual_core::rpc_transport::{BoundedLines, MAX_WORKSPACE_FRAME_BYTES};
use contextual_protocol::commands::EXECUTION_CAPABILITY;
use contextual_protocol::rpc::raw::{self, Message, Request};
use contextual_protocol::rpc::{
    CancelParams, InitializeParams, InitializeResult, PROTOCOL_VERSION, Response, RpcError, method,
};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{AGENT_VERSION, CAPABILITIES, core_error, dispatch, parse};

const QUEUE_FRAMES: usize = 8;
const MAX_REQUESTS: usize = 64;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

struct Active {
    task: AbortHandle,
    cooperative: Option<CancellationToken>,
}

type Running = Arc<Mutex<HashMap<u64, Active>>>;

fn enqueue(out: &mpsc::Sender<Vec<u8>>, closed: &CancellationToken, bytes: Vec<u8>) {
    if bytes.len() > MAX_WORKSPACE_FRAME_BYTES + 1 || out.try_send(bytes).is_err() {
        closed.cancel();
    }
}

fn response_bytes(response: &Response) -> std::io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(response).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn error_response(id: u64, error: RpcError) -> Vec<u8> {
    // RpcError contains ordinary serializable values. Failure still closes the connection
    // rather than panicking or inventing a successful command observation.
    response_bytes(&Response::err(id, error)).unwrap_or_default()
}

fn final_response(id: u64, payload: &str) -> Vec<u8> {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{payload}}}\n").into_bytes()
}

fn complete(
    running: &Running,
    id: u64,
    bytes: Vec<u8>,
    out: &mpsc::Sender<Vec<u8>>,
    closed: &CancellationToken,
) {
    let mut running = running.lock().unwrap_or_else(|error| error.into_inner());
    if running
        .get(&id)
        .is_some_and(|active| active.task.id() == tokio::task::id())
    {
        running.remove(&id);
        // Publication and identity retirement are atomic with cancellation/ID reuse.
        enqueue(out, closed, bytes);
    }
}

/// Serves an explicitly configured execution service. The trusted caller supplies its native
/// host, location, managed backend and durable journal; normal agent main still calls serve.
///
/// EOF, framing errors, write/flush failure and shutdown request cooperative Stop. This
/// function awaits the service's process workers before retiring RPC tasks. Dropping this
/// future also cancels their scope; orderly owners must await it before stopping Tokio.
pub async fn serve_with_execution<R, W>(
    reader: R,
    writer: W,
    service: Arc<AgentExecutionService>,
) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let scope = CancellationToken::new();
    let _scope_guard = scope.clone().drop_guard();
    let closed = CancellationToken::new();
    let _closed_guard = closed.clone().drop_guard();
    let writer_closed = closed.clone();
    let (out, mut outgoing) = mpsc::channel::<Vec<u8>>(QUEUE_FRAMES);
    let writer_task = tokio::spawn(async move {
        let _guard = writer_closed.clone().drop_guard();
        let mut writer = writer;
        loop {
            let bytes = tokio::select! {
                biased;
                _ = writer_closed.cancelled() => return Ok(()),
                bytes = outgoing.recv() => match bytes {
                    Some(bytes) => bytes,
                    None => return Ok(()),
                },
            };
            tokio::select! {
                biased;
                _ = writer_closed.cancelled() => return Ok(()),
                result = tokio::time::timeout(WRITE_TIMEOUT, async {
                    writer.write_all(&bytes).await?;
                    writer.flush().await
                }) => {
                    result.map_err(|_| std::io::Error::new(
                        std::io::ErrorKind::TimedOut, "Agent output stalled.",
                    ))??;
                }
            }
        }
    });

    let running: Running = Arc::default();
    let mut tasks = JoinSet::new();
    let mut lines = BoundedLines::new(reader, MAX_WORKSPACE_FRAME_BYTES);
    let mut read_error = None;
    let mut shutdown = None;
    loop {
        while tasks.try_join_next().is_some() {}
        let line = tokio::select! {
            biased;
            _ = closed.cancelled() => break,
            line = lines.next_line() => match line {
                Ok(Some(line)) => line,
                Ok(None) => {
                    closed.cancel();
                    break;
                }
                Err(error) => {
                    read_error = Some(error);
                    closed.cancel();
                    break;
                }
            },
        };
        if line.trim().is_empty() {
            continue;
        }
        let message = match raw::parse(&line) {
            Ok(message) => message,
            Err(_) => {
                read_error = Some(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Malformed workspace RPC envelope.",
                ));
                closed.cancel();
                break;
            }
        };
        match message {
            Message::Notification(notification) if notification.method == method::CANCEL => {
                if let Ok(CancelParams { id }) = parse(notification.params) {
                    let mut running = running.lock().unwrap_or_else(|error| error.into_inner());
                    if let Some(active) = running.get(&id) {
                        if let Some(cancel) = &active.cooperative {
                            // Keep the RPC ID and waiter until a final cleanup observation.
                            cancel.cancel();
                        } else if let Some(active) = running.remove(&id) {
                            active.task.abort();
                            enqueue(
                                &out,
                                &closed,
                                error_response(
                                    id,
                                    RpcError::new(RpcError::REQUEST_CANCELLED, "Cancelled"),
                                ),
                            );
                        }
                    }
                }
            }
            Message::Request(request) => {
                let mut map = running.lock().unwrap_or_else(|error| error.into_inner());
                if map.contains_key(&request.id) {
                    continue;
                }
                if request.method == method::SHUTDOWN {
                    shutdown = Some(request.id);
                    break;
                }
                if map.len() >= MAX_REQUESTS {
                    enqueue(
                        &out,
                        &closed,
                        error_response(
                            request.id,
                            RpcError::new(
                                RpcError::APPLICATION_ERROR,
                                "Agent request limit reached.",
                            ),
                        ),
                    );
                    continue;
                }
                let id = request.id;
                let command = matches!(
                    request.method.as_str(),
                    method::COMMAND_DISPATCH | method::COMMAND_CANCEL
                );
                let cancel = scope.child_token();
                let cooperative = command.then(|| cancel.clone());
                let task_service = service.clone();
                let task_running = running.clone();
                let task_out = out.clone();
                let task_closed = closed.clone();
                let task = tasks.spawn(async move {
                    let bytes =
                        execute_request(task_service, request, cancel, &task_out, &task_closed)
                            .await;
                    complete(&task_running, id, bytes, &task_out, &task_closed);
                });
                map.insert(id, Active { task, cooperative });
            }
            Message::Notification(_) | Message::Response(_) => {}
        }
    }

    scope.cancel();
    // Engine workers live in the service, not this JoinSet. Never abort them on disconnect.
    service.stop_and_wait().await;
    if closed.is_cancelled() {
        tasks.abort_all();
    }
    while tasks.join_next().await.is_some() {}
    if let Some(id) = shutdown {
        enqueue(
            &out,
            &closed,
            response_bytes(&Response::ok(id, Value::Null))?,
        );
    }
    drop(out);
    let written = writer_task.await.map_err(std::io::Error::other)?;
    if let Some(error) = read_error {
        return Err(error);
    }
    written
}

async fn execute_request(
    service: Arc<AgentExecutionService>,
    request: Request,
    cancel: CancellationToken,
    out: &mpsc::Sender<Vec<u8>>,
    closed: &CancellationToken,
) -> Vec<u8> {
    let id = request.id;
    let payload = request
        .params
        .as_deref()
        .map_or("null", |value| value.get());
    match request.method.as_str() {
        method::COMMAND_DISPATCH => {
            let operation = match service.dispatch(payload.as_bytes(), cancel) {
                Ok(operation) => operation,
                Err(error) => return error_response(id, core_error(error)),
            };
            let mut observations = operation.observations();
            loop {
                let current = observations.borrow_and_update().clone();
                match current {
                    Some(CommandObservation::Progress(payload)) => {
                        let bytes = format!(
                            "{{\"jsonrpc\":\"2.0\",\"method\":\"{}\",\"params\":{payload}}}\n",
                            method::COMMAND_PROGRESS,
                        )
                        .into_bytes();
                        // A full queue drops this cumulative snapshot, not the process or
                        // its retained output. The next snapshot/final contains the prefix.
                        let _ = out.try_send(bytes);
                    }
                    Some(CommandObservation::Finished(Some(payload))) => {
                        return final_response(id, &payload);
                    }
                    Some(CommandObservation::Finished(None)) => break,
                    None => {}
                }
                tokio::select! {
                    biased;
                    _ = closed.cancelled() => {
                        operation.stop();
                        return match operation.wait().await {
                            Ok(payload) => final_response(id, &payload),
                            Err(error) => error_response(id, core_error(error)),
                        };
                    }
                    changed = observations.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                }
            }
            error_response(
                id,
                RpcError::new(
                    RpcError::APPLICATION_ERROR,
                    "Command observation worker stopped.",
                ),
            )
        }
        method::COMMAND_CANCEL => match service.stop(payload.as_bytes()) {
            Ok(operation) => match operation.wait().await {
                Ok(payload) => final_response(id, &payload),
                Err(error) => error_response(id, core_error(error)),
            },
            Err(error) => error_response(id, core_error(error)),
        },
        method::COMMAND_PREPARE => {
            let result = match parse(request.params) {
                Ok(request) => service.prepare(request).await.map_err(core_error),
                Err(error) => Err(error),
            };
            match result {
                Ok(prepared) => match serde_json::to_value(prepared) {
                    Ok(value) => response_bytes(&Response::ok(id, value)).unwrap_or_default(),
                    Err(_) => error_response(
                        id,
                        RpcError::new(RpcError::INTERNAL_ERROR, "Could not encode preparation."),
                    ),
                },
                Err(error) => error_response(id, error),
            }
        }
        method::INITIALIZE => {
            let params: InitializeParams = match parse(request.params) {
                Ok(params) => params,
                Err(error) => return error_response(id, error),
            };
            if params.protocol_version != PROTOCOL_VERSION {
                return error_response(
                    id,
                    RpcError::new(RpcError::INVALID_PARAMS, "Unsupported workspace protocol."),
                );
            }
            use contextual_core::host::WorkspaceHost as _;
            for root in params.allowed_roots {
                if let Err(error) = service.host().allow_root(&root).await {
                    return error_response(id, core_error(error));
                }
            }
            let mut capabilities: Vec<String> =
                CAPABILITIES.iter().map(|value| (*value).into()).collect();
            capabilities.push(EXECUTION_CAPABILITY.into());
            match serde_json::to_value(InitializeResult {
                protocol_version: PROTOCOL_VERSION,
                agent_version: AGENT_VERSION.into(),
                capabilities,
            }) {
                Ok(value) => response_bytes(&Response::ok(id, value)).unwrap_or_default(),
                Err(_) => error_response(
                    id,
                    RpcError::new(RpcError::INTERNAL_ERROR, "Could not encode initialization."),
                ),
            }
        }
        _ => response_bytes(&dispatch(service.host().as_ref(), request).await).unwrap_or_default(),
    }
}
