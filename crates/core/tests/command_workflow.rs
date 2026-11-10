#![cfg(all(
    feature = "llm",
    any(
        all(target_os = "linux", not(target_env = "uclibc")),
        target_os = "macos",
        windows
    )
))]
//! The whole command workflow below the UI (plan milestone M6, part 1), with a scripted model
//! and a real process: the model proposes, the host prepares, the user reviews and approves,
//! the command runs, its result is observed, and only what the user shares goes back.
//!
//! The steps a person takes are explicit calls here (`approve`, `reject`, the share choice).
//! Nothing in between can take them: a proposal is recorded without being prepared, a plan
//! is prepared without being approved, and a restored chat holds no approval at all.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use contextual_core::chat::{SavedThread, ThreadState, history, tool_results};
use contextual_core::commands::desk::CommandDesk;
use contextual_core::commands::execution::{
    ExecutionResult, ExecutionStatus, ManagedCommandRunner,
};
use contextual_core::commands::history::{Answer, RecordStatus, ShareChoice, SharedResult};
use contextual_core::commands::journal::FileExecutionJournal;
use contextual_core::commands::{CallId, CommandState, ReviewPlan};
use contextual_core::host::{LocalHost, WorkspaceHost};
use contextual_core::llm::{
    ChatEvent, ConversationRequest, FoundryClient, FoundryConfig, MessageContent, PROPOSE_COMMAND,
    ToolCall,
};
use contextual_core::location::{Location, ProjectRoot};
use contextual_core::logic::prompt::ApiContent;
use contextual_protocol::commands::PrepareCommand;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const NOW: &str = "2026-11-10T12:00:00Z";

// ----- A scripted model -----

#[derive(Clone, Default)]
struct Script {
    replies: Arc<Mutex<VecDeque<ResponseTemplate>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Respond for Script {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.requests
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&request.body).unwrap());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("an unexpected request reached the model")
    }
}

