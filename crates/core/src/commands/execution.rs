//! Approval-gated execution supervision, independent of native process-tree APIs.
//!
//! There is deliberately no default native runner. The Git `CommandRunner` does not implement
//! this interface: its inherited environment and immediate-child cancellation are unsuitable.
//!
//! A backend must establish process-tree ownership before the requested program can run.
//! Its drop implementation must initiate cleanup even if its async operation is interrupted.
//! Native backends and workspace execution transport are subsequent integration work.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use contextual_protocol::commands::{CommandPlatform, PrepareCommand};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::{CancellationToken, DropGuard};

use super::{CallId, CommandRequest, InvocationId, LaunchPermit, Outcome, ReviewPlan};
use crate::host::WorkspaceHost;
use crate::location::{Location, ProjectRoot};
use crate::process::Cmd;
use crate::{Error, Result};

/// Raw bytes retained independently for each stream. Excess bytes are still drained.
pub const OUTPUT_LIMIT: usize = 64 * 1024;
const PREPARATION_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Output is plain, escaped text, never terminal instructions or Markdown actions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapturedOutput {
    pub text: String,
    pub bytes_seen: u64,
    pub bytes_retained: usize,
    pub truncated: bool,
    pub complete: bool,
    pub read_failed: bool,
}

/// Latest cumulative snapshots. A slow consumer can skip snapshots without losing the
/// retained output: it receives the latest complete prefix, not an unreliable delta sequence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionProgress {
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
}

/// Observable process outcome. Cancellation and timeout also have independent result flags,
/// because failed cleanup can make the final outcome unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExecutionStatus {
    Success,
    Failure,
    FailedToStart,
    Cancelled,
    TimedOut,
    Unknown,
}

/// In-memory result. It deliberately does not implement serde: unshared output must not
/// accidentally become part of saved chat history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub exit_code: Option<i32>,
    pub spawned: bool,
    pub cancelled: bool,
    pub timed_out: bool,
    pub output: ExecutionProgress,
    pub detail: Option<String>,
    pub history_error: Option<String>,
}

impl ExecutionResult {
    fn not_started(status: ExecutionStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            exit_code: None,
            spawned: false,
            cancelled: status == ExecutionStatus::Cancelled,
            timed_out: false,
            output: ExecutionProgress::default(),
            detail: Some(detail.into()),
            history_error: None,
        }
    }

    /// The approval gate can be completed from the observed result. Unknown outcomes retain
    /// its project slot; failure to save history does not fabricate a process outcome.
    pub fn outcome(&self) -> Outcome {
        match self.status {
            ExecutionStatus::Success => Outcome::Success,
            ExecutionStatus::Failure => Outcome::Failure {
                code: self.exit_code,
            },
            ExecutionStatus::FailedToStart => Outcome::FailedToStart,
            ExecutionStatus::Cancelled => Outcome::Cancelled,
            ExecutionStatus::TimedOut => Outcome::TimedOut,
            ExecutionStatus::Unknown => Outcome::Unknown,
        }
    }
}

/// Persistable execution intent. No approval token or complete inherited environment.
/// Scripts and arguments are part of the reviewed invocation and may themselves be private.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionIntent {
    pub invocation: InvocationId,
    pub call: CallId,
    pub project: ProjectRoot,
    pub platform: CommandPlatform,
    pub environment_policy: String,
    pub request: CommandRequest,
    pub cwd: String,
    pub program: String,
    pub args: Vec<String>,
}

impl std::fmt::Debug for ExecutionIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionIntent")
            .field("invocation", &self.invocation)
            .field("platform", &self.platform)
            .finish_non_exhaustive()
    }
}

/// Persistable terminal observation, intentionally excluding output contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionCompletion {
    pub invocation: InvocationId,
    pub call: CallId,
    pub status: ExecutionStatus,
    pub exit_code: Option<i32>,
    pub spawned: bool,
    pub cancelled: bool,
    pub timed_out: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub output_incomplete: bool,
}

