#![cfg(feature = "llm")]
//! `propose_command` over both Foundry transports, against a scripted mock provider.
//!
//! The streamed payloads here are written from the documented shapes of the Responses and
//! chat-completions APIs. They were not recorded from a live deployment.
//!
//! What these tests establish: calls are assembled from fragments and bounded; a call is
//! usable only when the whole response completed; every call keeps its identity even when it
//! is unusable; calls and results travel as native transcript items; and rebuilding a request
//! from history — after a chain reset or on the chat fallback — sends data and does nothing
//! else. Parsing produces proposals only: this crate's `llm` module cannot run anything.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use contextual_core::llm::{
    ChatEvent, ChatMessage, ConversationRequest, FoundryClient, FoundryConfig, MessageContent,
    PROPOSE_COMMAND, ToolCall, ToolResult, command_request,
};
use futures::StreamExt;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Answers requests in order from a script and records what was asked.
#[derive(Clone, Default)]
struct Script {
    replies: Arc<Mutex<VecDeque<ResponseTemplate>>>,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
}

impl Respond for Script {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.requests.lock().unwrap().push((
            request.url.path().to_owned(),
            serde_json::from_slice(&request.body).unwrap(),
        ));
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("the client sent a request that the test did not expect")
    }
}

impl Script {
    fn requests(&self) -> Vec<(String, Value)> {
        self.requests.lock().unwrap().clone()
    }
}

async fn provider(replies: Vec<ResponseTemplate>) -> (MockServer, Script) {
    // A fresh address per test: capability knowledge is cached per endpoint.
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
    (server, script)
}

fn client(server: &MockServer) -> FoundryClient {
    FoundryClient::new(FoundryConfig {
        endpoint: format!("{}/", server.uri()),
        api_key: "test-key".into(),
        api_version: "2025-04-01-preview".into(),
        deployment: "gpt-tools".into(),
        model_name: "gpt-tools".into(),
        max_completion_tokens: 4096,
        context_limit: None,
        input_price_per_1m: None,
        output_price_per_1m: None,
        tokens_per_minute: None,
    })
    .unwrap()
    .with_max_retries(0)
}

fn request(
    history: Vec<ChatMessage>,
    input: &str,
    previous: Option<&str>,
    results: Vec<ToolResult>,
) -> ConversationRequest {
    ConversationRequest {
        system: "SYS".into(),
        history,
        input: MessageContent::Text(input.into()),
        previous_response_id: previous.map(str::to_owned),
        command_tool: true,
        tool_results: results,
    }
}

async fn turn(client: &FoundryClient, request: ConversationRequest) -> Vec<ChatEvent> {
    client
        .stream_conversation(request, CancellationToken::new())
        .collect()
        .await
}

