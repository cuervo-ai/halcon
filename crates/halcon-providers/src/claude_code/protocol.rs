//! NDJSON serialization / deserialization for the Claude Code CLI protocol.
//!
//! ## Design: Last-User-Only strategy
//!
//! Unlike a direct API call that sends the full conversation each time, this
//! protocol sends **only the last user message** per turn.  The `claude` CLI
//! maintains conversation history internally, keyed by `session_id`.
//!
//! Sending the full history on every turn (original halcon behaviour) wastes
//! tokens and prevents the CLI from using its own cache.
//!
//! ## Control requests
//!
//! Model switching is done via an out-of-band `control_request` line (no re-spawn):
//! ```json
//! {"type":"control_request","request_id":"1","request":{"subtype":"set_model","model":"..."}}
//! ```

use serde::{Deserialize, Serialize};

use halcon_core::types::{ContentBlock, MessageContent, ModelChunk, ModelRequest, Role, StopReason, TokenUsage};

// ─────────────────────────────────────────────────────────────────────────────
// Outgoing request types (halcon → claude stdin)
// ─────────────────────────────────────────────────────────────────────────────

/// A user message sent to the Claude Code CLI (one NDJSON line to stdin).
#[derive(Debug, Serialize)]
pub struct NdjsonRequest {
    #[serde(rename = "type")]
    pub type_: &'static str,
    pub message: NdjsonUserMessage,
    pub session_id: String,
}

/// The message body inside an `NdjsonRequest`.
#[derive(Debug, Serialize)]
pub struct NdjsonUserMessage {
    pub role: &'static str,
    pub content: Vec<NdjsonTextContent>,
}

/// A single text content block in the request.
#[derive(Debug, Serialize)]
pub struct NdjsonTextContent {
    #[serde(rename = "type")]
    pub type_: &'static str,
    pub text: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Incoming chunk types (claude stdout → halcon)
// ─────────────────────────────────────────────────────────────────────────────

/// A chunk parsed from a single stdout line emitted by the CLI.
///
/// Unknown `type` values fall through to `Unknown` so deserialization never fails.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NdjsonChunk {
    /// A text (or tool_use) content block from Claude.
    Assistant { message: NdjsonAssistantMessage },
    /// Terminal chunk: end of one turn (success or error).
    Result {
        cost_usd: Option<f64>,
        usage: Option<NdjsonUsage>,
        #[serde(default)]
        is_error: bool,
        error: Option<String>,
    },
    /// CLI system / metadata events (model resolution, init).
    System {
        #[serde(default)]
        subtype: String,
        /// Model name resolved by the CLI (present on model-resolution events).
        #[serde(default)]
        model: Option<String>,
    },
    /// Response to a `control_request` we sent (e.g. `set_model` ack).
    ControlResponse { response: NdjsonControlResponseBody },
    /// Incoming permission request from the CLI: the subprocess wants to use a tool
    /// and requires an explicit allow/deny response before it will proceed.
    ///
    /// This is sent by Claude Code CLI in non-`--dangerously-skip-permissions` modes.
    /// The subprocess will block and time out if no `control_response` is sent back.
    ControlRequest {
        request_id: String,
        request: NdjsonIncomingRequest,
    },
    /// Catch-all for future / unknown event types.
    #[serde(other)]
    Unknown,
}

/// Body of an incoming `control_request` from the CLI.
#[derive(Debug, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
pub enum NdjsonIncomingRequest {
    /// The CLI wants to use a tool — must respond allow or deny.
    CanUseTool {
        tool_name: String,
        #[serde(default)]
        input: serde_json::Map<String, serde_json::Value>,
        #[serde(default)]
        tool_use_id: String,
    },
    /// Other subtypes we don't need to handle explicitly.
    #[serde(other)]
    Unknown,
}

/// An assistant message emitted by Claude.
#[derive(Debug, Deserialize)]
pub struct NdjsonAssistantMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Vec<NdjsonAssistantContent>,
}

/// A content block inside an assistant message.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NdjsonAssistantContent {
    Text { text: String },
    /// tool_use, thinking, and other future block types are silently skipped.
    #[serde(other)]
    Unknown,
}

/// Token usage inside a `result` chunk.
#[derive(Debug, Deserialize)]
pub struct NdjsonUsage {
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
}