/// Required write-ahead history seam. There is no no-op production implementation.
///
/// `record_intent` must return success only after durable, atomic storage. A restored intent
/// without a completion is unknown, never permission to replay the invocation.
/// Duplicate invocation IDs must be refused, including after reopening the journal.
/// A completion must match an existing intent and must not overwrite an earlier observation.
#[async_trait]
pub trait ExecutionJournal: Send + Sync {
    async fn record_intent(&self, intent: &ExecutionIntent) -> Result<()>;
    async fn record_completion(&self, completion: &ExecutionCompletion) -> Result<()>;
}

/// Observed root-process exit. The process-tree handle must remain valid for cleanup after
/// this observation; native implementations must not signal a recycled numeric process ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessExit {
    pub success: bool,
    pub code: Option<i32>,
}

/// Owned async output pipe, with no terminal semantics.
pub type OutputPipe = Box<dyn AsyncRead + Unpin + Send>;

/// Process-tree ownership, established before any requested program code can run.
///
/// A backend must:
/// - retain safe ownership through root exit and descendant cleanup;
/// - make `wait_root` cancellation-safe;
/// - terminate descendants and reap the root in `terminate_and_wait`;
/// - return an error if termination cannot be established;
/// - initiate cleanup on Drop, including when an async cleanup deadline expires.
///
/// Deliberately detached descendants may exceed OS process-group guarantees; native backends
/// must document those limitations rather than describe this as sandboxing.
#[async_trait]
pub trait ManagedProcessTree: Send {
    async fn wait_root(&mut self) -> Result<ProcessExit>;
    async fn terminate_and_wait(&mut self) -> Result<ProcessExit>;
}

/// A process that already has tree ownership and two drainable pipes.
pub struct ManagedChild {
    pub tree: Box<dyn ManagedProcessTree>,
    pub stdout: OutputPipe,
    pub stderr: OutputPipe,
}

/// Explicitly opted-in execution backend, distinct from the Git runner.
///
/// `spawn` must clear the parent environment, then set exactly `Cmd::env`; use null stdin;
/// preserve the argument vector and cwd; and never elevate, retry, substitute an interpreter,
/// or create an implicit shell. It must refuse unsupported process-tree ownership *before*
/// starting the requested program. Errors must mean no requested program was launched.
pub trait ManagedCommandRunner: Send + Sync {
    fn platform(&self) -> CommandPlatform;
    fn spawn(&self, command: Cmd) -> Result<ManagedChild>;
}

/// Execution service for an in-process project host. Remote execution must be implemented
/// through the agent, not by passing a RemoteHost alongside a local process backend.
pub struct ExecutionEngine {
    host: Arc<dyn WorkspaceHost>,
    runner: Arc<dyn ManagedCommandRunner>,
    journal: Arc<dyn ExecutionJournal>,
}

impl ExecutionEngine {
    /// The supplied host and runner must represent the same in-process host instance.
    /// Native host integration will own this construction; GPUI views must not construct it.
    pub fn new(
        host: Arc<dyn WorkspaceHost>,
        runner: Arc<dyn ManagedCommandRunner>,
        journal: Arc<dyn ExecutionJournal>,
    ) -> Self {
        Self {
            host,
            runner,
            journal,
        }
    }

    /// Starts supervision on the current Tokio runtime. The caller has already consumed one
    /// explicit approval through ApprovalGate::begin to obtain this non-cloneable permit.
    ///
    /// `scope` must be cancelled when the project/chat execution scope is closed or replaced.
    /// Dropping the returned handle also requests cancellation; cleanup stays owned by the
    /// worker while its runtime remains alive. Orderly shutdown must cancel and await workers.
    pub fn start(&self, permit: LaunchPermit, scope: CancellationToken) -> Result<ExecutionHandle> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::internal("Command execution requires a Tokio runtime."))?;
        let cancel = scope.child_token();
        let guard = cancel.clone().drop_guard();
        let worker_cancel = cancel.clone();
        let (progress_tx, progress) = watch::channel(ExecutionProgress::default());
        let host = self.host.clone();
        let runner = self.runner.clone();
        let journal = self.journal.clone();
        let invocation = permit.invocation().clone();
        let task = runtime.spawn(async move {
            supervise(permit, host, runner, journal, worker_cancel, progress_tx).await
        });
        Ok(ExecutionHandle {
            invocation,
            task,
            cancel,
            progress,
            guard,
        })
    }
}

