//! The actual Windows-to-WSL host path (plan milestone M4): this process starts the real
//! agent, in its explicit `--commands` mode, inside a real WSL distro through `wsl.exe`, and
//! drives approved commands over that connection with nothing faked.
//!
//! Built for Windows, the client is the one the Windows app uses. Built for Linux and run
//! inside WSL, the same tests reach `wsl.exe` through WSL interop: that still exercises the
//! real `wsl.exe` relay and the agent in the distro, but with the Linux build of the client.
//! The second form exists because Windows application control can refuse to run a locally
//! built, unsigned test binary.
//!
//! It needs a distro and a Linux build of the agent, so it only runs when told where they
//! are; otherwise each test reports that it was skipped:
//!
//! - `CONTEXTUAL_WSL_DISTRO`: the distro name, e.g. `Ubuntu`.
//! - `CONTEXTUAL_WSL_AGENT`: the path, inside that distro, of `contextual-agent`.
//!
//! From the Linux development host inside WSL:
//!
//! ```text
//! cargo build -p contextual-agent
//! cargo test -p contextual-core --no-default-features --target x86_64-pc-windows-gnu \
//!     --test wsl_execution --no-run
//! WSLENV=CONTEXTUAL_WSL_DISTRO:CONTEXTUAL_WSL_AGENT CONTEXTUAL_WSL_DISTRO=$WSL_DISTRO_NAME \
//!     CONTEXTUAL_WSL_AGENT=$PWD/target/debug/contextual-agent \
//!     <printed .exe> --test-threads=1 --nocapture
//! ```
//!
//! With the Linux build of the client, through the same `wsl.exe`:
//!
//! ```text
//! CONTEXTUAL_WSL_DISTRO=$WSL_DISTRO_NAME \
//!     CONTEXTUAL_WSL_AGENT=$PWD/target/debug/contextual-agent \
//!     cargo test -p contextual-core --no-default-features --test wsl_execution -- \
//!     --test-threads=1 --nocapture
//! ```
#![cfg(any(windows, target_os = "linux"))]

use std::process::Stdio;
use std::time::Duration;

use contextual_core::commands::execution::{
    ExecutionHandle, ExecutionResult, ExecutionStatus, TerminationEvidence,
};
use contextual_core::commands::{ApprovalGate, CallId, InvocationId, LaunchPermit, ReviewPlan};
use contextual_core::host::WorkspaceHost;
use contextual_core::location::{Location, ProjectRoot};
use contextual_core::remote::RemoteHost;
use contextual_core::wsl::decode_wsl_output;
use contextual_protocol::commands::{CommandPlatform, PrepareCommand};
use tokio::io::BufReader;
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const LONG: &str = "sleep 300 & echo started; wait";
const DEADLINE: Duration = Duration::from_secs(120);

struct Fixture {
    distro: String,
    program: String,
    /// Linux paths inside the distro.
    journal: String,
    root: String,
}

struct Agent {
    host: RemoteHost,
    /// The `wsl.exe` process that carries the connection.
    relay: Child,
}

