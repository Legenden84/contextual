//! Conversation turns (port of `streamConversation`).
//!
//! Prefers the Responses API: Azure stores each reply (`store: true`) and the next request sends
//! only the new turn plus `previous_response_id`. If the stored chain is gone it retries once with
//! the full history and reports `chainReset`. If the deployment has no Responses API (404 /
//! unsupported) it uses chat completions with `[system, ...history, input]` from then on.
//!
//! When a request asks for it, the `propose_command` tool is declared and the calls of a
//! completed response are reported as [`ChatEvent::ToolCall`]. Calls and their results travel
//! in the transcript as native items in both APIs. Building a request from history — on a
//! first attempt, an HTTP retry, a chain reset or the chat fallback — only produces JSON:
//! this module cannot reach the execution engine.

use super::client::{
    Api, FoundryClient, FoundryError, describe_error, error_payload_message, truthy,
};
use super::sse::{SseDecoder, SseEvent};
use super::tools::{self, CallAssembler, Field};
use super::types::{
    ApiKind, ChatEvent, ChatMessage, ChatRole, ContentPart, ConversationRequest, MessageContent,
};
use futures::{Stream, StreamExt};
use regex::Regex;
use serde_json::{Value, json};
use std::pin::Pin;
use std::sync::LazyLock;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_util::sync::{CancellationToken, DropGuard};

static PREVIOUS_RESPONSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)previous[_ ]response").unwrap());
static UNSUPPORTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(api[- ]?version|not supported|unsupported|operation|responses)").unwrap()
});
static TOOL_SUBJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(tools?|tool_choice|parallel_tool_calls|functions?|function[ _-]?call(s|ing)?)\b",
    )
    .unwrap()
});
static TOOL_REFUSAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(not supported|unsupported|does(n't| not) support|not available|not enabled|not allowed|unknown parameter|unrecognized (request )?(argument|parameter))",
    )
    .unwrap()
});

/// The deployment refused the tool declaration itself. Deliberately narrow: the message
/// must name tools and refuse them. Checked only for requests that declared the tool, and
/// before [`is_unsupported`], so that a tool refusal is never mistaken for a missing
/// Responses API and silently downgraded.
pub fn is_tools_unsupported(err: &FoundryError) -> bool {
    let message = err.raw_message();
    matches!(err.status(), Some(400 | 422))
        && !is_missing_previous(err)
        && TOOL_SUBJECT.is_match(&message)
        && TOOL_REFUSAL.is_match(&message)
}

/// The stored previous response is gone (expired, deleted, or from another resource).
pub fn is_missing_previous(err: &FoundryError) -> bool {
    matches!(err.status(), Some(400 | 404)) && PREVIOUS_RESPONSE.is_match(&err.raw_message())
}

/// The deployment or API version doesn't offer the Responses API.
pub fn is_unsupported(err: &FoundryError) -> bool {
    !is_missing_previous(err)
        && match err.status() {
            Some(404) => true,
            Some(400) => UNSUPPORTED.is_match(&err.raw_message()),
            _ => false,
        }
}

/// Chat-completions messages, images always with `detail: "high"` (as the old server sent them).
fn chat_message(m: &ChatMessage) -> Value {
    let role = match m.role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => {
            return json!({
                "role": "tool",
                "tool_call_id": m.tool_call_id.as_deref().unwrap_or(""),
                "content": chat_content(&m.content),
            });
        }
    };
    if m.tool_calls.is_empty() {
        return json!({ "role": role, "content": chat_content(&m.content) });
    }
    // An assistant message that issued calls. Empty text is sent as null, not as prose.
    let content = match &m.content {
        MessageContent::Text(text) if text.is_empty() => Value::Null,
        other => chat_content(other),
    };
    json!({
        "role": role,
        "content": content,
        "tool_calls": m.tool_calls.iter().map(|call| json!({
            "id": call.call_id,
            "type": "function",
            "function": { "name": call.name, "arguments": call.replayed_arguments() },
        })).collect::<Vec<_>>(),
    })
}

fn chat_content(c: &MessageContent) -> Value {
    match c {
        MessageContent::Text(s) => Value::String(s.clone()),
        MessageContent::Parts(parts) => Value::Array(
            parts
                .iter()
                .map(|p| match p {
                    ContentPart::Text { text } => json!({ "type": "text", "text": text }),
                    ContentPart::ImageUrl { image_url } => {
                        json!({ "type": "image_url", "image_url": { "url": image_url.url, "detail": "high" } })
                    }
                })
                .collect(),
        ),
    }
}

