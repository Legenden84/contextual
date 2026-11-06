#![cfg(feature = "llm")]
//! Foundry client tests against local mock servers (`wiremock`, plus a raw TCP server for
//! aborts), using recorded-shape Azure payloads from `fixtures/llm/`.
//!
//! `parity_with_node_server` replays `fixtures/llm/record.ts` (the old server on the openai SDK,
//! recorded into `fixtures/llm/expected.json`) and compares every HTTP request and every result.

use contextual_core::error::ErrorCode;
use contextual_core::llm::{
    ChatEvent, ChatMessage, ContentPart, ConversationRequest, FoundryClient, FoundryConfig,
    FoundryError, MessageContent, PrDraftInput, RepairInput, describe_error,
};
use futures::StreamExt;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/llm")
}

fn file(name: &str) -> String {
    std::fs::read_to_string(fixtures().join(name)).unwrap()
}

/// Mirrors `route()` in `fixtures/llm/record.ts`.
struct AzureLike {
    canned: Value,
}

impl AzureLike {
    fn new() -> Self {
        Self {
            canned: serde_json::from_str(&file("canned.json")).unwrap(),
        }
    }
    fn json(&self, status: u16, name: &str) -> ResponseTemplate {
        ResponseTemplate::new(status)
            .set_body_raw(self.canned[name].to_string(), "application/json")
    }
}

fn sse(name: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(file(name), "text/event-stream")
}

impl Respond for AzureLike {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
        let model = body["model"].as_str().unwrap_or("");
        if model == "gpt-401" {
            return self.json(401, "unauthorized");
        }
        if req.url.path().starts_with("/openai/v1/responses") {
            return match model {
                "gpt-chat" => self.json(404, "notFound"),
                _ if body["previous_response_id"] == "resp_gone" => {
                    self.json(400, "previousNotFound")
                }
                "gpt-fail" => sse("responses-failed.sse"),
                "gpt-incomplete" => sse("responses-incomplete.sse"),
                _ => sse("responses-stream.sse"),
            };
        }
        if body["stream"] == true {
            return sse("chat-stream.sse");
        }
        let system = body["messages"][0]["content"].as_str().unwrap_or("");
        if system.starts_with("You write git commit") {
            self.json(200, "commit")
        } else if system.starts_with("You write GitHub pull request") {
            self.json(200, "pr")
        } else if system.starts_with("You apply edits") {
            let user = body["messages"][1]["content"].as_str().unwrap_or("");
            self.json(
                200,
                if user.contains("CUT") {
                    "repairCut"
                } else {
                    "repair"
                },
            )
        } else {
            self.json(200, "ping")
        }
    }
}

async fn azure_like() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(AzureLike::new())
        .mount(&server)
        .await;
    server
}

fn cfg(endpoint: &str, deployment: &str) -> FoundryConfig {
    FoundryConfig {
        endpoint: endpoint.to_string(),
        api_key: "test-key".into(),
        api_version: "2025-04-01-preview".into(),
        deployment: deployment.into(),
        model_name: deployment.into(),
        max_completion_tokens: 4096,
        context_limit: None,
        input_price_per_1m: None,
        output_price_per_1m: None,
    }
}

fn client(server_uri: &str, deployment: &str) -> FoundryClient {
    FoundryClient::new(cfg(&format!("{server_uri}/"), deployment))
        .unwrap()
        .with_max_retries(0)
}

async fn chat(c: &FoundryClient, req: ConversationRequest) -> Vec<Value> {
    c.stream_conversation(req, CancellationToken::new())
        .map(|e| serde_json::to_value(e).unwrap())
        .collect()
        .await
}

fn req(
    history: Vec<ChatMessage>,
    input: impl Into<MessageContent>,
    previous: Option<&str>,
) -> ConversationRequest {
    ConversationRequest {
        system: "SYS".into(),
        history,
        input: input.into(),
        previous_response_id: previous.map(str::to_string),
    }
}

const IMAGE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";

