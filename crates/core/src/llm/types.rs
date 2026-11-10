//! Conversation types (wire-compatible with the old `/api/chat` request and NDJSON events).

use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: MessageContent,
}

impl ChatMessage {
    /// Estimated content plus message framing.
    pub fn estimated_tokens(&self) -> u64 {
        self.content
            .estimated_tokens()
            .saturating_add(MESSAGE_OVERHEAD_TOKENS)
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: MessageContent::Text(content.into()),
        }
    }
    pub fn user(content: impl Into<MessageContent>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: MessageContent::Text(content.into()),
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
}

impl ConversationRequest {
    /// The entire logical input, not just the incremental Responses API HTTP payload.
    pub fn estimated_input_tokens(&self) -> u64 {
        let current = estimate_tokens(&self.system)
            .saturating_add(self.input.estimated_tokens())
            // System message, current user message, and assistant reply prefix.
            .saturating_add(3 * MESSAGE_OVERHEAD_TOKENS);
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
