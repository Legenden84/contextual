//! Conversation types (wire-compatible with the old `/api/chat` request and NDJSON events).

use serde::{Deserialize, Serialize};

use super::pacing::WaitReason;
use super::tools::{ToolCall, ToolResult, definition_tokens};
use crate::logic::context::PICTURE_TOKENS;
use crate::logic::format::estimate_tokens;

/// Estimated role/message framing, also used for the assistant reply prefix.
/// Provider-specific framing and vision accounting still require a safety margin.
pub(crate) const MESSAGE_OVERHEAD_TOKENS: u64 = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Text, or a picture (user messages only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    ImageUrl { image_url: ImageUrl },
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
    /// A `data:image/…;base64,…` URL, sent with `detail: "high"` like the old server.
    pub fn image(url: impl Into<String>) -> Self {
        Self::ImageUrl {
            image_url: ImageUrl {
                url: url.into(),
                detail: Some("high".into()),
            },
        }
    }
}

/// A string or text-and-image parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    /// Estimates content without copying text or counting image data URLs as prose.
    pub fn estimated_tokens(&self) -> u64 {
        match self {
            Self::Text(text) => estimate_tokens(text),
            Self::Parts(parts) => parts.iter().fold(0u64, |total, part| {
                total.saturating_add(match part {
                    ContentPart::Text { text } => estimate_tokens(text),
                    ContentPart::ImageUrl { .. } => PICTURE_TOKENS,
                })
            }),
        }
    }
}

impl From<&str> for MessageContent {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}
impl From<String> for MessageContent {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}
impl From<Vec<ContentPart>> for MessageContent {
    fn from(p: Vec<ContentPart>) -> Self {
        Self::Parts(p)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
    /// The answer to one tool call of the preceding assistant message.
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: MessageContent,
    /// Calls the assistant issued in this message. History data, never work to perform.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For a `Tool` message: the call it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    /// Estimated content plus message framing, including any calls.
    pub fn estimated_tokens(&self) -> u64 {
        self.tool_calls.iter().fold(
            self.content
                .estimated_tokens()
                .saturating_add(MESSAGE_OVERHEAD_TOKENS),
            |total, call| total.saturating_add(call.estimated_tokens()),
        )
    }

    fn plain(role: ChatRole, content: MessageContent) -> Self {
        Self {
            role,
            content,
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::plain(ChatRole::System, MessageContent::Text(content.into()))
    }
    pub fn user(content: impl Into<MessageContent>) -> Self {
        Self::plain(ChatRole::User, content.into())
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain(ChatRole::Assistant, MessageContent::Text(content.into()))
    }
    /// An assistant message that issued calls; `text` may be empty for a tool-only reply.
    pub fn assistant_calls(text: impl Into<String>, calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls: calls,
            ..Self::assistant(text)
        }
    }
    /// The answer to one call.
    pub fn tool(call_id: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(call_id.into()),
            ..Self::plain(ChatRole::Tool, MessageContent::Text(output.into()))
        }
    }
}

/// One turn of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRequest {
    /// Rules + output format; sent with every request.
    pub system: String,
    /// The earlier turns exactly as sent (user) and received (assistant).
    /// Required for budget accounting even when `previous_response_id` is present.
    #[serde(default)]
    pub history: Vec<ChatMessage>,
    /// The new user turn.
    pub input: MessageContent,
    /// Last stored Azure response of this conversation, if any.
    #[serde(default)]
    pub previous_response_id: Option<String>,
    /// Offer the `propose_command` tool in this turn. Off by default.
    #[serde(default)]
    pub command_tool: bool,
    /// Answers to the calls of the last assistant message in `history`, sent before `input`.
    /// When they are present and `input` is empty text, no user message is sent.
    #[serde(default)]
    pub tool_results: Vec<ToolResult>,
}