/// Responses API input items (system messages dropped; they go in `instructions`).
fn responses_input(messages: &[ChatMessage]) -> Value {
    let mut items = Vec::new();
    for m in messages.iter().filter(|m| m.role != ChatRole::System) {
        match (&m.role, &m.content) {
            (ChatRole::Tool, c) => items.push(json!({
                "type": "function_call_output",
                "call_id": m.tool_call_id.as_deref().unwrap_or(""),
                "output": text_of(c),
            })),
            (ChatRole::Assistant, c) => {
                // A tool-only reply has no text item at all.
                let silent = matches!(c, MessageContent::Text(text) if text.is_empty());
                if m.tool_calls.is_empty() || !silent {
                    items.push(json!({ "role": "assistant", "content": text_of(c) }));
                }
                items.extend(m.tool_calls.iter().map(|call| {
                    json!({
                        "type": "function_call",
                        "call_id": call.call_id,
                        "name": call.name,
                        "arguments": call.replayed_arguments(),
                    })
                }));
            }
            (_, MessageContent::Text(s)) => items.push(json!({ "role": "user", "content": s })),
            (_, MessageContent::Parts(parts)) => items.push(json!({
                "role": "user",
                "content": parts.iter().map(|p| match p {
                    ContentPart::Text { text } => json!({ "type": "input_text", "text": text }),
                    ContentPart::ImageUrl { image_url } =>
                        json!({ "type": "input_image", "image_url": image_url.url, "detail": "high" }),
                }).collect::<Vec<_>>(),
            })),
        }
    }
    Value::Array(items)
}

/// What this request adds to the conversation: answers to the last calls, then the user's
/// message. With answers and no text, there is no user message.
fn new_turn(req: &ConversationRequest) -> Vec<ChatMessage> {
    let mut turn: Vec<ChatMessage> = tools::result_messages(req).collect();
    let silent = matches!(&req.input, MessageContent::Text(text) if text.is_empty());
    if turn.is_empty() || !silent {
        turn.push(ChatMessage::user(req.input.clone()));
    }
    turn
}

/// Whether the tool is declared, and whether the model may call it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolOffer {
    /// Not declared. Today's request bodies, byte for byte.
    None,
    /// Declared so that calls in the history stay valid, but calling is not allowed.
    HistoryOnly,
    /// Declared and callable.
    Offered,
}

impl ToolOffer {
    fn callable(self) -> bool {
        self == Self::Offered
    }
}

fn text_of(c: &MessageContent) -> Value {
    match c {
        MessageContent::Text(s) => Value::String(s.clone()),
        MessageContent::Parts(_) => chat_content(c),
    }
}

/// Request body for the Responses API.
pub fn responses_body(
    client: &FoundryClient,
    req: &ConversationRequest,
    previous: Option<&str>,
    offer: ToolOffer,
) -> Value {
    let input = new_turn(req);
    let turns: Vec<ChatMessage> = match previous {
        Some(_) => input,
        None => req.history.iter().cloned().chain(input).collect(),
    };
    let mut body = json!({
        "model": client.cfg.deployment,
        "instructions": req.system,
        "input": responses_input(&turns),
        "store": true,
        "stream": true,
        "max_output_tokens": client.cfg.max_completion_tokens,
    });
    if let Some(p) = previous {
        body["previous_response_id"] = Value::String(p.to_string());
    }
    if offer != ToolOffer::None {
        body["tools"] = json!([tools::responses_tool()]);
        if !offer.callable() {
            body["tool_choice"] = json!("none");
        }
    }
    body
}