fn calls(events: &[ChatEvent]) -> Vec<ToolCall> {
    events
        .iter()
        .filter_map(|event| match event {
            ChatEvent::ToolCall {
                call_id,
                name,
                arguments,
                problem,
            } => Some(ToolCall {
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
                problem: problem.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn text(events: &[ChatEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            ChatEvent::Delta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn finish(events: &[ChatEvent]) -> Option<String> {
    events.iter().find_map(|event| match event {
        ChatEvent::Done { finish_reason, .. } => finish_reason.clone(),
        _ => None,
    })
}

fn error(events: &[ChatEvent]) -> Option<String> {
    events.iter().find_map(|event| match event {
        ChatEvent::Error { message } => Some(message.clone()),
        _ => None,
    })
}

fn arguments(program: &str) -> String {
    json!({
        "version": 1,
        "purpose": "Check the project.",
        "command": { "kind": "exec", "program": program, "args": ["--all", "a b"] },
        "cwd": ".",
        "timeoutSeconds": 60,
        "environment": []
    })
    .to_string()
}

fn usable(call_id: &str, program: &str) -> ToolCall {
    ToolCall {
        call_id: call_id.into(),
        name: PROPOSE_COMMAND.into(),
        arguments: arguments(program),
        problem: None,
    }
}

fn rejected(call_id: &str) -> ToolResult {
    ToolResult {
        call_id: call_id.into(),
        output: "The user rejected this command. It was not run.".into(),
    }
}

// ----- Responses API payloads -----

fn responses_sse(events: &[Value]) -> ResponseTemplate {
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

fn created() -> Value {
    json!({ "type": "response.created", "response": { "id": "resp_1", "status": "in_progress" } })
}

fn function_item(item: &str, call_id: &str, name: &str, arguments: &str) -> Value {
    json!({
        "id": item, "type": "function_call", "call_id": call_id, "name": name,
        "arguments": arguments, "status": "completed"
    })
}

/// One function call as it is streamed: announced, its arguments in two pieces, closed.
fn streamed_call(item: &str, call_id: &str, name: &str, arguments: &str) -> Vec<Value> {
    let (head, tail) = arguments.split_at(arguments.len() / 2);
    vec![
        json!({ "type": "response.output_item.added", "item": {
            "id": item, "type": "function_call", "call_id": call_id, "name": name,
            "arguments": "", "status": "in_progress" } }),
        json!({ "type": "response.function_call_arguments.delta", "item_id": item, "delta": head }),
        json!({ "type": "response.function_call_arguments.delta", "item_id": item, "delta": tail }),
        json!({ "type": "response.function_call_arguments.done", "item_id": item, "arguments": arguments }),
        json!({ "type": "response.output_item.done", "item": function_item(item, call_id, name, arguments) }),
    ]
}

fn completed(output: Vec<Value>) -> Value {
    json!({ "type": "response.completed", "response": {
        "id": "resp_1", "status": "completed", "output": output,
        "usage": { "input_tokens": 10, "output_tokens": 5, "input_tokens_details": { "cached_tokens": 0 } }
    } })
}

fn text_reply(reply: &str) -> ResponseTemplate {
    responses_sse(&[
        created(),
        json!({ "type": "response.output_text.delta", "delta": reply }),
        completed(vec![]),
    ])
}

fn refusal(status: u16, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(
        json!({ "error": { "message": message, "type": "invalid_request_error" } }).to_string(),
        "application/json",
    )
}

/// Runs one Responses turn against a single scripted stream.
async fn responses_turn(events: Vec<Value>) -> (Vec<ChatEvent>, Vec<(String, Value)>) {
    let (server, script) = provider(vec![responses_sse(&events)]).await;
    let events = turn(&client(&server), request(vec![], "Check it", None, vec![])).await;
    (events, script.requests())
}

#[tokio::test]
async fn a_complete_call_split_across_fragments_becomes_one_proposal() {
    let wanted = arguments("cargo");
    let mut events = vec![created()];
    events.extend(streamed_call("fc_1", "call_1", PROPOSE_COMMAND, &wanted));
    events.push(completed(vec![function_item(
        "fc_1",
        "call_1",
        PROPOSE_COMMAND,
        &wanted,
    )]));
    let (events, requests) = responses_turn(events).await;

    assert_eq!(calls(&events), [usable("call_1", "cargo")]);
    // A tool-only reply: no prose is invented for it.
    assert_eq!(text(&events), "");
    assert_eq!(finish(&events).as_deref(), Some("stop"));
    assert!(error(&events).is_none());
    // The proposal is validated separately from the transport.
    let proposal = command_request(&calls(&events)[0]).unwrap();
    assert_eq!(proposal.timeout_seconds, 60);

    // The tool was declared, once, in the Responses shape.
    assert_eq!(requests.len(), 1);
    let (path, body) = &requests[0];
    assert!(path.ends_with("/responses"));
    assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["name"], PROPOSE_COMMAND);
    assert!(body["tools"][0]["parameters"]["properties"]["command"].is_object());
    assert!(body.get("tool_choice").is_none());
}

#[tokio::test]
async fn several_calls_and_text_arrive_in_order_and_each_stands_alone() {
    let (first, second) = (arguments("cargo"), arguments("git"));
    let mut events = vec![
        created(),
        json!({ "type": "response.output_text.delta", "delta": "Two checks: " }),
    ];
    // Interleave the two calls' events, as a provider may.
    let a = streamed_call("fc_1", "call_1", PROPOSE_COMMAND, &first);
    let b = streamed_call("fc_2", "call_2", PROPOSE_COMMAND, &second);
    for (x, y) in a.into_iter().zip(b) {
        events.push(x);
        events.push(y);
    }
    events.push(completed(vec![
        function_item("fc_1", "call_1", PROPOSE_COMMAND, &first),
        function_item("fc_2", "call_2", PROPOSE_COMMAND, &second),
    ]));
    let (events, _) = responses_turn(events).await;

    assert_eq!(text(&events), "Two checks: ");
    assert_eq!(
        calls(&events),
        [usable("call_1", "cargo"), usable("call_2", "git")]
    );
    // Calls are reported before usage and done, after the whole response ended.
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| match event {
            ChatEvent::Delta { .. } => "delta",
            ChatEvent::ToolCall { .. } => "call",
            ChatEvent::Usage { .. } => "usage",
            ChatEvent::Done { .. } => "done",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["delta", "call", "call", "usage", "done"]);
}

#[tokio::test]
async fn a_call_listed_only_in_the_completed_response_is_accepted() {
    let wanted = arguments("cargo");
    let (events, _) = responses_turn(vec![
        created(),
        completed(vec![function_item(
            "fc_1",
            "call_1",
            PROPOSE_COMMAND,
            &wanted,
        )]),
    ])
    .await;
    assert_eq!(calls(&events), [usable("call_1", "cargo")]);
}

#[tokio::test]
async fn unusable_calls_keep_their_identity_and_never_become_proposals() {
    let good = arguments("cargo");
    let mut events = vec![created()];
    // Not JSON.
    events.extend(streamed_call(
        "fc_1",
        "call_1",
        PROPOSE_COMMAND,
        "{\"version\": 1,",
    ));
    // Another tool.
    events.extend(streamed_call("fc_2", "call_2", "run_immediately", &good));
    // Two calls with one identity.
    events.extend(streamed_call("fc_3", "call_same", PROPOSE_COMMAND, &good));
    events.extend(streamed_call("fc_4", "call_same", PROPOSE_COMMAND, &good));
    // Valid JSON that is not a valid command: the transport passes it, validation refuses.
    events.extend(streamed_call(
        "fc_5",
        "call_5",
        PROPOSE_COMMAND,
        "{\"version\":2}",
    ));
    events.push(completed_without_output());
    let (events, _) = responses_turn(events).await;

    let found = calls(&events);
    let ids: Vec<&str> = found.iter().map(|call| call.call_id.as_str()).collect();
    assert_eq!(
        ids,
        ["call_1", "call_2", "call_same", "call_same", "call_5"]
    );
    assert!(
        found[..4].iter().all(|call| call.problem.is_some()),
        "{found:?}"
    );
    assert!(found[4].problem.is_none());
    assert!(found.iter().all(|call| command_request(call).is_err()));
    assert!(error(&events).is_none());
}

/// A completion event that does not repeat the output list.
fn completed_without_output() -> Value {
    json!({ "type": "response.completed", "response": { "id": "resp_1", "status": "completed" } })
}

#[tokio::test]
async fn a_response_that_did_not_complete_yields_no_usable_call() {
    let wanted = arguments("cargo");
    let streamed = || {
        let mut events = vec![created()];
        events.extend(streamed_call("fc_1", "call_1", PROPOSE_COMMAND, &wanted));
        events
    };

    // Cut off by the output limit, after the call's own "done" events.
    let mut cut = streamed();
    cut.push(json!({ "type": "response.incomplete", "response": {
        "id": "resp_1", "incomplete_details": { "reason": "max_output_tokens" } } }));
    let (events, _) = responses_turn(cut).await;
    assert_eq!(finish(&events).as_deref(), Some("length"));
    assert_eq!(calls(&events).len(), 1);
    assert_eq!(calls(&events)[0].call_id, "call_1");
    assert!(calls(&events)[0].problem.is_some());

    // The stream simply ends: an arguments-done event is not completion.
    let (events, _) = responses_turn(streamed()).await;
    assert!(calls(&events)[0].problem.is_some());
    assert!(command_request(&calls(&events)[0]).is_err());

    // The provider reports failure.
    let mut failed = streamed();
    failed.push(json!({ "type": "response.failed", "response": {
        "error": { "message": "The model failed." } } }));
    let (events, _) = responses_turn(failed).await;
    assert!(calls(&events).is_empty());
    assert!(error(&events).unwrap().ends_with("The model failed."));
}

#[tokio::test]
async fn a_call_that_the_provider_reports_two_ways_is_ambiguous() {
    let streamed = arguments("cargo");
    let mut events = vec![created()];
    events.extend(streamed_call("fc_1", "call_1", PROPOSE_COMMAND, &streamed));
    // Streamed but missing from the completed list.
    events.extend(streamed_call("fc_2", "call_2", PROPOSE_COMMAND, &streamed));
    // The completed response states different arguments for the first call.
    events.push(completed(vec![function_item(
        "fc_1",
        "call_1",
        PROPOSE_COMMAND,
        &arguments("rm"),
    )]));
    let (events, _) = responses_turn(events).await;

    let found = calls(&events);
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|call| call.problem.is_some()), "{found:?}");
    // The first value is kept for the record; the later one is not adopted.
    assert_eq!(found[0].arguments, streamed);
}

#[tokio::test]
async fn too_many_or_too_large_calls_are_bounded() {
    // More calls than can be reviewed: the turn fails, nothing is accepted.
    let wanted = arguments("cargo");
    let mut events = vec![created()];
    for index in 0..9 {
        events.extend(streamed_call(
            &format!("fc_{index}"),
            &format!("call_{index}"),
            PROPOSE_COMMAND,
            &wanted,
        ));
    }
    events.push(completed_without_output());
    let (events, _) = responses_turn(events).await;
    assert!(calls(&events).is_empty());
    assert!(error(&events).unwrap().contains("more tool calls"));

    // Arguments beyond the limit are not accumulated.
    let mut events = vec![
        created(),
        json!({ "type": "response.output_item.added", "item": {
            "id": "fc_1", "type": "function_call", "call_id": "call_1",
            "name": PROPOSE_COMMAND, "arguments": "" } }),
    ];
    let piece = "x".repeat(40_000);
    for _ in 0..5 {
        events.push(json!({
            "type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": piece }));
    }
    events.push(completed_without_output());
    let (events, _) = responses_turn(events).await;
    let found = calls(&events);
    assert!(found[0].arguments.len() <= 64 * 1024);
    assert!(found[0].problem.is_some());
}

#[tokio::test]
async fn a_call_made_without_an_offered_tool_is_unusable() {
    let wanted = arguments("cargo");
    let mut events = vec![created()];
    events.extend(streamed_call("fc_1", "call_1", PROPOSE_COMMAND, &wanted));
    events.push(completed_without_output());
    let (server, script) = provider(vec![responses_sse(&events)]).await;
    let mut plain = request(vec![], "Check it", None, vec![]);
    plain.command_tool = false;
    let events = turn(&client(&server), plain).await;

    assert!(script.requests()[0].1.get("tools").is_none());
    assert!(calls(&events)[0].problem.is_some());
}

// ----- Transcript: calls and results as history -----

fn asked(calls: Vec<ToolCall>) -> Vec<ChatMessage> {
    vec![
        ChatMessage::user("Check it"),
        ChatMessage::assistant_calls("", calls),
    ]
}

#[tokio::test]
async fn results_are_returned_with_the_original_call_identity() {
    let (server, script) = provider(vec![text_reply("Understood."), text_reply("Done.")]).await;
    let client = client(&server);
    let history = asked(vec![usable("call_1", "cargo"), usable("call_2", "git")]);

    // Results only: nothing is sent as a user message.
    let results = vec![rejected("call_2"), rejected("call_1")];
    let events = turn(
        &client,
        request(history.clone(), "", Some("resp_1"), results.clone()),
    )
    .await;
    assert_eq!(text(&events), "Understood.");
    // Results followed by what the user typed.
    turn(
        &client,
        request(history, "Try something else", Some("resp_1"), results),
    )
    .await;

    let requests = script.requests();
    let first = &requests[0].1;
    assert_eq!(first["previous_response_id"], "resp_1");
    assert_eq!(
        first["input"],
        json!([
            { "type": "function_call_output", "call_id": "call_2",
              "output": "The user rejected this command. It was not run." },
            { "type": "function_call_output", "call_id": "call_1",
              "output": "The user rejected this command. It was not run." },
        ])
    );
    let second = requests[1].1["input"].as_array().unwrap().clone();
    assert_eq!(second.len(), 3);
    assert_eq!(
        second[2],
        json!({ "role": "user", "content": "Try something else" })
    );
}

#[tokio::test]
async fn a_chain_reset_rebuilds_calls_and_results_as_history_and_does_nothing_else() {
    let (server, script) = provider(vec![
        refusal(400, "Previous response with id 'resp_gone' not found."),
        text_reply("Continuing."),
    ])
    .await;
    let mut history = asked(vec![usable("call_1", "cargo")]);
    history.push(ChatMessage::tool("call_1", "exit code 0\nall tests passed"));
    history.push(ChatMessage::assistant("The tests pass. One more check:"));
    history.push(ChatMessage::user("Go on"));
    history.push(ChatMessage::assistant_calls(
        "Checking.",
        vec![usable("call_2", "git")],
    ));

    let events = turn(
        &client(&server),
        request(
            history,
            "And then?",
            Some("resp_gone"),
            vec![rejected("call_2")],
        ),
    )
    .await;

    // The turn produced a reply and reported the reset. History produced no call events.
    assert_eq!(text(&events), "Continuing.");
    assert!(calls(&events).is_empty());
    assert!(events.iter().any(|event| matches!(
        event,
        ChatEvent::Done {
            chain_reset: true,
            ..
        }
    )));

    // Exactly two HTTP requests, and the second carries the whole conversation as data.
    let requests = script.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].1.get("previous_response_id").is_none());
    assert_eq!(
        requests[1].1["input"],
        json!([
            { "role": "user", "content": "Check it" },
            { "type": "function_call", "call_id": "call_1", "name": PROPOSE_COMMAND,
              "arguments": arguments("cargo") },
            { "type": "function_call_output", "call_id": "call_1",
              "output": "exit code 0\nall tests passed" },
            { "role": "assistant", "content": "The tests pass. One more check:" },
            { "role": "user", "content": "Go on" },
            { "role": "assistant", "content": "Checking." },
            { "type": "function_call", "call_id": "call_2", "name": PROPOSE_COMMAND,
              "arguments": arguments("git") },
            { "type": "function_call_output", "call_id": "call_2",
              "output": "The user rejected this command. It was not run." },
            { "role": "user", "content": "And then?" },
        ])
    );
}

#[tokio::test]
async fn an_inconsistent_transcript_is_refused_before_anything_is_sent() {
    let (server, script) = provider(vec![]).await;
    let client = client(&server);
    for (history, results) in [
        // A call with no result.
        (asked(vec![usable("call_1", "cargo")]), vec![]),
        // A result for a call that was never made.
        (
            vec![ChatMessage::user("Check it")],
            vec![rejected("call_9")],
        ),
        // One of two calls unanswered.
        (
            asked(vec![usable("call_1", "cargo"), usable("call_2", "git")]),
            vec![rejected("call_1")],
        ),
    ] {
        let events = turn(&client, request(history, "Next", None, results)).await;
        // A local refusal, worded as one: the provider was never involved.
        let message = error(&events).unwrap();
        assert!(message.starts_with("The conversation's command history is inconsistent"));
    }
    assert!(script.requests().is_empty());
}

#[tokio::test]
async fn a_call_with_unusable_arguments_is_replayed_without_them() {
    let (server, script) = provider(vec![text_reply("Noted.")]).await;
    let broken = ToolCall {
        call_id: "call_1".into(),
        name: PROPOSE_COMMAND.into(),
        arguments: "{\"version\": 1,".into(),
        problem: Some("The call's arguments are not a JSON object.".into()),
    };
    let results = vec![ToolResult {
        call_id: "call_1".into(),
        output: "This call could not be read and was not run.".into(),
    }];
    turn(
        &client(&server),
        request(asked(vec![broken]), "", None, results),
    )
    .await;

    let input = &script.requests()[0].1["input"];
    assert_eq!(input[1]["call_id"], "call_1");
    assert_eq!(input[1]["arguments"], "{}");
    assert_eq!(input[2]["call_id"], "call_1");
}

// ----- Chat completions -----

fn chat_sse(chunks: &[Value], marker: bool) -> ResponseTemplate {
    let mut body: String = chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect();
    if marker {
        body.push_str("data: [DONE]\n\n");
    }
    ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
}

fn chat_delta(delta: Value, finish: Option<&str>) -> Value {
    json!({ "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] })
}

/// Two indexed calls whose identity, name and arguments all arrive in pieces.
fn chat_calls(finish: &str) -> Vec<Value> {
    let (first, second) = (arguments("cargo"), arguments("git"));
    let (first_head, first_tail) = first.split_at(first.len() / 3);
    let (second_head, second_tail) = second.split_at(second.len() / 2);
    vec![
        chat_delta(json!({ "role": "assistant", "content": null }), None),
        chat_delta(
            json!({ "tool_calls": [{ "index": 0, "id": "call_", "type": "function",
            "function": { "name": "propose_", "arguments": "" } }] }),
            None,
        ),
        chat_delta(
            json!({ "tool_calls": [{ "index": 0, "id": "a",
            "function": { "name": "command", "arguments": first_head } }] }),
            None,
        ),
        chat_delta(
            json!({ "tool_calls": [
            { "index": 1, "id": "call_b", "type": "function",
              "function": { "name": PROPOSE_COMMAND, "arguments": second_head } },
            { "index": 0, "function": { "arguments": first_tail } },
        ] }),
            None,
        ),
        chat_delta(
            json!({ "tool_calls": [{ "index": 1,
            "function": { "arguments": second_tail } }] }),
            None,
        ),
        chat_delta(json!({}), Some(finish)),
        json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 5 } }),
    ]
}

/// A deployment without the Responses API: the first request is answered with 404.
async fn chat_provider(stream: ResponseTemplate) -> (MockServer, Script) {
    provider(vec![refusal(404, "Resource not found"), stream]).await
}

#[tokio::test]
async fn chat_completions_assembles_indexed_calls_split_across_chunks() {
    let (server, script) = chat_provider(chat_sse(&chat_calls("tool_calls"), true)).await;
    let client = client(&server);
    let events = turn(&client, request(vec![], "Check it", None, vec![])).await;

    assert_eq!(
        calls(&events),
        [usable("call_a", "cargo"), usable("call_b", "git")]
    );
    assert_eq!(text(&events), "");
    assert_eq!(finish(&events).as_deref(), Some("tool_calls"));
    assert_eq!(client.responses_supported(), Some(false));

    // The fallback request declares the tool in the chat-completions shape.
    let requests = script.requests();
    assert_eq!(requests.len(), 2);
    let body = &requests[1].1;
    assert!(requests[1].0.ends_with("/chat/completions"));
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], PROPOSE_COMMAND);
    assert!(body["tools"][0]["function"]["parameters"].is_object());
}

