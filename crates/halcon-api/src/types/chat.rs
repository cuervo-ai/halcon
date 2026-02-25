//! Chat session types for the Halcon API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a chat session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatSessionStatus {
    /// Session is idle, ready to receive a new message.
    Idle,
    /// Agent is currently executing a turn.
    Executing,
    /// Agent is waiting for the user to resolve a permission request.
    AwaitingPermission,
    /// The last turn ended with an error.
    Error,
    /// The turn was cancelled by the user.
    Cancelled,
}

/// A chat session (conversation) managed by the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: Uuid,
    pub title: Option<String>,
    pub model: String,
    pub provider: String,
    pub status: ChatSessionStatus,
    pub message_count: usize,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A message in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: Uuid,
    pub session_id: Uuid,
    pub role: ChatRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// Role of a chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

/// Token usage for a conversation turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatTokenUsage {
    pub input: u64,
    pub output: u64,
    pub thinking: u64,
    pub total: u64,
}

/// Request to create a new chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub model: String,
    pub provider: String,
    pub title: Option<String>,
    pub system_prompt: Option<String>,
    pub working_directory: Option<String>,
}

/// Response when a session is created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionResponse {
    pub session: ChatSession,
}

/// An inline media attachment in a chat message request (base64-encoded).
///
/// Mirrors `halcon_core::traits::MediaAttachmentInline` for HTTP serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachmentInline {
    /// Original filename (display and MIME hint).
    pub filename: String,
    /// MIME type: "image/jpeg", "image/png", "image/webp", "image/gif", "text/plain", etc.
    pub content_type: String,
    /// Base64-encoded raw file bytes.
    pub data_base64: String,
}

/// Metadata returned when an attachment has been processed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    pub filename: String,
    pub content_type: String,
    pub size_bytes: usize,
    pub modality: String,
}

/// Request to submit a user message and start execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitMessageRequest {
    pub content: String,
    pub orchestrate: Option<bool>,
    pub expert: Option<bool>,
    /// Optional inline media attachments (images, text files, etc.).
    /// Images with MIME type image/jpeg, image/png, image/webp, image/gif are
    /// forwarded as vision content blocks to the provider.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MediaAttachmentInline>,
}

/// Response when a message is submitted (sync, before streaming starts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitMessageResponse {
    pub session_id: Uuid,
    pub user_message_id: Uuid,
    pub status: ChatSessionStatus,
}

/// Request to resolve a permission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvePermissionRequest {
    pub decision: PermissionDecisionStr,
}

/// Permission decision as a string (for HTTP API).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecisionStr {
    Approve,
    Deny,
}

/// Response when a permission is resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvePermissionResponse {
    pub request_id: Uuid,
    pub decision: PermissionDecisionStr,
    pub tool_executed: bool,
}

/// List of chat sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessionsResponse {
    pub sessions: Vec<ChatSession>,
    pub total: usize,
}

/// A message returned by the messages endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageEntry {
    pub role: String,
    pub content: String,
}

/// Response for GET /sessions/{id}/messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMessagesResponse {
    pub session_id: uuid::Uuid,
    pub messages: Vec<ChatMessageEntry>,
    pub total: usize,
}

/// Request to update a session's title.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessionTitleRequest {
    pub title: String,
}

/// Response when a session title is updated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessionTitleResponse {
    pub session_id: Uuid,
    pub title: String,
}

/// Serializable snapshot of a session for cross-restart persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistableSession {
    pub session: ChatSession,
    pub history: Vec<(String, String)>,
}

/// Internal handle for a running chat session execution.
/// Stored in AppState.active_chat_sessions.
/// Only available when the server feature is enabled (requires tokio).
#[cfg(feature = "server")]
#[derive(Debug, Clone)]
pub struct ChatSessionHandle {
    pub session: ChatSession,
    pub cancellation_tx: tokio::sync::watch::Sender<bool>,
    pub permission_tx: Option<tokio::sync::mpsc::UnboundedSender<PermissionDecisionStr>>,
    /// In-memory conversation history: ordered (role, content) pairs.
    /// Shared via Arc so the event-translation task can append turns after completion.
    pub history: std::sync::Arc<tokio::sync::Mutex<Vec<(String, String)>>>,
}

#[cfg(feature = "server")]
impl ChatSessionHandle {
    pub fn new(session: ChatSession) -> (Self, tokio::sync::watch::Receiver<bool>) {
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        (
            Self {
                session,
                cancellation_tx: cancel_tx,
                permission_tx: None,
                history: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
            },
            cancel_rx,
        )
    }

    pub fn cancel(&self) {
        let _ = self.cancellation_tx.send(true);
    }
}
