//! JSON-RPC 2.0 framing between the app and `contextual-agent` (plan §6.6).
//!
//! One JSON object per line on the agent's stdin/stdout. stderr carries logs only.
//! The app sends requests and `$/cancel` notifications; the agent sends responses and
//! `$/progress` notifications.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bumped whenever a method, a parameter or a result changes shape. The app redeploys the agent
/// when the agent reports a different version.
pub const PROTOCOL_VERSION: u32 = 4;

pub const JSONRPC: &str = "2.0";

/// Method names. Params/results are the `fs` and `git` types of this crate.
pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const SHUTDOWN: &str = "shutdown";
    /// Adds a project root the user opened (`AllowRootParams`); later calls may use it.
    pub const ALLOW_ROOT: &str = "workspace/allowRoot";
    pub const LIST_DIRS: &str = "workspace/listDirs";
    pub const SCAN: &str = "workspace/scan";
    pub const READ: &str = "workspace/read";
    pub const APPLY: &str = "workspace/apply";
    pub const UNDO: &str = "workspace/undo";
    /// Source Control and pull requests: params `git::GitCall`, result `git::GitReply`.
    pub const GIT: &str = "workspace/git";
    /// Non-executing preparation: `commands::PrepareCommand` → `commands::PreparedCommand`.
    pub const COMMAND_PREPARE: &str = "workspace/commandPrepare";
    /// Reserved execution RPC: `commands::DispatchCommand` → `commands::CommandResult`.
    /// Requires `commands::EXECUTION_CAPABILITY`; not implemented by the default agent.
    pub const COMMAND_DISPATCH: &str = "workspace/commandDispatch";
    /// Reserved cooperative Stop RPC: `commands::CancelCommand` → `commands::CommandResult`.
    /// This is not `$/cancel`: completion must wait for process-tree cleanup.
    pub const COMMAND_CANCEL: &str = "workspace/commandCancel";
    /// Reserved bounded cumulative notification: `commands::CommandProgress`.
    pub const COMMAND_PROGRESS: &str = "$/commandProgress";
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
pub struct AllowRootParams {
    pub root: String,
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

pub mod raw {
    //! Incoming envelopes with payloads preserved as raw JSON.
    //!
    //! Deserialize command payloads directly from RawValue, never through Value. This keeps
    //! duplicate-field evidence available to the command validators. None of these types
    //! implement Debug: payloads can contain unshared output or restricted environments.

    use serde::{Deserialize, Deserializer};
    use serde_json::value::RawValue;

    use super::{JSONRPC, RpcError};

    pub struct Request {
        pub id: u64,
        pub method: String,
        pub params: Option<Box<RawValue>>,
    }

    pub struct Notification {
        pub method: String,
        pub params: Option<Box<RawValue>>,
    }

    pub struct Response {
        pub id: u64,
        pub result: Option<Box<RawValue>>,
        pub error: Option<RpcError>,
    }

    pub enum Message {
        Request(Request),
        Notification(Notification),
        Response(Response),
    }

    // Preserve the distinction between an absent field and a present JSON null. In particular,
    // result:null is a valid response, but id:null is not an invocation identity.
    fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        T::deserialize(deserializer).map(Some)
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        jsonrpc: String,
        #[serde(default, deserialize_with = "present")]
        id: Option<u64>,
        #[serde(default, deserialize_with = "present")]
        method: Option<String>,
        #[serde(default, deserialize_with = "present")]
        params: Option<Box<RawValue>>,
        #[serde(default, deserialize_with = "present")]
        result: Option<Box<RawValue>>,
        #[serde(default, deserialize_with = "present")]
        error: Option<RpcError>,
    }

    /// Validates the envelope without interpreting its payload.
    ///
    /// The framing layer must bound `line` before calling this function. Errors deliberately
    /// omit serde diagnostics and wire contents.
    pub fn parse(line: &str) -> Result<Message, &'static str> {
        const INVALID: &str = "Malformed workspace RPC envelope.";
        let envelope: Envelope = serde_json::from_str(line).map_err(|_| INVALID)?;
        if envelope.jsonrpc != JSONRPC {
            return Err(INVALID);
        }
        if let Some(method) = envelope.method {
            if method.is_empty()
                || method.len() > 128
                || method.chars().any(char::is_control)
                || envelope.result.is_some()
                || envelope.error.is_some()
            {
                return Err(INVALID);
            }
            return Ok(match envelope.id {
                Some(id) => Message::Request(Request {
                    id,
                    method,
                    params: envelope.params,
                }),
                None => Message::Notification(Notification {
                    method,
                    params: envelope.params,
                }),
            });
        }
        if envelope.params.is_some() || envelope.result.is_some() == envelope.error.is_some() {
            return Err(INVALID);
        }
        let id = envelope.id.ok_or(INVALID)?;
        Ok(Message::Response(Response {
            id,
            result: envelope.result,
            error: envelope.error,
        }))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[derive(Deserialize)]
        struct Payload {
            #[serde(rename = "value")]
            _value: u32,
        }

        #[test]
        fn duplicate_payload_fields_survive_request_envelope_decoding() {
            let line = r#"{"jsonrpc":"2.0","id":1,"method":"test","params":{"value":1,"value":2}}"#;
            let Message::Request(request) = parse(line).ok().unwrap() else {
                panic!("expected request");
            };
            let params = request.params.unwrap();
            assert_eq!(params.get(), r#"{"value":1,"value":2}"#);
            assert!(serde_json::from_str::<Payload>(params.get()).is_err());
        }

        #[test]
        fn duplicate_payload_fields_survive_response_envelope_decoding() {
            let line = r#"{"jsonrpc":"2.0","id":1,"result":{"value":1,"value":2}}"#;
            let Message::Response(response) = parse(line).ok().unwrap() else {
                panic!("expected response");
            };
            let result = response.result.unwrap();
            assert_eq!(result.get(), r#"{"value":1,"value":2}"#);
            assert!(serde_json::from_str::<Payload>(result.get()).is_err());
        }

        #[test]
        fn explicit_null_result_is_not_mistaken_for_an_absent_result() {
            let Message::Response(response) = parse(r#"{"jsonrpc":"2.0","id":1,"result":null}"#)
                .ok()
                .unwrap()
            else {
                panic!("expected response");
            };
            assert_eq!(response.result.unwrap().get(), "null");
        }

        #[test]
        fn ambiguous_duplicate_and_incomplete_envelopes_are_refused() {
            for line in [
                r#"{"jsonrpc":"2.0","id":1,"id":2,"method":"test"}"#,
                r#"{"jsonrpc":"2.0","id":null,"method":"test"}"#,
                r#"{"jsonrpc":"2.0","id":1}"#,
                r#"{"jsonrpc":"2.0","id":1,"result":null,"error":{"code":1,"message":"x"}}"#,
                r#"{"jsonrpc":"2.0","id":1,"method":"test","result":null}"#,
                r#"{"jsonrpc":"2.0","id":1,"method":null,"result":null}"#,
                r#"{"jsonrpc":"1.0","id":1,"method":"test"}"#,
                r#"{"jsonrpc":"2.0","method":"test","unexpected":true}"#,
            ] {
                assert!(parse(line).is_err(), "{line}");
            }
        }

        #[test]
        fn malformed_envelope_errors_do_not_echo_private_values() {
            let error = parse(r#"{"jsonrpc":"2.0","id":"private-value","method":"test"}"#)
                .err()
                .unwrap();
            assert_eq!(error, "Malformed workspace RPC envelope.");
        }
    }
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