/// Body of a `control_response` chunk.
#[derive(Debug, Deserialize)]
pub struct NdjsonControlResponseBody {
    pub request_id: String,
    pub subtype: String,
    #[serde(default)]
    pub error: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Key conversion: ModelRequest → NDJSON line
// ─────────────────────────────────────────────────────────────────────────────

/// Serialize a `ModelRequest` into a single NDJSON line for the Claude Code CLI.
///
/// ## Last-User-Only strategy
///
/// The full conversation history is **not** sent.  Only the text of the last
/// `Role::User` message is included.  The CLI maintains history internally via
/// `session_id`, so re-sending the full context wastes tokens and defeats
/// the CLI's internal prompt cache.
///
/// The system prompt is passed via `--system-prompt` at subprocess spawn time
/// (in `SpawnConfig`), not embedded here.
pub fn request_to_ndjson(req: &ModelRequest, session_id: &str) -> String {
    let last_user_text = extract_last_user_text(req);

    let ndjson = NdjsonRequest {
        type_: "user",
        message: NdjsonUserMessage {
            role: "user",
            content: vec![NdjsonTextContent {
                type_: "text",
                text: last_user_text,
            }],
        },
        session_id: session_id.to_string(),
    };

    serde_json::to_string(&ndjson).unwrap_or_else(|e| {
        tracing::error!(error = %e, "claude-code: failed to serialize NDJSON request");
        String::new()
    })
}

/// Build a `control_response` NDJSON line allowing a `can_use_tool` request.
///
/// Sent back to the CLI to unblock it when it asked permission to use a tool.
/// `tool_use_id` and `updated_input` are echoed back unchanged (allow-all policy).
pub fn control_response_allow_tool(
    request_id: &str,
    tool_use_id: &str,
    input: &serde_json::Map<String, serde_json::Value>,
) -> String {
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {
                "behavior": "allow",
                "updatedInput": input,
                "toolUseID": tool_use_id
            }
        }
    })
    .to_string()
}

/// Build a `control_response` NDJSON line denying a `can_use_tool` request.
///
/// Sent back to the CLI to block a tool call (e.g. when halcon policy forbids it).
pub fn control_response_deny_tool(request_id: &str, reason: &str) -> String {
    serde_json::json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {
                "behavior": "deny",
                "message": reason
            }
        }
    })
    .to_string()
}

