//! The native `propose_command` tool: its definition, the transcript types for calls and
//! results, and bounded assembly of streamed calls.
//!
//! Nothing here can run anything. A call that survives assembly is text from the model; it
//! becomes a [`CommandRequest`] only through [`command_request`], and that is still only a
//! proposal that the host must prepare and the user must approve. This module, and the whole
//! `llm` module, has no reference to the execution engine: supplying history, retrying a
//! request or resetting a stored chain builds JSON and nothing else.
//!
//! A call is usable only when the whole model response completed successfully. Calls from a
//! response that was cut off, calls with unknown names, oversized or malformed arguments, and
//! calls whose identity collides with another are kept, with their identity, and marked with
//! a `problem`: the transcript must still answer every call the provider issued.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::client::FoundryError;
use super::types::{ChatMessage, ChatRole, ConversationRequest, MESSAGE_OVERHEAD_TOKENS};
use crate::commands::{CommandRequest, MAX_REQUEST_BYTES, MAX_TIMEOUT_SECONDS, parse_arguments};
use crate::logic::format::estimate_tokens;

/// The only tool that is offered. Text that merely resembles a call is never a call.
pub const PROPOSE_COMMAND: &str = "propose_command";
/// Calls in one model response, and results in one request.
pub const MAX_TOOL_CALLS: usize = 8;
pub const MAX_CALL_ID_BYTES: usize = 256;
pub const MAX_TOOL_NAME_BYTES: usize = 64;
pub const MAX_ARGUMENT_BYTES: usize = MAX_REQUEST_BYTES;
/// One result returned to the model, such as output the user chose to share.
pub const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;

const DESCRIPTION: &str = "Propose one command to run in the user's project. Nothing runs \
when you call this: the user sees the exact command and decides whether to run it, edit it \
or reject it, and you receive only what they choose to share. Propose a command only when \
running it is needed; never describe a command in prose as if it had been run.";

/// A call issued by the model, exactly as received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    /// The provider's identity for this call; its result must carry the same value.
    pub call_id: String,
    pub name: String,
    /// The raw arguments text. Not validated as a command.
    pub arguments: String,
    /// Why this call can never become a proposal. It still needs a result in the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// What is returned to the model for one call: never execution by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: String,
    pub output: String,
}

/// Validates one usable call as a command proposal. Syntax and version-one policy only:
/// no host inspection, no approval, no execution.
pub fn command_request(call: &ToolCall) -> crate::Result<CommandRequest> {
    if let Some(problem) = &call.problem {
        return Err(crate::Error::invalid(problem.clone()));
    }
    if call.name != PROPOSE_COMMAND {
        return Err(crate::Error::invalid("Unknown tool."));
    }
    parse_arguments(&call.arguments)
}

/// JSON Schema for the tool's arguments: the versioned shape of [`CommandRequest`].
/// The schema guides the model; [`command_request`] is what decides.
pub fn command_tool_parameters() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["version", "purpose", "command", "cwd", "timeoutSeconds", "environment"],
        "properties": {
            "version": { "type": "integer", "enum": [1] },
            "purpose": {
                "type": "string",
                "description": "Why this command is needed, in one or two sentences."
            },
            "command": {
                "description": "Either a program with its exact argument list, or a complete script for a named shell.",
                "anyOf": [
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "program", "args"],
                        "properties": {
                            "kind": { "type": "string", "enum": ["exec"] },
                            "program": { "type": "string" },
                            "args": { "type": "array", "items": { "type": "string" } }
                        }
                    },
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["kind", "interpreter", "script"],
                        "properties": {
                            "kind": { "type": "string", "enum": ["shell"] },
                            "interpreter": {
                                "type": "string",
                                "enum": ["sh", "bash", "zsh", "pwsh", "windowsPowerShell", "cmd"]
                            },
                            "script": { "type": "string" }
                        }
                    }
                ]
            },
            "cwd": {
                "type": "string",
                "description": "Directory inside the project, relative, with forward slashes. \".\" is the project root."
            },
            "timeoutSeconds": { "type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_SECONDS },
            "environment": {
                "type": "array",
                "description": "Extra variables, usually empty. Only a small permitted set is accepted.",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "value"],
                    "properties": {
                        "name": { "type": "string" },
                        "value": { "type": "string" }
                    }
                }
            }
        }
    })
}

/// The tool as the Responses API declares it.
pub(crate) fn responses_tool() -> Value {
    json!({
        "type": "function",
        "name": PROPOSE_COMMAND,
        "description": DESCRIPTION,
        "parameters": command_tool_parameters(),
        "strict": false,
    })
}

