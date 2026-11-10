//! [`RemoteHost`]: a [`WorkspaceHost`] that forwards every call to `contextual-agent` over
//! JSON-RPC (plan §6.6). On Windows the agent runs inside a WSL distro; in tests it runs locally.
//!
//! Dropping a call's future sends `$/cancel` for it. If the agent dies, pending and later calls
//! fail with an `Io` error; the owner restarts it (with backoff) by building a new `RemoteHost`.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use contextual_protocol::commands::{PrepareCommand, PreparedCommand};
use contextual_protocol::fs::{
    Apply, ApplyResult, DirListing, ListDirs, ReadFiles, ReadFilesResult, Scan, ScanResult, Undo,
    UndoResult,
};
use contextual_protocol::git::{GitCall, GitReply};
use contextual_protocol::rpc::{
    AllowRootParams, CancelParams, InitializeParams, InitializeResult, Message, Notification,
    PROTOCOL_VERSION, Request, Response, method,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::error::ErrorCode;
use crate::host::WorkspaceHost;
use crate::{Error, Result};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

/// Serialize disconnection with request registration so a caller cannot be inserted after
/// pending requests have been cleared and then wait forever for a dead connection.
fn disconnect(pending: &Pending, alive: &AtomicBool) {
    let mut pending = pending.lock().unwrap_or_else(|error| error.into_inner());
    alive.store(false, Ordering::Relaxed);
    pending.clear();
}

pub struct RemoteHost {
    next_id: AtomicU64,
    pending: Pending,
    out: mpsc::UnboundedSender<Vec<u8>>,
    /// Kept so the process is killed when the host is dropped.
    _child: Option<Child>,
    info: InitializeResult,
    /// Cleared when either direction of the agent connection fails or closes.
    alive: Arc<AtomicBool>,
}

impl std::fmt::Debug for RemoteHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteHost")
            .field("agent", &self.info.agent_version)
            .finish()
    }
}