/// An owned running operation. Dropping it requests Stop, not Undo.
pub struct ExecutionHandle {
    invocation: InvocationId,
    task: JoinHandle<ExecutionResult>,
    cancel: CancellationToken,
    progress: watch::Receiver<ExecutionProgress>,
    guard: DropGuard,
}

impl ExecutionHandle {
    pub fn invocation(&self) -> &InvocationId {
        &self.invocation
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub fn progress(&self) -> watch::Receiver<ExecutionProgress> {
        self.progress.clone()
    }

    pub async fn wait(self) -> ExecutionResult {
        let Self {
            invocation: _,
            task,
            guard,
            progress,
            cancel: _,
        } = self;
        let result = match task.await {
            Ok(result) => result,
            Err(_) => ExecutionResult {
                status: ExecutionStatus::Unknown,
                exit_code: None,
                // A worker failure cannot establish whether spawning happened.
                spawned: true,
                cancelled: false,
                timed_out: false,
                output: progress.borrow().clone(),
                detail: Some("Execution worker stopped; command outcome is unknown.".into()),
                history_error: Some("No confirmed completion was recorded.".into()),
            },
        };
        drop(guard);
        result
    }
}

fn command_and_intent(permit: &LaunchPermit) -> Result<(Cmd, ExecutionIntent)> {
    let plan = permit.plan();
    plan.request.validate()?;
    if plan.binding.project.location != Location::Local {
        return Err(Error::forbidden(
            "Remote projects must execute through their selected agent, never locally.",
        ));
    }
    let text = |value: &std::ffi::OsStr| {
        value
            .to_str()
            .filter(|value| !value.contains('\0'))
            .map(str::to_owned)
            .ok_or_else(|| Error::invalid("Command parameters must be NUL-free UTF-8."))
    };
    let program = text(plan.program.as_os_str())?;
    let args: Vec<String> = plan
        .args
        .iter()
        .map(|arg| text(arg.as_os_str()))
        .collect::<Result<_>>()?;
    let environment: Vec<(String, String)> = plan
        .environment
        .iter()
        .map(|(name, value)| {
            let name = text(name.as_os_str())?;
            if name.is_empty() || name.contains('=') {
                return Err(Error::invalid("Invalid command environment variable name."));
            }
            Ok((name, text(value.as_os_str())?))
        })
        .collect::<Result<_>>()?;
    let intent = ExecutionIntent {
        invocation: permit.invocation().clone(),
        call: permit.call().clone(),
        project: plan.binding.project.clone(),
        platform: plan.platform,
        environment_policy: plan.environment_policy.clone(),
        request: plan.request.clone(),
        cwd: text(plan.directory.cwd().as_os_str())?,
        program: program.clone(),
        args: args.clone(),
    };
    let mut command = Cmd::new(program, args)
        .cwd(plan.directory.cwd())
        .timeout(Duration::from_secs(u64::from(plan.request.timeout_seconds)));
    // This is the complete environment for ManagedCommandRunner, not the inherited-env
    // overlay used by the separate Git SystemRunner implementation.
    command.env = environment;
    Ok((command, intent))
}

async fn supervise(
    permit: LaunchPermit,
    host: Arc<dyn WorkspaceHost>,
    runner: Arc<dyn ManagedCommandRunner>,
    journal: Arc<dyn ExecutionJournal>,
    cancel: CancellationToken,
    progress: watch::Sender<ExecutionProgress>,
) -> ExecutionResult {
    let (command, intent) = match command_and_intent(&permit) {
        Ok(value) => value,
        Err(error) => {
            return ExecutionResult::not_started(ExecutionStatus::FailedToStart, error.message);
        }
    };
    if cancel.is_cancelled() {
        return ExecutionResult::not_started(
            ExecutionStatus::Cancelled,
            "Cancelled before launch.",
        );
    }

    match tokio::time::timeout(PREPARATION_TIMEOUT, journal.record_intent(&intent)).await {
        Ok(Ok(())) => {}
        _ => {
            let mut result = ExecutionResult::not_started(
                ExecutionStatus::FailedToStart,
                "Command was not launched because execution intent could not be saved.",
            );
            result.history_error = Some("Execution intent was not confirmed saved.".into());
            return result;
        }
    }

    let mut result = run_checked(&permit, host, runner, command, &cancel, &progress).await;
    let completion = ExecutionCompletion {
        invocation: intent.invocation,
        call: intent.call,
        status: result.status,
        exit_code: result.exit_code,
        spawned: result.spawned,
        cancelled: result.cancelled,
        timed_out: result.timed_out,
        stdout_truncated: result.output.stdout.truncated,
        stderr_truncated: result.output.stderr.truncated,
        output_incomplete: result.spawned
            && (!result.output.stdout.complete
                || !result.output.stderr.complete
                || result.output.stdout.read_failed
                || result.output.stderr.read_failed),
    };
    if !matches!(
        tokio::time::timeout(PREPARATION_TIMEOUT, journal.record_completion(&completion)).await,
        Ok(Ok(()))
    ) {
        result.history_error = Some(
            "Completion could not be saved. Reopening this execution must show outcome unknown."
                .into(),
        );
    }
    result
}

async fn run_checked(
    permit: &LaunchPermit,
    host: Arc<dyn WorkspaceHost>,
    runner: Arc<dyn ManagedCommandRunner>,
    mut command: Cmd,
    cancel: &CancellationToken,
    progress: &watch::Sender<ExecutionProgress>,
) -> ExecutionResult {
    let plan = permit.plan();
    let json = match serde_json::to_string(&plan.request) {
        Ok(json) => json,
        Err(_) => {
            return ExecutionResult::not_started(
                ExecutionStatus::FailedToStart,
                "Command parameters could not be serialized.",
            );
        }
    };
    let request = PrepareCommand {
        root: plan.binding.project.path.clone(),
        project_epoch: plan.binding.project_epoch.clone(),
        command_json: json,
    };
    let prepared = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            return ExecutionResult::not_started(
                ExecutionStatus::Cancelled, "Cancelled before launch.",
            );
        }
        prepared = tokio::time::timeout(PREPARATION_TIMEOUT, host.prepare_command(request)) => prepared,
    };
    let current = match prepared {
        Ok(Ok(prepared)) => ReviewPlan::from_prepared(plan.binding.project.clone(), prepared),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(Error::tool("Command revalidation timed out.")),
    };
    match current {
        Ok(current) if &current == plan && runner.platform() == plan.platform => {}
        _ => {
            return ExecutionResult::not_started(
                ExecutionStatus::FailedToStart,
                "Command preparation changed or could not be revalidated. Review and approve again.",
            );
        }
    }
    if cancel.is_cancelled() {
        return ExecutionResult::not_started(
            ExecutionStatus::Cancelled,
            "Cancelled before launch.",
        );
    }
    command.cancel = Some(cancel.clone());
    let timeout = command.timeout;
    let child = match runner.spawn(command) {
        Ok(child) => child,
        Err(error) => {
            return ExecutionResult::not_started(
                ExecutionStatus::FailedToStart,
                escape_controls(&error.message),
            );
        }
    };
    run_child(child, timeout, cancel, progress).await
}

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    seen: u64,
    complete: bool,
    read_failed: bool,
}

