//! Fake process trees over the executable agent connection, with a real durable journal.
//! These do not verify native process creation, WSL deployment, or power-loss durability.
#![cfg(unix)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use contextual_agent::serve_with_execution;
use contextual_core::Result;
use contextual_core::commands::execution::{
    Containment, ExecutionStatus, ManagedChild, ManagedCommandRunner, ManagedProcessTree,
    OUTPUT_LIMIT, ProcessExit, TerminationEvidence,
};
use contextual_core::commands::journal::FileExecutionJournal;
use contextual_core::commands::service::AgentExecutionService;
use contextual_core::commands::wire::{ObservationReceiver, encode_dispatch};
use contextual_core::commands::{ApprovalGate, CallId, LaunchPermit, ReviewPlan};
use contextual_core::host::{LocalHost, WorkspaceHost};
use contextual_core::location::{Location, ProjectRoot};
use contextual_core::process::Cmd;
use contextual_core::remote::RemoteHost;
use contextual_protocol::commands::{
    CancelCommand, CommandExecutionIdentity, CommandPlatform, DispatchCommand, EXECUTION_VERSION,
    PrepareCommand,
};
use contextual_protocol::rpc::{Request, Response, method};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Control {
    launches: AtomicUsize,
    cleaned: AtomicUsize,
    spawned: Notify,
    cleanup_entered: Notify,
    root_exit: CancellationToken,
    cleanup_release: CancellationToken,
    /// Cleanup reports that termination could not be established.
    fail_cleanup: AtomicBool,
    /// The fake tree counts as ended once its owner lets go of the project.
    owner_bound: AtomicBool,
}

struct Runner(Arc<Control>);

impl ManagedCommandRunner for Runner {
    fn platform(&self) -> CommandPlatform {
        if cfg!(target_os = "macos") {
            CommandPlatform::Macos
        } else {
            CommandPlatform::Linux
        }
    }

    fn containment(&self) -> Containment {
        if self.0.owner_bound.load(Ordering::SeqCst) {
            Containment::OwnerBound
        } else {
            Containment::Unverifiable
        }
    }

    fn spawn(&self, command: Cmd) -> Result<ManagedChild> {
        assert_eq!(command.args, ["a b", "", "$HOME", "&"]);
        assert!(command.cwd.is_some());
        assert!(
            !command
                .env
                .iter()
                .any(|(name, _)| name == "AZURE_OPENAI_API_KEY")
        );
        self.0.launches.fetch_add(1, Ordering::SeqCst);
        self.0.spawned.notify_one();
        Ok(ManagedChild {
            tree: Box::new(Tree(self.0.clone())),
            stdout: Box::new(std::io::Cursor::new(vec![b'x'; OUTPUT_LIMIT * 2])),
            stderr: Box::new(std::io::Cursor::new(b"stderr\x1b[31m".to_vec())),
        })
    }
}

struct Tree(Arc<Control>);

#[async_trait]
impl ManagedProcessTree for Tree {
    async fn wait_root(&mut self) -> Result<ProcessExit> {
        self.0.root_exit.cancelled().await;
        Ok(ProcessExit {
            success: true,
            code: Some(0),
        })
    }

    async fn terminate_and_wait(&mut self) -> Result<ProcessExit> {
        self.0.cleanup_entered.notify_one();
        if self.0.fail_cleanup.load(Ordering::SeqCst) {
            return Err(contextual_core::Error::tool("test cleanup failure"));
        }
        self.0.cleanup_release.cancelled().await;
        self.0.cleaned.fetch_add(1, Ordering::SeqCst);
        Ok(ProcessExit {
            success: false,
            code: None,
        })
    }
}

struct Fixture {
    _storage: tempfile::TempDir,
    project: tempfile::TempDir,
    program: std::path::PathBuf,
    journal: Arc<FileExecutionJournal>,
    control: Arc<Control>,
    service: Arc<AgentExecutionService>,
}

fn location() -> Location {
    Location::Wsl {
        distro: "connection-test".into(),
    }
}

