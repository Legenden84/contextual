//! JSON-RPC 2.0 framing between the app and `contextual-agent` (plan §6.6).
//!
//! One JSON object per line on the agent's stdin/stdout. stderr carries logs only.
//! The app sends requests and `$/cancel` notifications; the agent sends responses and
//! `$/progress` notifications.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped whenever a method, a parameter or a result changes shape. The app redeploys the agent
/// when the agent reports a different version.
pub const PROTOCOL_VERSION: u32 = 1;

pub const JSONRPC: &str = "2.0";

/// Method names. Params/results are the `fs` and `git` types of this crate.
pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const SHUTDOWN: &str = "shutdown";
    pub const LIST_DIRS: &str = "workspace/listDirs";
    pub const SCAN: &str = "workspace/scan";
    pub const READ: &str = "workspace/read";
    pub const APPLY: &str = "workspace/apply";
    pub const UNDO: &str = "workspace/undo";
    /// Source Control and pull requests: params `git::GitCall`, result `git::GitReply`.
    pub const GIT: &str = "workspace/git";
    /// Notification, app → agent: `{ id }` of the request to cancel.
    pub const CANCEL: &str = "$/cancel";
    /// Notification, agent → app: `{ id, message }` for long operations.
    pub const PROGRESS: &str = "$/progress";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC.into(),
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Notification {
    pub fn new(method: &str, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC.into(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC.into(),
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: u64, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC.into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// `data` carries the core's `{ code, message }` error so the app can rebuild it exactly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// Request cancelled by `$/cancel` (LSP's value).
    pub const REQUEST_CANCELLED: i64 = -32800;
    /// A core error; see `data`.
    pub const APPLICATION_ERROR: i64 = -32000;

    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

/// Anything the agent can receive or the app can read, for one line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: u32,
    pub app_version: String,
    /// Project roots the user opened; every workspace call must stay inside one of them.
    #[serde(default)]
    pub allowed_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: u32,
    pub agent_version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelParams {
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgressParams {
    pub id: u64,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_round_trip() {
        let lines = [
            json!({"jsonrpc":"2.0","id":1,"method":"workspace/scan","params":{"root":"/x"}}),
            json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}}),
            json!({"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"nope","data":{"code":"conflict","message":"nope"}}}),
            json!({"jsonrpc":"2.0","method":"$/cancel","params":{"id":3}}),
        ];
        let parsed: Vec<Message> = lines
            .iter()
            .map(|l| serde_json::from_value(l.clone()).unwrap())
            .collect();
        assert!(matches!(parsed[0], Message::Request(_)));
        assert!(matches!(parsed[1], Message::Response(_)));
        assert!(matches!(
            parsed[2],
            Message::Response(Response { error: Some(_), .. })
        ));
        assert!(matches!(parsed[3], Message::Notification(_)));
        for (m, l) in parsed.iter().zip(&lines) {
            assert_eq!(&serde_json::to_value(m).unwrap(), l);
        }
    }
}