impl RemoteHost {
    /// Starts the agent with `command` (stdin/stdout piped, stderr forwarded to our log) and
    /// performs the `initialize` handshake.
    pub async fn spawn(
        mut command: Command,
        app_version: &str,
        allowed_roots: Vec<String>,
    ) -> Result<Self> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = command
            .spawn()
            .map_err(|e| Error::tool(format!("Couldn't start contextual-agent: {e}")))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::info!(target: "contextual_agent", "{line}");
                }
            });
        }
        let mut host = Self::connect(stdout, stdin, app_version, allowed_roots).await?;
        host._child = Some(child);
        Ok(host)
    }

    /// Connects over an existing stream pair (used by [`spawn`](Self::spawn) and by tests).
    pub async fn connect<R, W>(
        reader: R,
        writer: W,
        app_version: &str,
        allowed_roots: Vec<String>,
    ) -> Result<Self>
    where
        R: AsyncBufRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let pending: Pending = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let closed = CancellationToken::new();
        let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let writer_pending = pending.clone();
        let writer_alive = alive.clone();
        let writer_closed = closed.clone();
        tokio::spawn(async move {
            let mut writer = writer;
            loop {
                let line = tokio::select! {
                    biased;
                    _ = writer_closed.cancelled() => break,
                    line = out_rx.recv() => match line {
                        Some(line) => line,
                        None => break,
                    },
                };
                let written = tokio::select! {
                    biased;
                    _ = writer_closed.cancelled() => break,
                    result = async {
                        writer.write_all(&line).await?;
                        writer.flush().await
                    } => result,
                };
                if written.is_err() {
                    break;
                }
            }
            // A broken stdin can leave stdout open indefinitely. Do not wait for reader EOF
            // to fail callers, and do not send queued messages after the connection is lost.
            disconnect(&writer_pending, &writer_alive);
            writer_closed.cancel();
        });
        let reader_pending = pending.clone();
        let reader_alive = alive.clone();
        tokio::spawn(async move {
            let mut lines = reader.lines();
            loop {
                let line = tokio::select! {
                    biased;
                    _ = closed.cancelled() => break,
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => line,
                        Ok(None) | Err(_) => break,
                    },
                };
                match serde_json::from_str::<Message>(&line) {
                    Ok(Message::Response(resp)) => {
                        let tx = reader_pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&resp.id);
                        if let Some(tx) = tx {
                            let _ = tx.send(resp);
                        }
                    }
                    Ok(Message::Notification(n)) => {
                        // Future command progress may contain unshared output or secrets.
                        tracing::debug!(target: "contextual_agent", "notification {}", n.method)
                    }
                    Ok(Message::Request(r)) => {
                        tracing::warn!("agent sent a request ({}); ignored", r.method)
                    }
                    Err(e) => tracing::warn!("unparseable agent output: {e}"),
                }
            }
            disconnect(&reader_pending, &reader_alive);
            closed.cancel();
        });

        let mut host = Self {
            next_id: AtomicU64::new(1),
            pending,
            out,
            _child: None,
            alive,
            info: InitializeResult {
                protocol_version: 0,
                agent_version: String::new(),
                capabilities: vec![],
            },
        };
        let info: InitializeResult = host
            .request(
                method::INITIALIZE,
                InitializeParams {
                    protocol_version: PROTOCOL_VERSION,
                    app_version: app_version.into(),
                    allowed_roots,
                },
            )
            .await?;
        if info.protocol_version != PROTOCOL_VERSION {
            return Err(Error::new(
                ErrorCode::Tool,
                format!(
                    "contextual-agent {} speaks protocol {}, expected {PROTOCOL_VERSION}; redeploy it.",
                    info.agent_version, info.protocol_version
                ),
            ));
        }
        host.info = info;
        Ok(host)
    }

    /// Whether the agent is still running.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn info(&self) -> &InitializeResult {
        &self.info
    }

    /// Sends one request and waits for its response. Dropping the future cancels the request.
    pub async fn request<P: Serialize, T: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<T> {
        let mut candidate = self.next_id.load(Ordering::Relaxed);
        let id = loop {
            let next = candidate
                .checked_add(1)
                .ok_or_else(|| Error::internal("Workspace request identities exhausted."))?;
            match self.next_id.compare_exchange_weak(
                candidate,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break candidate,
                Err(current) => candidate = current,
            }
        };
        let params = serde_json::to_value(params)?;
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            if !self.is_alive() {
                return Err(agent_gone());
            }
            pending.insert(id, tx);
        }
        let mut guard = CancelOnDrop {
            id,
            host: Some(self),
        };
        self.send(&Request::new(id, method, params))?;
        let resp = rx.await.map_err(|_| agent_gone())?;
        guard.host = None;
        if let Some(err) = resp.error {
            return Err(
                match err
                    .data
                    .and_then(|d| serde_json::from_value::<Error>(d).ok())
                {
                    Some(core) => core,
                    None => {
                        Error::tool(format!("contextual-agent: {} ({})", err.message, err.code))
                    }
                },
            );
        }
        Ok(serde_json::from_value(resp.result.unwrap_or(Value::Null))?)
    }

    fn send<M: Serialize>(&self, msg: &M) -> Result<()> {
        if !self.is_alive() {
            return Err(agent_gone());
        }
        let mut line = serde_json::to_vec(msg)?;
        line.push(b'\n');
        self.out.send(line).map_err(|_| agent_gone())
    }

    /// Asks the agent to exit after finishing in-flight requests.
    pub async fn shutdown(&self) -> Result<()> {
        self.request::<_, Value>(method::SHUTDOWN, Value::Null)
            .await
            .map(|_| ())
    }
}

/// Message of the error a call gets when the agent died under it.
pub const AGENT_STOPPED: &str = "The workspace agent stopped. Reopen the project to restart it.";

fn agent_gone() -> Error {
    Error::new(ErrorCode::Io, AGENT_STOPPED)
}

struct CancelOnDrop<'a> {
    id: u64,
    host: Option<&'a RemoteHost>,
}

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        if let Some(host) = self.host {
            host.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.id);
            let params = serde_json::to_value(CancelParams { id: self.id }).unwrap_or_default();
            let _ = host.send(&Notification::new(method::CANCEL, params));
        }
    }
}