impl ConversationRequest {
    /// The entire logical input, not just the incremental Responses API HTTP payload.
    pub fn estimated_input_tokens(&self) -> u64 {
        let current = estimate_tokens(&self.system)
            .saturating_add(self.input.estimated_tokens())
            // System message, current user message, and assistant reply prefix.
            .saturating_add(3 * MESSAGE_OVERHEAD_TOKENS);
        let current = if self.command_tool {
            current.saturating_add(definition_tokens())
        } else {
            current
        };
        let current = self.tool_results.iter().fold(current, |total, result| {
            total.saturating_add(result.estimated_tokens())
        });
        self.history.iter().fold(current, |total, message| {
            total.saturating_add(message.estimated_tokens())
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiKind {
    Responses,
    Chat,
}

/// Streamed conversation events; serialises like the old NDJSON lines (`{"type":"delta",…}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ChatEvent {
    Delta {
        text: String,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
    },
    /// One call from a response that has ended. Sent before `Usage` and `Done`, in the
    /// provider's order. A call with a `problem` can never become a proposal, but it still
    /// has to be answered in the transcript. Receiving this event runs nothing.
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
        problem: Option<String>,
    },
    /// The deployment refused the tool declaration. The turn continues without it; text
    /// that resembles a command request stays inert.
    ToolsUnsupported {
        message: String,
    },
    /// The request is held for about `seconds` before it is sent (again); the turn goes on
    /// by itself afterwards. Sent again if the wait is extended.
    Waiting {
        seconds: u64,
        reason: WaitReason,
    },
    Done {
        finish_reason: Option<String>,
        response_id: Option<String>,
        api: ApiKind,
        chain_reset: bool,
    },
    Error {
        message: String,
    },
}

/// Inputs for a model-written PR title and description.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrDraftInput {
    pub notes: String,
    pub diff: String,
    pub branch: String,
    pub commits: String,
}

/// Inputs for rebuilding a file whose edit blocks couldn't be placed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairInput {
    pub path: String,
    pub original: String,
    pub edits: String,
}

/// `/api/edit/repair` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    pub content: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_serialise_like_ndjson() {
        let e = ChatEvent::Done {
            finish_reason: None,
            response_id: Some("r".into()),
            api: ApiKind::Chat,
            chain_reset: true,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"type":"done","finishReason":null,"responseId":"r","api":"chat","chainReset":true})
        );
        let u = ChatEvent::Usage {
            input_tokens: 1,
            output_tokens: 2,
            cached_tokens: 3,
        };
        assert_eq!(
            serde_json::to_value(&u).unwrap(),
            json!({"type":"usage","inputTokens":1,"outputTokens":2,"cachedTokens":3})
        );
    }

    #[test]
    fn input_estimate_counts_instructions_history_and_framing() {
        let request = ConversationRequest {
            system: "abcd".into(),
            history: vec![ChatMessage::user("abcd"), ChatMessage::assistant("abcd")],
            input: "abcd".into(),
            previous_response_id: Some("stored-response".into()),
            command_tool: false,
            tool_results: Vec::new(),
        };
        assert_eq!(
            request.estimated_input_tokens(),
            4 + 5 * MESSAGE_OVERHEAD_TOKENS
        );
    }

    #[test]
    fn stored_response_id_does_not_reduce_the_input_estimate() {
        let mut request = ConversationRequest {
            system: "Rules".into(),
            history: vec![ChatMessage::user("An earlier file")],
            input: "Continue".into(),
            previous_response_id: None,
            command_tool: false,
            tool_results: Vec::new(),
        };
        let full = request.estimated_input_tokens();
        request.previous_response_id = Some("stored-response".into());
        assert_eq!(request.estimated_input_tokens(), full);
    }

    #[test]
    fn multipart_estimate_counts_pictures_without_counting_base64_as_text() {
        let content = MessageContent::Parts(vec![
            ContentPart::text("abcd"),
            ContentPart::image(format!("data:image/png;base64,{}", "A".repeat(100_000))),
            ContentPart::image("https://example.com/reference.png"),
        ]);
        assert_eq!(content.estimated_tokens(), 1 + 2 * PICTURE_TOKENS);
    }

    #[test]
    fn request_parses() {
        let r: ConversationRequest = serde_json::from_value(json!({
            "system": "s",
            "history": [{"role":"user","content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA"}}]},
                        {"role":"assistant","content":"b"}],
            "input": "c",
            "previousResponseId": null
        }))
        .unwrap();
        assert_eq!(r.history.len(), 2);
        assert_eq!(r.input, MessageContent::Text("c".into()));
    }
}