fn history() -> Vec<ChatMessage> {
    vec![
        ChatMessage::user(vec![
            ContentPart::text("Files:\n\nsrc/a.ts"),
            ContentPart::image(IMAGE),
        ]),
        ChatMessage::assistant("Earlier reply"),
    ]
}

fn outcome<T: serde::Serialize>(r: contextual_core::Result<T>) -> Value {
    match r {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e.message }),
    }
}

fn recorded_requests(reqs: Vec<Request>) -> Vec<Value> {
    reqs.into_iter()
        .map(|r| {
            let header = |k: &str| r.headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
            let url = match r.url.query() {
                Some(q) => format!("{}?{q}", r.url.path()),
                None => r.url.path().to_string(),
            };
            json!({
                "method": r.method.to_string(),
                "url": url,
                "headers": { "api-key": header("api-key"), "authorization": header("authorization"), "content-type": header("content-type") },
                "body": serde_json::from_slice::<Value>(&r.body).unwrap_or(json!({})),
            })
        })
        .collect()
}

#[tokio::test]
async fn parity_with_node_server() {
    let server = azure_like().await;
    let uri = server.uri();
    let c = |d: &str| client(&uri, d);
    let image_input = vec![ContentPart::text("Look"), ContentPart::image(IMAGE)];
    let never = CancellationToken::new();

    let mut results = serde_json::Map::new();
    results.insert(
        "fresh".into(),
        chat(&c("gpt-resp"), req(vec![], image_input, None))
            .await
            .into(),
    );
    results.insert(
        "fullHistory".into(),
        chat(&c("gpt-resp"), req(history(), "Again", None))
            .await
            .into(),
    );
    results.insert(
        "chained".into(),
        chat(&c("gpt-resp"), req(history(), "Next", Some("resp_prev1")))
            .await
            .into(),
    );
    results.insert(
        "chainReset".into(),
        chat(
            &c("gpt-resp"),
            req(history(), "After expiry", Some("resp_gone")),
        )
        .await
        .into(),
    );
    results.insert(
        "fallback".into(),
        chat(&c("gpt-chat"), req(history(), "Hello", Some("resp_prev1")))
            .await
            .into(),
    );
    results.insert(
        "fallbackAgain".into(),
        chat(&c("gpt-chat"), req(vec![], "Hello again", None))
            .await
            .into(),
    );
    results.insert(
        "failed".into(),
        chat(&c("gpt-fail"), req(vec![], "x", None)).await.into(),
    );
    results.insert(
        "incomplete".into(),
        chat(&c("gpt-incomplete"), req(vec![], "x", None))
            .await
            .into(),
    );
    results.insert(
        "unauthorized".into(),
        chat(&c("gpt-401"), req(vec![], "x", None)).await.into(),
    );

    results.insert(
        "test".into(),
        outcome(
            c("gpt-resp")
                .test_connection(&never)
                .await
                .map(|_| "number"),
        ),
    );
    results.insert(
        "test401".into(),
        outcome(c("gpt-401").test_connection(&never).await),
    );
    results.insert(
        "draftCommit".into(),
        outcome(
            c("gpt-resp")
                .draft_commit_message("diff --git a/x b/x\n+1", &never)
                .await,
        ),
    );
    let pr = PrDraftInput {
        notes: String::new(),
        diff: "diff --git a/x b/x".into(),
        branch: "feature/x".into(),
        commits: "- Add x".into(),
    };
    results.insert(
        "draftPr".into(),
        outcome(c("gpt-resp").draft_pull_request(&pr, &never).await),
    );
    let repair = RepairInput {
        path: "src/a.ts".into(),
        original: "export const a = 1\n".into(),
        edits: "<<<<<<< SEARCH\na = 1\n=======\na = 2\n>>>>>>> REPLACE".into(),
    };
    results.insert(
        "repair".into(),
        outcome(c("gpt-resp").repair_file(&repair, &never).await),
    );
    let cut = RepairInput {
        path: "src/a.ts".into(),
        original: "x".into(),
        edits: "CUT".into(),
    };
    results.insert(
        "repairCut".into(),
        outcome(c("gpt-resp").repair_file(&cut, &never).await),
    );

    let expected: Value = serde_json::from_str(&file("expected.json")).unwrap();
    let requests = recorded_requests(server.received_requests().await.unwrap());
    let want_requests = expected["requests"].as_array().unwrap();
    assert_eq!(requests.len(), want_requests.len());
    for (i, (got, want)) in requests.iter().zip(want_requests).enumerate() {
        assert_eq!(got, want, "request {i}");
    }
    for (key, want) in expected["results"].as_object().unwrap() {
        assert_eq!(&results[key], want, "{key}");
    }
    assert_eq!(
        results.len(),
        expected["results"].as_object().unwrap().len()
    );
}

