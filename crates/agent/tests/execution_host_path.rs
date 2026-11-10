//! The complete approved host path with nothing faked (plan milestone M4): a client
//! `RemoteHost`, the wire protocol, a separate process of the real agent binary in its
//! explicit `--commands` mode, this platform's native process backend and the durable journal.
//!
//! The agent here runs on the same machine as the client. The Windows-to-WSL connection is
//! covered separately by `crates/core/tests/wsl_execution.rs`.
#![cfg(any(
    all(target_os = "linux", not(target_env = "uclibc")),
    target_os = "macos"
))]

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use contextual_core::commands::execution::{
    ExecutionHandle, ExecutionResult, ExecutionStatus, ManagedCommandRunner, TerminationEvidence,
};
use contextual_core::commands::journal::FileExecutionJournal;
use contextual_core::commands::{ApprovalGate, CallId, InvocationId, LaunchPermit, ReviewPlan};
use contextual_core::host::{LocalHost, WorkspaceHost};
use contextual_core::location::{Location, ProjectRoot};
use contextual_core::remote::{AgentLauncher, RemoteHost, SupervisedHost};
use contextual_protocol::commands::PrepareCommand;
use tokio::io::BufReader;
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

const AGENT: &str = env!("CARGO_BIN_EXE_contextual-agent");
const DISTRO: &str = "host-path-test";
const LONG: &str = "sleep 300 & echo started; wait";
const DEADLINE: Duration = Duration::from_secs(60);

fn location() -> Location {
    Location::Wsl {
        distro: DISTRO.into(),
    }
}

fn native_runner() -> Arc<dyn ManagedCommandRunner> {
    #[cfg(target_os = "linux")]
    let runner = contextual_core::commands::execution::linux::LinuxCommandRunner::new();
    #[cfg(target_os = "macos")]
    let runner = contextual_core::commands::execution::macos::MacosCommandRunner::new();
    Arc::new(runner)
}

struct Fixture {
    _storage: tempfile::TempDir,
    _project: tempfile::TempDir,
    journal: PathBuf,
    root: PathBuf,
}

/// One agent process and the client connection to it.
struct Agent {
    host: RemoteHost,
    process: Child,
}

async fn spawn_agent(journal: &Path, root: &Path) -> Agent {
    let mut process = Command::new(AGENT)
        .args(["--commands", "--location", &format!("wsl:{DISTRO}")])
        .arg("--journal")
        .arg(journal)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stdin = process.stdin.take().unwrap();
    let stdout = BufReader::new(process.stdout.take().unwrap());
    let host = RemoteHost::connect(stdout, stdin, "test", vec![root.to_str().unwrap().into()])
        .await
        .unwrap()
        .with_command_location(location())
        .unwrap();
    Agent { host, process }
}

impl Fixture {
    fn new() -> Self {
        let storage = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        Self {
            journal: storage.path().join("journal"),
            root: project.path().canonicalize().unwrap(),
            _storage: storage,
            _project: project,
        }
    }

    async fn agent(&self) -> Agent {
        spawn_agent(&self.journal, &self.root).await
    }

    fn root(&self) -> String {
        self.root.to_str().unwrap().to_owned()
    }

    fn ran(&self) -> bool {
        self.root.join("ran").exists()
    }

    /// The agent's own durable history, read from outside the agent.
    async fn journal(&self) -> FileExecutionJournal {
        FileExecutionJournal::open(self.journal.clone())
            .await
            .unwrap()
    }

    async fn plan(
        &self,
        host: &dyn WorkspaceHost,
        program: &str,
        args: &[&str],
        cwd: &str,
        timeout_seconds: u32,
    ) -> ReviewPlan {
        let prepared = host
            .prepare_command(PrepareCommand {
                root: self.root(),
                project_epoch: "project-1".into(),
                command_json: serde_json::json!({
                    "version": 1,
                    "purpose": "Exercise the complete host path.",
                    "command": { "kind": "exec", "program": program, "args": args },
                    "cwd": cwd,
                    "timeoutSeconds": timeout_seconds,
                    "environment": []
                })
                .to_string(),
            })
            .await
            .unwrap();
        ReviewPlan::from_prepared(
            ProjectRoot {
                location: location(),
                path: self.root(),
            },
            prepared,
        )
        .unwrap()
    }

    async fn shell(&self, host: &dyn WorkspaceHost, script: &str, timeout: u32) -> ReviewPlan {
        self.plan(host, "/bin/sh", &["-c", script], ".", timeout)
            .await
    }