impl Fixture {
    async fn new() -> Self {
        use std::os::unix::fs::PermissionsExt as _;

        let storage = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let program = project.path().join("fixture-tool");
        std::fs::write(&program, "never executed by these tests\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let journal = Arc::new(
            FileExecutionJournal::open(storage.path().join("journal"))
                .await
                .unwrap(),
        );
        let control = Arc::new(Control::default());
        let host = Arc::new(LocalHost::new());
        host.allow_root(project.path().to_str().unwrap())
            .await
            .unwrap();
        let service = Arc::new(AgentExecutionService::new(
            host,
            location(),
            Arc::new(Runner(control.clone())),
            journal.clone(),
        ));
        Self {
            _storage: storage,
            project,
            program,
            journal,
            control,
            service,
        }
    }

    fn request(&self) -> PrepareCommand {
        PrepareCommand {
            root: self.project.path().to_str().unwrap().into(),
            project_epoch: "project-1".into(),
            command_json: serde_json::json!({
                "version": 1,
                "purpose": "Exercise the fake process connection.",
                "command": {
                    "kind": "exec",
                    "program": self.program.to_str().unwrap(),
                    "args": ["a b", "", "$HOME", "&"]
                },
                "cwd": ".",
                "timeoutSeconds": 30,
                "environment": []
            })
            .to_string(),
        }
    }

    async fn connect(&self) -> (RemoteHost, tokio::task::JoinHandle<std::io::Result<()>>) {
        let (client, server) = tokio::io::duplex(4096);
        let (server_read, server_write) = tokio::io::split(server);
        let service = self.service.clone();
        let task = tokio::spawn(async move {
            serve_with_execution(BufReader::new(server_read), server_write, service).await
        });
        let (client_read, client_write) = tokio::io::split(client);
        let host = RemoteHost::connect(
            BufReader::new(client_read),
            client_write,
            "test",
            vec![self.request().root],
        )
        .await
        .unwrap()
        .with_command_location(location())
        .unwrap();
        (host, task)
    }

