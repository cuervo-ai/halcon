//! OpenAI Chat Completions API types.
//!
//! Shared by OpenAI and DeepSeek providers (same wire format).

use serde::{Deserialize, Serialize};

// --- SSE error chunk types (Phase 1 / PR #A) ---
//
// When upstream rejects mid-stream (Cenzontle gateway exhausting fallback,
// Azure rejecting payload, etc.), the SSE stream emits:
//   data: {"error":{"message":"All LLM providers failed.","type":"api_error",...}}
// Without an explicit type, the generic `OpenAISseChunk` parse fails and the
// chunk is silently dropped — Halcon then interprets the stream as
// `EmptyResponse` ("Agent completed silencioso" symptom).
//
// `upstream_*` and `request_id` are Cenzontle extensions proposed in
// `docs/cenzontle-required-changes.md` (C-P2-2). All optional via
// `#[serde(default)]` so the parser stays compatible with the strict OpenAI
// shape AND the enriched Cenzontle shape.

/// SSE error chunk shape: `{"error": {...}}` with no `id`/`choices`.
#[derive(Debug, Deserialize)]
pub struct OpenAIErrorChunk {
    pub error: OpenAIErrorBody,
}

/// Body of an SSE error chunk.
#[derive(Debug, Deserialize)]
pub struct OpenAIErrorBody {
    pub message: String,
    #[serde(default, rename = "type")]
    pub error_type: Option<String>,
    /// HTTP-style code or vendor-specific string code.
    #[serde(default)]
    pub code: Option<serde_json::Value>,
    /// Cenzontle extension: which upstream provider failed (e.g. "OPENAI").
    #[serde(default, rename = "upstreamProvider")]
    pub upstream_provider: Option<String>,
    /// Cenzontle extension: which upstream model failed.
    #[serde(default, rename = "upstreamModel")]
    pub upstream_model: Option<String>,
    /// Cenzontle extension: HTTP status returned by upstream.
    #[serde(default, rename = "upstreamStatus")]
    pub upstream_status: Option<u16>,
    /// Cenzontle extension: server-correlated request_id (matches Halcon's
    /// `x-request-id` UUID when propagated end-to-end).
    #[serde(default, rename = "requestId")]
    pub request_id: Option<String>,
}

// --- Request types ---

#[derive(Debug, Serialize)]
pub struct OpenAIChatRequest {
    pub model: String,
    pub messages: Vec<OpenAIChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Used by OpenAI reasoning models (o1, o3-mini) instead of max_tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAITool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
}

#[derive(Debug, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAIChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OpenAIMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Message content: either a plain string or structured parts (vision).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum OpenAIMessageContent {
    Text(String),
    /// Multi-part message containing text and/or image_url blocks (OpenAI Vision API).
    Parts(Vec<OpenAIContentPart>),
}

/// A single part in a multi-part message.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum OpenAIContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OpenAIImageUrl },
}

/// OpenAI vision image URL (base64 data URI or remote URL).
#[derive(Debug, Clone, Serialize)]
pub struct OpenAIImageUrl {
    /// Either `"data:image/jpeg;base64,..."` or a remote `https://...` URL.
    pub url: String,
    /// Resolution hint: `"auto"`, `"low"`, or `"high"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAIFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAITool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAIFunctionDef,
}

#[derive(Debug, Serialize)]
pub struct OpenAIFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// --- SSE response types ---

