//! Types for the Cenzontle agent orchestration and MCP APIs.
//!
//! These types map to Cenzontle's REST endpoints:
//! - `/v1/agents/sessions` — agent session management
//! - `/v1/agents/sessions/:id/tasks` — task submission + streaming
//! - `/v1/mcp/tools` — MCP tool discovery and invocation
//! - `/v1/agents` — agent listing

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Agent Sessions ──────────────────────────────────────────────────────────

/// Request body for `POST /v1/agents/sessions`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    /// Specific agent ID to use (None = let Cenzontle route).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Session metadata for context enrichment.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Response from `POST /v1/agents/sessions` or `GET /v1/agents/sessions/:id`.
///
/// Real format: `{"id": "session_xxx", "status": "active", "agentIds": [], ...}`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub agent_ids: Vec<String>,
}

// ── Task Submission ─────────────────────────────────────────────────────────

/// Request body for `POST /v1/agents/sessions/:id/tasks`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitTaskRequest {
    /// Natural-language instruction for the agent.
    pub input: String,
    /// Agent type hint (e.g. "ORCHESTRATOR", "CONVERSATIONAL", "TASK").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Local context gathered from Halcón CLI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<TaskContext>,
    /// Priority (higher = more urgent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u32>,
}

/// Local context sent alongside a task for RAG enrichment.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskContext {
    /// Working directory on the client.
    pub cwd: String,
    /// Git branch name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// Git status (porcelain format).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_status: Option<String>,
    /// Key file contents from the project.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileContext>,
}

/// A file sent as context to the agent.
#[derive(Debug, Serialize)]
pub struct FileContext {
    pub path: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

// ── Task Streaming Events (SSE) ─────────────────────────────────────────────

/// A server-sent event from task execution.
///
/// Field names use `#[serde(alias)]` to accept both snake_case and camelCase
/// from the backend (Cenzontle may send either depending on the transport).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEvent {
    /// Agent started processing.
    Started {
        #[serde(default, alias = "agentId")]
        agent_id: Option<String>,
    },
    /// Agent is reasoning/thinking.
    Thinking {
        content: String,
    },
    /// Incremental content from the agent.
    Content {
        content: String,
    },
    /// Agent called a tool.
    ToolCall {
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    /// Tool returned a result.
    ToolResult {
        name: String,
        output: String,
        #[serde(default, alias = "isError")]
        is_error: bool,
    },
    /// Execution plan step.
    PlanStep {
        step: String,
        #[serde(default)]
        index: usize,
        #[serde(default)]
        total: usize,
    },
    /// Task completed successfully.
    Completed {
        output: String,
        #[serde(default, alias = "tokensUsed")]
        tokens_used: Option<u64>,
    },
    /// Task failed.
    Error {
        message: String,
        #[serde(default)]
        code: Option<String>,
    },
    /// Unknown event type (forward compatibility).
    #[serde(other)]
    Unknown,
}

/// Synchronous JSON response from `POST /v1/agents/sessions/:id/tasks`.
///
/// Returned when the task completes without SSE streaming (e.g., fast results,
/// no agents available, or single-shot execution).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSyncResponse {
    pub task_id: String,
    pub session_id: String,
    #[serde(default)]
    pub combined_output: String,
    #[serde(default)]
    pub all_succeeded: bool,
    #[serde(default)]
    pub agent_results: Vec<AgentResultEntry>,
    #[serde(default)]
    pub total_latency_ms: u64,
    #[serde(default)]
    pub strategy: Option<String>,
}

/// A single agent's result within a synchronous task response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentResultEntry {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub agent_name: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub token_usage: Option<TokenUsageInfo>,
}

/// Token usage reported by an agent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageInfo {
    #[serde(default)]
    pub total_tokens: u64,
}

/// Convenience: task result after streaming completes.
#[derive(Debug, Clone, Default)]
pub struct TaskResult {
    pub output: String,
    pub thinking: String,
    pub tool_calls: Vec<String>,
    pub tokens_used: u64,
    pub success: bool,
    pub error: Option<String>,
}

// ── Agent Listing ───────────────────────────────────────────────────────────

/// A registered agent in Cenzontle.
///
/// `GET /v1/agents` returns a bare JSON array of these (no wrapper object).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

// ── MCP Tools ───────────────────────────────────────────────────────────────

/// Response from `GET /v1/mcp/tools`.
#[derive(Debug, Deserialize)]
pub struct McpToolListResponse {
    pub tools: Vec<McpToolDef>,
}

/// A single MCP tool definition.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_empty_object")]
    pub input_schema: serde_json::Value,
}

fn default_empty_object() -> serde_json::Value {
    serde_json::json!({})
}

/// Request body for `POST /v1/mcp/tools/call`.
#[derive(Debug, Serialize)]
pub struct McpToolCallRequest {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Response from `POST /v1/mcp/tools/call`.
///
/// The `content` field is an array of MCP content blocks (usually `[{type: "text", text: "..."}]`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallResponse {
    #[serde(default)]
    pub content: Vec<McpContentBlock>,
    #[serde(default)]
    pub is_error: bool,
}

/// A content block in an MCP tool response.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
    },
    #[serde(other)]
    Other,
}

impl McpToolCallResponse {
    /// Extract all text content from the response blocks.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                McpContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ── Knowledge Search (RAG) ──────────────────────────────────────────────────

/// Request body for RAG knowledge search.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSearchRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f64>,
}

/// Response from knowledge search.
#[derive(Debug, Deserialize)]
pub struct KnowledgeSearchResponse {
    pub chunks: Vec<KnowledgeChunk>,
}