#[async_trait]
impl WorkspaceHost for RemoteHost {
    async fn allow_root(&self, root: &str) -> Result<()> {
        self.request::<_, Value>(
            method::ALLOW_ROOT,
            AllowRootParams {
                root: root.to_string(),
            },
        )
        .await
        .map(|_| ())
    }
    async fn list_dirs(&self, req: ListDirs) -> Result<DirListing> {
        self.request(method::LIST_DIRS, req).await
    }
    async fn scan(&self, req: Scan) -> Result<ScanResult> {
        self.request(method::SCAN, req).await
    }
    async fn read(&self, req: ReadFiles) -> Result<ReadFilesResult> {
        self.request(method::READ, req).await
    }
    async fn apply(&self, req: Apply) -> Result<ApplyResult> {
        self.request(method::APPLY, req).await
    }
    async fn undo(&self, req: Undo) -> Result<UndoResult> {
        self.request(method::UNDO, req).await
    }
    async fn prepare_command(&self, req: PrepareCommand) -> Result<PreparedCommand> {
        if !self
            .info
            .capabilities
            .iter()
            .any(|capability| capability == "commandPrepare")
        {
            return Err(Error::tool(
                "The workspace agent does not support command preparation. Redeploy it.",
            ));
        }
        self.request(method::COMMAND_PREPARE, req).await
    }
    async fn git(&self, call: GitCall) -> Result<GitReply> {
        self.request(method::GIT, call).await
    }
}

/// Starts an agent (locally, or inside a WSL distro) and connects to it.
#[async_trait]
pub trait AgentLauncher: Send + Sync {
    async fn launch(&self) -> Result<RemoteHost>;
    /// "WSL: Ubuntu", for messages.
    fn describe(&self) -> String;
}

/// Spawns an agent binary on this machine (development and tests).
pub struct LocalAgentLauncher {
    pub binary: std::path::PathBuf,
    pub app_version: String,
    pub env: Vec<(String, String)>,
}

#[async_trait]
impl AgentLauncher for LocalAgentLauncher {
    async fn launch(&self) -> Result<RemoteHost> {
        let mut cmd = Command::new(&self.binary);
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        RemoteHost::spawn(cmd, &self.app_version, Vec::new()).await
    }
    fn describe(&self) -> String {
        format!("local agent {}", self.binary.display())
    }
}

/// A [`WorkspaceHost`] backed by an agent that is started on first use and restarted (with
/// backoff) when it dies. Allowed roots are re-sent after a restart. Read-only calls are retried
/// once on a fresh agent; saves, undo and git mutations are not, so nothing can apply twice.
pub struct SupervisedHost {
    launcher: Arc<dyn AgentLauncher>,
    current: tokio::sync::Mutex<Option<Arc<RemoteHost>>>,
    roots: Mutex<Vec<String>>,
    failures: Mutex<u32>,
}

impl std::fmt::Debug for SupervisedHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupervisedHost")
            .field("launcher", &self.launcher.describe())
            .finish()
    }
}