/// The tool as chat completions declares it.
pub(crate) fn chat_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": PROPOSE_COMMAND,
            "description": DESCRIPTION,
            "parameters": command_tool_parameters(),
        }
    })
}

/// Estimated cost of declaring the tool, counted in the logical input budget.
pub(crate) fn definition_tokens() -> u64 {
    estimate_tokens(DESCRIPTION)
        .saturating_add(estimate_tokens(&command_tool_parameters().to_string()))
        .saturating_add(MESSAGE_OVERHEAD_TOKENS)
}

impl ToolCall {
    pub(crate) fn estimated_tokens(&self) -> u64 {
        estimate_tokens(&self.call_id)
            .saturating_add(estimate_tokens(&self.name))
            .saturating_add(estimate_tokens(&self.arguments))
            .saturating_add(MESSAGE_OVERHEAD_TOKENS)
    }

    /// Arguments as they are replayed in a full history. Text that is not a JSON object
    /// would make a provider reject the whole request, so it is replaced; the call keeps
    /// its identity and its recorded problem.
    pub(crate) fn replayed_arguments(&self) -> &str {
        if is_json_object(&self.arguments) {
            &self.arguments
        } else {
            "{}"
        }
    }
}

impl ToolResult {
    pub(crate) fn estimated_tokens(&self) -> u64 {
        estimate_tokens(&self.call_id)
            .saturating_add(estimate_tokens(&self.output))
            .saturating_add(MESSAGE_OVERHEAD_TOKENS)
    }
}

fn is_json_object(text: &str) -> bool {
    matches!(serde_json::from_str::<Value>(text), Ok(Value::Object(_)))
}

fn bounded_identity(text: &str, limit: usize) -> bool {
    !text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
}

fn inconsistent(detail: &str) -> FoundryError {
    FoundryError::Rejected(format!(
        "The conversation's command history is inconsistent ({detail}). Nothing was sent."
    ))
}

/// Whether any call or result appears anywhere in the logical conversation.
pub(crate) fn has_tool_items(req: &ConversationRequest) -> bool {
    !req.tool_results.is_empty()
        || req
            .history
            .iter()
            .any(|message| message.role == ChatRole::Tool || !message.tool_calls.is_empty())
}

/// Checks call/result correlation before anything is sent: every call is answered exactly
/// once, by the messages that directly follow it or by this request's results, and no
/// result exists without its call. Sizes and counts are bounded here too.
pub(crate) fn validate_transcript(req: &ConversationRequest) -> Result<(), FoundryError> {
    fn answer(pending: &mut Vec<String>, call_id: &str, bytes: usize) -> Result<(), FoundryError> {
        if bytes > MAX_TOOL_RESULT_BYTES {
            return Err(inconsistent("a command result exceeds the size limit"));
        }
        match pending.iter().position(|id| id == call_id) {
            Some(index) => {
                pending.remove(index);
                Ok(())
            }
            None => Err(inconsistent("a result has no matching call")),
        }
    }

    let mut pending: Vec<String> = Vec::new();
    for message in &req.history {
        if message.role == ChatRole::Tool {
            let call_id = message.tool_call_id.as_deref().unwrap_or("");
            let bytes = serde_json::to_string(&message.content).map_or(usize::MAX, |s| s.len());
            answer(&mut pending, call_id, bytes)?;
            continue;
        }
        if message.tool_call_id.is_some() {
            return Err(inconsistent("only a result can name a call"));
        }
        if !pending.is_empty() {
            return Err(inconsistent("a call was never answered"));
        }
        if message.tool_calls.is_empty() {
            continue;
        }
        if message.role != ChatRole::Assistant || message.tool_calls.len() > MAX_TOOL_CALLS {
            return Err(inconsistent("calls are misplaced or too many"));
        }
        for call in &message.tool_calls {
            if !bounded_identity(&call.call_id, MAX_CALL_ID_BYTES)
                || !bounded_identity(&call.name, MAX_TOOL_NAME_BYTES)
                || call.arguments.len() > MAX_ARGUMENT_BYTES
                || pending.contains(&call.call_id)
            {
                return Err(inconsistent("a call is malformed or duplicated"));
            }
            pending.push(call.call_id.clone());
        }
    }
    if req.tool_results.len() > MAX_TOOL_CALLS {
        return Err(inconsistent("too many command results"));
    }
    for result in &req.tool_results {
        answer(&mut pending, &result.call_id, result.output.len())?;
    }
    if !pending.is_empty() {
        return Err(inconsistent("a call was never answered"));
    }
    Ok(())
}

