//! Azure AI Foundry: configuration, the Responses API with chat-completions fallback, connection
//! test, commit/PR drafts and edit repair. Plain `reqwest` + an SSE parser; no OpenAI SDK.
//!
//! ```ignore
//! let client = FoundryClient::new(foundry_config_from_env().map_err(ConfigProblems::into_error)?)?;
//! let mut events = client.stream_conversation(request, CancellationToken::new());
//! while let Some(ev) = events.next().await { /* ChatEvent::Delta / Usage / Done / Error */ }
//! ```

pub mod client;
pub mod config;
pub mod conversation;
pub mod pacing;
pub mod sse;
pub mod tasks;
pub mod tools;
pub mod types;

pub use client::{FoundryClient, FoundryError, describe_error};
pub use config::{
    ConfigProblems, FoundryConfig, foundry_config_from_env, public_config, read_foundry_config,
};
pub use conversation::ChatStream;
pub use pacing::WaitReason;
pub use tools::{PROPOSE_COMMAND, ToolCall, ToolResult, command_request};
pub use types::{
    ApiKind, ChatEvent, ChatMessage, ChatRole, ContentPart, ConversationRequest, ImageUrl,
    MessageContent, PrDraftInput, RepairInput, RepairResult,
};