#[tokio::test]
async fn streams_deltas_usage_and_done_over_a_channel() {
    let server = azure_like().await;
    let c = client(&server.uri(), "gpt-resp");
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let task = tokio::spawn({
        let c = c.clone();
        async move {
            c.run_conversation(req(vec![], "hi", None), tx, CancellationToken::new())
                .await
        }
    });
    let mut events = Vec::new();
    while let Some(e) = rx.recv().await {
        events.push(e);
    }
    task.await.unwrap();
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Delta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        text,
        "Here is the change – café ☕:\n\n### FILE: src/a.ts\n```ts\nexport const a = 2\n```\n"
    );
    assert_eq!(
        events[events.len() - 2..],
        [
            ChatEvent::Usage {
                input_tokens: 5321,
                output_tokens: 187,
                cached_tokens: 4864
            },
            ChatEvent::Done {
                finish_reason: Some("stop".into()),
                response_id: Some("resp_68f1c2a9e4b0819089".into()),
                api: contextual_core::llm::ApiKind::Responses,
                chain_reset: false,
            }
        ]
    );
    assert_eq!(c.responses_supported(), Some(true));
}

#[tokio::test]
async fn falls_back_and_remembers() {
    let server = azure_like().await;
    let c = client(&server.uri(), "gpt-chat");
    chat(&c, req(vec![], "a", None)).await;
    assert_eq!(c.responses_supported(), Some(false));
    chat(&c, req(vec![], "b", None)).await;
    let paths: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect();
    assert_eq!(
        paths,
        [
            "/openai/v1/responses",
            "/openai/deployments/gpt-chat/chat/completions",
            "/openai/deployments/gpt-chat/chat/completions"
        ]
    );
}

#[tokio::test]
async fn cancelling_before_headers_ends_quietly() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(sse("responses-stream.sse").set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let c = client(&server.uri(), "gpt-slow");
    let cancel = CancellationToken::new();
    let stream = c.stream_conversation(req(vec![], "x", None), cancel.clone());
    let started = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
    });
    let events: Vec<ChatEvent> = stream.collect().await;
    assert!(events.is_empty(), "{events:?}");
    assert!(started.elapsed() < Duration::from_secs(5));

    let token = CancellationToken::new();
    token.cancel();
    let e = c.test_connection(&token).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Cancelled);
}

/// A server that sends headers and one delta, then keeps the connection open and reports when
/// the client goes away.
async fn hanging_server() -> (String, tokio::sync::oneshot::Receiver<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 64 * 1024];
        // Read the request head and body (small JSON).
        let mut seen = Vec::new();
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            seen.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&seen);
            if let Some(head_end) = text.find("\r\n\r\n") {
                let len = text[..head_end]
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().to_string())
                    })
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                if seen.len() >= head_end + 4 + len {
                    break;
                }
            }
        }
        let event = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n";
        let head =
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(event.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
        // Wait until the client closes the connection.
        loop {
            match sock.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let _ = closed_tx.send(());
    });
    (format!("http://{addr}"), closed_rx)
}

#[tokio::test]
async fn dropping_the_stream_aborts_the_request() {
    let (uri, closed) = hanging_server().await;
    let c = client(&uri, "gpt-hang");
    let mut stream = c.stream_conversation(req(vec![], "x", None), CancellationToken::new());
    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap();
    assert_eq!(first, Some(ChatEvent::Delta { text: "Hi".into() }));
    drop(stream);
    tokio::time::timeout(Duration::from_secs(5), closed)
        .await
        .expect("connection was not closed")
        .unwrap();
}