impl Capture {
    fn push(&mut self, bytes: &[u8]) {
        self.seen = self.seen.saturating_add(bytes.len() as u64);
        let keep = bytes
            .len()
            .min(OUTPUT_LIMIT.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&bytes[..keep]);
    }

    fn snapshot(&self) -> CapturedOutput {
        CapturedOutput {
            text: escape_controls(&String::from_utf8_lossy(&self.bytes)),
            bytes_seen: self.seen,
            bytes_retained: self.bytes.len(),
            truncated: self.seen > self.bytes.len() as u64,
            complete: self.complete,
            read_failed: self.read_failed,
        }
    }
}

/// ESC, C1 controls and bidi/invisible formatting characters are made visible rather than
/// interpreted. Escaping instead of stripping also prevents OSC links from becoming UI links.
/// These strings must still be rendered as plain text, never parsed as Markdown.
fn escape_controls(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        if (character.is_control() && !matches!(character, '\n' | '\t'))
            || matches!(
                character,
                '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
            )
        {
            escaped.extend(character.escape_unicode());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

type SharedCapture = Arc<Mutex<Capture>>;

async fn drain(mut pipe: OutputPipe, capture: SharedCapture) {
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) => {
                capture
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .complete = true;
                return;
            }
            Ok(count) => {
                capture
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(&buffer[..count]);
            }
            Err(_) => {
                capture
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .read_failed = true;
                return;
            }
        }
    }
}