fn sse(events: &[Value]) -> ResponseTemplate {
    let body: String = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect();
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn completed(id: &str, output: Vec<Value>) -> Value {
    json!({ "type": "response.completed", "response": { "id": id, "output": output } })
}

/// A reply that proposes these commands and says nothing.
fn proposing(id: &str, calls: &[(&str, &Value)]) -> ResponseTemplate {
    let items = calls
        .iter()
        .enumerate()
        .map(|(index, (call_id, arguments))| {
            json!({
                "id": format!("fc_{index}"), "type": "function_call", "call_id": call_id,
                "name": PROPOSE_COMMAND, "arguments": arguments.to_string()
            })
        })
        .collect();
    sse(&[completed(id, items)])
}

fn saying(id: &str, text: &str) -> ResponseTemplate {
    sse(&[
        json!({ "type": "response.output_text.delta", "delta": text }),
        completed(id, vec![]),
    ])
}

// ----- A project on this machine with the native backend -----

fn native_runner() -> Arc<dyn ManagedCommandRunner> {
    #[cfg(target_os = "linux")]
    let runner = contextual_core::commands::execution::linux::LinuxCommandRunner::new();
    #[cfg(target_os = "macos")]
    let runner = contextual_core::commands::execution::macos::MacosCommandRunner::new();
    #[cfg(windows)]
    let runner = contextual_core::commands::execution::windows::WindowsCommandRunner::new();
    Arc::new(runner)
}

/// Prints `hello`, a line on stderr, and creates the file `ran` in the project.
fn marker_command() -> Value {
    let (program, flag, script) = if cfg!(windows) {
        (
            format!(
                "{}\\System32\\cmd.exe",
                std::env::var("SystemRoot").unwrap()
            ),
            "/c",
            // The caret keeps the printed words out of the command text, so that a test
            // can tell output from the proposal itself.
            "echo hel^lo& echo prob^lem>&2& echo x>ran",
        )
    } else {
        (
            "/bin/sh".to_owned(),
            "-c",
            "echo hel''lo; echo prob''lem >&2; : > ran",
        )
    };
    json!({
        "version": 1,
        "purpose": "Create the marker.",
        "command": { "kind": "exec", "program": program, "args": [flag, script] },
        "cwd": ".",
        "timeoutSeconds": 30,
        "environment": []
    })
}

struct Workbench {
    _storage: tempfile::TempDir,
    _project: tempfile::TempDir,
    root: String,
    journal: std::path::PathBuf,
    host: Arc<LocalHost>,
    client: FoundryClient,
    script: Script,
    _server: MockServer,
}

impl Workbench {
    async fn new(replies: Vec<ResponseTemplate>) -> Self {
        let storage = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(project.path())
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let journal = storage.path().join("journal");
        let host = Self::host(&journal, &root).await;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = MockServer::builder().listener(listener).start().await;
        let script = Script {
            replies: Arc::new(Mutex::new(replies.into())),
            ..Script::default()
        };
        Mock::given(any())
            .respond_with(script.clone())
            .mount(&server)
            .await;
        let client = FoundryClient::new(FoundryConfig {
            endpoint: format!("{}/", server.uri()),
            api_key: "test-key".into(),
            api_version: "2025-04-01-preview".into(),
            deployment: "gpt-workflow".into(),
            model_name: "gpt-workflow".into(),
            max_completion_tokens: 4096,
            context_limit: None,
            input_price_per_1m: None,
            output_price_per_1m: None,
        })
        .unwrap()
        .with_max_retries(0);
        Self {
            _storage: storage,
            _project: project,
            root,
            journal,
            host,
            client,
            script,
            _server: server,
        }
    }

    /// A host with execution explicitly configured, as a test harness and never by default.
    async fn host(journal: &std::path::Path, root: &str) -> Arc<LocalHost> {
        let journal = FileExecutionJournal::open(journal.to_path_buf())
            .await
            .unwrap();
        let host = Arc::new(LocalHost::with_command_backend(
            native_runner(),
            Arc::new(journal),
        ));
        host.allow_root(root).await.unwrap();
        host
    }

    fn ran(&self) -> bool {
        std::path::Path::new(&self.root).join("ran").exists()
    }

    /// One model turn: sends what the chat holds plus `input` and the planned answers, and
    /// records the completed round. This is the only place a reply's calls enter the chat.
    async fn turn(
        &self,
        thread: &mut ThreadState,
        input: &str,
        shared: &BTreeMap<String, SharedResult>,
    ) {
        let answers = thread.plan_answers(shared).unwrap();
        let request = ConversationRequest {
            system: "SYS".into(),
            history: history(thread),
            input: MessageContent::Text(input.into()),
            previous_response_id: thread.response_id.clone(),
            command_tool: true,
            tool_results: tool_results(&answers),
        };
        let events: Vec<ChatEvent> = self
            .client
            .stream_conversation(request, CancellationToken::new())
            .collect()
            .await;
        let mut reply = String::new();
        let mut calls = Vec::new();
        for event in events {
            match event {
                ChatEvent::Delta { text } => reply.push_str(&text),
                ChatEvent::ToolCall {
                    call_id,
                    name,
                    arguments,
                    problem,
                } => calls.push(ToolCall {
                    call_id,
                    name,
                    arguments,
                    problem,
                }),
                ChatEvent::Done { response_id, .. } => thread.response_id = response_id,
                ChatEvent::Error { message } => panic!("the model turn failed: {message}"),
                _ => {}
            }
        }
        thread
            .record_command_round(
                answers,
                (!input.is_empty()).then(|| ApiContent::Text(input.into())),
                "SYS",
                false,
                reply,
                calls,
                None,
                &Location::Local,
                NOW,
            )
            .unwrap();
    }

    /// Host preparation of a recorded proposal: inspection only.
    async fn prepare(&self, host: &LocalHost, thread: &ThreadState, call: &CallId) -> ReviewPlan {
        let request = thread.command(call).unwrap().request().unwrap();
        let prepared = host
            .prepare_command(PrepareCommand {
                root: self.root.clone(),
                project_epoch: "project-1".into(),
                command_json: serde_json::to_string(request).unwrap(),
            })
            .await
            .unwrap();
        ReviewPlan::from_prepared(
            ProjectRoot {
                location: Location::Local,
                path: self.root.clone(),
            },
            prepared,
        )
        .unwrap()
    }

    /// The user presses Run on the shown plan; the command runs to its end.
    async fn run(
        &self,
        thread: &mut ThreadState,
        desk: &mut CommandDesk,
        call: &CallId,
    ) -> ExecutionResult {
        let shown = desk.plan(call).unwrap().clone();
        let approval = desk.approve(call, &shown).unwrap();
        let current = self.prepare(&self.host, thread, call).await;
        let permit = desk.begin(approval, &current).unwrap();
        let invocation = permit.invocation().clone();
        thread
            .commands
            .begin(call, invocation.clone(), shown.request.clone(), NOW)
            .unwrap();
        let serving: Arc<dyn WorkspaceHost> = self.host.clone();
        let result = serving
            .start_command(permit, CancellationToken::new())
            .unwrap()
            .wait()
            .await;
        thread
            .commands
            .finish(call, &invocation, &result, NOW)
            .unwrap();
        desk.finished(call, &invocation, result.clone()).unwrap();
        result
    }
}

fn call_of(thread: &ThreadState, call: &str) -> CallId {
    CallId {
        turn: thread.unanswered_calls().0.unwrap().to_owned(),
        call: call.into(),
    }
}

#[tokio::test]
async fn proposal_review_approval_execution_observation_and_sharing() {
    let command = marker_command();
    let bench = Workbench::new(vec![
        proposing("resp_1", &[("call_1", &command)]),
        saying("resp_2", "It printed hello."),
    ])
    .await;
    let mut thread = ThreadState::fresh();
    let mut desk = CommandDesk::new();

    // The model proposes. The proposal is recorded; nothing is prepared or approved.
    bench
        .turn(&mut thread, "Create the marker", &BTreeMap::new())
        .await;
    let call = call_of(&thread, "call_1");
    assert_eq!(
        thread.command(&call).unwrap().status(),
        RecordStatus::NeedsReview
    );
    assert!(desk.plan(&call).is_none() && !bench.ran());

    // The host prepares it and the plan is put up for review. Still nothing runs.
    let plan = bench.prepare(&bench.host, &thread, &call).await;
    desk.review(&call, plan).unwrap();
    assert_eq!(desk.state(&call), Some(CommandState::NeedsReview));
    assert!(!bench.ran());

    // The user presses Run.
    let result = bench.run(&mut thread, &mut desk, &call).await;
    assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
    assert!(result.output.stdout.text.starts_with("hello"));
    assert!(bench.ran());
    assert_eq!(
        thread.command(&call).unwrap().status(),
        RecordStatus::Finished(ExecutionStatus::Success)
    );
    // Running sent nothing to the model: there has been exactly one request so far.
    assert_eq!(bench.script.requests.lock().unwrap().len(), 1);

    // The user shares stdout and omits stderr.
    let (invocation, held) = desk.output(&call).unwrap();
    let attempt = thread.command(&call).unwrap().attempts.last().unwrap();
    assert_eq!(&attempt.invocation, invocation);
    let choice = ShareChoice {
        stdout: true,
        ..ShareChoice::default()
    };
    let shared = SharedResult::observed(&call, attempt, held, &choice);
    let mut sharing = BTreeMap::new();
    sharing.insert("call_1".to_owned(), shared);
    bench.turn(&mut thread, "", &sharing).await;
    desk.forget_output(&call);

    // The model received exactly that result for exactly that call.
    let requests = bench.script.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["previous_response_id"], "resp_1");
    let input = requests[1]["input"].as_array().unwrap();
    assert_eq!(input.len(), 1);
    assert_eq!(input[0]["type"], "function_call_output");
    assert_eq!(input[0]["call_id"], "call_1");
    let told: Value = serde_json::from_str(input[0]["output"].as_str().unwrap()).unwrap();
    assert_eq!(told["status"], "success");
    assert!(
        told["stdout"]["text"]
            .as_str()
            .unwrap()
            .starts_with("hello")
    );
    assert_eq!(
        told["stderr"],
        json!({ "text": "", "truncated": false, "omitted": true })
    );

    // The chat records what was shared and what was not.
    assert!(matches!(
        thread
            .command(&call)
            .unwrap()
            .answered
            .as_ref()
            .unwrap()
            .answer,
        Answer::Outcome {
            stdout_shared: true,
            stderr_shared: false,
            ..
        }
    ));
    let saved = serde_json::to_string(&SavedThread::new(thread, Vec::new())).unwrap();
    assert!(saved.contains("hello") && !saved.contains("problem"));
}