/// Request body for streamed chat completions.
pub fn chat_body(client: &FoundryClient, req: &ConversationRequest, offer: ToolOffer) -> Value {
    let mut messages = vec![chat_message(&ChatMessage::system(req.system.clone()))];
    messages.extend(req.history.iter().map(chat_message));
    messages.extend(new_turn(req).iter().map(chat_message));
    let mut body = json!({
        "model": client.cfg.deployment,
        "messages": messages,
        "max_completion_tokens": client.cfg.max_completion_tokens,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if offer != ToolOffer::None {
        body["tools"] = json!([tools::chat_tool()]);
        if !offer.callable() {
            body["tool_choice"] = json!("none");
        }
    }
    body
}

/// Reports the calls of a response that has ended, usable or not.
async fn send_calls(
    tx: &mpsc::Sender<ChatEvent>,
    calls: Vec<tools::ToolCall>,
) -> Result<(), FoundryError> {
    for call in calls {
        let event = ChatEvent::ToolCall {
            call_id: call.call_id,
            name: call.name,
            arguments: call.arguments,
            problem: call.problem,
        };
        send(tx, event).await?;
    }
    Ok(())
}

fn tools_unsupported_notice() -> ChatEvent {
    ChatEvent::ToolsUnsupported {
        message: "This deployment does not accept tool declarations, so the assistant cannot \
                  propose commands here. Anything it writes about running a command is text only."
            .into(),
    }
}

/// Assembles one Responses `function_call` item, from a streamed event or from the
/// completed response. Values that disagree with earlier ones make the call ambiguous.
fn confirm_function_call(
    calls: &mut CallAssembler,
    key: &str,
    item: &Value,
) -> Result<(), FoundryError> {
    for (field, name) in [(Field::CallId, "call_id"), (Field::Name, "name")] {
        if let Some(value) = item[name].as_str() {
            calls.confirm(key, field, value)?;
        }
    }
    // An item is first announced with empty arguments; that is not a value to compare.
    match item["arguments"].as_str() {
        Some(arguments) if !arguments.is_empty() => calls.confirm(key, Field::Arguments, arguments),
        _ if !calls.knows(key) => calls.append(key, Field::Arguments, ""),
        _ => Ok(()),
    }
}

/// Sends an event; `false` when the receiver is gone (treated as cancellation).
async fn emit(tx: &mpsc::Sender<ChatEvent>, ev: ChatEvent) -> bool {
    tx.send(ev).await.is_ok()
}

/// Pulls JSON events out of an SSE body, applying the SDK's rules: stop at `[DONE]`; an `error`
/// event or a payload with a truthy `error` fails the stream.
struct SseReader {
    body: Pin<Box<dyn Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    decoder: SseDecoder,
    pending: std::collections::VecDeque<SseEvent>,
    finished: bool,
    /// The stream ended with its `[DONE]` marker rather than by the connection closing.
    marked_done: bool,
}

impl SseReader {
    fn new(resp: reqwest::Response) -> Self {
        Self {
            body: Box::pin(resp.bytes_stream()),
            decoder: SseDecoder::new(),
            pending: Default::default(),
            finished: false,
            marked_done: false,
        }
    }

    async fn next(&mut self, cancel: &CancellationToken) -> Result<Option<Value>, FoundryError> {
        loop {
            if let Some(sse) = self.pending.pop_front() {
                if sse.data == "[DONE]" {
                    self.pending.clear();
                    self.finished = true;
                    self.marked_done = true;
                    return Ok(None);
                }
                let data: Value = serde_json::from_str(&sse.data).map_err(|_| {
                    FoundryError::Failed(
                        "Error reading response: malformed server-sent event JSON.".into(),
                    )
                })?;
                if sse.event.as_deref() == Some("error") {
                    let err = data.get("error").filter(|e| truthy(e)).unwrap_or(&data);
                    return Err(FoundryError::Failed(error_payload_message(err)));
                }
                if let Some(err) = data.get("error").filter(|e| truthy(e)) {
                    return Err(FoundryError::Failed(error_payload_message(err)));
                }
                return Ok(Some(data));
            }
            if self.finished {
                return Ok(None);
            }
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(FoundryError::Cancelled),
                c = self.body.next() => c,
            };
            match chunk {
                Some(Ok(bytes)) => self.pending.extend(self.decoder.push(&bytes)),
                Some(Err(e)) => {
                    return Err(FoundryError::Failed(format!(
                        "The response stream broke off: {e}"
                    )));
                }
                None => {
                    self.pending.extend(self.decoder.finish());
                    self.finished = true;
                }
            }
        }
    }
}

/// `Err(Cancelled)` when the receiver is gone.
async fn send(tx: &mpsc::Sender<ChatEvent>, ev: ChatEvent) -> Result<(), FoundryError> {
    if emit(tx, ev).await {
        Ok(())
    } else {
        Err(FoundryError::Cancelled)
    }
}

impl FoundryClient {
    async fn start_responses(
        &self,
        req: &ConversationRequest,
        previous: Option<&str>,
        offer: ToolOffer,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, FoundryError> {
        let body = responses_body(self, req, previous, offer);
        self.post(Api::Responses, &body, cancel).await
    }

    async fn pump_responses(
        &self,
        resp: reqwest::Response,
        chain_reset: bool,
        offer: ToolOffer,
        tx: &mpsc::Sender<ChatEvent>,
        cancel: &CancellationToken,
    ) -> Result<(), FoundryError> {
        let mut response_id: Option<String> = None;
        let mut finish_reason: Option<String> = None;
        let mut usage: Option<Value> = None;
        // Function-call items by output item identity. They become final only when the
        // whole response completes; an arguments-done event alone proves nothing.
        let mut calls = CallAssembler::default();
        let mut completed = false;
        let mut reader = SseReader::new(resp);
        while let Some(ev) = reader.next(cancel).await? {
            let id_of = |ev: &Value| ev["response"]["id"].as_str().map(str::to_string);
            match ev["type"].as_str().unwrap_or("") {
                t @ ("response.output_item.added" | "response.output_item.done")
                    if ev["item"]["type"] == "function_call" =>
                {
                    let item = &ev["item"];
                    if let Some(key) = item["id"].as_str() {
                        confirm_function_call(&mut calls, key, item)?;
                        if t == "response.output_item.done" {
                            calls.close(key)?;
                        }
                    }
                }
                "response.function_call_arguments.delta" => {
                    if let (Some(key), Some(delta)) = (ev["item_id"].as_str(), ev["delta"].as_str())
                    {
                        calls.append(key, Field::Arguments, delta)?;
                    }
                }
                "response.function_call_arguments.done" => {
                    if let (Some(key), Some(arguments)) =
                        (ev["item_id"].as_str(), ev["arguments"].as_str())
                    {
                        calls.confirm(key, Field::Arguments, arguments)?;
                    }
                }
                "response.output_text.delta" => {
                    let text = ev["delta"].as_str().unwrap_or("").to_string();
                    send(tx, ChatEvent::Delta { text }).await?;
                }
                "response.created" => response_id = id_of(&ev),
                t @ ("response.completed" | "response.incomplete") => {
                    if t == "response.completed" {
                        completed = true;
                        // The completed response lists its output. Every function call in
                        // it must agree with what was streamed, and nothing streamed may be
                        // missing from it.
                        if let Some(output) = ev["response"]["output"].as_array() {
                            let mut listed = Vec::new();
                            for item in output.iter().filter(|i| i["type"] == "function_call") {
                                if let Some(key) = item["id"].as_str() {
                                    confirm_function_call(&mut calls, key, item)?;
                                    calls.close(key)?;
                                    listed.push(key.to_owned());
                                }
                            }
                            calls.flag_unlisted(&listed);
                        }
                    }
                    response_id = id_of(&ev);
                    usage = ev["response"]
                        .get("usage")
                        .filter(|u| !u.is_null())
                        .cloned();
                    let reason = ev["response"]["incomplete_details"]["reason"].as_str();
                    finish_reason = Some(if t == "response.completed" {
                        "stop".into()
                    } else if reason == Some("max_output_tokens") {
                        "length".into()
                    } else {
                        reason.unwrap_or("incomplete").into()
                    });
                }
                "response.failed" => {
                    let msg = ev["response"]["error"]["message"]
                        .as_str()
                        .unwrap_or("The model failed to respond.");
                    return Err(FoundryError::Failed(msg.to_string()));
                }
                "error" => {
                    return Err(FoundryError::Failed(
                        ev["message"].as_str().unwrap_or("").to_string(),
                    ));
                }
                _ => {}
            }
        }
        send_calls(tx, calls.finish(completed, offer.callable(), true)).await?;
        if let Some(u) = usage {
            let n = |v: &Value| v.as_u64().unwrap_or(0);
            let ev = ChatEvent::Usage {
                input_tokens: n(&u["input_tokens"]),
                output_tokens: n(&u["output_tokens"]),
                cached_tokens: n(&u["input_tokens_details"]["cached_tokens"]),
            };
            send(tx, ev).await?;
        }
        send(
            tx,
            ChatEvent::Done {
                finish_reason,
                response_id,
                api: ApiKind::Responses,
                chain_reset,
            },
        )
        .await
    }

    async fn stream_chat(
        &self,
        req: &ConversationRequest,
        offer: ToolOffer,
        tx: &mpsc::Sender<ChatEvent>,
        cancel: &CancellationToken,
    ) -> Result<(), FoundryError> {
        let body = chat_body(self, req, offer);
        let resp = self.post(Api::Chat, &body, cancel).await?;
        let mut finish_reason: Option<String> = None;
        // Indexed `delta.tool_calls`, whose identity, name and arguments can each arrive
        // in pieces across chunks.
        let mut calls = CallAssembler::default();
        let mut reader = SseReader::new(resp);
        while let Some(chunk) = reader.next(cancel).await? {
            let choice = &chunk["choices"][0];
            for delta in choice["delta"]["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let Some(index) = delta["index"].as_u64() else {
                    continue;
                };
                let key = index.to_string();
                for (field, value) in [
                    (Field::CallId, &delta["id"]),
                    (Field::Name, &delta["function"]["name"]),
                    (Field::Arguments, &delta["function"]["arguments"]),
                ] {
                    // Touch the slot even for an empty fragment, so that the call exists.
                    calls.append(&key, field, value.as_str().unwrap_or(""))?;
                }
            }
            if let Some(text) = choice["delta"]["content"]
                .as_str()
                .filter(|t| !t.is_empty())
            {
                send(
                    tx,
                    ChatEvent::Delta {
                        text: text.to_string(),
                    },
                )
                .await?;
            }
            if let Some(r) = choice["finish_reason"].as_str().filter(|r| !r.is_empty()) {
                finish_reason = Some(r.to_string());
            }
            let usage = &chunk["usage"];
            if usage.is_object() {
                let n = |v: &Value| v.as_u64().unwrap_or(0);
                let ev = ChatEvent::Usage {
                    input_tokens: n(&usage["prompt_tokens"]),
                    output_tokens: n(&usage["completion_tokens"]),
                    cached_tokens: n(&usage["prompt_tokens_details"]["cached_tokens"]),
                };
                send(tx, ev).await?;
            }
        }
        // Calls are final only when the model stopped in order to call tools and the
        // stream ended with its marker, not because the connection closed.
        let completed = finish_reason.as_deref() == Some("tool_calls") && reader.marked_done;
        if !calls.is_empty() {
            send_calls(tx, calls.finish(completed, offer.callable(), false)).await?;
        }
        let done = ChatEvent::Done {
            finish_reason,
            response_id: None,
            api: ApiKind::Chat,
            chain_reset: req.previous_response_id.is_some(),
        };
        send(tx, done).await
    }

    async fn conversation(
        &self,
        req: &ConversationRequest,
        tx: &mpsc::Sender<ChatEvent>,
        cancel: &CancellationToken,
    ) -> Result<(), FoundryError> {
        if cancel.is_cancelled() {
            return Err(FoundryError::Cancelled);
        }
        // Call/result correlation is checked before anything leaves this process.
        tools::validate_transcript(req)?;
        // Count the whole logical conversation before choosing either transport. A stored
        // response ID, expired-chain retry or fallback does not make history free. The tool
        // definition, calls and results are part of that count.
        self.check_input_budget(req.estimated_input_tokens(), self.cfg.max_completion_tokens)?;

        let has_history = tools::has_tool_items(req);
        let mut offer = if req.command_tool && self.tools_supported() != Some(false) {
            ToolOffer::Offered
        } else if has_history {
            ToolOffer::HistoryOnly
        } else {
            ToolOffer::None
        };
        if req.command_tool && offer != ToolOffer::Offered {
            send(tx, tools_unsupported_notice()).await?;
        }
        loop {
            match self.attempt(req, offer, tx, cancel).await {
                Err(error) if offer != ToolOffer::None && is_tools_unsupported(&error) => {
                    // Learned separately from Responses support, and never silently.
                    self.set_tools_supported(false);
                    tracing::warn!(
                        "Tool declarations refused ({}); continuing without command proposals.",
                        error.raw_message()
                    );
                    if has_history {
                        // The conversation already contains calls, which cannot be sent
                        // to a deployment that refuses tools.
                        return Err(FoundryError::Rejected(
                            "This deployment no longer accepts tool declarations, and the \
                             conversation contains command proposals. Start a new chat."
                                .into(),
                        ));
                    }
                    send(tx, tools_unsupported_notice()).await?;
                    offer = ToolOffer::None;
                }
                result => return result,
            }
        }
    }

    /// One pass over the transports with a fixed tool declaration.
    async fn attempt(
        &self,
        req: &ConversationRequest,
        offer: ToolOffer,
        tx: &mpsc::Sender<ChatEvent>,
        cancel: &CancellationToken,
    ) -> Result<(), FoundryError> {
        let declared = offer != ToolOffer::None;
        if self.responses_supported() != Some(false) {
            let previous = req.previous_response_id.as_deref();
            let started = match self.start_responses(req, previous, offer, cancel).await {
                Ok(resp) => Some((resp, false)),
                Err(FoundryError::Cancelled) => return Err(FoundryError::Cancelled),
                Err(e) if previous.is_some() && is_missing_previous(&e) => {
                    // The whole history is rebuilt as data, including earlier calls and
                    // their results. Nothing in it is work to perform.
                    Some((self.start_responses(req, None, offer, cancel).await?, true))
                }
                // A refusal of the tool declaration is not a missing Responses API.
                Err(e) if declared && is_tools_unsupported(&e) => return Err(e),
                Err(e) if is_unsupported(&e) => {
                    self.set_responses_supported(false);
                    tracing::warn!(
                        "Responses API not available ({}); using chat completions.",
                        e.raw_message()
                    );
                    None
                }
                Err(e) => return Err(e),
            };
            if let Some((resp, chain_reset)) = started {
                self.set_responses_supported(true);
                if offer.callable() {
                    self.set_tools_supported(true);
                }
                return self
                    .pump_responses(resp, chain_reset, offer, tx, cancel)
                    .await;
            }
        }
        self.stream_chat(req, offer, tx, cancel).await
    }

    /// Runs one conversation turn, sending [`ChatEvent`]s to `tx`: deltas, then usage, then
    /// done — or a final `Error` event. Nothing more is sent once `cancel` fires or `tx`'s
    /// receiver is dropped; the HTTP request is dropped (aborted) then.
    pub async fn run_conversation(
        &self,
        req: ConversationRequest,
        tx: mpsc::Sender<ChatEvent>,
        cancel: CancellationToken,
    ) {
        match self.conversation(&req, &tx, &cancel).await {
            Ok(()) | Err(FoundryError::Cancelled) => {}
            Err(_) if cancel.is_cancelled() => {}
            Err(e) => {
                let _ = tx
                    .send(ChatEvent::Error {
                        message: describe_error(&e).1,
                    })
                    .await;
            }
        }
    }

    /// [`Self::run_conversation`] as a [`Stream`]. Spawns a task on the current tokio runtime
    /// (call it from inside one). Dropping the stream cancels the turn and aborts the request;
    /// cancelling `cancel` ends the stream.
    pub fn stream_conversation(
        &self,
        req: ConversationRequest,
        cancel: CancellationToken,
    ) -> ChatStream {
        let (tx, rx) = mpsc::channel(64);
        let child = cancel.child_token();
        let client = self.clone();
        let token = child.clone();
        tokio::spawn(async move { client.run_conversation(req, tx, token).await });
        ChatStream {
            rx,
            _guard: child.drop_guard(),
        }
    }
}

/// Stream of [`ChatEvent`]s for one turn; dropping it cancels the turn.
pub struct ChatStream {
    rx: mpsc::Receiver<ChatEvent>,
    _guard: DropGuard,
}

impl Stream for ChatStream {
    type Item = ChatEvent;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ChatEvent>> {
        self.rx.poll_recv(cx)
    }
}

impl std::fmt::Debug for ChatStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatStream").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_errors() {
        let missing = FoundryError::Status {
            status: 400,
            message: "400 Previous response with id 'resp_1' not found.".into(),
        };
        assert!(is_missing_previous(&missing));
        assert!(!is_unsupported(&missing));
        let nf = FoundryError::Status {
            status: 404,
            message: "404 Resource not found".into(),
        };
        assert!(is_unsupported(&nf) && !is_missing_previous(&nf));
        let bad = FoundryError::Status {
            status: 400,
            message: "400 The API version is not supported".into(),
        };
        assert!(is_unsupported(&bad));
        let other = FoundryError::Status {
            status: 400,
            message: "400 Invalid value for max_output_tokens".into(),
        };
        assert!(!is_unsupported(&other));
    }
}
