//! [`RemoteHost`]: a [`WorkspaceHost`] that forwards every call to `contextual-agent` over
//! JSON-RPC (plan §6.6). On Windows the agent runs inside a WSL distro; in tests it runs locally.
//!
//! Dropping a call's future sends `$/cancel` for it. If the agent dies, pending and later calls
//! fail with an `Io` error; the owner restarts it (with backoff) by building a new `RemoteHost`.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use contextual_protocol::fs::{
    Apply, ApplyResult, DirListing, ListDirs, ReadFiles, ReadFilesResult, Scan, ScanResult, Undo,
    UndoResult,
};
use contextual_protocol::git::{GitCall, GitReply};
use contextual_protocol::rpc::{
    CancelParams, InitializeParams, InitializeResult, Message, Notification, PROTOCOL_VERSION,
    Request, Response, method,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use crate::error::ErrorCode;
use crate::host::WorkspaceHost;
use crate::{Error, Result};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

pub struct RemoteHost {
    next_id: AtomicU64,
    pending: Pending,
    out: mpsc::UnboundedSender<Vec<u8>>,
    /// Kept so the process is killed when the host is dropped.
    _child: Option<Child>,
    info: InitializeResult,
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
        let (out, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            let mut writer = writer;
            while let Some(line) = out_rx.recv().await {
                if writer.write_all(&line).await.is_err() || writer.flush().await.is_err() {
                    break;
                }
            }
        });
        let reader_pending = pending.clone();
        tokio::spawn(async move {
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
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
                        tracing::debug!(target: "contextual_agent", "{} {}", n.method, n.params)
                    }
                    Ok(Message::Request(r)) => {
                        tracing::warn!("agent sent a request ({}); ignored", r.method)
                    }
                    Err(e) => tracing::warn!("unparseable agent output: {e}: {line}"),
                }
            }
            // Agent gone: fail everything still waiting.
            reader_pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        });

        let mut host = Self {
            next_id: AtomicU64::new(1),
            pending,
            out,
            _child: None,
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

    pub fn info(&self) -> &InitializeResult {
        &self.info
    }

    /// Sends one request and waits for its response. Dropping the future cancels the request.
    pub async fn request<P: Serialize, T: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let mut guard = CancelOnDrop {
            id,
            host: Some(self),
        };
        self.send(&Request::new(id, method, serde_json::to_value(params)?))?;
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

fn agent_gone() -> Error {
    Error::new(
        ErrorCode::Io,
        "The workspace agent stopped. Reopen the project to restart it.",
    )
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
    async fn allow_root(&self, _root: &str) -> Result<()> {
        // Roots travel in `initialize`; a newly opened root needs a fresh connection, which keeps
        // the agent's allowlist exactly what the app opened.
        Err(Error::invalid(
            "Allowed roots are set when the agent connects; reconnect to add one.",
        ))
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
    async fn git(&self, call: GitCall) -> Result<GitReply> {
        self.request(method::GIT, call).await
    }
}
