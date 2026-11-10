//! Approval-gated execution supervision, independent of native process-tree APIs.
//!
//! There is deliberately no default native runner. The Git `CommandRunner` does not implement
//! this interface: its inherited environment and immediate-child cancellation are unsuitable.
//!
//! A backend must establish process-tree ownership before the requested program can run.
//! Its drop implementation must initiate cleanup even if its async operation is interrupted.
//! Explicitly configured LocalHosts expose this supervisor through WorkspaceHost::start_command.
//! The agent service uses the same supervisor after establishing its connection binding.
//! No native backend is selected by either path.

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
    /// Canonical project root on the execution host, independent of a requested path alias.
    /// None is accepted when reading older history; new intents must provide it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_root: Option<String>,
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
    /// Refuses admission when recovery is unavailable or an earlier execution for this project
    /// remains unresolved. The execution service holds its host reservation during this check
    /// and through intent publication, execution, and completion recording.
    ///
    /// Default refusal prevents older journal implementations from silently skipping recovery.
    async fn check_recovery(
        &self,
        _project: &ProjectRoot,
        _canonical_root: &std::path::Path,
    ) -> Result<()> {
        Err(Error::tool(
            "This execution journal does not support recovery admission.",
        ))
    }

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
    ///
    /// Admission is reserved synchronously; a busy project returns an error instead of queuing.
    /// On a start error no worker was launched; the caller must finish its approval-gate attempt
    /// as FailedToStart. A new attempt still requires fresh approval.
    pub fn start(&self, permit: LaunchPermit, scope: CancellationToken) -> Result<ExecutionHandle> {
        if permit.plan().binding.project.location != Location::Local {
            return Err(Error::forbidden(
                "Remote projects must execute through their selected agent, never locally.",
            ));
        }
        self.start_bound(permit, scope)
    }

    // Only the agent-owned service can enter here. It has checked the independently stored
    // preparation binding and canonical admission key, not relabelled a RemoteHost as local.
    pub(super) fn start_agent(
        &self,
        dispatch: super::wire::DecodedDispatch,
        scope: CancellationToken,
    ) -> Result<ExecutionHandle> {
        let permit = LaunchPermit {
            invocation: dispatch.invocation().clone(),
            call: dispatch.call().clone(),
            plan: dispatch.plan().clone(),
        };
        self.start_bound(permit, scope)
    }

    fn start_bound(
        &self,
        permit: LaunchPermit,
        scope: CancellationToken,
    ) -> Result<ExecutionHandle> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::internal("Command execution requires a Tokio runtime."))?;
        let admission = self.host.local_command_admission().ok_or_else(|| {
            Error::forbidden("This workspace host cannot execute through an in-process runner.")
        })?;
        let lease = admission.reserve(permit.plan().directory.root(), permit.invocation())?;
        let cancel = scope.child_token();
        let guard = cancel.clone().drop_guard();
        let worker_cancel = cancel.clone();
        let (progress_tx, progress) = watch::channel(ExecutionProgress::default());
        let host = self.host.clone();
        let runner = self.runner.clone();
        let journal = self.journal.clone();
        let invocation = permit.invocation().clone();
        let task = runtime.spawn(async move {
            let result = supervise(permit, host, runner, journal, worker_cancel, progress_tx).await;
            lease.finish(&result);
            result
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
    pub(crate) fn from_worker(
        invocation: InvocationId,
        task: JoinHandle<ExecutionResult>,
        cancel: CancellationToken,
        progress: watch::Receiver<ExecutionProgress>,
    ) -> Self {
        Self {
            invocation,
            task,
            guard: cancel.clone().drop_guard(),
            cancel,
            progress,
        }
    }

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
    // Entry points, not the journal's location label, determine where execution happens.
    // Agent intents retain the app's exact WSL location without translating native paths.
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
        canonical_root: Some(text(plan.directory.root().as_os_str())?),
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
    // An accepted attempt must reach durable replay protection even if Stop arrives before
    // spawn. Cancellation is checked by run_checked after intent publication. These bounded
    // journal operations are never mistaken for a process-cleanup acknowledgement.
    let recovery = tokio::time::timeout(
        PREPARATION_TIMEOUT,
        journal.check_recovery(&intent.project, permit.plan().directory.root()),
    )
    .await;
    if !matches!(recovery, Ok(Ok(()))) {
        let mut result = ExecutionResult::not_started(
            ExecutionStatus::FailedToStart,
            "Execution history could not be cleared for this project. \
             An earlier command may be unresolved; nothing was launched.",
        );
        result.history_error = Some("Recovery admission was not confirmed.".into());
        return result;
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
pub(super) fn escape_controls(text: &str) -> String {
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

/// Own each join separately: timing out a join of both streams loses which task's result
/// was already consumed. Awaiting that completed JoinHandle again would panic.
async fn finish_drain(mut task: JoinHandle<()>) {
    if tokio::time::timeout(DRAIN_TIMEOUT, &mut task)
        .await
        .is_err()
    {
        task.abort();
        let _ = task.await;
    }
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
    let out_task = tokio::spawn(drain(stdout, out.clone()));
    let err_task = tokio::spawn(drain(stderr, err.clone()));
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

    // Drain concurrently within the same bounded interval. Each helper consumes its task
    // result once and aborts/awaits only its own timed-out task.
    tokio::join!(finish_drain(out_task), finish_drain(err_task));

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

/// Linux process-group ownership for the native execution adapter.
///
/// This component does not authorize or launch commands. The production launch adapter is
/// still required; ordinary app and agent startup do not construct this type.
#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
pub mod linux {
    use std::process::Child;
    use std::time::Duration;

    use async_trait::async_trait;
    use nix::errno::Errno;
    use nix::sys::signal::{Signal, killpg};
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
    use nix::unistd::Pid;

    use super::{ManagedProcessTree, ProcessExit};
    use crate::{Error, Result};

    const POLL_INTERVAL: Duration = Duration::from_millis(20);

    /// Exclusively owned, unreaped process-group leader.
    ///
    /// The trusted launch adapter must establish `process_group(0)` before target code runs,
    /// take the output handles, and transfer the Child here without ever waiting on it.
    /// The host must not use automatic child reaping or competing process-wide waiters.
    /// These are integration preconditions, not properties granted by model arguments.
    ///
    /// Root exit is observed with WNOWAIT. SIGKILL is sent while the unreaped root reserves
    /// the group's identity. After reaping, only signal-zero existence checks are allowed.
    /// An absent group confirms cleanup; permissions errors or a group that persists leave
    /// cleanup unconfirmed. A recycled group can delay confirmation but is never killed.
    ///
    /// This is process-group containment, not a sandbox. Descendants that deliberately
    /// leave the group are outside its guarantee. Unreaped orphan zombies may conservatively
    /// prevent confirmation even though they can no longer execute.
    pub struct LinuxProcessTree {
        child: Option<Child>,
        group: Pid,
        exit: Option<ProcessExit>,
        kill_requested: bool,
        ownership_lost: bool,
        cleanup_confirmed: bool,
    }

    impl LinuxProcessTree {
        /// Takes exclusive ownership immediately, including cleanup responsibility on Drop.
        ///
        /// `child` must be a never-reaped group leader created with `process_group(0)`.
        /// This adoption API is for trusted native launch code, not an executable host endpoint.
        pub fn from_unreaped_group(child: Child) -> Self {
            // Linux Child IDs originate as positive pid_t values, which fit in i32.
            let group = Pid::from_raw(child.id() as i32);
            Self {
                child: Some(child),
                group,
                exit: None,
                kill_requested: false,
                ownership_lost: false,
                cleanup_confirmed: false,
            }
        }

        fn ownership_error() -> Error {
            Error::tool("Native command process ownership could not be verified.")
        }

        /// Always queries the kernel while the Child is retained, even if exit was observed
        /// earlier. In particular, ECHILD must invalidate further destructive signalling.
        fn poll_root(&mut self) -> Result<Option<ProcessExit>> {
            if self.ownership_lost || self.group.as_raw() <= 1 {
                return Err(Self::ownership_error());
            }
            if self.child.is_none() {
                return self.exit.map(Some).ok_or_else(Self::ownership_error);
            }

            let status = waitid(
                Id::Pid(self.group),
                WaitPidFlag::WEXITED | WaitPidFlag::WNOWAIT | WaitPidFlag::WNOHANG,
            );
            let exit = match status {
                Ok(WaitStatus::StillAlive) | Err(Errno::EINTR) => return Ok(None),
                Ok(WaitStatus::Exited(pid, code)) if pid == self.group => ProcessExit {
                    success: code == 0,
                    code: Some(code),
                },
                Ok(WaitStatus::Signaled(pid, _, _)) if pid == self.group => ProcessExit {
                    success: false,
                    code: None,
                },
                _ => {
                    self.ownership_lost = true;
                    return Err(Self::ownership_error());
                }
            };
            self.exit = Some(exit);
            Ok(Some(exit))
        }

        fn signal_owned_group(&mut self) -> Result<()> {
            if self.child.is_none() {
                return Err(Self::ownership_error());
            }
            self.poll_root()?;
            match killpg(self.group, Signal::SIGKILL) {
                Ok(()) | Err(Errno::ESRCH) => {
                    self.kill_requested = true;
                    Ok(())
                }
                Err(_) => Err(Error::tool(
                    "Native command process-group termination could not be requested.",
                )),
            }
        }

        fn reap_observed_root(&mut self) -> Result<ProcessExit> {
            // Remove signalling authority BEFORE calling wait. Even an error must not leave
            // a Drop path that could signal a numeric group whose root may have been reaped.
            let mut child = self.child.take().ok_or_else(Self::ownership_error)?;
            match child.wait() {
                Ok(status) => {
                    let exit = ProcessExit {
                        success: status.success(),
                        code: status.code(),
                    };
                    self.exit = Some(exit);
                    Ok(exit)
                }
                Err(_) => {
                    self.ownership_lost = true;
                    Err(Self::ownership_error())
                }
            }
        }
    }

    /// Signal zero checks existence/permissions without delivering a signal.
    ///
    /// This remains safe after root reaping: a reused numeric group cannot receive SIGKILL
    /// from this path. ESRCH establishes that the original group no longer exists; any other
    /// error refuses confirmation.
    fn group_is_absent(group: Pid) -> Result<bool> {
        if group.as_raw() <= 1 {
            return Err(LinuxProcessTree::ownership_error());
        }
        match killpg(group, None::<Signal>) {
            Err(Errno::ESRCH) => Ok(true),
            Ok(()) | Err(Errno::EINTR) => Ok(false),
            Err(_) => Err(Error::tool(
                "Native command process-group disappearance could not be verified.",
            )),
        }
    }

    #[async_trait]
    impl ManagedProcessTree for LinuxProcessTree {
        async fn wait_root(&mut self) -> Result<ProcessExit> {
            loop {
                if let Some(exit) = self.poll_root()? {
                    return Ok(exit);
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }

        async fn terminate_and_wait(&mut self) -> Result<ProcessExit> {
            if self.ownership_lost {
                return Err(Self::ownership_error());
            }
            if self.cleanup_confirmed {
                return self.exit.ok_or_else(Self::ownership_error);
            }
            if !self.kill_requested {
                self.signal_owned_group()?;
            }

            while self.child.is_some() {
                if self.poll_root()?.is_some() {
                    self.reap_observed_root()?;
                    break;
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }

            // No destructive signal is permitted below this point, including on Drop.
            // A supervisor deadline can interrupt this loop without releasing an unresolved
            // project slot or authorizing another invocation.
            loop {
                if group_is_absent(self.group)? {
                    self.cleanup_confirmed = true;
                    return self.exit.ok_or_else(Self::ownership_error);
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }

    impl Drop for LinuxProcessTree {
        fn drop(&mut self) {
            if self.child.is_none() {
                return;
            }

            // The root is still retained, so another cleanup request is safe under exclusive
            // reaping ownership. If that ownership was lost, never signal or wait by stale PID.
            let _ = self.signal_owned_group();
            let Some(mut child) = self.child.take() else {
                return;
            };
            if self.ownership_lost {
                return;
            }

            // An OS thread, rather than a Tokio task, lets reaping continue after runtime
            // shutdown. It never sends another signal. Drop initiates cleanup; it does not
            // establish a terminal observation or release execution admission.
            //
            // If thread creation fails, dropping std::process::Child does not reap it.
            // No subsequent path uses its numeric identity to send a destructive signal.
            let _ = std::thread::Builder::new()
                .name("contextual-command-reaper".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }

    #[cfg(test)]
    mod tests {
        use std::os::unix::process::CommandExt as _;
        use std::process::{Command, Stdio};

        use tokio::sync::watch;
        use tokio_util::sync::CancellationToken;

        use super::*;
        use crate::commands::execution::{
            ExecutionProgress, ExecutionStatus, ManagedChild, OUTPUT_LIMIT, run_child,
        };

        /// Fixed test scripts only. This is not the production ManagedCommandRunner:
        /// executable resolution and exact approved launch remain separate implementation work.
        fn fixture(script: &str) -> (ManagedChild, Pid) {
            let mut child = Command::new("/bin/sh")
                .args(["-c", script])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            let stderr = child.stderr.take().unwrap();
            let tree = LinuxProcessTree::from_unreaped_group(child);
            let group = tree.group;
            // The tree owns cleanup before either fallible I/O conversion.
            let stdout = tokio::process::ChildStdout::from_std(stdout).unwrap();
            let stderr = tokio::process::ChildStderr::from_std(stderr).unwrap();
            (
                ManagedChild {
                    tree: Box::new(tree),
                    stdout: Box::new(stdout),
                    stderr: Box::new(stderr),
                },
                group,
            )
        }

        #[tokio::test]
        async fn repeated_root_observation_does_not_reap_before_cleanup() {
            tokio::time::timeout(Duration::from_secs(10), async {
                let (mut child, group) = fixture("exit 23");
                let expected = ProcessExit {
                    success: false,
                    code: Some(23),
                };

                assert_eq!(child.tree.wait_root().await.unwrap(), expected);
                assert_eq!(child.tree.wait_root().await.unwrap(), expected);
                assert!(matches!(
                    waitid(
                        Id::Pid(group),
                        WaitPidFlag::WEXITED | WaitPidFlag::WNOWAIT | WaitPidFlag::WNOHANG,
                    ),
                    Ok(WaitStatus::Exited(pid, 23)) if pid == group
                ));

                assert_eq!(child.tree.terminate_and_wait().await.unwrap(), expected);
                assert_eq!(child.tree.terminate_and_wait().await.unwrap(), expected);
                assert!(group_is_absent(group).unwrap());
            })
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn native_tree_runs_through_bounded_supervisor_capture() {
            tokio::time::timeout(Duration::from_secs(20), async {
                let (child, group) = fixture(
                    "i=0; while [ \"$i\" -lt 8192 ]; do \
                     printf '0123456789abcdef'; \
                     printf 'fedcba9876543210' >&2; \
                     i=$((i + 1)); done",
                );
                let (progress, _) = watch::channel(ExecutionProgress::default());
                let result = run_child(
                    child,
                    Duration::from_secs(10),
                    &CancellationToken::new(),
                    &progress,
                )
                .await;

                assert_eq!(result.status, ExecutionStatus::Success);
                for output in [&result.output.stdout, &result.output.stderr] {
                    assert_eq!(output.bytes_seen, 8192 * 16);
                    assert_eq!(output.bytes_retained, OUTPUT_LIMIT);
                    assert!(output.truncated && output.complete);
                    assert!(!output.read_failed);
                }
                assert!(group_is_absent(group).unwrap());
            })
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn native_timeout_terminates_the_root() {
            tokio::time::timeout(Duration::from_secs(10), async {
                let (child, group) = fixture("exec sleep 30");
                let (progress, _) = watch::channel(ExecutionProgress::default());
                let result = run_child(
                    child,
                    Duration::from_millis(50),
                    &CancellationToken::new(),
                    &progress,
                )
                .await;

                assert_eq!(result.status, ExecutionStatus::TimedOut);
                assert!(result.spawned && result.timed_out);
                assert!(group_is_absent(group).unwrap());
            })
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn native_cancellation_terminates_an_already_spawned_root() {
            tokio::time::timeout(Duration::from_secs(10), async {
                let (child, group) = fixture("exec sleep 30");
                let cancel = CancellationToken::new();
                cancel.cancel();
                let (progress, _) = watch::channel(ExecutionProgress::default());
                let result =
                    run_child(child, Duration::from_secs(30), &cancel, &progress).await;

                assert_eq!(result.status, ExecutionStatus::Cancelled);
                assert!(result.spawned && result.cancelled);
                assert!(group_is_absent(group).unwrap());
            })
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn early_root_exit_never_reports_completion_while_group_remains() {
            tokio::time::timeout(Duration::from_secs(10), async {
                let (child, group) = fixture("sleep 30 & exit 23");
                let (progress, _) = watch::channel(ExecutionProgress::default());
                let result = run_child(
                    child,
                    Duration::from_secs(30),
                    &CancellationToken::new(),
                    &progress,
                )
                .await;

                assert_eq!(result.exit_code, Some(23));
                match result.status {
                    ExecutionStatus::Failure => assert!(group_is_absent(group).unwrap()),
                    // Some test hosts do not promptly reap orphan zombies. Their continued
                    // group existence must remain Unknown, not a fabricated cleanup success.
                    ExecutionStatus::Unknown => assert!(result.detail.is_some()),
                    other => panic!("unexpected native cleanup result: {other:?}"),
                }
            })
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn dropping_owned_tree_initiates_cleanup() {
            tokio::time::timeout(Duration::from_secs(10), async {
                let (child, group) = fixture("exec sleep 30");
                drop(child);

                while !group_is_absent(group).unwrap() {
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            })
            .await
            .unwrap();
        }
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
        fail_recovery: bool,
        intents: AtomicUsize,
        completions: AtomicUsize,
    }

    #[async_trait]
    impl ExecutionJournal for Journal {
        async fn check_recovery(&self, _: &ProjectRoot, _: &std::path::Path) -> Result<()> {
            if self.fail_recovery {
                Err(Error::internal("test recovery failure"))
            } else {
                Ok(())
            }
        }

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
        proposal_on(Arc::new(LocalHost::new())).await
    }

    async fn proposal_on(
        host: Arc<LocalHost>,
    ) -> (
        tempfile::TempDir,
        Arc<LocalHost>,
        ApprovalGate,
        CallId,
        ReviewPlan,
    ) {
        let root = tempfile::tempdir().unwrap();
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
    async fn default_local_host_refuses_even_a_consumed_approval() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();

        assert!(
            host.start_command(permit, CancellationToken::new())
                .is_err()
        );
        gate.finish(&call, Outcome::FailedToStart).unwrap();
        assert!(!gate.busy());
        assert!(gate.approve(&call, &plan).is_err());
    }

    #[tokio::test]
    async fn configured_workspace_host_executes_only_after_consuming_approval() {
        let (_probe_root, _probe_host, _probe_gate, _probe_call, probe) = proposal().await;
        let backend = Arc::new(runner(&probe));
        let journal = Arc::new(Journal::default());
        let configured = Arc::new(LocalHost::with_command_backend(
            backend.clone(),
            journal.clone(),
        ));
        let (_root, host, mut gate, call, plan) = proposal_on(configured).await;
        let host: Arc<dyn WorkspaceHost> = host;

        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
        assert_eq!(journal.intents.load(Ordering::SeqCst), 0);
        let approval = gate.approve(&call, &plan).unwrap();
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
        let permit = gate.begin(approval, &plan).unwrap();
        let result = host
            .start_command(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;

        assert_eq!(result.status, ExecutionStatus::Success);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 1);
        assert_eq!(journal.intents.load(Ordering::SeqCst), 1);
        assert_eq!(journal.completions.load(Ordering::SeqCst), 1);
        gate.finish(&call, result.outcome()).unwrap();
        assert!(gate.approve(&call, &plan).is_err());
    }

    #[tokio::test]
    async fn workspace_host_stop_finishes_cleanup_without_starting_another_attempt() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (_probe_root, _probe_host, _probe_gate, _probe_call, probe) = proposal().await;
            let mut backend = runner(&probe);
            backend.delay = Duration::from_secs(60);
            let backend = Arc::new(backend);
            let journal = Arc::new(Journal::default());
            let configured = Arc::new(LocalHost::with_command_backend(
                backend.clone(),
                journal.clone(),
            ));
            let (_root, host, mut gate, call, plan) = proposal_on(configured).await;
            let approval = gate.approve(&call, &plan).unwrap();
            let permit = gate.begin(approval, &plan).unwrap();
            let handle = host
                .start_command(permit, CancellationToken::new())
                .unwrap();
            let mut progress = handle.progress();
            while backend.launches.load(Ordering::SeqCst) == 0 {
                progress.changed().await.unwrap();
            }

            // An independent conversation and a cloned host still cannot queue approved work.
            let other_host = host.as_ref().clone();
            let mut other_gate = ApprovalGate::new(plan.binding.clone());
            let other_call = CallId {
                turn: "other-turn".into(),
                call: "other-call".into(),
            };
            other_gate
                .propose(other_call.clone(), plan.clone())
                .unwrap();
            let approval = other_gate.approve(&other_call, &plan).unwrap();
            let permit = other_gate.begin(approval, &plan).unwrap();
            assert!(
                other_host
                    .start_command(permit, CancellationToken::new())
                    .is_err()
            );
            other_gate
                .finish(&other_call, Outcome::FailedToStart)
                .unwrap();

            handle.stop();
            let result = handle.wait().await;
            assert_eq!(result.status, ExecutionStatus::Cancelled);
            assert!(result.cancelled && result.spawned);
            assert!(result.history_error.is_none());
            assert_eq!(journal.completions.load(Ordering::SeqCst), 1);
            assert_eq!(backend.launches.load(Ordering::SeqCst), 1);
            gate.finish(&call, result.outcome()).unwrap();
            assert!(!gate.busy());
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn configuring_a_new_host_does_not_accept_an_earlier_hosts_preparation() {
        let (root, _old_host, mut gate, call, plan) = proposal().await;
        let backend = Arc::new(runner(&plan));
        let host = LocalHost::with_command_backend(backend.clone(), Arc::new(Journal::default()));
        host.allow_root(root.path().to_str().unwrap())
            .await
            .unwrap();
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let result = host
            .start_command(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;

        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert!(!result.spawned);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn configured_local_host_does_not_execute_a_wsl_permit() {
        let (_root, _host, _gate, call, mut plan) = proposal().await;
        let backend = Arc::new(runner(&plan));
        let journal = Arc::new(Journal::default());
        let host = LocalHost::with_command_backend(backend.clone(), journal.clone());
        plan.binding.project.location = Location::Wsl {
            distro: "Ubuntu".into(),
        };
        let mut gate = ApprovalGate::new(plan.binding.clone());
        gate.propose(call.clone(), plan.clone()).unwrap();
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();

        assert!(
            host.start_command(permit, CancellationToken::new())
                .is_err()
        );
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
        assert_eq!(journal.intents.load(Ordering::SeqCst), 0);
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
    async fn failed_recovery_refuses_intent_and_launch() {
        let (_root, host, mut gate, call, plan) = proposal().await;
        let backend = Arc::new(runner(&plan));
        let journal = Arc::new(Journal {
            fail_recovery: true,
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

        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert!(result.history_error.is_some());
        assert!(!result.spawned);
        assert_eq!(journal.intents.load(Ordering::SeqCst), 0);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn journal_without_recovery_support_fails_closed() {
        struct LegacyJournal {
            intents: AtomicUsize,
        }

        #[async_trait]
        impl ExecutionJournal for LegacyJournal {
            async fn record_intent(&self, _: &ExecutionIntent) -> Result<()> {
                self.intents.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }

            async fn record_completion(&self, _: &ExecutionCompletion) -> Result<()> {
                Ok(())
            }
        }

        let (_root, host, mut gate, call, plan) = proposal().await;
        let backend = Arc::new(runner(&plan));
        let journal = Arc::new(LegacyJournal {
            intents: AtomicUsize::new(0),
        });
        let engine = ExecutionEngine::new(host, backend.clone(), journal.clone());
        let approval = gate.approve(&call, &plan).unwrap();
        let permit = gate.begin(approval, &plan).unwrap();
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;

        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert_eq!(journal.intents.load(Ordering::SeqCst), 0);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fresh_approval_cannot_bypass_an_unknown_recovered_invocation() {
        use crate::commands::journal::FileExecutionJournal;

        let (_root, host, mut old_gate, old_call, plan) = proposal().await;
        let storage = tempfile::tempdir().unwrap();
        let journal = Arc::new(
            FileExecutionJournal::open(storage.path().join("journal"))
                .await
                .unwrap(),
        );
        let approval = old_gate.approve(&old_call, &plan).unwrap();
        let old_permit = old_gate.begin(approval, &plan).unwrap();
        let (_, old_intent) = command_and_intent(&old_permit).unwrap();
        journal.record_intent(&old_intent).await.unwrap();

        let mut fresh_gate = ApprovalGate::new(plan.binding.clone());
        let call = CallId {
            turn: "new-turn".into(),
            call: "new-call".into(),
        };
        fresh_gate.propose(call.clone(), plan.clone()).unwrap();
        let approval = fresh_gate.approve(&call, &plan).unwrap();
        let permit = fresh_gate.begin(approval, &plan).unwrap();
        assert_ne!(permit.invocation(), old_permit.invocation());

        let backend = Arc::new(runner(&plan));
        let engine = ExecutionEngine::new(host, backend.clone(), journal.clone());
        let result = engine
            .start(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;

        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);
        assert_eq!(journal.records().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn separate_engines_cannot_queue_work_behind_an_active_reservation() {
        struct HeldJournal {
            entered: tokio::sync::Notify,
            release: tokio::sync::Notify,
        }

        #[async_trait]
        impl ExecutionJournal for HeldJournal {
            async fn check_recovery(&self, _: &ProjectRoot, _: &std::path::Path) -> Result<()> {
                Ok(())
            }

            async fn record_intent(&self, _: &ExecutionIntent) -> Result<()> {
                self.entered.notify_one();
                self.release.notified().await;
                Ok(())
            }

            async fn record_completion(&self, _: &ExecutionCompletion) -> Result<()> {
                Ok(())
            }
        }

        let (_root, host, mut first_gate, first_call, plan) = proposal().await;
        let journal = Arc::new(HeldJournal {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let backend = Arc::new(runner(&plan));
        let first_engine = ExecutionEngine::new(host.clone(), backend.clone(), journal.clone());
        let second_engine =
            ExecutionEngine::new(host, backend.clone(), Arc::new(Journal::default()));
        let approval = first_gate.approve(&first_call, &plan).unwrap();
        let permit = first_gate.begin(approval, &plan).unwrap();
        let first = first_engine
            .start(permit, CancellationToken::new())
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), journal.entered.notified())
            .await
            .unwrap();

        let mut second_gate = ApprovalGate::new(plan.binding.clone());
        let second_call = CallId {
            turn: "other-turn".into(),
            call: "other-call".into(),
        };
        second_gate
            .propose(second_call.clone(), plan.clone())
            .unwrap();
        let approval = second_gate.approve(&second_call, &plan).unwrap();
        let permit = second_gate.begin(approval, &plan).unwrap();
        assert!(
            second_engine
                .start(permit, CancellationToken::new())
                .is_err()
        );
        second_gate
            .finish(&second_call, Outcome::FailedToStart)
            .unwrap();
        assert_eq!(backend.launches.load(Ordering::SeqCst), 0);

        journal.release.notify_one();
        assert_eq!(first.wait().await.status, ExecutionStatus::Success);
        assert_eq!(backend.launches.load(Ordering::SeqCst), 1);
        assert_eq!(
            second_gate.state(&second_call),
            Some(crate::commands::CommandState::Finished(
                Outcome::FailedToStart
            ))
        );
    }

    #[test]
    fn separate_local_hosts_share_native_admission() {
        let first = LocalHost::new().local_command_admission().unwrap();
        let second = LocalHost::new().local_command_admission().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
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

    #[tokio::test(start_paused = true)]
    async fn drain_timeout_does_not_repoll_the_other_completed_stream() {
        // Exercise both orderings: stdout reaches EOF first, then stderr reaches EOF first.
        for stdout_finishes in [true, false] {
            let (held_reader, mut held_writer) = tokio::io::duplex(64);
            let eof: OutputPipe = Box::new(std::io::Cursor::new(b"complete".to_vec()));
            let held: OutputPipe = Box::new(held_reader);
            let (stdout, stderr) = if stdout_finishes {
                (eof, held)
            } else {
                (held, eof)
            };
            let child = ManagedChild {
                tree: Box::new(Tree {
                    writer: None,
                    delay: Duration::ZERO,
                    fail_cleanup: false,
                }),
                stdout,
                stderr,
            };
            let cancel = CancellationToken::new();
            let (progress, observed) = watch::channel(ExecutionProgress::default());
            let started = tokio::time::Instant::now();

            let result = run_child(child, Duration::from_secs(60), &cancel, &progress).await;

            assert_eq!(result.status, ExecutionStatus::Success);
            let (finished, incomplete) = if stdout_finishes {
                (&result.output.stdout, &result.output.stderr)
            } else {
                (&result.output.stderr, &result.output.stdout)
            };
            assert_eq!(finished.text, "complete");
            assert!(finished.complete);
            assert!(!incomplete.complete);
            assert!(!incomplete.read_failed);
            assert_eq!(*observed.borrow(), result.output);
            assert!(started.elapsed() >= DRAIN_TIMEOUT);
            assert!(started.elapsed() < DRAIN_TIMEOUT + Duration::from_secs(1));
            // The timed-out reader was aborted and awaited, not detached.
            assert!(held_writer.write_all(b"x").await.is_err());
        }
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

/// Actual-process probes for M2.1's ownership strategy, not a production launch adapter.
/// Fixtures use piped stdin solely to hold the root until observation is installed.
/// They do not change the managed backend's requirement to launch commands with null stdin.
#[cfg(all(
    test,
    any(
        all(target_os = "linux", not(target_env = "uclibc")),
        target_os = "macos"
    )
))]
mod unix_ownership_tests {
    use std::io::Write as _;
    use std::os::unix::process::CommandExt as _;
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::time::Duration;

    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;
    use tokio::io::{AsyncBufReadExt as _, BufReader};

    const DEADLINE: Duration = Duration::from_secs(5);
    const POLL_INTERVAL: Duration = Duration::from_millis(10);

    const EXIT_WITH_DESCENDANT: &str = "\
        IFS= read -r start || exit 90
        sleep 30 &
        printf '%s\\n' \"$!\"
        exit 23
    ";

    const WAIT_WITH_DESCENDANT: &str = "\
        IFS= read -r start || exit 90
        sleep 30 &
        printf '%s\\n' \"$!\"
        wait
    ";

    /// This fixture exclusively owns reaping. No Tokio Child or competing waiter receives
    /// the root, and group signalling stops permanently before the root is reaped.
    struct Group {
        child: Option<Child>,
        pid: Pid,
    }

    impl Group {
        fn spawn(script: &str) -> Self {
            let child = Command::new("/bin/sh")
                .args(["-c", script])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap();
            let pid = Pid::from_raw(i32::try_from(child.id()).unwrap());
            assert!(pid.as_raw() > 1);
            Self {
                child: Some(child),
                pid,
            }
        }

        async fn release(&mut self) -> Pid {
            let child = self.child.as_mut().unwrap();
            let stdout = child.stdout.take().unwrap();
            let mut output = BufReader::new(tokio::process::ChildStdout::from_std(stdout).unwrap());
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(b"go\n").unwrap();
            drop(stdin);

            let mut line = String::new();
            assert!(output.read_line(&mut line).await.unwrap() > 0);
            let descendant = Pid::from_raw(line.trim().parse().unwrap());
            assert!(descendant.as_raw() > 1);
            assert_ne!(descendant, self.pid);
            descendant
        }

        fn stop(&self) {
            assert!(self.child.is_some(), "never signal after reaping");
            killpg(self.pid, Signal::SIGKILL).unwrap();
        }

        fn reap(&mut self) -> ExitStatus {
            // Taking the child disables all subsequent Drop signalling, including if wait
            // fails. These tests call this only after non-reaping root-exit observation.
            self.child.take().unwrap().wait().unwrap()
        }
    }

    impl Drop for Group {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                // Test failures/timeouts must also initiate cleanup. Signal while the root
                // is still owned, then hand off reaping without any further numeric signals.
                let _ = killpg(self.pid, Signal::SIGKILL);
                let _ = std::thread::Builder::new()
                    .name("command-ownership-test-reaper".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn root_observation(pid: Pid) -> nix::sys::wait::WaitStatus {
        use nix::sys::wait::{Id, WaitPidFlag, waitid};

        waitid(
            Id::Pid(pid),
            WaitPidFlag::WEXITED | WaitPidFlag::WNOWAIT | WaitPidFlag::WNOHANG,
        )
        .unwrap()
    }

    #[cfg(target_os = "linux")]
    async fn wait_root_without_reaping(pid: Pid) -> nix::sys::wait::WaitStatus {
        loop {
            let status = root_observation(pid);
            if status != nix::sys::wait::WaitStatus::StillAlive {
                return status;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    #[cfg(target_os = "linux")]
    fn descendant_is_terminal(pid: Pid) -> bool {
        let stat = match std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw())) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(error) => panic!("could not observe test descendant: {error}"),
        };
        // comm is parenthesized and may itself contain spaces or parentheses.
        let fields = stat.rsplit_once(')').unwrap().1;
        matches!(fields.split_whitespace().next(), Some("Z" | "X"))
    }

    #[cfg(target_os = "linux")]
    async fn wait_descendant_terminal(pid: Pid) {
        while !descendant_is_terminal(pid) {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn linux_root_exit_keeps_ownership_until_descendant_cleanup() {
        use nix::sys::wait::WaitStatus;

        tokio::time::timeout(DEADLINE, async {
            let mut group = Group::spawn(EXIT_WITH_DESCENDANT);
            let descendant = group.release().await;

            let expected = WaitStatus::Exited(group.pid, 23);
            assert_eq!(wait_root_without_reaping(group.pid).await, expected);
            // A second observation must still find the same unreaped child.
            assert_eq!(root_observation(group.pid), expected);
            assert!(!descendant_is_terminal(descendant));

            group.stop();
            // Successful signal delivery alone is not our termination assertion.
            wait_descendant_terminal(descendant).await;
            assert_eq!(root_observation(group.pid), expected);
            assert_eq!(group.reap().code(), Some(23));
        })
        .await
        .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn linux_cancellation_terminates_running_root_and_known_descendant() {
        use nix::sys::wait::WaitStatus;

        tokio::time::timeout(DEADLINE, async {
            let mut group = Group::spawn(WAIT_WITH_DESCENDANT);
            let descendant = group.release().await;
            assert_eq!(root_observation(group.pid), WaitStatus::StillAlive);

            group.stop();

            assert!(matches!(
                wait_root_without_reaping(group.pid).await,
                WaitStatus::Signaled(pid, Signal::SIGKILL, _) if pid == group.pid
            ));
            wait_descendant_terminal(descendant).await;
            assert!(!group.reap().success());
        })
        .await
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    struct ExitWatch {
        queue: nix::sys::event::Kqueue,
        event: nix::sys::event::KEvent,
    }

    #[cfg(target_os = "macos")]
    impl ExitWatch {
        fn new(pid: Pid, notes: nix::sys::event::FilterFlag) -> Self {
            use nix::sys::event::{EvFlags, EventFilter, KEvent, Kqueue};

            let queue = Kqueue::new().unwrap();
            let event = KEvent::new(
                usize::try_from(pid.as_raw()).unwrap(),
                EventFilter::EVFILT_PROC,
                EvFlags::EV_ADD | EvFlags::EV_ONESHOT | EvFlags::EV_RECEIPT,
                notes,
                0,
                0,
            );
            let mut receipt = [event];
            let count = queue
                .kevent(
                    &[event],
                    &mut receipt,
                    Some(nix::libc::timespec {
                        tv_sec: 0,
                        tv_nsec: 0,
                    }),
                )
                .unwrap();
            assert_eq!(count, 1);
            assert!(receipt[0].flags().contains(EvFlags::EV_ERROR));
            assert_eq!(receipt[0].data(), 0, "exit-watch registration failed");
            Self { queue, event }
        }

        async fn wait(&self) -> isize {
            use nix::sys::event::{EvFlags, EventFilter, FilterFlag};

            loop {
                let mut events = [self.event];
                let count = self
                    .queue
                    .kevent(
                        &[],
                        &mut events,
                        Some(nix::libc::timespec {
                            tv_sec: 0,
                            tv_nsec: 0,
                        }),
                    )
                    .unwrap();
                if count != 0 {
                    let event = events[0];
                    assert_eq!(event.ident(), self.event.ident());
                    assert_eq!(event.filter().unwrap(), EventFilter::EVFILT_PROC);
                    assert!(!event.flags().contains(EvFlags::EV_ERROR));
                    assert!(event.fflags().contains(FilterFlag::NOTE_EXIT));
                    return event.data();
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_root_exit_keeps_ownership_until_descendant_cleanup() {
        use nix::sys::event::FilterFlag;

        tokio::time::timeout(DEADLINE, async {
            let mut group = Group::spawn(EXIT_WITH_DESCENDANT);
            // The fixture's stdin handshake prevents root exit before registration.
            let root = ExitWatch::new(
                group.pid,
                FilterFlag::NOTE_EXIT | FilterFlag::NOTE_EXITSTATUS,
            );
            let descendant = group.release().await;
            let descendant_exit = ExitWatch::new(descendant, FilterFlag::NOTE_EXIT);

            assert_eq!(root.wait().await & 0xffff, 23 << 8);

            group.stop();
            // Observe the known descendant's exit, rather than treating killpg as an ack.
            descendant_exit.wait().await;
            // kqueue observation did not reap the root: its original status is available.
            assert_eq!(group.reap().code(), Some(23));
        })
        .await
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_cancellation_terminates_running_root_and_known_descendant() {
        use std::os::unix::process::ExitStatusExt as _;

        use nix::sys::event::FilterFlag;

        tokio::time::timeout(DEADLINE, async {
            let mut group = Group::spawn(WAIT_WITH_DESCENDANT);
            let root = ExitWatch::new(group.pid, FilterFlag::NOTE_EXIT);
            let descendant = group.release().await;
            let descendant_exit = ExitWatch::new(descendant, FilterFlag::NOTE_EXIT);

            group.stop();

            root.wait().await;
            descendant_exit.wait().await;
            assert_eq!(group.reap().signal(), Some(Signal::SIGKILL as i32));
        })
        .await
        .unwrap();
    }
}