    async fn plan(&self, host: &RemoteHost) -> ReviewPlan {
        ReviewPlan::from_prepared(
            ProjectRoot {
                location: location(),
                path: self.request().root,
            },
            host.prepare_command(self.request()).await.unwrap(),
        )
        .unwrap()
    }
}

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
async fn approved_remote_dispatch_delivers_bounded_cumulative_output() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let (host, server) = fixture.connect().await;
        let plan = fixture.plan(&host).await;
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 0);
        let handle = host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap();
        fixture.control.spawned.notified().await;
        let mut progress = handle.progress();
        while progress.borrow().stdout.bytes_seen < (OUTPUT_LIMIT * 2) as u64 {
            progress.changed().await.unwrap();
        }
        assert_eq!(progress.borrow().stdout.text.len(), OUTPUT_LIMIT);
        assert!(progress.borrow().stdout.truncated);
        assert!(
            host.start_command(permit(&plan), CancellationToken::new())
                .is_err(),
            "independent gates must not queue work"
        );

        fixture.control.cleanup_release.cancel();
        fixture.control.root_exit.cancel();
        let result = handle.wait().await;
        assert_eq!(result.status, ExecutionStatus::Success);
        assert!(result.output.stderr.text.contains("\\u{1b}[31m"));
        assert_eq!(result.output.stdout.bytes_seen, (OUTPUT_LIMIT * 2) as u64);
        assert!(result.history_error.is_none());
        assert_eq!(fixture.journal.records().await.unwrap().len(), 1);
        host.close();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stop_waits_for_cleanup_instead_of_aborting_the_dispatch_rpc() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let (host, server) = fixture.connect().await;
        let plan = fixture.plan(&host).await;
        let handle = host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap();
        fixture.control.spawned.notified().await;
        handle.stop();
        let waiter = tokio::spawn(handle.wait());
        fixture.control.cleanup_entered.notified().await;
        assert!(!waiter.is_finished());
        assert_eq!(fixture.control.cleaned.load(Ordering::SeqCst), 0);
        fixture.control.cleanup_release.cancel();

        let result = waiter.await.unwrap();
        assert_eq!(result.status, ExecutionStatus::Cancelled);
        assert!(result.cancelled && result.spawned);
        assert!(result.history_error.is_none());
        assert_eq!(fixture.control.cleaned.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 1);
        host.close();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stale_host_preparation_is_refused_on_the_agent_before_spawn() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let (host, server) = fixture.connect().await;
        let plan = fixture.plan(&host).await;
        let permit = permit(&plan);
        std::fs::remove_file(&fixture.program).unwrap();

        let result = host
            .start_command(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert!(!result.spawned);
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.journal.records().await.unwrap().len(), 1);
        host.close();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn replacement_connection_cannot_dispatch_an_old_review() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let (old, old_server) = fixture.connect().await;
        let plan = fixture.plan(&old).await;
        let permit = permit(&plan);
        old.close();
        old_server.await.unwrap().unwrap();

        let (replacement, server) = fixture.connect().await;
        // Even when a service happens to survive reconnection, the app must explicitly
        // prepare/review on the new connection. It cannot transplant its old cached review.
        assert!(
            replacement
                .start_command(permit, CancellationToken::new())
                .is_err()
        );
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 0);
        replacement.close();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn disconnected_outcome_stays_blocked_after_reconnection_and_agent_cleanup() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let (host, server) = fixture.connect().await;
        let plan = fixture.plan(&host).await;
        let handle = host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap();
        fixture.control.spawned.notified().await;
        host.close();
        let result = handle.wait().await;
        assert_eq!(result.status, ExecutionStatus::Unknown);
        fixture.control.cleanup_entered.notified().await;
        assert!(!server.is_finished());
        fixture.control.cleanup_release.cancel();
        server.await.unwrap().unwrap();

        let (replacement, server) = fixture.connect().await;
        let fresh = fixture.plan(&replacement).await;
        assert!(
            replacement
                .start_command(permit(&fresh), CancellationToken::new())
                .is_err(),
            "reconnection is not reconciliation"
        );
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 1);
        replacement.close();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn explicit_reconciliation_releases_a_disconnected_outcome_the_agent_resolved() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let root = fixture.request().root;
        let (host, server) = fixture.connect().await;
        let plan = fixture.plan(&host).await;
        let handle = host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap();
        fixture.control.spawned.notified().await;

        // While the agent's worker still owns the project, nothing can be reconciled.
        assert!(host.reconcile_commands(&root).await.is_err());

        host.close();
        assert_eq!(handle.wait().await.status, ExecutionStatus::Unknown);
        fixture.control.cleanup_entered.notified().await;
        fixture.control.cleanup_release.cancel();
        server.await.unwrap().unwrap();

        // Reconnecting and reviewing again is not reconciliation.
        let (replacement, server) = fixture.connect().await;
        let fresh = fixture.plan(&replacement).await;
        assert!(
            replacement
                .start_command(permit(&fresh), CancellationToken::new())
                .is_err()
        );

        // The agent recorded a confirmed outcome, so it reports nothing unresolved. Only
        // that explicit answer releases this client, and it starts nothing by itself.
        let report = replacement.reconcile_commands(&root).await.unwrap();
        assert!(!report.blocked() && report.invocations.is_empty());
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 1);

        // A new, separately approved attempt now runs.
        let fresh = fixture.plan(&replacement).await;
        let handle = replacement
            .start_command(permit(&fresh), CancellationToken::new())
            .unwrap();
        fixture.control.root_exit.cancel();
        assert_eq!(handle.wait().await.status, ExecutionStatus::Success);
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 2);
        assert_eq!(fixture.journal.records().await.unwrap().len(), 2);
        replacement.close();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_unknown_agent_outcome_stays_blocked_through_reconnection_and_reconciliation() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let root = fixture.request().root;
        fixture.control.fail_cleanup.store(true, Ordering::SeqCst);
        let (host, server) = fixture.connect().await;
        let plan = fixture.plan(&host).await;
        let handle = host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap();
        fixture.control.spawned.notified().await;
        fixture.control.root_exit.cancel();
        assert_eq!(handle.wait().await.status, ExecutionStatus::Unknown);
        host.close();
        server.await.unwrap().unwrap();

        for _ in 0..2 {
            let (replacement, server) = fixture.connect().await;
            // The fake backend's containment cannot be checked: nothing is established.
            let report = replacement.reconcile_commands(&root).await.unwrap();
            assert!(report.blocked());
            assert_eq!(report.invocations.len(), 1);
            assert!(report.invocations[0].detail.is_some());
            let fresh = fixture.plan(&replacement).await;
            assert!(
                replacement
                    .start_command(permit(&fresh), CancellationToken::new())
                    .is_err()
            );
            replacement.close();
            server.await.unwrap().unwrap();
        }
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_unknown_agent_outcome_is_released_once_the_agent_establishes_termination() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let root = fixture.request().root;
        fixture.control.fail_cleanup.store(true, Ordering::SeqCst);
        fixture.control.owner_bound.store(true, Ordering::SeqCst);
        let (host, server) = fixture.connect().await;
        let plan = fixture.plan(&host).await;
        let handle = host
            .start_command(permit(&plan), CancellationToken::new())
            .unwrap();
        fixture.control.spawned.notified().await;
        fixture.control.root_exit.cancel();
        let unknown = handle.wait().await;
        assert_eq!(unknown.status, ExecutionStatus::Unknown);
        host.close();
        server.await.unwrap().unwrap();

        let (replacement, server) = fixture.connect().await;
        let fresh = fixture.plan(&replacement).await;
        assert!(
            replacement
                .start_command(permit(&fresh), CancellationToken::new())
                .is_err()
        );
        let report = replacement.reconcile_commands(&root).await.unwrap();
        assert!(!report.blocked());
        assert_eq!(report.invocations.len(), 1);
        assert_eq!(
            report.invocations[0].evidence,
            Some(TerminationEvidence::OwnerBoundTreeReleased)
        );
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 1);

        fixture.control.fail_cleanup.store(false, Ordering::SeqCst);
        fixture.control.cleanup_release.cancel();
        let fresh = fixture.plan(&replacement).await;
        let handle = replacement
            .start_command(permit(&fresh), CancellationToken::new())
            .unwrap();
        assert_eq!(handle.wait().await.status, ExecutionStatus::Success);
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 2);

        // The first invocation keeps its unknown outcome in the agent's history.
        let records = fixture.journal.records().await.unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(
            records
                .iter()
                .filter(|record| record.status() == ExecutionStatus::Unknown
                    && record.reconciliation.is_some())
                .count(),
            1
        );
        replacement.close();
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}