#[tokio::test]
async fn chat_calls_need_the_tool_finish_reason_and_a_clean_end() {
    // Cut off by the output limit.
    let (server, _) = chat_provider(chat_sse(&chat_calls("length"), true)).await;
    let events = turn(&client(&server), request(vec![], "Check it", None, vec![])).await;
    let found = calls(&events);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].call_id, "call_a");
    assert!(found.iter().all(|call| call.problem.is_some()));

    // The right finish reason, but the stream ended without its marker.
    let (server, _) = chat_provider(chat_sse(&chat_calls("tool_calls"), false)).await;
    let events = turn(&client(&server), request(vec![], "Check it", None, vec![])).await;
    assert!(calls(&events).iter().all(|call| call.problem.is_some()));

    // No finish reason at all.
    let mut unfinished = chat_calls("tool_calls");
    unfinished.remove(5);
    let (server, _) = chat_provider(chat_sse(&unfinished, true)).await;
    let events = turn(&client(&server), request(vec![], "Check it", None, vec![])).await;
    assert!(
        calls(&events)
            .iter()
            .all(|call| command_request(call).is_err())
    );
}

#[tokio::test]
async fn chat_history_carries_calls_and_correlated_tool_messages() {
    let reply = chat_sse(
        &[
            chat_delta(json!({ "content": "Understood." }), None),
            chat_delta(json!({}), Some("stop")),
        ],
        true,
    );
    let (server, script) = chat_provider(reply).await;
    let mut history = asked(vec![usable("call_1", "cargo")]);
    history.push(ChatMessage::tool("call_1", "exit code 0"));
    history.push(ChatMessage::assistant_calls(
        "One more.",
        vec![usable("call_2", "git")],
    ));

    let events = turn(
        &client(&server),
        request(history, "", Some("resp_1"), vec![rejected("call_2")]),
    )
    .await;
    assert_eq!(text(&events), "Understood.");
    assert!(calls(&events).is_empty());

    let call = |id: &str, program: &str| {
        json!({ "id": id, "type": "function",
                "function": { "name": PROPOSE_COMMAND, "arguments": arguments(program) } })
    };
    assert_eq!(
        script.requests()[1].1["messages"],
        json!([
            { "role": "system", "content": "SYS" },
            { "role": "user", "content": "Check it" },
            // A tool-only reply has no text; it is not turned into prose.
            { "role": "assistant", "content": null, "tool_calls": [call("call_1", "cargo")] },
            { "role": "tool", "tool_call_id": "call_1", "content": "exit code 0" },
            { "role": "assistant", "content": "One more.", "tool_calls": [call("call_2", "git")] },
            { "role": "tool", "tool_call_id": "call_2",
              "content": "The user rejected this command. It was not run." },
        ])
    );
}