#[tokio::test]
async fn a_rejected_proposal_never_runs_and_the_model_is_told_so() {
    let command = marker_command();
    let bench = Workbench::new(vec![
        proposing("resp_1", &[("call_1", &command)]),
        saying("resp_2", "Understood."),
    ])
    .await;
    let mut thread = ThreadState::fresh();
    let mut desk = CommandDesk::new();
    bench
        .turn(&mut thread, "Create the marker", &BTreeMap::new())
        .await;
    let call = call_of(&thread, "call_1");
    let plan = bench.prepare(&bench.host, &thread, &call).await;
    desk.review(&call, plan.clone()).unwrap();

    desk.reject(&call).unwrap();
    thread.commands.reject(&call, NOW).unwrap();
    // A rejection is final for this proposal.
    assert!(desk.approve(&call, &plan).is_err());

    bench.turn(&mut thread, "", &BTreeMap::new()).await;
    let requests = bench.script.requests.lock().unwrap().clone();
    let told: Value =
        serde_json::from_str(requests[1]["input"][0]["output"].as_str().unwrap()).unwrap();
    assert_eq!(told["status"], "rejected");
    assert!(told.get("stdout").is_none());
    assert!(!bench.ran());
    assert_eq!(
        thread.command(&call).unwrap().status(),
        RecordStatus::Rejected
    );
}

