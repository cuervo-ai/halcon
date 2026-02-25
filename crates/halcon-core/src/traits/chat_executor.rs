//! ChatExecutor trait — primary port for headless chat execution.
//!
//! Defined in halcon-core (not halcon-cli) to break the circular dependency:
//!   halcon-api cannot import halcon-cli (halcon-cli already imports halcon-api).
//!   halcon-core has no such constraint — both crates can depend on it freely.

use std::collections::HashMap;

use async_trait::async_trait;
use uuid::Uuid;

/// An inline media attachment sent with a chat message (base64-encoded).
///
/// Defined here (in halcon-core) so both halcon-api and halcon-cli can use it
/// without introducing a circular dependency.
#[derive(Debug, Clone)]
pub struct MediaAttachmentInline {
    /// Original filename, used for display and MIME-type hint.
    pub filename: String,
    /// MIME type detected from magic bytes: "image/jpeg", "image/png",
    /// "image/webp", "image/gif", "text/plain", etc.
    pub content_type: String,
    /// Base64-encoded raw file bytes (standard alphabet, no line breaks).
    pub data_base64: String,
}

impl MediaAttachmentInline {
    /// Detect the primary modality from the MIME type.
    pub fn modality(&self) -> &'static str {
        let ct = self.content_type.as_str();
        if ct.starts_with("image/") {
            "image"
        } else if ct.starts_with("audio/") {
            "audio"
        } else if ct.starts_with("video/") {
            "video"
        } else {
            "text"
        }
    }

    /// Return true if this attachment is a vision-capable image.
    pub fn is_vision_image(&self) -> bool {
        matches!(
            self.content_type.as_str(),
            "image/jpeg" | "image/png" | "image/webp" | "image/gif"
        )
    }
}

/// Minimal input for a headless chat execution turn.
#[derive(Debug, Clone)]
pub struct ChatExecutionInput {
    pub session_id: Uuid,
    pub user_message: String,
    pub model: String,
    pub provider: String,
    pub working_directory: String,
    pub orchestrate: bool,
    pub expert: bool,
    pub system_prompt: Option<String>,
    pub history: Vec<ChatHistoryMessage>,
    /// Optional media attachments to include with this turn.
    /// Images are sent as vision content blocks; other types are described as text.
    pub media_attachments: Vec<MediaAttachmentInline>,
}

/// A historical message in the conversation.
#[derive(Debug, Clone)]
pub struct ChatHistoryMessage {
    pub role: String, // "user" | "assistant" | "system"
    pub content: String,
}

/// An event emitted by the executor during a chat turn.
#[derive(Debug, Clone)]
pub enum ChatExecutionEvent {
    /// A streamed text token (output or thinking).
    Token {
        text: String,
        is_thinking: bool,
        sequence_num: u64,
    },
    /// Periodic thinking progress (throttled, not per-token).
    ThinkingProgress {
        chars_so_far: usize,
        elapsed_secs: f32,
    },
    /// A tool invocation started.
    ToolStarted {
        name: String,
        risk_level: String,
    },
    /// A tool invocation completed.
    ToolCompleted {
        name: String,
        duration_ms: u64,
        success: bool,
    },
    /// User permission is required before a tool can execute.
    PermissionRequired {
        request_id: Uuid,
        tool_name: String,
        risk_level: String,
        description: String,
        deadline_secs: u64,
        args_preview: HashMap<String, String>,
    },
    /// A permission request timed out — the tool was automatically denied (fail-closed).
    /// B1: Allows clients to dismiss pending permission modals deterministically
    /// without relying on silence / guessing whether the timeout fired.
    PermissionExpired {
        request_id: Uuid,
    },
    /// A sub-agent was spawned by the orchestrator.
    SubAgentStarted {
        id: String,
        description: String,
        wave: usize,
        allowed_tools: Vec<String>,
    },
    /// A sub-agent completed its task.
    SubAgentCompleted {
        id: String,
        success: bool,
        summary: String,
        tools_used: Vec<String>,
        duration_ms: u64,
    },
    /// The turn completed successfully.
    Completed {
        assistant_message_id: Uuid,
        stop_reason: String,
        input_tokens: u64,
        output_tokens: u64,
        total_duration_ms: u64,
    },
    /// The turn failed.
    Failed {
        error_code: String,
        message: String,
        recoverable: bool,
    },
}

/// Primary port for headless chat execution.
///
/// Implementations (e.g. AgentBridgeImpl in halcon-cli) call run_agent_loop()
/// and translate the results into ChatExecutionEvent via the event_tx channel.
///
/// # Channel protocol
/// - `event_tx`: executor emits events; handler translates to WsServerEvent
/// - `cancel_rx`: HTTP DELETE /active sends `true` → executor detects and stops
/// - `perm_decision_rx`: HTTP POST /permissions sends `(request_id, approved)` → executor routes to PermissionChecker
///
/// # Invariants
/// - Executor MUST emit `Completed` or `Failed` before returning from `execute()`.
/// - `event_tx` may be dropped if the WebSocket client disconnects; executor MUST
///   not panic on send errors — drop the event and continue.
#[async_trait]
pub trait ChatExecutor: Send + Sync {
    async fn execute(
        &self,
        input: ChatExecutionInput,
        event_tx: tokio::sync::mpsc::UnboundedSender<ChatExecutionEvent>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        perm_decision_rx: tokio::sync::mpsc::UnboundedReceiver<(Uuid, bool)>,
    );
}