// ----- Deployments without tool support -----

#[tokio::test]
async fn a_refused_tool_declaration_is_announced_and_not_mistaken_for_a_missing_api() {
    let (server, script) = provider(vec![
        refusal(
            400,
            "Unsupported parameter: 'tools' is not supported with this model.",
        ),
        text_reply("I would run `cargo test`."),
        text_reply("Still text."),
    ])
    .await;
    let client = client(&server);
    let events = turn(&client, request(vec![], "Check it", None, vec![])).await;

    // Announced, then answered in text on the same API. The text is only text.
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ChatEvent::ToolsUnsupported { .. }))
    );
    assert_eq!(text(&events), "I would run `cargo test`.");
    assert!(calls(&events).is_empty() && error(&events).is_none());
    assert_eq!(client.tools_supported(), Some(false));
    assert_eq!(client.responses_supported(), Some(true));

    let requests = script.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|(path, _)| path.ends_with("/responses"))
    );
    assert!(requests[0].1.get("tools").is_some());
    assert!(requests[1].1.get("tools").is_none());

    // Later turns say so again and do not try the declaration a second time.
    let events = turn(&client, request(vec![], "Again", None, vec![])).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ChatEvent::ToolsUnsupported { .. }))
    );
    assert_eq!(script.requests().len(), 3);
    assert!(script.requests()[2].1.get("tools").is_none());
}

