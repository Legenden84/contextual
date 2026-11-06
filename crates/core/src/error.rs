//! One error type for the core, with a stable machine-readable code for the UI and the agent
//! protocol, plus a human message (shown as-is in Banners/Toasts).

use serde::{Deserialize, Serialize};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Bad input from the caller (missing field, invalid path…). HTTP 400 in the old server.
    InvalidInput,
    /// Path escapes the project root or root not allowed.
    Forbidden,
    NotFound,
    /// Save/undo refused because files changed since the model saw them (optimistic check).
    Conflict,
    /// Foundry is not configured.
    NotConfigured,
    /// The model endpoint returned an error or the request failed.
    Upstream,
    /// git / gh failed or is missing.
    Tool,
    Cancelled,
    Io,
    Internal,
}

#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize, PartialEq, Eq)]
#[error("{message}")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Forbidden, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }
    pub fn upstream(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Upstream, message)
    }
    pub fn tool(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Tool, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        let code = match e.kind() {
            std::io::ErrorKind::NotFound => ErrorCode::NotFound,
            std::io::ErrorKind::PermissionDenied => ErrorCode::Forbidden,
            _ => ErrorCode::Io,
        };
        Self::new(code, e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new(ErrorCode::InvalidInput, e.to_string())
    }
}