/// Build a `control_request` NDJSON line for switching the active model.
///
/// The CLI responds with a matching `control_response` carrying the same
/// `request_id`.  No re-spawn is needed.
pub fn control_request_set_model(model: &str, request_id: u64) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id.to_string(),
        "request": {
            "subtype": "set_model",
            "model": model
        }
    })
    .to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Chunk → ModelChunk mapping
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a parsed `NdjsonChunk` into zero or more `ModelChunk`s.
///
/// | `NdjsonChunk`       | Produces                                          |
/// |---------------------|---------------------------------------------------|
/// | `Assistant`         | One `TextDelta` per text content block            |
/// | `Result` (ok)       | `Usage` (if present) + `Done(EndTurn)`            |
/// | `Result` (error)    | `Error(message)`                                  |
/// | `System`            | Empty (ignored)                                   |
/// | `ControlResponse`   | Empty (handled by `ManagedProcess::send_set_model`)|
/// | `Unknown`           | Empty (forward-compatible catch-all)              |
pub fn ndjson_chunk_to_model_chunks(chunk: NdjsonChunk) -> Vec<ModelChunk> {
    match chunk {
        NdjsonChunk::Assistant { message } => message
            .content
            .into_iter()
            .filter_map(|c| match c {
                NdjsonAssistantContent::Text { text } => {
                    // Strip leading/trailing whitespace — some models (e.g. Opus) prepend
                    // "\n\n" to responses which would render as blank lines in the terminal.
                    let trimmed = text.trim().to_string();
                    if trimmed.is_empty() { None } else { Some(ModelChunk::TextDelta(trimmed)) }
                }
                NdjsonAssistantContent::Unknown => None,
            })
            .collect(),

        NdjsonChunk::Result { cost_usd: _, usage, is_error, error } => {
            if is_error {
                let msg = error.unwrap_or_else(|| "claude-code returned an error".to_string());
                return vec![ModelChunk::Error(msg)];
            }
            let mut out = Vec::with_capacity(2);
            if let Some(u) = usage {
                out.push(ModelChunk::Usage(TokenUsage {
                    input_tokens: u.input_tokens.unwrap_or(0),
                    output_tokens: u.output_tokens.unwrap_or(0),
                    cache_read_tokens: u.cache_read_input_tokens,
                    cache_creation_tokens: u.cache_creation_input_tokens,
                    ..Default::default()
                }));
            }
            out.push(ModelChunk::Done(StopReason::EndTurn));
            out
        }

        // ControlRequest must be handled by ManagedProcess (needs I/O to respond).
        // It never maps to a ModelChunk — it's a side-channel protocol message.
        NdjsonChunk::System { .. }
        | NdjsonChunk::ControlResponse { .. }
        | NdjsonChunk::ControlRequest { .. }
        | NdjsonChunk::Unknown => vec![],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Extract the text of the last `Role::User` message in the request.
fn extract_last_user_text(req: &ModelRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| extract_message_text(&m.content))
        .unwrap_or_default()
}

fn extract_message_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(t) => t.clone(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::ToolResult { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::{ChatMessage, ModelRequest};

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage { role: Role::User, content: MessageContent::Text(text.into()) }
    }
    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage { role: Role::Assistant, content: MessageContent::Text(text.into()) }
    }

    fn req_with_messages(messages: Vec<ChatMessage>) -> ModelRequest {
        ModelRequest {
            model: "claude-opus-4-6".into(),
            messages,
            tools: vec![],
            max_tokens: Some(256),
            temperature: None,
            system: None,
            stream: true,
        }
    }

    // ── request_to_ndjson ────────────────────────────────────────────────────

    #[test]
    fn produces_valid_json() {
        let req = req_with_messages(vec![user_msg("hello")]);
        let line = request_to_ndjson(&req, "sess-1");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["session_id"], "sess-1");
    }

    #[test]
    fn sends_only_last_user_message() {
        let req = req_with_messages(vec![
            user_msg("first"),
            assistant_msg("got it"),
            user_msg("second"),
        ]);
        let line = request_to_ndjson(&req, "s");
        assert!(line.contains("second"), "must contain last user msg");
        assert!(!line.contains("first"), "must NOT contain earlier user msg");
        assert!(!line.contains("got it"), "must NOT contain assistant msg");
    }

    #[test]
    fn single_user_message_included() {
        let req = req_with_messages(vec![user_msg("what is 2+2?")]);
        let line = request_to_ndjson(&req, "s");
        assert!(line.contains("what is 2+2?"));
    }

    #[test]
    fn system_prompt_not_embedded_in_ndjson() {
        // system is passed via --system-prompt flag at spawn time, not here
        let mut req = req_with_messages(vec![user_msg("hi")]);
        req.system = Some("You are a tester.".into());
        let line = request_to_ndjson(&req, "s");
        assert!(!line.contains("You are a tester."),
            "system prompt must NOT be embedded in the NDJSON line");
    }

    #[test]
    fn empty_messages_produces_empty_text() {
        let req = req_with_messages(vec![]);
        let line = request_to_ndjson(&req, "s");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let text = &v["message"]["content"][0]["text"];
        assert_eq!(text, "");
    }

    // ── control_request_set_model ────────────────────────────────────────────

    #[test]
    fn control_request_valid_json() {
        let line = control_request_set_model("claude-opus-4-6", 7);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "control_request");
        assert_eq!(v["request_id"], "7");
        assert_eq!(v["request"]["subtype"], "set_model");
        assert_eq!(v["request"]["model"], "claude-opus-4-6");
    }

    // ── ndjson_chunk_to_model_chunks ─────────────────────────────────────────

    #[test]
    fn assistant_text_maps_to_text_delta() {
        let chunk = NdjsonChunk::Assistant {
            message: NdjsonAssistantMessage {
                role: Some("assistant".into()),
                content: vec![NdjsonAssistantContent::Text { text: "Hello!".into() }],
            },
        };
        let out = ndjson_chunk_to_model_chunks(chunk);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], ModelChunk::TextDelta(t) if t == "Hello!"));
    }

    #[test]
    fn assistant_text_strips_leading_newlines() {
        // Opus 4.6 prepends "\n\n" to responses — these render as blank lines without trimming.
        let chunk = NdjsonChunk::Assistant {
            message: NdjsonAssistantMessage {
                role: None,
                content: vec![NdjsonAssistantContent::Text { text: "\n\nHALCON_CC_OK".into() }],
            },
        };
        let out = ndjson_chunk_to_model_chunks(chunk);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], ModelChunk::TextDelta(t) if t == "HALCON_CC_OK"));
    }

    #[test]
    fn assistant_whitespace_only_text_skipped() {
        let chunk = NdjsonChunk::Assistant {
            message: NdjsonAssistantMessage {
                role: None,
                content: vec![NdjsonAssistantContent::Text { text: "\n\n".into() }],
            },
        };
        let out = ndjson_chunk_to_model_chunks(chunk);
        assert!(out.is_empty(), "whitespace-only text should produce no chunks");
    }

    #[test]
    fn result_with_usage_maps_to_usage_and_done() {
        let chunk = NdjsonChunk::Result {
            cost_usd: Some(0.001),
            usage: Some(NdjsonUsage {
                input_tokens: Some(10),
                output_tokens: Some(20),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            }),
            is_error: false,
            error: None,
        };
        let out = ndjson_chunk_to_model_chunks(chunk);
        assert_eq!(out.len(), 2);
        assert!(matches!(&out[0], ModelChunk::Usage(u) if u.input_tokens == 10 && u.output_tokens == 20));
        assert!(matches!(&out[1], ModelChunk::Done(StopReason::EndTurn)));
    }

    #[test]
    fn result_without_usage_emits_only_done() {
        let chunk = NdjsonChunk::Result {
            cost_usd: None,
            usage: None,
            is_error: false,
            error: None,
        };
        let out = ndjson_chunk_to_model_chunks(chunk);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], ModelChunk::Done(StopReason::EndTurn)));
    }

    #[test]
    fn result_error_maps_to_error_chunk() {
        let chunk = NdjsonChunk::Result {
            cost_usd: None,
            usage: None,
            is_error: true,
            error: Some("permission denied".into()),
        };
        let out = ndjson_chunk_to_model_chunks(chunk);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], ModelChunk::Error(e) if e.contains("permission denied")));
    }

    #[test]
    fn result_error_no_message_uses_fallback() {
        let chunk =
            NdjsonChunk::Result { cost_usd: None, usage: None, is_error: true, error: None };
        let out = ndjson_chunk_to_model_chunks(chunk);
        assert!(matches!(&out[0], ModelChunk::Error(_)));
    }

    #[test]
    fn system_chunk_produces_empty() {
        let chunk = NdjsonChunk::System { subtype: "init".into(), model: None };
        assert!(ndjson_chunk_to_model_chunks(chunk).is_empty());
    }

    #[test]
    fn control_response_produces_empty() {
        let chunk = NdjsonChunk::ControlResponse {
            response: NdjsonControlResponseBody {
                request_id: "1".into(),
                subtype: "success".into(),
                error: None,
            },
        };
        assert!(ndjson_chunk_to_model_chunks(chunk).is_empty());
    }

    #[test]
    fn unknown_chunk_produces_empty() {
        let raw = r#"{"type":"totally_unknown_event","data":42}"#;
        let chunk: NdjsonChunk = serde_json::from_str(raw).unwrap();
        assert!(ndjson_chunk_to_model_chunks(chunk).is_empty());
    }

    #[test]
    fn assistant_unknown_content_skipped() {
        let chunk = NdjsonChunk::Assistant {
            message: NdjsonAssistantMessage {
                role: None,
                content: vec![
                    NdjsonAssistantContent::Unknown,
                    NdjsonAssistantContent::Text { text: "visible".into() },
                ],
            },
        };
        let out = ndjson_chunk_to_model_chunks(chunk);
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], ModelChunk::TextDelta(t) if t == "visible"));
    }

    #[test]
    fn cache_tokens_preserved_in_usage() {
        let chunk = NdjsonChunk::Result {
            cost_usd: None,
            usage: Some(NdjsonUsage {
                input_tokens: Some(50),
                output_tokens: Some(10),
                cache_read_input_tokens: Some(30),
                cache_creation_input_tokens: Some(5),
            }),
            is_error: false,
            error: None,
        };
        let out = ndjson_chunk_to_model_chunks(chunk);
        if let ModelChunk::Usage(u) = &out[0] {
            assert_eq!(u.cache_read_tokens, Some(30));
            assert_eq!(u.cache_creation_tokens, Some(5));
        } else {
            panic!("expected Usage chunk");
        }
    }

    #[test]
    fn multi_turn_sends_only_last_user_turn() {
        let req = req_with_messages(vec![
            user_msg("Q1"),
            assistant_msg("A1"),
            user_msg("Q2"),
            assistant_msg("A2"),
            user_msg("Q3"),
        ]);
        let line = request_to_ndjson(&req, "s");
        assert!(line.contains("Q3"));
        assert!(!line.contains("Q1"));
        assert!(!line.contains("Q2"));
        assert!(!line.contains("A1"));
        assert!(!line.contains("A2"));
    }
}
