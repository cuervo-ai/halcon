use serde::{Deserialize, Serialize};

/// Information about a model available through a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub supports_streaming: bool,
    pub supports_tools: bool,
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_reasoning: bool,
    pub cost_per_input_token: f64,
    pub cost_per_output_token: f64,
}

/// A request to a model provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub stream: bool,
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: MessageContent,
}

/// Message content: either plain text or structured blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

impl MessageContent {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            MessageContent::Text(s) => Some(s),
            _ => None,
        }
    }
}

/// Supported image media types (detected via magic bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImageMediaType {
    Jpeg,
    Png,
    Webp,
    Gif,
}

impl ImageMediaType {
    /// Detect image type from the first bytes of file data.
    pub fn from_magic(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) { return Some(Self::Jpeg); }
        if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) { return Some(Self::Png); }
        if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            return Some(Self::Webp);
        }
        if bytes.starts_with(b"GIF8") { return Some(Self::Gif); }
        None
    }

    /// Return the canonical MIME type string.
    pub fn as_mime_str(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png  => "image/png",
            Self::Webp => "image/webp",
            Self::Gif  => "image/gif",
        }
    }
}

/// Source of image data for multimodal requests.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ImageSource {
    /// Base64-encoded image data with known media type.
    Base64 { media_type: ImageMediaType, data: String },
    /// A URL pointing to an image (not supported by all providers).
    Url { url: String },
    /// A local filesystem path (must be resolved before API use).
    LocalPath { path: String },
}

/// A content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    /// An image block (for vision-capable models).
    #[serde(rename = "image")]
    Image { source: ImageSource },
    /// The result of audio transcription.
    #[serde(rename = "audio_transcript")]
    AudioTranscript { text: String, duration_secs: Option<f32>, confidence: Option<f32> },
}

/// Conversation role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

/// Tool definition for model API calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A chunk from a streaming model response.
#[derive(Debug, Clone)]
pub enum ModelChunk {
    /// A text delta.
    TextDelta(String),
    /// A tool use content block has started (emitted on content_block_start).
    ToolUseStart {
        index: u32,
        id: String,
        name: String,
    },
    /// A partial JSON delta for tool input (emitted on input_json_delta).
    ToolUseDelta { index: u32, partial_json: String },
    /// A fully assembled tool use (produced by the accumulator, not directly by the provider).
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Usage information (may arrive at end of stream).
    Usage(TokenUsage),
    /// Stream completed with a stop reason.
    Done(StopReason),
    /// An error occurred during streaming.
    Error(String),
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    StopSequence,
}

/// Token usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: Option<u32>,
    pub cache_creation_tokens: Option<u32>,
}

impl TokenUsage {
    pub fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

/// Estimated cost for a model request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenCost {
    pub estimated_input_tokens: u32,
    pub estimated_cost_usd: f64,
}