/// A single knowledge chunk from RAG search.
#[derive(Debug, Deserialize)]
pub struct KnowledgeChunk {
    pub content: String,
    pub score: f64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default = "default_empty_object")]
    pub metadata: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_session_request_serializes() {
        let req = CreateSessionRequest {
            agent_id: Some("orch-1".into()),
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("agentId"));
        assert!(json.contains("orch-1"));
        // Empty metadata should be skipped
        assert!(!json.contains("metadata"));
    }

    #[test]
    fn task_event_deserializes_content() {
        let json = r#"{"type": "content", "content": "hello"}"#;
        let event: TaskEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, TaskEvent::Content { content } if content == "hello"));
    }

    #[test]
    fn task_event_deserializes_completed() {
        let json = r#"{"type": "completed", "output": "done", "tokens_used": 150}"#;
        let event: TaskEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, TaskEvent::Completed { output, tokens_used } if output == "done" && tokens_used == Some(150)));
    }

    #[test]
    fn task_event_deserializes_error() {
        let json = r#"{"type": "error", "message": "rate limited", "code": "429"}"#;
        let event: TaskEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, TaskEvent::Error { message, .. } if message == "rate limited"));
    }

    #[test]
    fn mcp_tool_def_deserializes_minimal() {
        let json = r#"{"name": "llm_chat"}"#;
        let tool: McpToolDef = serde_json::from_str(json).unwrap();
        assert_eq!(tool.name, "llm_chat");
        assert!(tool.description.is_none());
        assert_eq!(tool.input_schema, serde_json::json!({}));
    }

    #[test]
    fn knowledge_chunk_deserializes() {
        let json = r#"{"content": "Rust is fast", "score": 0.95, "source": "docs.md"}"#;
        let chunk: KnowledgeChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.content, "Rust is fast");
        assert_eq!(chunk.score, 0.95);
        assert_eq!(chunk.source.as_deref(), Some("docs.md"));
    }

    #[test]
    fn submit_task_request_serializes() {
        let req = SubmitTaskRequest {
            input: "Analyze this repo".into(),
            agent_type: Some("ORCHESTRATOR".into()),
            context: None,
            priority: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("Analyze this repo"));
        assert!(json.contains("ORCHESTRATOR"));
        // None fields should be skipped
        assert!(!json.contains("context"));
        assert!(!json.contains("priority"));
    }

    // ── Forward compatibility ───────────────────────────────────────────

    #[test]
    fn task_event_unknown_type_deserializes_as_unknown() {
        let json = r#"{"type": "heartbeat"}"#;
        let event: TaskEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, TaskEvent::Unknown));
    }

    // ── camelCase alias support ─────────────────────────────────────────

    #[test]
    fn task_event_completed_accepts_camel_case_tokens_used() {
        let json = r#"{"type": "completed", "output": "ok", "tokensUsed": 200}"#;
        let event: TaskEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, TaskEvent::Completed { tokens_used: Some(200), .. }));
    }

    #[test]
    fn task_event_tool_result_accepts_camel_case_is_error() {
        let json = r#"{"type": "tool_result", "name": "bash", "output": "fail", "isError": true}"#;
        let event: TaskEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, TaskEvent::ToolResult { is_error: true, .. }));
    }

    #[test]
    fn task_event_started_accepts_camel_case_agent_id() {
        let json = r#"{"type": "started", "agentId": "bot-1"}"#;
        let event: TaskEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, TaskEvent::Started { agent_id: Some(id) } if id == "bot-1"));
    }

    // ── Real API format tests ───────────────────────────────────────────

    #[test]
    fn agent_session_deserializes_from_real_api() {
        let json = r#"{"id":"session_123","status":"active","agentIds":[],"createdAt":"2026-01-01T00:00:00Z"}"#;
        let session: AgentSession = serde_json::from_str(json).unwrap();
        assert_eq!(session.id, "session_123");
        assert_eq!(session.status, "active");
        assert!(session.agent_ids.is_empty());
    }

    #[test]
    fn mcp_tool_call_response_text_extracts_content() {
        let json = r#"{"content":[{"type":"text","text":"hello"},{"type":"text","text":"world"}],"isError":false}"#;
        let resp: McpToolCallResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.text(), "hello\nworld");
        assert!(!resp.is_error);
    }

    #[test]
    fn mcp_tool_call_response_empty_content() {
        let json = r#"{"content":[],"isError":false}"#;
        let resp: McpToolCallResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.text(), "");
    }

    #[test]
    fn task_sync_response_deserializes() {
        let json = r#"{
            "taskId": "task_1",
            "sessionId": "session_1",
            "combinedOutput": "Hello!",
            "allSucceeded": true,
            "agentResults": [{
                "agentId": "bot-1",
                "agentName": "Test",
                "output": "Hello!",
                "success": true,
                "latencyMs": 1000,
                "tokenUsage": {"totalTokens": 500}
            }],
            "totalLatencyMs": 1000,
            "strategy": "first_success"
        }"#;
        let resp: TaskSyncResponse = serde_json::from_str(json).unwrap();
        assert!(resp.all_succeeded);
        assert_eq!(resp.combined_output, "Hello!");
        assert_eq!(resp.agent_results[0].latency_ms, 1000);
        assert_eq!(resp.agent_results[0].token_usage.as_ref().unwrap().total_tokens, 500);
    }

    #[test]
    fn knowledge_chunk_metadata_defaults_to_empty_object() {
        let json = r#"{"content": "text", "score": 0.5}"#;
        let chunk: KnowledgeChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.metadata, serde_json::json!({}));
    }
}
