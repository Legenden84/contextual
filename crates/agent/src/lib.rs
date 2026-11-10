//! `contextual-agent`: serves a [`WorkspaceHost`] over newline-delimited JSON-RPC 2.0 (plan §6.6).
//!
//! The Windows app starts one per WSL distro (`wsl.exe -d <distro> -- contextual-agent`) and
//! talks to it over stdin/stdout. Requests run concurrently; `$/cancel` aborts one and answers it
//! with `RequestCancelled`. The agent exits when stdin closes or after `shutdown`.
//! A duplicate active request ID is discarded without dispatch or a second response: the
//! original operation retains its identity and cancellation handle. Ordinary startup does not
//! expose execution. The explicitly configured `serve_with_execution` connection uses separate
//! cooperative command ownership; it never treats task abortion as process-tree cleanup.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use contextual_core::host::WorkspaceHost;
use contextual_core::rpc_transport::{BoundedLines, MAX_WORKSPACE_FRAME_BYTES};
use contextual_protocol::rpc::raw::{self, Message, Request};
use contextual_protocol::rpc::{
    CancelParams, InitializeParams, InitializeResult, PROTOCOL_VERSION, Response, RpcError, method,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::value::RawValue;
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};

mod execution;

pub use execution::serve_with_execution;

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

/// Publishes completion only while this task still owns the request ID.
fn complete_request(
    running: &Mutex<HashMap<u64, AbortHandle>>,
    task_id: tokio::task::Id,
    response: Response,
    out: &mpsc::UnboundedSender<Response>,
) {
    let mut running = running.lock().unwrap_or_else(|error| error.into_inner());
    if running
        .get(&response.id)
        .is_some_and(|handle| handle.id() == task_id)
    {
        // Abort does not interrupt a task's current poll. A cancelled task may arrive here
        // after the numeric RPC ID has been reused; it must not retire the new owner.
        // Keep publication under the lock so cancellation cannot also answer this request.
        running.remove(&response.id);
        let _ = out.send(response);
    }
}

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
    let mut lines = BoundedLines::new(reader, MAX_WORKSPACE_FRAME_BYTES);
    while let Some(line) = match lines.next_line().await {
        Ok(line) => line,
        Err(error) => {
            // Do not leave a detached writer waiting on a broken or malformed connection.
            // JoinSet retains the existing task-abort behavior for a reader error.
            writer_task.abort();
            let _ = writer_task.await;
            return Err(error);
        }
    } {
        while tasks.try_join_next().is_some() {}
        if line.trim().is_empty() {
            continue;
        }
        let msg = match raw::parse(&line) {
            Ok(message) => message,
            Err(_) => {
                // An invalid or ambiguous envelope has no trustworthy identity to answer.
                // Never log wire contents or payload-bearing deserializer diagnostics.
                tracing::warn!("discarded malformed workspace RPC envelope");
                continue;
            }
        };
        match msg {
            Message::Notification(n) if n.method == method::CANCEL => {
                if let Ok(CancelParams { id }) = parse(n.params)
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
            Message::Request(req)
                if running
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains_key(&req.id) =>
            {
                // A second response with the same ID could be mistaken for the original
                // operation's result. Reject the duplicate without disturbing that operation.
                // This also applies to shutdown: it must not bypass active-ID ownership.
                tracing::warn!(id = req.id, "discarded duplicate active RPC request");
            }
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
                    complete_request(&running_for_task, tokio::task::id(), resp, &out);
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

async fn initialize(
    host: &dyn WorkspaceHost,
    params: Option<Box<RawValue>>,
) -> Result<Value, RpcError> {
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

async fn call<P, T, F, Fut>(params: Option<Box<RawValue>>, f: F) -> Result<Value, RpcError>
where
    P: DeserializeOwned,
    T: Serialize,
    F: FnOnce(P) -> Fut,
    Fut: Future<Output = contextual_core::Result<T>>,
{
    let p = parse(params)?;
    to_value(f(p).await.map_err(core_error)?)
}

fn parse<P: DeserializeOwned>(params: Option<Box<RawValue>>) -> Result<P, RpcError> {
    let json = params.as_deref().map_or("null", RawValue::get);
    serde_json::from_str(json)
        .map_err(|_| RpcError::new(RpcError::INVALID_PARAMS, "Malformed request parameters."))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use contextual_core::{Error, Result};
    use contextual_protocol::commands::{PrepareCommand, PreparedCommand};
    use contextual_protocol::fs::{
        Apply, ApplyResult, DirListing, ListDirs, ReadFiles, ReadFilesResult, Scan, ScanResult,
        Undo, UndoResult,
    };
    use contextual_protocol::git::{GitCall, GitReply};
    use contextual_protocol::rpc::Request;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::sync::Notify;

    #[tokio::test]
    async fn late_cancelled_completion_cannot_retire_a_reused_rpc_id() {
        let running = Mutex::new(HashMap::new());
        let (out, mut responses) = mpsc::unbounded_channel();
        let mut tasks = JoinSet::new();

        let original = tasks.spawn(std::future::pending::<()>());
        let original_id = original.id();
        running.lock().unwrap().insert(7, original);

        // Model cancellation followed by immediate reuse of the numeric RPC ID.
        // The old task may still be finishing its current poll on another runtime thread.
        running.lock().unwrap().remove(&7).unwrap().abort();
        let replacement = tasks.spawn(std::future::pending::<()>());
        let replacement_id = replacement.id();
        running.lock().unwrap().insert(7, replacement);

        complete_request(
            &running,
            original_id,
            Response::ok(7, Value::String("stale completion".into())),
            &out,
        );

        assert!(matches!(
            responses.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert_eq!(
            running.lock().unwrap().get(&7).unwrap().id(),
            replacement_id
        );

        let expected = Response::ok(7, Value::String("replacement completion".into()));
        complete_request(&running, replacement_id, expected.clone(), &out);
        assert_eq!(responses.try_recv().unwrap(), expected);
        assert!(running.lock().unwrap().is_empty());

        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }

    #[tokio::test]
    async fn cancelled_request_without_replacement_cannot_publish_completion() {
        let running = Mutex::new(HashMap::new());
        let (out, mut responses) = mpsc::unbounded_channel();
        let mut tasks = JoinSet::new();
        let task = tasks.spawn(std::future::pending::<()>());
        let task_id = task.id();
        running.lock().unwrap().insert(7, task);
        running.lock().unwrap().remove(&7).unwrap().abort();

        complete_request(&running, task_id, Response::ok(7, Value::Null), &out);

        assert!(matches!(
            responses.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        while tasks.join_next().await.is_some() {}
    }

    #[tokio::test]
    async fn matching_task_can_publish_completion_only_once() {
        let running = Mutex::new(HashMap::new());
        let (out, mut responses) = mpsc::unbounded_channel();
        let mut tasks = JoinSet::new();
        let task = tasks.spawn(std::future::pending::<()>());
        let task_id = task.id();
        running.lock().unwrap().insert(7, task);
        let expected = Response::ok(7, Value::Null);

        complete_request(&running, task_id, expected.clone(), &out);
        complete_request(&running, task_id, expected.clone(), &out);

        assert_eq!(responses.try_recv().unwrap(), expected);
        assert!(matches!(
            responses.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(running.lock().unwrap().is_empty());

        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }

    #[derive(Default)]
    struct HeldPreparation {
        calls: AtomicUsize,
        entered: Notify,
        release: Notify,
    }

    #[async_trait]
    impl WorkspaceHost for HeldPreparation {
        async fn allow_root(&self, _: &str) -> Result<()> {
            Ok(())
        }

        async fn list_dirs(&self, _: ListDirs) -> Result<DirListing> {
            unreachable!("not used by this transport test")
        }

        async fn scan(&self, _: Scan) -> Result<ScanResult> {
            unreachable!("not used by this transport test")
        }

        async fn read(&self, _: ReadFiles) -> Result<ReadFilesResult> {
            unreachable!("not used by this transport test")
        }

        async fn apply(&self, _: Apply) -> Result<ApplyResult> {
            unreachable!("not used by this transport test")
        }

        async fn undo(&self, _: Undo) -> Result<UndoResult> {
            unreachable!("not used by this transport test")
        }

        async fn git(&self, _: GitCall) -> Result<GitReply> {
            unreachable!("not used by this transport test")
        }

        async fn prepare_command(&self, _: PrepareCommand) -> Result<PreparedCommand> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            self.release.notified().await;
            Err(Error::tool("Original preparation completed."))
        }
    }

    async fn send(writer: &mut (impl AsyncWrite + Unpin), request: &Request) {
        let mut bytes = serde_json::to_vec(request).unwrap();
        bytes.push(b'\n');
        writer.write_all(&bytes).await.unwrap();
    }

    #[tokio::test]
    async fn duplicate_preparation_fields_are_rejected_before_host_dispatch() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let host = Arc::new(HeldPreparation::default());
            let (client, server) = tokio::io::duplex(8192);
            let (server_read, server_write) = tokio::io::split(server);
            let server_host = host.clone();
            let server_task = tokio::spawn(async move {
                serve(BufReader::new(server_read), server_write, server_host).await
            });
            let (client_read, mut client_write) = tokio::io::split(client);
            let mut responses = BufReader::new(client_read).lines();

            client_write
                .write_all(
                    b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"workspace/commandPrepare\",\
                      \"params\":{\"root\":\"/first\",\"root\":\"/second\",\
                      \"projectEpoch\":\"test-epoch\",\"commandJson\":\"{}\"}}\n",
                )
                .await
                .unwrap();

            let response: Response =
                serde_json::from_str(&responses.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(response.id, 7);
            assert_eq!(response.error.unwrap().code, RpcError::INVALID_PARAMS);
            assert_eq!(host.calls.load(Ordering::SeqCst), 0);

            send(
                &mut client_write,
                &Request::new(9, method::SHUTDOWN, Value::Null),
            )
            .await;
            let shutdown: Response =
                serde_json::from_str(&responses.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(shutdown.id, 9);
            server_task.await.unwrap().unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn unterminated_request_is_never_dispatched() {
        let host = Arc::new(HeldPreparation::default());
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"workspace/commandPrepare\",\
                      \"params\":{\"root\":\"/project\",\"projectEpoch\":\"epoch\",\
                      \"commandJson\":\"{}\"}}";
        let error = serve(&input[..], tokio::io::sink(), host.clone())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn duplicate_active_ids_preserve_the_original_operation_and_response() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let host = Arc::new(HeldPreparation::default());
            let (client, server) = tokio::io::duplex(8192);
            let (server_read, server_write) = tokio::io::split(server);
            let server_host = host.clone();
            let server_task = tokio::spawn(async move {
                serve(BufReader::new(server_read), server_write, server_host).await
            });
            let (client_read, mut client_write) = tokio::io::split(client);
            let mut responses = BufReader::new(client_read).lines();

            let proposal = Request::new(
                7,
                method::COMMAND_PREPARE,
                serde_json::to_value(PrepareCommand {
                    root: "/test-project".into(),
                    project_epoch: "test-epoch".into(),
                    command_json: "{}".into(),
                })
                .unwrap(),
            );
            send(&mut client_write, &proposal).await;
            host.entered.notified().await;

            send(&mut client_write, &proposal).await;
            send(
                &mut client_write,
                &Request::new(7, method::SHUTDOWN, Value::Null),
            )
            .await;
            // This response proves both duplicate frames were processed. Neither duplicate
            // may dispatch, answer for ID 7, replace its handle, or shut the server down.
            send(
                &mut client_write,
                &Request::new(
                    8,
                    method::INITIALIZE,
                    serde_json::to_value(InitializeParams {
                        protocol_version: PROTOCOL_VERSION,
                        app_version: "test".into(),
                        allowed_roots: Vec::new(),
                    })
                    .unwrap(),
                ),
            )
            .await;
            let barrier: Response =
                serde_json::from_str(&responses.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(barrier.id, 8);
            assert!(barrier.error.is_none());
            assert_eq!(host.calls.load(Ordering::SeqCst), 1);

            host.release.notify_one();
            let original: Response =
                serde_json::from_str(&responses.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(original.id, 7);
            assert_eq!(
                original.error.unwrap().message,
                "Original preparation completed."
            );

            send(
                &mut client_write,
                &Request::new(9, method::SHUTDOWN, Value::Null),
            )
            .await;
            let shutdown: Response =
                serde_json::from_str(&responses.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(shutdown.id, 9);
            assert!(shutdown.error.is_none());
            assert!(responses.next_line().await.unwrap().is_none());
            server_task.await.unwrap().unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn duplicate_active_id_does_not_replace_the_original_cancellation_handle() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let host = Arc::new(HeldPreparation::default());
            let (client, server) = tokio::io::duplex(8192);
            let (server_read, server_write) = tokio::io::split(server);
            let server_host = host.clone();
            let server_task = tokio::spawn(async move {
                serve(BufReader::new(server_read), server_write, server_host).await
            });
            let (client_read, mut client_write) = tokio::io::split(client);
            let mut responses = BufReader::new(client_read).lines();
            let proposal = Request::new(
                7,
                method::COMMAND_PREPARE,
                serde_json::to_value(PrepareCommand {
                    root: "/test-project".into(),
                    project_epoch: "test-epoch".into(),
                    command_json: "{}".into(),
                })
                .unwrap(),
            );
            send(&mut client_write, &proposal).await;
            host.entered.notified().await;
            send(&mut client_write, &proposal).await;

            let notification = contextual_protocol::rpc::Notification::new(
                method::CANCEL,
                serde_json::to_value(CancelParams { id: 7 }).unwrap(),
            );
            let mut bytes = serde_json::to_vec(&notification).unwrap();
            bytes.push(b'\n');
            client_write.write_all(&bytes).await.unwrap();
            let cancelled: Response =
                serde_json::from_str(&responses.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(cancelled.id, 7);
            assert_eq!(cancelled.error.unwrap().code, RpcError::REQUEST_CANCELLED);
            assert_eq!(host.calls.load(Ordering::SeqCst), 1);

            // Never release the held preparation. Shutdown can finish only if cancellation
            // retained and aborted the original task, rather than a duplicate's task.
            send(
                &mut client_write,
                &Request::new(9, method::SHUTDOWN, Value::Null),
            )
            .await;
            let shutdown: Response =
                serde_json::from_str(&responses.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(shutdown.id, 9);
            assert!(responses.next_line().await.unwrap().is_none());
            server_task.await.unwrap().unwrap();
        })
        .await
        .unwrap();
    }
}