#[derive(Debug, Deserialize)]
pub struct OpenAISseChunk {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub choices: Vec<OpenAIChoice>,
    #[serde(default)]
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIChoice {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub delta: Option<OpenAIDelta>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIDelta {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<OpenAIToolCallDelta>>,
    /// DeepSeek Reasoner chain-of-thought tokens.
    ///
    /// During the thinking phase `reasoning_content` is populated while `content`
    /// is empty/null. When thinking finishes, `content` carries the final answer.
    /// If the entire response is produced in `reasoning_content` with no `content`
    /// phase, we fall back to emitting it as regular text so the response is visible.
    #[serde(default)]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIToolCallDelta {
    #[serde(default)]
    pub index: u32,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<OpenAIFunctionDelta>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

/// Breakdown of completion tokens, as returned by OpenAI/DeepSeek with `include_usage=true`.
#[derive(Debug, Deserialize, Default)]
pub struct CompletionTokensDetails {
    /// Tokens consumed by chain-of-thought reasoning (o1, o3-mini, deepseek-reasoner).
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Breakdown present when reasoning models (o1, o3-mini, deepseek-reasoner) are used.
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

// --- Error response ---

#[derive(Debug, Deserialize)]
pub struct OpenAIErrorResponse {
    pub error: OpenAIErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_basic_request() {
        let req = OpenAIChatRequest {
            model: "gpt-4o".into(),
            messages: vec![OpenAIChatMessage {
                role: "user".into(),
                content: Some(OpenAIMessageContent::Text("hello".into())),
                tool_calls: None,
                tool_call_id: None,
            }],
            max_tokens: Some(1024),
            max_completion_tokens: None,
            temperature: Some(0.7),
            stream: true,
            tools: vec![],
            stream_options: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("gpt-4o"));
        assert!(json.contains("\"stream\":true"));
        assert!(!json.contains("tools")); // empty vec skipped
        assert!(!json.contains("stream_options")); // None skipped
        assert!(!json.contains("max_completion_tokens")); // None skipped
    }

    #[test]
    fn serialize_request_with_tools() {
        let req = OpenAIChatRequest {
            model: "gpt-4o".into(),
            messages: vec![OpenAIChatMessage {
                role: "user".into(),
                content: Some(OpenAIMessageContent::Text("read a file".into())),
                tool_calls: None,
                tool_call_id: None,
            }],
            max_tokens: Some(1024),
            max_completion_tokens: None,
            temperature: None,
            stream: true,
            tools: vec![OpenAITool {
                tool_type: "function".into(),
                function: OpenAIFunctionDef {
                    name: "file_read".into(),
                    description: "Read a file".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"]
                    }),
                },
            }],
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"tools\""));
        assert!(json.contains("file_read"));
        assert!(json.contains("include_usage"));
    }

    #[test]
    fn serialize_tool_result_message() {
        let msg = OpenAIChatMessage {
            role: "tool".into(),
            content: Some(OpenAIMessageContent::Text("file contents here".into())),
            tool_calls: None,
            tool_call_id: Some("call_abc123".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"tool\""));
        assert!(json.contains("call_abc123"));
    }

    #[test]
    fn deserialize_sse_text_chunk() {
        let json = r#"{"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunk: OpenAISseChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        let delta = chunk.choices[0].delta.as_ref().unwrap();
        assert_eq!(delta.content.as_deref(), Some("Hello"));
        assert!(chunk.choices[0].finish_reason.is_none());
    }

    #[test]
    fn deserialize_sse_tool_call_chunk() {
        let json = r#"{"id":"chatcmpl-abc","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_xyz","function":{"name":"bash","arguments":""}}]},"finish_reason":null}]}"#;
        let chunk: OpenAISseChunk = serde_json::from_str(json).unwrap();
        let delta = chunk.choices[0].delta.as_ref().unwrap();
        let tc = &delta.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.id.as_deref(), Some("call_xyz"));
        assert_eq!(tc.function.as_ref().unwrap().name.as_deref(), Some("bash"));
    }