async fn send(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    id: u64,
    method: &str,
    params: Value,
) {
    let mut bytes = serde_json::to_vec(&Request::new(id, method, params)).unwrap();
    bytes.push(b'\n');
    writer.write_all(&bytes).await.unwrap();
}

async fn response(
    lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
) -> Response {
    loop {
        let line = lines.next_line().await.unwrap().unwrap();
        if let Ok(response) = serde_json::from_str::<Response>(&line) {
            return response;
        }
    }
}

#[tokio::test]
async fn duplicate_invocations_are_refused_while_active_and_after_journal_reopen() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = Fixture::new().await;
        let prepared = fixture.service.prepare(fixture.request()).await.unwrap();
        let plan = ReviewPlan::from_prepared(
            ProjectRoot {
                location: location(),
                path: fixture.request().root,
            },
            prepared,
        )
        .unwrap();
        let bytes = encode_dispatch(permit(&plan)).unwrap();
        let envelope: DispatchCommand = serde_json::from_slice(&bytes).unwrap();
        let identity: CommandExecutionIdentity = envelope.execution.clone();
        let params: Value = serde_json::from_slice(&bytes).unwrap();

        let (client, server) = tokio::io::duplex(4096);
        let (server_read, server_write) = tokio::io::split(server);
        let service = fixture.service.clone();
        let server = tokio::spawn(async move {
            serve_with_execution(BufReader::new(server_read), server_write, service).await
        });
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut lines = BufReader::new(client_read).lines();
        send(
            &mut client_write,
            1,
            method::COMMAND_DISPATCH,
            params.clone(),
        )
        .await;
        fixture.control.spawned.notified().await;

        // Same active RPC ID is silent; another RPC ID with the same invocation is refused.
        send(
            &mut client_write,
            1,
            method::COMMAND_DISPATCH,
            params.clone(),
        )
        .await;
        send(
            &mut client_write,
            2,
            method::COMMAND_DISPATCH,
            params.clone(),
        )
        .await;
        let duplicate = response(&mut lines).await;
        assert_eq!(duplicate.id, 2);
        assert!(duplicate.error.is_some());
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 1);

        send(
            &mut client_write,
            3,
            method::COMMAND_CANCEL,
            serde_json::to_value(CancelCommand {
                version: EXECUTION_VERSION,
                execution: identity.clone(),
            })
            .unwrap(),
        )
        .await;
        fixture.control.cleanup_entered.notified().await;
        fixture.control.cleanup_release.cancel();
        let first = response(&mut lines).await;
        let second = response(&mut lines).await;
        assert!(matches!((first.id, second.id), (1, 3) | (3, 1)));
        assert_eq!(first.result, second.result);
        let mut receiver = ObservationReceiver::new(identity).unwrap();
        let final_bytes = serde_json::to_vec(&first.result.unwrap()).unwrap();
        assert_eq!(
            receiver.finish(&final_bytes).unwrap().status,
            ExecutionStatus::Cancelled
        );

        send(&mut client_write, 4, method::COMMAND_DISPATCH, params).await;
        assert!(response(&mut lines).await.error.is_some());
        drop(client_write);
        drop(lines);
        server.await.unwrap().unwrap();

        // A new service with a new host epoch cannot evade the immutable invocation intent.
        let journal = Arc::new(
            FileExecutionJournal::open(fixture.journal.dir().to_path_buf())
                .await
                .unwrap(),
        );
        let native = Arc::new(LocalHost::new());
        native.allow_root(&fixture.request().root).await.unwrap();
        let service = Arc::new(AgentExecutionService::new(
            native,
            location(),
            Arc::new(Runner(fixture.control.clone())),
            journal,
        ));
        let fresh = service.prepare(fixture.request()).await.unwrap();
        let fresh_plan = ReviewPlan::from_prepared(
            ProjectRoot {
                location: location(),
                path: fixture.request().root,
            },
            fresh,
        )
        .unwrap();
        let mut replay: DispatchCommand =
            serde_json::from_slice(&encode_dispatch(permit(&fresh_plan)).unwrap()).unwrap();
        replay.execution.invocation_id = envelope.execution.invocation_id;
        let (client, server) = tokio::io::duplex(4096);
        let (server_read, server_write) = tokio::io::split(server);
        let server = tokio::spawn(async move {
            serve_with_execution(BufReader::new(server_read), server_write, service).await
        });
        let (client_read, mut client_write) = tokio::io::split(client);
        let mut lines = BufReader::new(client_read).lines();
        send(
            &mut client_write,
            5,
            method::COMMAND_DISPATCH,
            serde_json::to_value(&replay).unwrap(),
        )
        .await;
        let final_response = response(&mut lines).await;
        let mut receiver = ObservationReceiver::new(replay.execution).unwrap();
        let result = receiver
            .finish(&serde_json::to_vec(&final_response.result.unwrap()).unwrap())
            .unwrap();
        assert_eq!(result.status, ExecutionStatus::FailedToStart);
        assert!(result.history_error.is_some());
        assert_eq!(fixture.control.launches.load(Ordering::SeqCst), 1);
        drop(client_write);
        drop(lines);
        server.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}