#[tokio::test]
async fn a_restored_chat_can_launch_nothing_without_fresh_preparation_and_approval() {
    let command = marker_command();
    let bench = Workbench::new(vec![proposing(
        "resp_1",
        &[("call_1", &command), ("call_2", &command)],
    )])
    .await;
    let mut thread = ThreadState::fresh();
    let mut desk = CommandDesk::new();
    bench
        .turn(&mut thread, "Create the marker", &BTreeMap::new())
        .await;
    let (first, second) = (call_of(&thread, "call_1"), call_of(&thread, "call_2"));
    // The model proposed the same invocation twice in one turn: only one can be reviewed.
    let plan = bench.prepare(&bench.host, &thread, &first).await;
    desk.review(&first, plan.clone()).unwrap();
    assert!(desk.review(&second, plan.clone()).is_err());

    // The first is approved and dispatched; the app stops before recording its result.
    let approval = desk.approve(&first, &plan).unwrap();
    let permit = desk.begin(approval, &plan).unwrap();
    thread
        .commands
        .begin(
            &first,
            permit.invocation().clone(),
            plan.request.clone(),
            NOW,
        )
        .unwrap();
    let saved = serde_json::to_string(&SavedThread::new(thread, Vec::new())).unwrap();
    let serving: Arc<dyn WorkspaceHost> = bench.host.clone();
    let lost = serving
        .start_command(permit, CancellationToken::new())
        .unwrap()
        .wait()
        .await;
    assert_eq!(lost.status, ExecutionStatus::Success);
    std::fs::remove_file(std::path::Path::new(&bench.root).join("ran")).unwrap();
    drop(desk);

    // Restart: a new host, an empty desk, and the chat as it was saved.
    let host = Workbench::host(&bench.journal, &bench.root).await;
    let mut restored: SavedThread = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.thread.restore_commands(NOW), 1);
    let thread = &mut restored.thread;
    let mut desk = CommandDesk::new();

    // The dispatched attempt is unknown, and is not run again by anything.
    assert_eq!(
        thread.command(&first).unwrap().status(),
        RecordStatus::Finished(ExecutionStatus::Unknown)
    );
    assert!(
        thread.command(&first).unwrap().attempts[0]
            .observed
            .unwrap()
            .interrupted
    );
    assert!(!bench.ran());
    // Nothing is approved or even shown: the old plan cannot be approved on the new desk.
    assert!(desk.plan(&first).is_none() && desk.state(&first).is_none());
    assert!(desk.approve(&first, &plan).is_err());
    assert!(desk.approve(&second, &plan).is_err());
    assert!(!desk.busy() && !bench.ran());

    // Only fresh preparation, review and approval can start something: a new attempt.
    let fresh = bench.prepare(&host, thread, &first).await;
    assert!(fresh.binding != plan.binding);
    desk.review(&first, fresh.clone()).unwrap();
    assert!(!bench.ran());
    let approval = desk.approve(&first, &fresh).unwrap();
    let permit = desk.begin(approval, &fresh).unwrap();
    let invocation = permit.invocation().clone();
    thread
        .commands
        .begin(&first, invocation.clone(), fresh.request.clone(), NOW)
        .unwrap();
    let serving: Arc<dyn WorkspaceHost> = host.clone();
    let result = serving
        .start_command(permit, CancellationToken::new())
        .unwrap()
        .wait()
        .await;
    thread
        .commands
        .finish(&first, &invocation, &result, NOW)
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Success, "{result:?}");
    assert!(bench.ran());
    // The earlier attempt is kept next to the new one.
    let attempts = &thread.command(&first).unwrap().attempts;
    assert_eq!(attempts.len(), 2);
    assert_ne!(attempts[0].invocation, attempts[1].invocation);
}