#[tokio::test]
async fn other_refusals_are_not_read_as_missing_tool_support() {
    let (server, _) = provider(vec![refusal(400, "Invalid value for 'max_output_tokens'.")]).await;
    let client = client(&server);
    let events = turn(&client, request(vec![], "Check it", None, vec![])).await;
    assert!(error(&events).is_some());
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ChatEvent::ToolsUnsupported { .. }))
    );
    assert_eq!(client.tools_supported(), None);
}

#[tokio::test]
async fn a_conversation_with_proposals_is_not_silently_stripped_of_them() {
    let (server, script) = provider(vec![refusal(
        400,
        "Unsupported parameter: 'tools' is not supported with this model.",
    )])
    .await;
    let mut history = asked(vec![usable("call_1", "cargo")]);
    history.push(ChatMessage::tool("call_1", "exit code 0"));
    let events = turn(&client(&server), request(history, "Next", None, vec![])).await;

    assert!(error(&events).unwrap().contains("command proposals"));
    assert_eq!(script.requests().len(), 1);
}

#[tokio::test]
async fn history_keeps_the_tool_declared_but_uncallable_when_proposals_are_off() {
    let (server, script) = provider(vec![text_reply("Fine.")]).await;
    let mut history = asked(vec![usable("call_1", "cargo")]);
    history.push(ChatMessage::tool("call_1", "exit code 0"));
    let mut plain = request(history, "Next", None, vec![]);
    plain.command_tool = false;
    turn(&client(&server), plain).await;

    let body = &script.requests()[0].1;
    assert_eq!(body["tools"][0]["name"], PROPOSE_COMMAND);
    assert_eq!(body["tool_choice"], "none");
}

// ----- Budget -----

#[test]
fn the_tool_definition_calls_and_results_count_toward_the_input_budget() {
    let plain = ConversationRequest {
        command_tool: false,
        ..request(vec![ChatMessage::user("Check it")], "Next", None, vec![])
    };
    let base = plain.estimated_input_tokens();

    let declared = ConversationRequest {
        command_tool: true,
        ..plain.clone()
    };
    assert!(declared.estimated_input_tokens() > base + 100);

    let mut history = asked(vec![usable("call_1", "cargo")]);
    let with_call = ConversationRequest {
        history: history.clone(),
        ..plain.clone()
    };
    assert!(with_call.estimated_input_tokens() > base + 20);

    history.push(ChatMessage::tool("call_1", "x".repeat(4000)));
    let with_result = ConversationRequest {
        history,
        ..plain.clone()
    };
    assert!(with_result.estimated_input_tokens() > with_call.estimated_input_tokens() + 500);

    let pending = ConversationRequest {
        tool_results: vec![ToolResult {
            call_id: "call_1".into(),
            output: "x".repeat(4000),
        }],
        ..with_call.clone()
    };
    assert!(pending.estimated_input_tokens() > with_call.estimated_input_tokens() + 500);
}