    #[test]
    fn deserialize_sse_finish_reasons() {
        for (reason, expected) in [
            ("stop", "stop"),
            ("length", "length"),
            ("tool_calls", "tool_calls"),
        ] {
            let json = format!(
                r#"{{"id":"chatcmpl-abc","choices":[{{"index":0,"delta":{{}},"finish_reason":"{reason}"}}]}}"#,
            );
            let chunk: OpenAISseChunk = serde_json::from_str(&json).unwrap();
            assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some(expected));
        }
    }

    #[test]
    fn deserialize_sse_usage() {
        let json = r#"{"id":"chatcmpl-abc","choices":[],"usage":{"prompt_tokens":25,"completion_tokens":100}}"#;
        let chunk: OpenAISseChunk = serde_json::from_str(json).unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 100);
    }

    #[test]
    fn deserialize_error_response() {
        let json = r#"{"error":{"message":"Invalid API key","type":"invalid_request_error"}}"#;
        let resp: OpenAIErrorResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.error.message, "Invalid API key");
        assert_eq!(
            resp.error.error_type.as_deref(),
            Some("invalid_request_error")
        );
    }

    #[test]
    fn deserialize_usage_with_completion_tokens_details() {
        // DeepSeek Reasoner and OpenAI o1/o3-mini send reasoning_tokens inside completion_tokens_details.
        let json = r#"{
            "prompt_tokens": 50,
            "completion_tokens": 1500,
            "completion_tokens_details": {"reasoning_tokens": 1200}
        }"#;
        let usage: OpenAIUsage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.prompt_tokens, 50);
        assert_eq!(usage.completion_tokens, 1500);
        let details = usage.completion_tokens_details.unwrap();
        assert_eq!(details.reasoning_tokens, Some(1200));
    }

    #[test]
    fn deserialize_usage_without_completion_tokens_details() {
        // Standard providers omit the field — must deserialise cleanly.
        let json = r#"{"prompt_tokens":10,"completion_tokens":20}"#;
        let usage: OpenAIUsage = serde_json::from_str(json).unwrap();
        assert!(usage.completion_tokens_details.is_none());
    }

    #[test]
    fn completion_tokens_details_defaults_reasoning_tokens_to_none() {
        let details = CompletionTokensDetails::default();
        assert!(details.reasoning_tokens.is_none());
    }

    // ── SSE error chunk (PR #A) ─────────────────────────────────────────────

    #[test]
    fn deserialize_minimal_error_chunk_openai_shape() {
        let json = r#"{"error":{"message":"Internal server error","type":"server_error"}}"#;
        let chunk: OpenAIErrorChunk = serde_json::from_str(json).expect("must parse");
        assert_eq!(chunk.error.message, "Internal server error");
        assert_eq!(chunk.error.error_type.as_deref(), Some("server_error"));
        assert!(chunk.error.upstream_provider.is_none());
        assert!(chunk.error.request_id.is_none());
    }

    #[test]
    fn deserialize_cenzontle_extended_error_chunk() {
        // Cenzontle C-P2-2 extension shape with full upstream attribution.
        let json = r#"{"error":{"message":"All LLM providers failed. Attempted 2 provider(s).","type":"api_error","upstreamProvider":"OPENAI","upstreamModel":"deepseek-v3-2-coding","upstreamStatus":404,"requestId":"abc-123"}}"#;
        let chunk: OpenAIErrorChunk = serde_json::from_str(json).expect("must parse");
        assert_eq!(chunk.error.upstream_provider.as_deref(), Some("OPENAI"));
        assert_eq!(
            chunk.error.upstream_model.as_deref(),
            Some("deepseek-v3-2-coding")
        );
        assert_eq!(chunk.error.upstream_status, Some(404));
        assert_eq!(chunk.error.request_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn error_chunk_does_not_match_normal_data_chunk() {
        // The whole point of the parse-as-error fallback path: a normal data
        // chunk must NOT be misinterpreted as an error envelope. Verifying
        // that `from_str::<OpenAIErrorChunk>` rejects a chunk shape with
        // `id` and `choices` but no `error`.
        let normal = r#"{"id":"chatcmpl-xxx","choices":[{"index":0,"delta":{"content":"hi"}}]}"#;
        let result = serde_json::from_str::<OpenAIErrorChunk>(normal);
        assert!(
            result.is_err(),
            "must NOT match a normal data chunk: {result:?}"
        );
    }

    #[test]
    fn error_chunk_accepts_both_string_and_int_code() {
        // OpenAI returns string codes ("rate_limit_exceeded"); some gateways return ints.
        let with_string = r#"{"error":{"message":"x","code":"rate_limit_exceeded"}}"#;
        let chunk: OpenAIErrorChunk = serde_json::from_str(with_string).unwrap();
        assert_eq!(
            chunk.error.code,
            Some(serde_json::json!("rate_limit_exceeded"))
        );

        let with_int = r#"{"error":{"message":"x","code":429}}"#;
        let chunk: OpenAIErrorChunk = serde_json::from_str(with_int).unwrap();
        assert_eq!(chunk.error.code, Some(serde_json::json!(429)));
    }

    #[test]
    fn error_chunk_partial_upstream_extension_ok() {
        // Forward-compat: partial Cenzontle extension (only some fields present).
        let json = r#"{"error":{"message":"All providers failed.","upstreamProvider":"OPENAI"}}"#;
        let chunk: OpenAIErrorChunk = serde_json::from_str(json).expect("must parse");
        assert_eq!(chunk.error.upstream_provider.as_deref(), Some("OPENAI"));
        assert!(chunk.error.upstream_model.is_none());
        assert!(chunk.error.upstream_status.is_none());
    }
}