/// This request's results as transcript messages, in the order given.
pub(crate) fn result_messages(req: &ConversationRequest) -> impl Iterator<Item = ChatMessage> + '_ {
    req.tool_results
        .iter()
        .map(|result| ChatMessage::tool(result.call_id.clone(), result.output.clone()))
}

#[derive(Default)]
struct Partial {
    call_id: String,
    name: String,
    arguments: String,
    /// The provider said this call's arguments are complete.
    closed: bool,
    problem: Option<&'static str>,
}

const TOO_LARGE: &str = "The call exceeds a size limit.";
const AMBIGUOUS: &str = "The call was reported inconsistently by the provider.";

impl Partial {
    fn flag(&mut self, problem: &'static str) {
        self.problem.get_or_insert(problem);
    }

    /// Appends a streamed fragment without growing past the limit.
    fn append(&mut self, field: Field, fragment: &str) {
        let (text, limit) = match field {
            Field::CallId => (&mut self.call_id, MAX_CALL_ID_BYTES),
            Field::Name => (&mut self.name, MAX_TOOL_NAME_BYTES),
            Field::Arguments => (&mut self.arguments, MAX_ARGUMENT_BYTES),
        };
        if text.len().saturating_add(fragment.len()) > limit {
            self.flag(TOO_LARGE);
        } else {
            text.push_str(fragment);
        }
    }

