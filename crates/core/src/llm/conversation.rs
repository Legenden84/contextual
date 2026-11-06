//! Conversation turns (port of `streamConversation`).
//!
//! Prefers the Responses API: Azure stores each reply (`store: true`) and the next request sends
//! only the new turn plus `previous_response_id`. If the stored chain is gone it retries once with
//! the full history and reports `chainReset`. If the deployment has no Responses API (404 /
//! unsupported) it uses chat completions with `[system, ...history, input]` from then on.

use super::client::{
    Api, FoundryClient, FoundryError, describe_error, error_payload_message, truthy,
};
use super::sse::{SseDecoder, SseEvent};
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
    };
    json!({ "role": role, "content": chat_content(&m.content) })
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
    Value::Array(
        messages
            .iter()
            .filter(|m| m.role != ChatRole::System)
            .map(|m| match (&m.role, &m.content) {
                (ChatRole::Assistant, c) => json!({ "role": "assistant", "content": text_of(c) }),
                (_, MessageContent::Text(s)) => json!({ "role": "user", "content": s }),
                (_, MessageContent::Parts(parts)) => json!({
                    "role": "user",
                    "content": parts.iter().map(|p| match p {
                        ContentPart::Text { text } => json!({ "type": "input_text", "text": text }),
                        ContentPart::ImageUrl { image_url } =>
                            json!({ "type": "input_image", "image_url": image_url.url, "detail": "high" }),
                    }).collect::<Vec<_>>(),
                }),
            })
            .collect(),
    )
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
) -> Value {
    let input = ChatMessage {
        role: ChatRole::User,
        content: req.input.clone(),
    };
    let turns: Vec<ChatMessage> = match previous {
        Some(_) => vec![input],
        None => req
            .history
            .iter()
            .cloned()
            .chain(std::iter::once(input))
            .collect(),
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
    body
}

/// Request body for streamed chat completions.
pub fn chat_body(client: &FoundryClient, req: &ConversationRequest) -> Value {
    let mut messages = vec![chat_message(&ChatMessage::system(req.system.clone()))];
    messages.extend(req.history.iter().map(chat_message));
    messages.push(chat_message(&ChatMessage {
        role: ChatRole::User,
        content: req.input.clone(),
    }));
    json!({
        "model": client.cfg.deployment,
        "messages": messages,
        "max_completion_tokens": client.cfg.max_completion_tokens,
        "stream": true,
        "stream_options": { "include_usage": true },
    })
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
}

impl SseReader {
    fn new(resp: reqwest::Response) -> Self {
        Self {
            body: Box::pin(resp.bytes_stream()),
            decoder: SseDecoder::new(),
            pending: Default::default(),
            finished: false,
        }
    }

    async fn next(&mut self, cancel: &CancellationToken) -> Result<Option<Value>, FoundryError> {
        loop {
            if let Some(sse) = self.pending.pop_front() {
                if sse.data == "[DONE]" {
                    self.pending.clear();
                    self.finished = true;
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
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, FoundryError> {
        let body = responses_body(self, req, previous);
        self.post(Api::Responses, &body, cancel).await
    }

    async fn pump_responses(
        &self,
        resp: reqwest::Response,
        chain_reset: bool,
        tx: &mpsc::Sender<ChatEvent>,
        cancel: &CancellationToken,
    ) -> Result<(), FoundryError> {
        let mut response_id: Option<String> = None;
        let mut finish_reason: Option<String> = None;
        let mut usage: Option<Value> = None;
        let mut reader = SseReader::new(resp);
        while let Some(ev) = reader.next(cancel).await? {
            let id_of = |ev: &Value| ev["response"]["id"].as_str().map(str::to_string);
            match ev["type"].as_str().unwrap_or("") {
                "response.output_text.delta" => {
                    let text = ev["delta"].as_str().unwrap_or("").to_string();
                    send(tx, ChatEvent::Delta { text }).await?;
                }
                "response.created" => response_id = id_of(&ev),
                t @ ("response.completed" | "response.incomplete") => {
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
        tx: &mpsc::Sender<ChatEvent>,
        cancel: &CancellationToken,
    ) -> Result<(), FoundryError> {
        let body = chat_body(self, req);
        let resp = self.post(Api::Chat, &body, cancel).await?;
        let mut finish_reason: Option<String> = None;
        let mut reader = SseReader::new(resp);
        while let Some(chunk) = reader.next(cancel).await? {
            let choice = &chunk["choices"][0];
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
        if self.responses_supported() != Some(false) {
            let previous = req.previous_response_id.as_deref();
            let started = match self.start_responses(req, previous, cancel).await {
                Ok(resp) => Some((resp, false)),
                Err(FoundryError::Cancelled) => return Err(FoundryError::Cancelled),
                Err(e) if previous.is_some() && is_missing_previous(&e) => {
                    Some((self.start_responses(req, None, cancel).await?, true))
                }
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
                return self.pump_responses(resp, chain_reset, tx, cancel).await;
            }
        }
        self.stream_chat(req, tx, cancel).await
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