fn snapshot(stdout: &SharedCapture, stderr: &SharedCapture) -> ExecutionProgress {
    ExecutionProgress {
        stdout: stdout
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot(),
        stderr: stderr
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot(),
    }
}

enum End {
    Exited(ProcessExit),
    WaitFailed,
    Cancelled,
    TimedOut,
}

async fn run_child(
    child: ManagedChild,
    timeout: Duration,
    cancel: &CancellationToken,
    progress: &watch::Sender<ExecutionProgress>,
) -> ExecutionResult {
    let ManagedChild {
        mut tree,
        stdout,
        stderr,
    } = child;
    let out: SharedCapture = Arc::default();
    let err: SharedCapture = Arc::default();
    let mut out_task = tokio::spawn(drain(stdout, out.clone()));
    let mut err_task = tokio::spawn(drain(stderr, err.clone()));
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut interval = tokio::time::interval(PROGRESS_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let end = {
        let wait = tree.wait_root();
        tokio::pin!(wait);
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break End::Cancelled,
                _ = &mut deadline => break End::TimedOut,
                result = &mut wait => break match result {
                    Ok(exit) => End::Exited(exit),
                    Err(_) => End::WaitFailed,
                },
                _ = interval.tick() => {
                    progress.send_replace(snapshot(&out, &err));
                }
            }
        }
    };

    // This also handles descendants remaining after a successful root exit. The backend
    // retains tree ownership until cleanup, rather than acting on a possibly recycled PID.
    let cleanup = tokio::time::timeout(CLEANUP_TIMEOUT, tree.terminate_and_wait()).await;
    let confirmed = matches!(&cleanup, Ok(Ok(_)));
    let exit = match (&end, &cleanup) {
        (End::Exited(exit), _) => Some(*exit),
        (_, Ok(Ok(exit))) => Some(*exit),
        _ => None,
    };
    drop(tree);

    // Never let a descendant holding a pipe open keep execution supervision alive forever.
    if tokio::time::timeout(DRAIN_TIMEOUT, async {
        let _ = tokio::join!(&mut out_task, &mut err_task);
    })
    .await
    .is_err()
    {
        out_task.abort();
        err_task.abort();
        let _ = tokio::join!(out_task, err_task);
    }

    let output = snapshot(&out, &err);
    progress.send_replace(output.clone());
    let status = if !confirmed || matches!(end, End::WaitFailed) {
        ExecutionStatus::Unknown
    } else {
        match end {
            End::Cancelled => ExecutionStatus::Cancelled,
            End::TimedOut => ExecutionStatus::TimedOut,
            End::Exited(exit) if exit.success => ExecutionStatus::Success,
            End::Exited(_) => ExecutionStatus::Failure,
            End::WaitFailed => ExecutionStatus::Unknown,
        }
    };
    ExecutionResult {
        status,
        exit_code: exit.and_then(|exit| exit.code),
        spawned: true,
        cancelled: matches!(end, End::Cancelled),
        timed_out: matches!(end, End::TimedOut),
        output,
        detail: (!confirmed).then(|| {
            "Process-tree termination could not be confirmed. Side effects may continue; \
             the outcome is unknown."
                .into()
        }),
        history_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncWriteExt;

    use crate::commands::ApprovalGate;
    use crate::host::LocalHost;

    #[derive(Default)]
    struct Journal {
        fail_intent: bool,
        fail_completion: bool,
        intents: AtomicUsize,
        completions: AtomicUsize,
    }

    #[async_trait]
    impl ExecutionJournal for Journal {
        async fn record_intent(&self, _: &ExecutionIntent) -> Result<()> {
            if self.fail_intent {
                return Err(Error::internal("test journal failure"));
            }
            self.intents.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn record_completion(&self, _: &ExecutionCompletion) -> Result<()> {
            if self.fail_completion {
                return Err(Error::internal("test completion failure"));
            }
            self.completions.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Runner {
        platform: CommandPlatform,
        launches: AtomicUsize,
        delay: Duration,
        output_size: usize,
        fail_spawn: bool,
        fail_cleanup: bool,
    }

    impl ManagedCommandRunner for Runner {
        fn platform(&self) -> CommandPlatform {
            self.platform
        }

        fn spawn(&self, command: Cmd) -> Result<ManagedChild> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            assert!(command.cwd.is_some());
            assert!(
                !command
                    .env
                    .iter()
                    .any(|(name, _)| name == "AZURE_OPENAI_API_KEY")
            );
            if self.fail_spawn {
                return Err(Error::tool("test spawn failure"));
            }
            // Small pipes deliberately exercise draining/backpressure past the capture limit.
            let (stdout, mut out_writer) = tokio::io::duplex(64);
            let (stderr, mut err_writer) = tokio::io::duplex(64);
            let count = self.output_size;
            let writer = tokio::spawn(async move {
                let _ = out_writer.write_all(&vec![b'x'; count]).await;
                let _ = err_writer.write_all(b"stderr\x1b[31m").await;
            });
            Ok(ManagedChild {
                tree: Box::new(Tree {
                    writer: Some(writer),
                    delay: self.delay,
                    fail_cleanup: self.fail_cleanup,
                }),
                stdout: Box::new(stdout),
                stderr: Box::new(stderr),
            })
        }
    }

    struct Tree {
        writer: Option<JoinHandle<()>>,
        delay: Duration,
        fail_cleanup: bool,
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            if let Some(writer) = &self.writer {
                writer.abort();
            }
        }
    }

    #[async_trait]
    impl ManagedProcessTree for Tree {
        async fn wait_root(&mut self) -> Result<ProcessExit> {
            if let Some(writer) = self.writer.as_mut() {
                let _ = writer.await;
            }
            self.writer = None;
            tokio::time::sleep(self.delay).await;
            Ok(ProcessExit {
                success: true,
                code: Some(0),
            })
        }

        async fn terminate_and_wait(&mut self) -> Result<ProcessExit> {
            if let Some(writer) = self.writer.take() {
                writer.abort();
                let _ = writer.await;
            }
            if self.fail_cleanup {
                return Err(Error::tool("test cleanup failure"));
            }
            Ok(ProcessExit {
                success: false,
                code: None,
            })
        }
    }

    async fn proposal() -> (
        tempfile::TempDir,
        Arc<LocalHost>,
        ApprovalGate,
        CallId,
        ReviewPlan,
    ) {
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(LocalHost::new());
        let path = root.path().to_str().unwrap().to_owned();
        host.allow_root(&path).await.unwrap();
        let prepared = host
            .prepare_command(PrepareCommand {
                root: path.clone(),
                project_epoch: "project-1".into(),
                command_json: serde_json::json!({
                    "version": 1,
                    "purpose": "Exercise the fake execution backend.",
                    "command": {
                        "kind": "exec",
                        "program": std::env::current_exe().unwrap().to_str().unwrap(),
                        "args": []
                    },
                    "cwd": ".",
                    "timeoutSeconds": 1,
                    "environment": []
                })
                .to_string(),
            })
            .await
            .unwrap();
        let plan = ReviewPlan::from_prepared(
            ProjectRoot {
                location: Location::Local,
                path,
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
        (root, host, gate, call, plan)
    }

    fn runner(plan: &ReviewPlan) -> Runner {
        Runner {
            platform: plan.platform,
            launches: AtomicUsize::new(0),
            delay: Duration::ZERO,
            output_size: 10,
            fail_spawn: false,
            fail_cleanup: false,
        }
    }

    #[tokio::test]
    async fn preparation_and_rejection_never_reach_the_runner() {
        let (_root, _host, mut gate, call, plan) = proposal().await;
        let runner = runner(&plan);
        assert_eq!(runner.launches.load(Ordering::SeqCst), 0);
        let approval = gate.approve(&call, &plan).unwrap();
        gate.reject(&call).unwrap();
        assert!(gate.begin(approval, &plan).is_err());
        assert_eq!(runner.launches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn output_limits_do_not_stop_draining_and_progress_is_cumulative() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let mut backend = runner(&plan);
        backend.output_size = OUTPUT_LIMIT * 2;
        let backend = Arc::new(backend);
        let journal = Arc::new(Journal::default());
        let engine = ExecutionEngine::new(host, backend.clone(), journal.clone());
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let handle = engine.start(permit, CancellationToken::new()).unwrap();
        let progress = handle.progress();
        let result = handle.wait().await;

        assert_eq!(result.status, ExecutionStatus::Success);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.output.stdout.bytes_seen, (OUTPUT_LIMIT * 2) as u64);
        assert_eq!(result.output.stdout.bytes_retained, OUTPUT_LIMIT);
        assert!(result.output.stdout.truncated && result.output.stdout.complete);
        assert!(result.output.stderr.text.contains("\\u{1b}[31m"));
        assert_eq!(*progress.borrow(), result.output);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 1);
        assert_eq!(journal.intents.load(Ordering::SeqCst), 1);
        assert_eq!(journal.completions.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn journal_failure_refuses_launch() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let backend = Arc::new(runner(&plan));
        let journal = Arc::new(Journal {
            fail_intent: true,
            ..Journal::default()
        });
        let engine = ExecutionEngine::new(host, backend.clone(), journal);
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert!(result.history_error.is_some());
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn supervisor_records_a_real_journal_without_persisting_output() {
        use crate::commands::journal::FileExecutionJournal;

        let (_root, host, mut gate, call, plan) = proposal().await;
        let storage = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            FileExecutionJournal::open(storage.path().join("journal"))
                .await
                .unwrap(),
        );
        let backend = Arc::new(runner(&plan));
        let engine = ExecutionEngine::new(host, backend.clone(), journal.clone());
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let invocation = permit.invocation().clone();
        let handle = engine.start(permit, CancellationToken::new()).unwrap();
        assert_eq!(handle.invocation(), &invocation);
        let result = handle.wait().await;

        let restored = journal.load(&invocation).await.unwrap();
        assert_eq!(restored.status(), ExecutionStatus::Success);
        assert_eq!(restored.intent.call, call);
        assert_eq!(restored.intent.request, plan.request);
        assert!(!restored.blocks_execution());
        assert!(result.history_error.is_none());
        assert_eq!(backend.launches.load(Ordering::SeqCst), 1);
        let completion = std::fs::read_to_string(
            journal
                .dir()
                .join(format!("{}.completion.json", invocation.as_str())),
        )
        .unwrap();
        assert!(!completion.contains("stderr\\u"));
        assert!(!completion.contains("\"stdout\":"));
        assert!(!completion.contains("\"stderr\":"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_existing_durable_intent_refuses_dispatch_without_retry() {
        use crate::commands::journal::FileExecutionJournal;

        let (_root, host, mut gate, call, plan) = proposal().await;
        let storage = tempfile::tempdir().unwrap();
        let journal = FileExecutionJournal::open(storage.path().join("journal"))
            .await
            .unwrap();
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let (_, intent) = command_and_intent(&permit).unwrap();
        journal.record_intent(&intent).await.unwrap();

        let reopened = Arc::new(
            FileExecutionJournal::open(storage.path().join("journal"))
                .await
                .unwrap(),
        );
        let backend = Arc::new(runner(&plan));
        let engine = ExecutionEngine::new(host, backend.clone(), reopened.clone());
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;

        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert!(!result.spawned);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
        assert!(
            reopened
                .load(&intent.invocation)
                .await
                .unwrap()
                .blocks_execution()
        );
    }

    #[tokio::test]
    async fn completion_write_failure_does_not_fabricate_a_process_failure() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let backend = Arc::new(runner(&plan));
        let journal = Arc::new(Journal {
            fail_completion: true,
            ..Journal::default()
        });
        let engine = ExecutionEngine::new(host, backend.clone(), journal.clone());
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;

        assert_eq!(result.status, ExecutionStatus::Success);
        assert!(result.history_error.is_some());
        assert_eq!(backend.launches.load(Ordering::SeqCst), 1);
        assert_eq!(journal.intents.load(Ordering::SeqCst), 1);
        assert_eq!(journal.completions.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancelled_scope_refuses_launch() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let backend = Arc::new(runner(&plan));
        let engine = ExecutionEngine::new(host, backend.clone(), Arc::new(Journal::default()));
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = engine.start(permit, cancel).unwrap().wait().await;
        assert_eq!(result.status, ExecutionStatus::Cancelled);
        assert!(!result.spawned);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn changed_host_instance_refuses_launch_without_retry() {
        let (root, _host, mut gate, call, plan) = proposal().await;
        let replacement = Arc::new(LocalHost::new());
        replacement
            .allow_root(root.path().to_str().unwrap())
            .await
            .unwrap();
        let backend = Arc::new(runner(&plan));
        let engine =
            ExecutionEngine::new(replacement, backend.clone(), Arc::new(Journal::default()));
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failed_start_is_distinct_and_not_retried() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let mut backend = runner(&plan);
        backend.fail_spawn = true;
        let backend = Arc::new(backend);
        let engine = ExecutionEngine::new(host, backend.clone(), Arc::new(Journal::default()));
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert!(!result.spawned);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn timeout_terminates_the_fake_tree() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let mut backend = runner(&plan);
        backend.delay = Duration::from_secs(60);
        let engine = ExecutionEngine::new(host, Arc::new(backend), Arc::new(Journal::default()));
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::TimedOut);
        assert!(result.timed_out && result.spawned);
    }

    #[tokio::test]
    async fn failed_cleanup_leaves_the_gate_blocked() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let mut backend = runner(&plan);
        backend.fail_cleanup = true;
        let engine = ExecutionEngine::new(host, Arc::new(backend), Arc::new(Journal::default()));
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::Unknown);
        gate.finish(&call, result.outcome()).unwrap();
        assert!(gate.busy());
    }

    #[test]
    fn controls_are_visible_not_interpreted() {
        assert_eq!(
            escape_controls("a\r\x1b]8;;https://example.com\x07b\u{202e}\n"),
            "a\\u{d}\\u{1b}]8;;https://example.com\\u{7}b\\u{202e}\n",
        );
    }

    #[test]
    fn split_utf8_is_reconstructed_from_the_cumulative_bytes() {
        let mut capture = Capture::default();
        let bytes = "雪".as_bytes();
        capture.push(&bytes[..1]);
        capture.push(&bytes[1..]);
        assert_eq!(capture.snapshot().text, "雪");
    }
}