    /// Records a complete value reported by the provider. A value that disagrees with what
    /// was already assembled makes the call ambiguous; the first value is kept.
    fn confirm(&mut self, field: Field, value: &str) {
        let text = match field {
            Field::CallId => &self.call_id,
            Field::Name => &self.name,
            Field::Arguments => &self.arguments,
        };
        if text.is_empty() {
            self.append(field, value);
        } else if text != value {
            self.flag(AMBIGUOUS);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Field {
    CallId,
    Name,
    Arguments,
}

/// Collects the calls of one model response, in the order the provider introduced them.
/// Keys are the provider's own slot identities: output item IDs or stream indexes.
#[derive(Default)]
pub(crate) struct CallAssembler {
    keys: Vec<String>,
    calls: Vec<Partial>,
}

impl CallAssembler {
    fn slot(&mut self, key: &str) -> Result<&mut Partial, FoundryError> {
        if let Some(index) = self.keys.iter().position(|known| known == key) {
            return Ok(&mut self.calls[index]);
        }
        if self.calls.len() >= MAX_TOOL_CALLS || key.len() > MAX_CALL_ID_BYTES {
            return Err(FoundryError::Failed(
                "The model issued more tool calls than can be reviewed. None was accepted.".into(),
            ));
        }
        self.keys.push(key.to_owned());
        self.calls.push(Partial::default());
        Ok(self.calls.last_mut().expect("just pushed"))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// A streamed fragment for one slot.
    pub(crate) fn append(
        &mut self,
        key: &str,
        field: Field,
        fragment: &str,
    ) -> Result<(), FoundryError> {
        self.slot(key)?.append(field, fragment);
        Ok(())
    }

    /// A complete value for one slot, as stated by the provider.
    pub(crate) fn confirm(
        &mut self,
        key: &str,
        field: Field,
        value: &str,
    ) -> Result<(), FoundryError> {
        self.slot(key)?.confirm(field, value);
        Ok(())
    }

    /// The provider stated that this slot's call is complete.
    pub(crate) fn close(&mut self, key: &str) -> Result<(), FoundryError> {
        self.slot(key)?.closed = true;
        Ok(())
    }

    /// Whether the provider has introduced this slot.
    pub(crate) fn knows(&self, key: &str) -> bool {
        self.keys.iter().any(|known| known == key)
    }

    /// Marks a slot that the provider's final summary does not contain.
    pub(crate) fn flag_unlisted(&mut self, listed: &[String]) {
        for (key, call) in self.keys.iter().zip(&mut self.calls) {
            if !listed.contains(key) {
                call.flag(AMBIGUOUS);
            }
        }
    }

    /// Ends assembly. `completed` is whether the whole response ended successfully in the
    /// way that makes tool calls final; `offered` is whether the tool was declared at all.
    /// Every slot is returned, usable or not, so that no call identity is lost.
    pub(crate) fn finish(
        self,
        completed: bool,
        offered: bool,
        require_closed: bool,
    ) -> Vec<ToolCall> {
        let ids: Vec<String> = self.calls.iter().map(|call| call.call_id.clone()).collect();
        self.calls
            .into_iter()
            .map(|call| {
                let duplicated = ids.iter().filter(|id| **id == call.call_id).count() > 1;
                let problem = if !completed {
                    Some("The model's response did not complete, so this call is not usable.")
                } else if !offered {
                    Some("No tool was offered in this request.")
                } else if let Some(problem) = call.problem {
                    Some(problem)
                } else if require_closed && !call.closed {
                    Some("The call was never completed by the provider.")
                } else if !bounded_identity(&call.call_id, MAX_CALL_ID_BYTES)
                    || !bounded_identity(&call.name, MAX_TOOL_NAME_BYTES)
                {
                    Some("The call has no usable identity or name.")
                } else if duplicated {
                    Some("Another call in the same response has the same identity.")
                } else if call.name != PROPOSE_COMMAND {
                    Some("Unknown tool.")
                } else if !is_json_object(&call.arguments) {
                    Some("The call's arguments are not a JSON object.")
                } else {
                    None
                };
                ToolCall {
                    call_id: call.call_id,
                    name: call.name,
                    arguments: call.arguments,
                    problem: problem.map(str::to_owned),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::MessageContent;

    fn arguments() -> String {
        json!({
            "version": 1,
            "purpose": "List files.",
            "command": { "kind": "exec", "program": "ls", "args": ["-la"] },
            "cwd": ".",
            "timeoutSeconds": 30,
            "environment": []
        })
        .to_string()
    }

    fn call(id: &str) -> ToolCall {
        ToolCall {
            call_id: id.into(),
            name: PROPOSE_COMMAND.into(),
            arguments: arguments(),
            problem: None,
        }
    }

    fn request(history: Vec<ChatMessage>, results: Vec<ToolResult>) -> ConversationRequest {
        ConversationRequest {
            system: "SYS".into(),
            history,
            input: MessageContent::Text(String::new()),
            previous_response_id: None,
            command_tool: true,
            tool_results: results,
        }
    }

    fn result(id: &str) -> ToolResult {
        ToolResult {
            call_id: id.into(),
            output: "The user rejected this command.".into(),
        }
    }

    #[test]
    fn the_schema_accepts_what_the_parser_accepts() {
        let schema = command_tool_parameters();
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        // Every field of the request, and nothing else, is declared and required.
        let serialized = serde_json::to_value(parse_arguments(&arguments()).unwrap()).unwrap();
        let mut fields: Vec<&str> = serialized
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut declared = required.clone();
        fields.sort_unstable();
        declared.sort_unstable();
        assert_eq!(fields, declared);
        assert_eq!(
            schema["properties"].as_object().unwrap().len(),
            declared.len()
        );
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn a_usable_call_becomes_a_proposal_and_nothing_else_does() {
        let request = command_request(&call("call_1")).unwrap();
        assert_eq!(request.purpose, "List files.");

        let mut flagged = call("call_1");
        flagged.problem = Some("The model's response did not complete.".into());
        assert!(command_request(&flagged).is_err());
        let mut other = call("call_1");
        other.name = "run_now".into();
        assert!(command_request(&other).is_err());
        let mut extra = call("call_1");
        extra.arguments = extra.arguments.replacen('{', "{\"approved\":true,", 1);
        assert!(command_request(&extra).is_err());
    }

    #[test]
    fn assembly_joins_fragments_and_keeps_every_identity() {
        let mut assembler = CallAssembler::default();
        let text = arguments();
        let (head, tail) = text.split_at(text.len() / 2);
        assembler.append("0", Field::CallId, "call_").unwrap();
        assembler.append("0", Field::CallId, "1").unwrap();
        assembler.append("0", Field::Name, "propose_").unwrap();
        assembler.append("0", Field::Name, "command").unwrap();
        assembler.append("0", Field::Arguments, head).unwrap();
        assembler.append("1", Field::CallId, "call_2").unwrap();
        assembler.append("1", Field::Name, PROPOSE_COMMAND).unwrap();
        assembler.append("0", Field::Arguments, tail).unwrap();
        assembler
            .append("1", Field::Arguments, "{\"version\":")
            .unwrap();

        let calls = assembler.finish(true, true, false);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], call("call_1"));
        assert_eq!(calls[1].call_id, "call_2");
        assert!(calls[1].problem.is_some());
        assert!(command_request(&calls[1]).is_err());
    }

    #[test]
    fn an_incomplete_response_makes_every_call_unusable() {
        let mut assembler = CallAssembler::default();
        assembler.confirm("a", Field::CallId, "call_1").unwrap();
        assembler
            .confirm("a", Field::Name, PROPOSE_COMMAND)
            .unwrap();
        assembler
            .confirm("a", Field::Arguments, &arguments())
            .unwrap();
        let calls = assembler.finish(false, true, false);
        assert_eq!(calls[0].call_id, "call_1");
        assert!(calls[0].problem.is_some());
    }

    #[test]
    fn collisions_unknown_names_and_disagreement_are_flagged_not_merged() {
        let mut assembler = CallAssembler::default();
        for key in ["a", "b"] {
            assembler.confirm(key, Field::CallId, "call_same").unwrap();
            assembler
                .confirm(key, Field::Name, PROPOSE_COMMAND)
                .unwrap();
            assembler
                .confirm(key, Field::Arguments, &arguments())
                .unwrap();
        }
        assembler.confirm("c", Field::CallId, "call_3").unwrap();
        assembler
            .confirm("c", Field::Name, "delete_everything")
            .unwrap();
        assembler.confirm("c", Field::Arguments, "{}").unwrap();
        assembler.confirm("d", Field::CallId, "call_4").unwrap();
        assembler
            .confirm("d", Field::Name, PROPOSE_COMMAND)
            .unwrap();
        assembler
            .confirm("d", Field::Arguments, &arguments())
            .unwrap();
        assembler.confirm("d", Field::Arguments, "{}").unwrap();

        let calls = assembler.finish(true, true, false);
        assert_eq!(calls.len(), 4);
        assert!(calls.iter().all(|call| call.problem.is_some()));
        // The first value is kept; a later disagreeing one is not adopted.
        assert_eq!(calls[3].arguments, arguments());
    }

    #[test]
    fn sizes_and_counts_are_bounded_without_unbounded_buffers() {
        let mut assembler = CallAssembler::default();
        assembler.append("0", Field::CallId, "call_1").unwrap();
        assembler.append("0", Field::Name, PROPOSE_COMMAND).unwrap();
        let chunk = "x".repeat(MAX_ARGUMENT_BYTES / 2 + 1);
        for _ in 0..8 {
            assembler.append("0", Field::Arguments, &chunk).unwrap();
        }
        for index in 1..MAX_TOOL_CALLS {
            assembler
                .append(&index.to_string(), Field::CallId, "x")
                .unwrap();
        }
        assert!(
            assembler
                .append("one-too-many", Field::CallId, "x")
                .is_err()
        );
        let calls = assembler.finish(true, true, false);
        assert!(calls[0].arguments.len() <= MAX_ARGUMENT_BYTES);
        assert!(calls[0].problem.is_some());
    }

    #[test]
    fn every_call_must_be_answered_exactly_once() {
        let asked = ChatMessage::assistant_calls("", vec![call("call_1"), call("call_2")]);
        let user = ChatMessage::user("Go on");

        // Answered by this request's results.
        validate_transcript(&request(
            vec![user.clone(), asked.clone()],
            vec![result("call_2"), result("call_1")],
        ))
        .unwrap();
        // Answered inside the history.
        validate_transcript(&request(
            vec![
                user.clone(),
                asked.clone(),
                ChatMessage::tool("call_1", "rejected"),
                ChatMessage::tool("call_2", "rejected"),
                ChatMessage::assistant("Understood."),
            ],
            vec![],
        ))
        .unwrap();

        for (history, results) in [
            // Unanswered.
            (vec![user.clone(), asked.clone()], vec![result("call_1")]),
            // Answered twice.
            (
                vec![user.clone(), asked.clone()],
                vec![result("call_1"), result("call_1"), result("call_2")],
            ),
            // A result without a call.
            (vec![user.clone()], vec![result("call_1")]),
            // Something else between a call and its result.
            (
                vec![
                    asked.clone(),
                    user.clone(),
                    ChatMessage::tool("call_1", "x"),
                ],
                vec![result("call_2")],
            ),
            // Two calls with one identity.
            (
                vec![ChatMessage::assistant_calls(
                    "",
                    vec![call("call_1"), call("call_1")],
                )],
                vec![result("call_1"), result("call_1")],
            ),
        ] {
            assert!(validate_transcript(&request(history, results)).is_err());
        }
    }
}