    /// One freshly reviewed and approved attempt to create the marker file.
    async fn attempt(&self, host: &dyn WorkspaceHost) -> contextual_core::Result<ExecutionResult> {
        let plan = self.shell(host, ": > ran", 30).await;
        match host.start_command(permit(&plan), CancellationToken::new()) {
            Ok(handle) => Ok(handle.wait().await),
            Err(error) => Err(error),
        }
    }

    /// The process group the agent recorded for an invocation.
    async fn group(&self, invocation: &InvocationId) -> i32 {
        let path = self
            .journal
            .join(format!("{}.launched.json", invocation.as_str()));
        until(|| path.exists()).await;
        let launched: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        launched["identity"]["group"].as_i64().unwrap() as i32
    }
}

/// A consumed approval for exactly this reviewed plan.
fn permit(plan: &ReviewPlan) -> LaunchPermit {
    let mut gate = ApprovalGate::new(plan.binding.clone());
    let call = CallId {
        turn: "turn".into(),
        call: "call".into(),
    };
    gate.propose(call.clone(), plan.clone()).unwrap();
    let approval = gate.approve(&call, plan).unwrap();
    gate.begin(approval, plan).unwrap()
}

fn signal_group(group: i32, signal: &str) -> bool {
    std::process::Command::new("/bin/kill")
        .args([&format!("-{signal}"), "--", &format!("-{group}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// Whether any member of the group can still run. A zombie cannot, and an orphan can stay
/// one for as long as whatever adopted it does not reap it.
fn group_alive(group: i32) -> bool {
    let members = std::process::Command::new("ps")
        .args(["-eo", "pgid=,stat="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&members.stdout)
        .lines()
        .any(|line| {
            let mut fields = line.split_whitespace();
            fields.next() == Some(group.to_string().as_str())
                && fields.next().is_some_and(|state| !state.starts_with('Z'))
        })
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("condition was not reached in time");
}

/// Starts the long command and returns once its first output has arrived.
async fn start_long(
    fixture: &Fixture,
    host: &dyn WorkspaceHost,
    timeout: u32,
) -> (ExecutionHandle, InvocationId) {
    let plan = fixture.shell(host, LONG, timeout).await;
    let handle = host
        .start_command(permit(&plan), CancellationToken::new())
        .unwrap();
    let invocation = handle.invocation().clone();
    let mut progress = handle.progress();
    while progress.borrow_and_update().stdout.text.is_empty() {
        progress.changed().await.unwrap();
    }
    (handle, invocation)
}

#[tokio::test]
async fn an_approved_command_runs_once_through_a_real_agent_process() {
    tokio::time::timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut agent = fixture.agent().await;
        let script = "printf 'out:%s' \"$1\"; printf err >&2; : > ran; exit 3";
        let plan = fixture
            .plan(
                &agent.host,
                "/bin/sh",
                &["-c", script, "sh", "a b"],
                ".",
                30,
            )
            .await;
        assert!(!fixture.ran());

        let result = agent
            .host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::Failure, "{result:?}");
        assert_eq!(result.exit_code, Some(3));
        assert_eq!(result.output.stdout.text, "out:a b");
        assert_eq!(result.output.stderr.text, "err");
        assert!(fixture.ran());

        // Orderly shutdown: the agent answers, then exits by itself.
        agent.host.shutdown().await.unwrap();
        assert!(agent.process.wait().await.unwrap().success());
        let records = fixture.journal().await.records().await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status(), ExecutionStatus::Failure);
        assert_eq!(records[0].intent.project.location, location());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stop_and_timeout_end_the_whole_tree_and_leave_the_project_usable() {
    tokio::time::timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let agent = fixture.agent().await;

        let (handle, invocation) = start_long(&fixture, &agent.host, 300).await;
        let group = fixture.group(&invocation).await;
        assert!(group_alive(group));
        handle.stop();
        let result = handle.wait().await;
        assert_eq!(result.status, ExecutionStatus::Cancelled, "{result:?}");
        assert!(result.detail.is_none() && result.output.stdout.complete);
        assert!(!group_alive(group));

        let (handle, invocation) = start_long(&fixture, &agent.host, 1).await;
        let group = fixture.group(&invocation).await;
        let result = handle.wait().await;
        assert_eq!(result.status, ExecutionStatus::TimedOut, "{result:?}");
        assert!(!group_alive(group));

        // Both outcomes are confirmed, so nothing is blocked.
        let result = fixture.attempt(&agent.host).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
        assert!(fixture.ran());
        let records = fixture.journal().await.records().await.unwrap();
        let statuses: Vec<_> = records.iter().map(|record| record.status()).collect();
        assert_eq!(
            statuses,
            [
                ExecutionStatus::Cancelled,
                ExecutionStatus::TimedOut,
                ExecutionStatus::Success
            ]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_lost_connection_makes_the_agent_clean_up_and_only_reconciliation_releases_the_client() {
    tokio::time::timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut agent = fixture.agent().await;
        let (handle, invocation) = start_long(&fixture, &agent.host, 300).await;
        let group = fixture.group(&invocation).await;

        // The client goes away. It can no longer observe anything: outcome unknown.
        agent.host.close();
        assert_eq!(handle.wait().await.status, ExecutionStatus::Unknown);

        // The agent stops the command, records what it observed, and exits on its own.
        assert!(agent.process.wait().await.unwrap().success());
        assert!(!group_alive(group));
        let record = fixture.journal().await.load(&invocation).await.unwrap();
        assert_eq!(record.status(), ExecutionStatus::Cancelled);

        // A new agent and a fresh review do not release the client.
        let replacement = fixture.agent().await;
        assert!(fixture.attempt(&replacement.host).await.is_err());
        assert!(!fixture.ran());
        // The agent's history is resolved, and it says so only when asked.
        let report = replacement
            .host
            .reconcile_commands(&fixture.root())
            .await
            .unwrap();
        assert!(!report.blocked() && report.invocations.is_empty());
        assert!(!fixture.ran());
        let result = fixture.attempt(&replacement.host).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
        assert!(fixture.ran());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_killed_agent_leaves_the_project_blocked_until_its_command_is_gone() {
    tokio::time::timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut agent = fixture.agent().await;
        let (handle, invocation) = start_long(&fixture, &agent.host, 300).await;
        let group = fixture.group(&invocation).await;

        // The agent dies without any cleanup.
        agent.process.kill().await.unwrap();
        assert_eq!(handle.wait().await.status, ExecutionStatus::Unknown);
        assert!(group_alive(group));

        let replacement = fixture.agent().await;
        // Its command still runs: nothing is established and nothing new is admitted.
        for _ in 0..2 {
            let report = replacement
                .host
                .reconcile_commands(&fixture.root())
                .await
                .unwrap();
            assert!(report.blocked(), "{report:?}");
            assert_eq!(report.invocations[0].invocation, invocation);
            assert!(fixture.attempt(&replacement.host).await.is_err());
        }
        assert!(!fixture.ran());

        // Reconciliation never ends a process. The test does, as the user would.
        assert!(signal_group(group, "KILL"));
        until(|| !group_alive(group)).await;
        let report = replacement
            .host
            .reconcile_commands(&fixture.root())
            .await
            .unwrap();
        assert!(!report.blocked(), "{report:?}");
        assert_eq!(
            report.invocations[0].evidence,
            Some(TerminationEvidence::ProcessGroupAbsent)
        );
        assert!(!fixture.ran());

        // Only a new, separately approved attempt runs; the lost one is never replayed.
        let result = fixture.attempt(&replacement.host).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
        let journal = fixture.journal().await;
        assert_eq!(journal.records().await.unwrap().len(), 2);
        let lost = journal.load(&invocation).await.unwrap();
        assert_eq!(lost.status(), ExecutionStatus::Unknown);
        assert!(lost.completion.is_none() && lost.reconciliation.is_some());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn orderly_shutdown_stops_a_running_command_before_the_agent_exits() {
    tokio::time::timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let mut agent = fixture.agent().await;
        let (handle, invocation) = start_long(&fixture, &agent.host, 300).await;
        let group = fixture.group(&invocation).await;

        agent.host.shutdown().await.unwrap();
        assert!(agent.process.wait().await.unwrap().success());
        assert!(!group_alive(group));
        let result = handle.wait().await;
        assert!(
            matches!(
                result.status,
                ExecutionStatus::Cancelled | ExecutionStatus::Unknown
            ),
            "{result:?}"
        );
        // Whatever the client saw, the agent recorded a confirmed outcome.
        let record = fixture.journal().await.load(&invocation).await.unwrap();
        assert_eq!(record.status(), ExecutionStatus::Cancelled);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_executable_or_directory_that_changed_after_review_is_refused_before_launch() {
    tokio::time::timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let agent = fixture.agent().await;
        let host = &agent.host;

        // The reviewed executable is gone at launch.
        let tool = fixture.root.join("tool");
        std::fs::copy("/bin/sh", &tool).unwrap();
        let plan = fixture
            .plan(host, "./tool", &["-c", ": > ran"], ".", 30)
            .await;
        std::fs::remove_file(&tool).unwrap();
        let result = host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::FailedToStart, "{result:?}");
        assert!(!result.spawned);

        // The reviewed working directory now leads somewhere else.
        let reviewed = fixture.root.join("reviewed");
        let other = fixture.root.join("other");
        std::fs::create_dir(&reviewed).unwrap();
        std::fs::create_dir(&other).unwrap();
        let plan = fixture
            .plan(host, "/bin/sh", &["-c", ": > ran"], "reviewed", 30)
            .await;
        std::fs::remove_dir(&reviewed).unwrap();
        std::os::unix::fs::symlink(&other, &reviewed).unwrap();
        let result = host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::FailedToStart, "{result:?}");
        assert!(!result.spawned);

        // Nothing ran anywhere, and the refusals do not block a properly reviewed command.
        assert!(!other.join("ran").exists() && !fixture.ran());
        let result = fixture.attempt(host).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
    })
    .await
    .unwrap();
}

/// Starts agents and keeps them, so that a test can end one.
struct Launcher {
    journal: PathBuf,
    root: PathBuf,
    processes: Mutex<Vec<Child>>,
}

#[async_trait]
impl AgentLauncher for Launcher {
    async fn launch(&self) -> contextual_core::Result<RemoteHost> {
        let Agent { host, process } = spawn_agent(&self.journal, &self.root).await;
        self.processes.lock().unwrap().push(process);
        Ok(host)
    }

    fn describe(&self) -> String {
        "agent with command execution".into()
    }
}

#[tokio::test]
async fn a_dispatch_is_never_retried_on_another_agent_or_run_locally() {
    tokio::time::timeout(DEADLINE, async {
        let fixture = Fixture::new();
        let launcher = Arc::new(Launcher {
            journal: fixture.journal.clone(),
            root: fixture.root.clone(),
            processes: Mutex::default(),
        });
        let supervised = SupervisedHost::new(launcher.clone());
        supervised.allow_root(&fixture.root()).await.unwrap();
        let plan = fixture.shell(&supervised, ": > ran", 30).await;
        assert_eq!(launcher.processes.lock().unwrap().len(), 1);

        // The reviewed agent dies before the approved dispatch.
        let reviewed = supervised.connected().await.unwrap();
        let mut first = launcher.processes.lock().unwrap().pop().unwrap();
        first.kill().await.unwrap();
        until(|| !reviewed.is_alive()).await;

        // The approval is consumed without a new agent being started for it.
        assert!(
            supervised
                .start_command(permit(&plan), CancellationToken::new())
                .is_err()
        );
        assert!(launcher.processes.lock().unwrap().is_empty());

        // An in-process host with a working native backend refuses the agent's project.
        let local = LocalHost::with_command_backend(
            native_runner(),
            Arc::new(
                FileExecutionJournal::open(fixture.journal.with_extension("local"))
                    .await
                    .unwrap(),
            ),
        );
        local.allow_root(&fixture.root()).await.unwrap();
        assert!(
            local
                .start_command(permit(&plan), CancellationToken::new())
                .is_err()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!fixture.ran());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_agent_started_the_default_way_cannot_run_anything() {
    tokio::time::timeout(DEADLINE, async {
        let fixture = Fixture::new();
        // Exactly how the app starts it unless the development switch is on.
        let mut process = Command::new(AGENT)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = process.stdin.take().unwrap();
        let stdout = BufReader::new(process.stdout.take().unwrap());
        let host = RemoteHost::connect(stdout, stdin, "test", vec![fixture.root()])
            .await
            .unwrap()
            .with_command_location(location())
            .unwrap();

        // It still prepares (inspection only), but offers no execution and no reconciliation.
        let plan = fixture.shell(&host, ": > ran", 30).await;
        assert!(
            host.start_command(permit(&plan), CancellationToken::new())
                .is_err()
        );
        assert!(host.reconcile_commands(&fixture.root()).await.is_err());
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!fixture.ran());
        assert!(!fixture.journal.exists());
    })
    .await
    .unwrap();
}