#[tokio::test]
async fn cancelling_mid_stream_aborts_the_request() {
    let (uri, closed) = hanging_server().await;
    let c = client(&uri, "gpt-hang2");
    let cancel = CancellationToken::new();
    let mut stream = c.stream_conversation(req(vec![], "x", None), cancel.clone());
    assert!(matches!(stream.next().await, Some(ChatEvent::Delta { .. })));
    cancel.cancel();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap(),
        None
    );
    tokio::time::timeout(Duration::from_secs(5), closed)
        .await
        .expect("connection was not closed")
        .unwrap();
}

#[tokio::test]
async fn retries_server_errors_like_the_sdk() {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after-ms", "10"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(any())
        .respond_with(AzureLike::new())
        .with_priority(2)
        .mount(&server)
        .await;
    let c = FoundryClient::new(cfg(&server.uri(), "gpt-resp")).unwrap();
    let ms = c.test_connection(&CancellationToken::new()).await.unwrap();
    assert!(ms < 5_000);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);

    // Without retries the 503 comes through, SDK-style.
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let c = client(&server.uri(), "gpt-resp");
    let e = c
        .test_connection(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(e.message, "Foundry returned 503: 503 status code (no body)");
}

#[tokio::test]
async fn error_mapping() {
    // Nothing listening.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let c = client(&format!("http://127.0.0.1:{port}"), "gpt-x");
    let e = c
        .test_connection(&CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(
        (e.code, e.message.as_str()),
        (
            ErrorCode::Upstream,
            "Could not reach Foundry: Connection error."
        )
    );
    let events = chat(&c, req(vec![], "x", None)).await;
    assert_eq!(
        events,
        [json!({"type":"error","message":"Could not reach Foundry: Connection error."})]
    );

    // Malformed SSE.
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("data: {not json\n\n", "text/event-stream"),
        )
        .mount(&server)
        .await;
    let events = chat(&client(&server.uri(), "gpt-bad"), req(vec![], "x", None)).await;
    assert_eq!(
        events,
        [
            json!({"type":"error","message":"Could not reach Foundry: Error reading response: malformed server-sent event JSON."})
        ]
    );

    // A mid-stream `error` event.
    let server = MockServer::start().await;
    let body = "event: error\ndata: {\"type\":\"error\",\"code\":\"rate_limit_exceeded\",\"message\":\"Rate limit reached\",\"param\":null}\n\n";
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    let events = chat(&client(&server.uri(), "gpt-err"), req(vec![], "x", None)).await;
    assert_eq!(
        events,
        [json!({"type":"error","message":"Could not reach Foundry: Rate limit reached"})]
    );

    assert_eq!(
        describe_error(&FoundryError::Status {
            status: 429,
            message: "429 Too many".into()
        }),
        (429, "Foundry returned 429: 429 Too many".to_string())
    );
    assert_eq!(
        describe_error(&FoundryError::Timeout).1,
        "Could not reach Foundry: Request timed out."
    );
}

#[tokio::test]
async fn image_detail_is_always_high() {
    let server = azure_like().await;
    let responses = client(&server.uri(), "gpt-images");
    // "gpt-chat" has no Responses API, so its turn goes to chat completions.
    let chat_only = client(&server.uri(), "gpt-chat");
    let parts = vec![ContentPart::ImageUrl {
        image_url: contextual_core::llm::ImageUrl {
            url: IMAGE.into(),
            detail: Some("low".into()),
        },
    }];
    chat(&chat_only, req(vec![], parts.clone(), None)).await;
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[1].body).unwrap();
    assert_eq!(
        body["messages"][1]["content"][0]["image_url"]["detail"],
        "high"
    );
    chat(&responses, req(vec![], parts, None)).await;
    let reqs = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[2].body).unwrap();
    assert_eq!(body["input"][0]["content"][0]["detail"], "high");
}
