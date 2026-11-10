//! `contextual-agent`: serves a [`WorkspaceHost`] over newline-delimited JSON-RPC 2.0 (plan §6.6).
//!
//! The Windows app starts one per WSL distro (`wsl.exe -d <distro> -- contextual-agent`) and
//! talks to it over stdin/stdout. Requests run concurrently; `$/cancel` aborts one and answers it
//! with `RequestCancelled`. The agent exits when stdin closes or after `shutdown`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use contextual_core::host::WorkspaceHost;
use contextual_protocol::rpc::{
    CancelParams, InitializeParams, InitializeResult, Message, PROTOCOL_VERSION, Request, Response,
    RpcError, method,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};

pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Capabilities reported by `initialize`; the app checks these before using a method.
pub const CAPABILITIES: &[&str] = &[
    "allowRoot",
    "listDirs",
    "scan",
    "read",
    "apply",
    "undo",
    "git",
    "commandPrepare",
];

/// Reads requests from `reader` until EOF or `shutdown`, writing responses to `writer`.
pub async fn serve<R, W>(reader: R, writer: W, host: Arc<dyn WorkspaceHost>) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Response>();
    let writer_task = tokio::spawn(async move {
        let mut writer = writer;
        while let Some(resp) = out_rx.recv().await {
            let mut line = serde_json::to_vec(&resp).expect("responses serialise");
            line.push(b'\n');
            writer.write_all(&line).await?;
            writer.flush().await?;
        }
        Ok::<_, std::io::Error>(())
    });

    let running: Arc<Mutex<HashMap<u64, AbortHandle>>> = Arc::default();
    let mut tasks = JoinSet::new();
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        while tasks.try_join_next().is_some() {}
        if line.trim().is_empty() {
            continue;
        }
        let msg: Message = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("unparseable message: {e}");
                // No id to answer to; JSON-RPC uses null, which our u64 id can't hold. Log only.
                continue;
            }
        };
        match msg {
            Message::Notification(n) if n.method == method::CANCEL => {
                if let Ok(CancelParams { id }) = serde_json::from_value(n.params)
                    && let Some(handle) = running
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id)
                {
                    handle.abort();
                    let _ = out_tx.send(Response::err(
                        id,
                        RpcError::new(RpcError::REQUEST_CANCELLED, "Cancelled"),
                    ));
                }
            }
            Message::Notification(n) => tracing::debug!("ignored notification {}", n.method),
            Message::Response(_) => tracing::debug!("ignored response"),
            Message::Request(req) if req.method == method::SHUTDOWN => {
                let _ = out_tx.send(Response::ok(req.id, Value::Null));
                break;
            }
            Message::Request(req) => {
                let id = req.id;
                let host = host.clone();
                let out = out_tx.clone();
                let running_for_task = running.clone();
                // Hold the lock while spawning so a fast task can't remove itself before it is inserted.
                let mut map = running.lock().unwrap_or_else(|e| e.into_inner());
                let handle = tasks.spawn(async move {
                    let resp = dispatch(host.as_ref(), req).await;
                    // Only answer if not cancelled meanwhile (cancel already answered).
                    if running_for_task
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id)
                        .is_some()
                    {
                        let _ = out.send(resp);
                    }
                });
                map.insert(id, handle);
            }
        }
    }

    // Let in-flight requests finish, then close the writer.
    while tasks.join_next().await.is_some() {}
    drop(out_tx);
    writer_task.await.map_err(std::io::Error::other)?
}

async fn dispatch(host: &dyn WorkspaceHost, req: Request) -> Response {
    let id = req.id;
    let result = match req.method.as_str() {
        method::INITIALIZE => initialize(host, req.params).await,
        method::ALLOW_ROOT => {
            call(
                req.params,
                |p: contextual_protocol::rpc::AllowRootParams| async move {
                    host.allow_root(&p.root).await
                },
            )
            .await
        }
        method::LIST_DIRS => call(req.params, |p| host.list_dirs(p)).await,
        method::SCAN => call(req.params, |p| host.scan(p)).await,
        method::READ => call(req.params, |p| host.read(p)).await,
        method::APPLY => call(req.params, |p| host.apply(p)).await,
        method::UNDO => call(req.params, |p| host.undo(p)).await,
        method::GIT => call(req.params, |p| host.git(p)).await,
        method::COMMAND_PREPARE => call(req.params, |p| host.prepare_command(p)).await,
        other => Err(RpcError::new(
            RpcError::METHOD_NOT_FOUND,
            format!("Unknown method: {other}"),
        )),
    };
    match result {
        Ok(v) => Response::ok(id, v),
        Err(e) => Response::err(id, e),
    }
}

async fn initialize(host: &dyn WorkspaceHost, params: Value) -> Result<Value, RpcError> {
    let p: InitializeParams = parse(params)?;
    for root in &p.allowed_roots {
        host.allow_root(root).await.map_err(core_error)?;
    }
    to_value(InitializeResult {
        protocol_version: PROTOCOL_VERSION,
        agent_version: AGENT_VERSION.to_string(),
        capabilities: CAPABILITIES.iter().map(|c| c.to_string()).collect(),
    })
}

async fn call<P, T, F, Fut>(params: Value, f: F) -> Result<Value, RpcError>
where
    P: DeserializeOwned,
    T: Serialize,
    F: FnOnce(P) -> Fut,
    Fut: Future<Output = contextual_core::Result<T>>,
{
    let p = parse(params)?;
    to_value(f(p).await.map_err(core_error)?)
}

fn parse<P: DeserializeOwned>(params: Value) -> Result<P, RpcError> {
    serde_json::from_value(params)
        .map_err(|e| RpcError::new(RpcError::INVALID_PARAMS, e.to_string()))
}

fn to_value<T: Serialize>(v: T) -> Result<Value, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::new(RpcError::INTERNAL_ERROR, e.to_string()))
}

/// Core errors travel in `data` so the app can rebuild the exact `contextual_core::Error`.
fn core_error(e: contextual_core::Error) -> RpcError {
    RpcError {
        code: RpcError::APPLICATION_ERROR,
        message: e.message.clone(),
        data: serde_json::to_value(&e).ok(),
    }
}