/// Delay before restart attempt `n` (0 = first start): 0, 0.5, 1, 2, 4, 8 s, then 8 s.
fn backoff(n: u32) -> std::time::Duration {
    if n == 0 {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_millis(250u64 << n.min(5))
}

fn is_agent_gone(e: &Error) -> bool {
    e.code == ErrorCode::Io && e.message == AGENT_STOPPED
}

impl SupervisedHost {
    pub fn new(launcher: Arc<dyn AgentLauncher>) -> Self {
        Self {
            launcher,
            current: tokio::sync::Mutex::new(None),
            roots: Mutex::default(),
            failures: Mutex::new(0),
        }
    }

    /// The running agent, starting (or restarting) it if needed.
    pub async fn connected(&self) -> Result<Arc<RemoteHost>> {
        let mut current = self.current.lock().await;
        if let Some(h) = current.as_ref().filter(|h| h.is_alive()) {
            return Ok(h.clone());
        }
        let attempt = *self.failures.lock().unwrap_or_else(|e| e.into_inner());
        tokio::time::sleep(backoff(attempt)).await;
        let host = match self.launcher.launch().await {
            Ok(h) => h,
            Err(e) => {
                *self.failures.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                return Err(Error::new(
                    e.code,
                    format!(
                        "Couldn't start the workspace agent ({}): {}",
                        self.launcher.describe(),
                        e.message
                    ),
                ));
            }
        };
        let roots = self.roots.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for r in roots {
            host.allow_root(&r).await?;
        }
        // A restart counts as a failure until the agent has been up for a while; a clean first
        // start resets the counter.
        let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        *failures = if current.is_some() { *failures + 1 } else { 0 };
        let host = Arc::new(host);
        *current = Some(host.clone());
        Ok(host)
    }

    /// Runs `f` on the agent; when it died under the call and `retry` is set, once more on a
    /// fresh agent.
    async fn call<T, F, Fut>(&self, retry: bool, f: F) -> Result<T>
    where
        F: Fn(Arc<RemoteHost>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let host = self.connected().await?;
        match f(host).await {
            Err(e) if retry && is_agent_gone(&e) => f(self.connected().await?).await,
            other => other,
        }
    }
}

#[async_trait]
impl WorkspaceHost for SupervisedHost {
    async fn allow_root(&self, root: &str) -> Result<()> {
        {
            let mut roots = self.roots.lock().unwrap_or_else(|e| e.into_inner());
            if !roots.iter().any(|r| r == root) {
                roots.push(root.to_string());
            }
        }
        // Also sent on every (re)start, so a fresh agent knows it.
        self.call(true, |h| async move { h.allow_root(root).await })
            .await
    }
    async fn list_dirs(&self, req: ListDirs) -> Result<DirListing> {
        self.call(true, |h| {
            let req = req.clone();
            async move { h.list_dirs(req).await }
        })
        .await
    }
    async fn scan(&self, req: Scan) -> Result<ScanResult> {
        self.call(true, |h| {
            let req = req.clone();
            async move { h.scan(req).await }
        })
        .await
    }
    async fn read(&self, req: ReadFiles) -> Result<ReadFilesResult> {
        self.call(true, |h| {
            let req = req.clone();
            async move { h.read(req).await }
        })
        .await
    }
    async fn apply(&self, req: Apply) -> Result<ApplyResult> {
        self.call(false, |h| {
            let req = req.clone();
            async move { h.apply(req).await }
        })
        .await
    }
    async fn undo(&self, req: Undo) -> Result<UndoResult> {
        self.call(false, |h| {
            let req = req.clone();
            async move { h.undo(req).await }
        })
        .await
    }
    async fn prepare_command(&self, req: PrepareCommand) -> Result<PreparedCommand> {
        // Preparation is non-executing, but do not hide a connection failure by repeating it
        // on a different agent instance. A subsequent explicit preparation gets a new epoch.
        self.call(false, |h| {
            let req = req.clone();
            async move { h.prepare_command(req).await }
        })
        .await
    }
    async fn git(&self, call: GitCall) -> Result<GitReply> {
        let read_only = matches!(
            call,
            GitCall::Status(_)
                | GitCall::Diff(_)
                | GitCall::CommitDiff(_)
                | GitCall::Branches(_)
                | GitCall::Compare(_)
                | GitCall::PrDiff(_)
        );
        self.call(read_only, |h| {
            let call = call.clone();
            async move { h.git(call).await }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    struct FailingWriter {
        inner: tokio::io::DuplexStream,
        fail: Arc<AtomicBool>,
        fail_flush: bool,
    }

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if self.fail.load(Ordering::SeqCst) && !self.fail_flush {
                return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
            }
            Pin::new(&mut self.inner).poll_write(cx, bytes)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            if self.fail.load(Ordering::SeqCst) && self.fail_flush {
                return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
            }
            Pin::new(&mut self.inner).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    async fn connection(
        fail_flush: bool,
    ) -> (
        RemoteHost,
        tokio::io::DuplexStream,
        tokio::io::DuplexStream,
        Arc<AtomicBool>,
    ) {
        let (to_agent, mut agent_input) = tokio::io::duplex(4096);
        let (mut agent_output, from_agent) = tokio::io::duplex(4096);
        let fail = Arc::new(AtomicBool::new(false));
        let writer = FailingWriter {
            inner: to_agent,
            fail: fail.clone(),
            fail_flush,
        };
        let handshake = async {
            let mut line = String::new();
            BufReader::new(&mut agent_input)
                .read_line(&mut line)
                .await
                .unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            assert_eq!(request.method, method::INITIALIZE);
            let response = Response::ok(
                request.id,
                serde_json::to_value(InitializeResult {
                    protocol_version: PROTOCOL_VERSION,
                    agent_version: "test".into(),
                    capabilities: Vec::new(),
                })
                .unwrap(),
            );
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            agent_output.write_all(&bytes).await.unwrap();
        };
        let (host, ()) = tokio::join!(
            RemoteHost::connect(BufReader::new(from_agent), writer, "test", Vec::new()),
            handshake,
        );
        (host.unwrap(), agent_input, agent_output, fail)
    }

    #[tokio::test]
    async fn reader_eof_fails_pending_and_later_requests() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (host, agent_input, agent_output, _fail) = connection(false).await;
            let host = Arc::new(host);
            let first_host = host.clone();
            let first = tokio::spawn(async move {
                first_host
                    .request::<_, Value>("test/first", Value::Null)
                    .await
            });
            let second_host = host.clone();
            let second = tokio::spawn(async move {
                second_host
                    .request::<_, Value>("test/second", Value::Null)
                    .await
            });

            let mut requests = BufReader::new(agent_input).lines();
            assert!(requests.next_line().await.unwrap().is_some());
            assert!(requests.next_line().await.unwrap().is_some());
            // The other direction remains open: reader EOF alone must fail all callers.
            drop(agent_output);

            assert_eq!(first.await.unwrap().unwrap_err().code, ErrorCode::Io);
            assert_eq!(second.await.unwrap().unwrap_err().code, ErrorCode::Io);
            assert!(!host.is_alive());
            assert!(host.pending.lock().unwrap().is_empty());
            let error = host
                .request::<_, Value>("test/later", Value::Null)
                .await
                .unwrap_err();
            assert_eq!(error.message, AGENT_STOPPED);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn write_or_flush_failure_fails_callers_even_with_agent_output_open() {
        tokio::time::timeout(Duration::from_secs(5), async {
            for fail_flush in [false, true] {
                let (host, agent_input, _agent_output, fail) = connection(fail_flush).await;
                let host = Arc::new(host);
                let first_host = host.clone();
                let first = tokio::spawn(async move {
                    first_host
                        .request::<_, Value>("test/first", Value::Null)
                        .await
                });

                let mut requests = BufReader::new(agent_input).lines();
                let first_request: Request =
                    serde_json::from_str(&requests.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(first_request.method, "test/first");

                fail.store(true, Ordering::SeqCst);
                let error = host
                    .request::<_, Value>("test/broken-write", Value::Null)
                    .await
                    .unwrap_err();
                assert_eq!(error.code, ErrorCode::Io);
                assert_eq!(first.await.unwrap().unwrap_err().code, ErrorCode::Io);
                assert!(!host.is_alive());
                assert!(host.pending.lock().unwrap().is_empty());
                let error = host
                    .request::<_, Value>("test/later", Value::Null)
                    .await
                    .unwrap_err();
                assert_eq!(error.message, AGENT_STOPPED);
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn exhausted_request_ids_are_not_reused() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (host, _agent_input, _agent_output, _fail) = connection(false).await;
            host.next_id.store(u64::MAX, Ordering::Relaxed);
            assert!(
                host.request::<_, Value>("test/exhausted", Value::Null)
                    .await
                    .is_err()
            );
            assert_eq!(host.next_id.load(Ordering::Relaxed), u64::MAX);
            assert!(host.pending.lock().unwrap().is_empty());
        })
        .await
        .unwrap();
    }
}
