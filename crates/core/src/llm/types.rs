//! Conversation types (wire-compatible with the old `/api/chat` request and NDJSON events).

use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub history: Vec<ChatMessage>,
    /// The new user turn.
    pub input: MessageContent,
    /// Last stored Azure response of this conversation, if any.
    #[serde(default)]
    pub previous_response_id: Option<String>,
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