fn wsl_command(distro: &str) -> Command {
    let mut command = Command::new("wsl.exe");
    command.args(["-d", distro, "-e"]);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// Runs a shell script inside the distro and returns its output.
async fn wsl(distro: &str, script: &str) -> String {
    let output = wsl_command(distro)
        .args(["/bin/sh", "-c", script])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    decode_wsl_output(&output.stdout).trim().to_owned()
}

impl Fixture {
    /// None when this machine was not set up for the test.
    async fn new() -> Option<Self> {
        let (Ok(distro), Ok(program)) = (
            std::env::var("CONTEXTUAL_WSL_DISTRO"),
            std::env::var("CONTEXTUAL_WSL_AGENT"),
        ) else {
            eprintln!("skipped: CONTEXTUAL_WSL_DISTRO and CONTEXTUAL_WSL_AGENT are not set");
            return None;
        };
        let base = wsl(&distro, "mktemp -d").await;
        assert!(
            base.starts_with('/'),
            "could not reach the distro: {base:?}"
        );
        let root = wsl(
            &distro,
            &format!("mkdir '{base}/project' && cd '{base}/project' && pwd -P"),
        )
        .await;
        Some(Self {
            journal: format!("{base}/journal"),
            root,
            distro,
            program,
        })
    }

    fn location(&self) -> Location {
        Location::Wsl {
            distro: self.distro.clone(),
        }
    }

    async fn agent(&self) -> Agent {
        let mut relay = wsl_command(&self.distro)
            .args([
                self.program.as_str(),
                "--commands",
                "--location",
                &format!("wsl:{}", self.distro),
                "--journal",
                &self.journal,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = relay.stdin.take().unwrap();
        let stdout = BufReader::new(relay.stdout.take().unwrap());
        let host = RemoteHost::connect(stdout, stdin, "test", vec![self.root.clone()])
            .await
            .unwrap()
            .with_command_location(self.location())
            .unwrap();
        Agent { host, relay }
    }

    async fn plan(&self, host: &RemoteHost, args: &[&str], timeout_seconds: u32) -> ReviewPlan {
        let prepared = host
            .prepare_command(PrepareCommand {
                root: self.root.clone(),
                project_epoch: "project-1".into(),
                command_json: serde_json::json!({
                    "version": 1,
                    "purpose": "Exercise the Windows-to-WSL host path.",
                    "command": { "kind": "exec", "program": "/bin/sh", "args": args },
                    "cwd": ".",
                    "timeoutSeconds": timeout_seconds,
                    "environment": []
                })
                .to_string(),
            })
            .await
            .unwrap();
        ReviewPlan::from_prepared(
            ProjectRoot {
                location: self.location(),
                path: self.root.clone(),
            },
            prepared,
        )
        .unwrap()
    }

    /// One freshly reviewed and approved attempt to create the marker file.
    async fn attempt(&self, host: &RemoteHost) -> contextual_core::Result<ExecutionResult> {
        let plan = self.plan(host, &["-c", ": > ran"], 30).await;
        match host.start_command(permit(&plan), CancellationToken::new()) {
            Ok(handle) => Ok(handle.wait().await),
            Err(error) => Err(error),
        }
    }

    async fn ran(&self) -> bool {
        wsl(
            &self.distro,
            &format!("test -e '{}/ran' && echo yes", self.root),
        )
        .await
            == "yes"
    }

    /// Starts the long command and returns once its first output has crossed into Windows.
    async fn start_long(&self, host: &RemoteHost, timeout: u32) -> (ExecutionHandle, InvocationId) {
        let plan = self.plan(host, &["-c", LONG], timeout).await;
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

    /// The Linux process group that the agent recorded for an invocation.
    async fn group(&self, invocation: &InvocationId) -> i32 {
        let path = format!("{}/{}.launched.json", self.journal, invocation.as_str());
        for _ in 0..100 {
            let text = wsl(&self.distro, &format!("cat '{path}' 2>/dev/null")).await;
            if let Ok(launched) = serde_json::from_str::<serde_json::Value>(&text) {
                return launched["identity"]["group"].as_i64().unwrap() as i32;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the agent never recorded a process group");
    }

    /// Whether any member of the group can still run. A zombie cannot, and inside WSL an
    /// orphan may stay a zombie indefinitely: its adopting session `init` does not reap it.
    async fn group_alive(&self, group: i32) -> bool {
        wsl(
            &self.distro,
            &format!(
                "ps -eo pgid=,stat= | awk '$1 == {group} && $2 !~ /^Z/ {{ live = 1 }} \
                 END {{ if (live) print \"alive\" }}'"
            ),
        )
        .await
            == "alive"
    }

    async fn until_group_gone(&self, group: i32) {
        for _ in 0..100 {
            if !self.group_alive(group).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the process group never went away");
    }

    async fn status_on_record(&self, invocation: &InvocationId) -> String {
        let path = format!("{}/{}.completion.json", self.journal, invocation.as_str());
        let text = wsl(&self.distro, &format!("cat '{path}' 2>/dev/null")).await;
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|record| record["completion"]["status"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "none".into())
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

#[tokio::test]
async fn an_approved_command_runs_inside_wsl_and_reports_back_to_windows() {
    tokio::time::timeout(DEADLINE, async {
        let Some(fixture) = Fixture::new().await else {
            return;
        };
        let mut agent = fixture.agent().await;
        let script = "printf 'out:%s' \"$1\"; uname -s >&2; : > ran; exit 3";
        let plan = fixture
            .plan(&agent.host, &["-c", script, "sh", "a b"], 30)
            .await;
        // Prepared and resolved by the Linux host, not by Windows.
        assert_eq!(plan.platform, CommandPlatform::Linux);
        assert!(!fixture.ran().await);

        let result = agent
            .host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::Failure, "{result:?}");
        assert_eq!(result.exit_code, Some(3));
        assert_eq!(result.output.stdout.text, "out:a b");
        assert_eq!(result.output.stderr.text, "Linux\n");
        assert!(fixture.ran().await);

        // Orderly shutdown: the agent answers and the relay ends by itself.
        agent.host.shutdown().await.unwrap();
        assert!(agent.relay.wait().await.unwrap().success());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stop_and_timeout_from_windows_end_the_tree_inside_wsl() {
    tokio::time::timeout(DEADLINE, async {
        let Some(fixture) = Fixture::new().await else {
            return;
        };
        let agent = fixture.agent().await;

        let (handle, invocation) = fixture.start_long(&agent.host, 300).await;
        let group = fixture.group(&invocation).await;
        assert!(fixture.group_alive(group).await);
        handle.stop();
        let result = handle.wait().await;
        assert_eq!(result.status, ExecutionStatus::Cancelled, "{result:?}");
        assert!(result.detail.is_none() && result.output.stdout.complete);
        assert!(!fixture.group_alive(group).await);

        let (handle, invocation) = fixture.start_long(&agent.host, 1).await;
        let group = fixture.group(&invocation).await;
        let result = handle.wait().await;
        assert_eq!(result.status, ExecutionStatus::TimedOut, "{result:?}");
        assert!(!fixture.group_alive(group).await);

        let result = fixture.attempt(&agent.host).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
        assert!(fixture.ran().await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_lost_connection_is_cleaned_up_inside_wsl_and_released_only_by_reconciliation() {
    tokio::time::timeout(DEADLINE, async {
        let Some(fixture) = Fixture::new().await else {
            return;
        };
        let mut agent = fixture.agent().await;
        let (handle, invocation) = fixture.start_long(&agent.host, 300).await;
        let group = fixture.group(&invocation).await;

        agent.host.close();
        assert_eq!(handle.wait().await.status, ExecutionStatus::Unknown);
        // The agent stops the command and exits; the relay ends with it.
        let _ = agent.relay.wait().await.unwrap();
        fixture.until_group_gone(group).await;
        assert_eq!(fixture.status_on_record(&invocation).await, "cancelled");

        let replacement = fixture.agent().await;
        assert!(fixture.attempt(&replacement.host).await.is_err());
        assert!(!fixture.ran().await);
        let report = replacement
            .host
            .reconcile_commands(&fixture.root)
            .await
            .unwrap();
        assert!(!report.blocked() && report.invocations.is_empty());
        let result = fixture.attempt(&replacement.host).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
        assert!(fixture.ran().await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_agent_killed_inside_wsl_blocks_the_project_until_its_command_is_gone() {
    tokio::time::timeout(DEADLINE, async {
        let Some(fixture) = Fixture::new().await else {
            return;
        };
        let agent = fixture.agent().await;
        let (handle, invocation) = fixture.start_long(&agent.host, 300).await;
        let group = fixture.group(&invocation).await;

        // The agent process dies inside the distro without any cleanup. The journal path is
        // unique to this test, so it identifies exactly this agent.
        wsl(
            &fixture.distro,
            &format!("pkill -KILL -f -- '--journal {}'", fixture.journal),
        )
        .await;
        assert_eq!(handle.wait().await.status, ExecutionStatus::Unknown);
        assert!(fixture.group_alive(group).await);
        assert_eq!(fixture.status_on_record(&invocation).await, "none");

        let replacement = fixture.agent().await;
        let report = replacement
            .host
            .reconcile_commands(&fixture.root)
            .await
            .unwrap();
        assert!(report.blocked(), "{report:?}");
        assert!(fixture.attempt(&replacement.host).await.is_err());
        assert!(!fixture.ran().await);

        // Reconciliation never ends a process. The test does, as the user would.
        wsl(&fixture.distro, &format!("/bin/kill -KILL -- -{group}")).await;
        fixture.until_group_gone(group).await;
        let report = replacement
            .host
            .reconcile_commands(&fixture.root)
            .await
            .unwrap();
        assert!(!report.blocked(), "{report:?}");
        assert_eq!(
            report.invocations[0].evidence,
            Some(TerminationEvidence::ProcessGroupAbsent)
        );
        assert!(!fixture.ran().await);

        let result = fixture.attempt(&replacement.host).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
        assert!(fixture.ran().await);
    })
    .await
    .unwrap();
}

/// What happens to a running command when the Windows side of the connection is ended
/// abruptly, as when the app is killed. Either the agent notices and cleans up, or it is
/// taken down with its command still running; the project must end up usable only after
/// termination is established.
#[tokio::test]
async fn a_killed_relay_never_leaves_the_project_usable_while_its_command_runs() {
    tokio::time::timeout(DEADLINE, async {
        let Some(fixture) = Fixture::new().await else {
            return;
        };
        let mut agent = fixture.agent().await;
        let (handle, invocation) = fixture.start_long(&agent.host, 300).await;
        let group = fixture.group(&invocation).await;

        agent.relay.kill().await.unwrap();
        assert_eq!(handle.wait().await.status, ExecutionStatus::Unknown);

        // Give the agent time to notice and clean up, if it is going to.
        let mut cleaned_up = false;
        for _ in 0..50 {
            if !fixture.group_alive(group).await {
                cleaned_up = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        eprintln!(
            "after killing wsl.exe: command {}; agent recorded {:?}",
            if cleaned_up { "ended" } else { "still running" },
            fixture.status_on_record(&invocation).await
        );

        let replacement = fixture.agent().await;
        assert!(fixture.attempt(&replacement.host).await.is_err());
        if !cleaned_up {
            // Either the old agent still supervises it (reconciliation is refused), or the
            // agent is gone and the group is observed alive (reconciliation stays blocked).
            let report = replacement.host.reconcile_commands(&fixture.root).await;
            assert!(report.map_or(true, |report| report.blocked()));
            assert!(fixture.attempt(&replacement.host).await.is_err());
            wsl(&fixture.distro, &format!("/bin/kill -KILL -- -{group}")).await;
            fixture.until_group_gone(group).await;
        }
        assert!(!fixture.ran().await);

        // Once the command is gone, an explicit reconciliation releases the project. A
        // surviving agent may need a moment to finish recording.
        let mut released = false;
        for _ in 0..50 {
            if let Ok(report) = replacement.host.reconcile_commands(&fixture.root).await
                && !report.blocked()
            {
                released = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(released);
        let result = fixture.attempt(&replacement.host).await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
        assert!(fixture.ran().await);
    })
    .await
    .unwrap();
}
