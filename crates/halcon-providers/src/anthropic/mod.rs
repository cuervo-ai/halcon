//! Anthropic Claude API provider with SSE streaming.
//!
//! Implements `ModelProvider` for the Anthropic Messages API.
//! Uses reqwest + eventsource-stream to parse Server-Sent Events.
//! Includes configurable timeouts and retry with exponential backoff.

pub mod types;

use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use tracing::{debug, info, instrument, warn};

use halcon_core::error::{HalconError, Result};
use halcon_core::traits::ModelProvider;
use halcon_core::types::{
    ChatMessage, HttpConfig, MessageContent, ModelChunk, ModelInfo, ModelRequest, StopReason,
    TokenCost, TokenUsage,
};

use crate::http;
use types::{
    ApiContentBlock, ApiImageSource, ApiMessage, ApiMessageContent, ApiRequest, ApiSystem,
    ApiSystemBlock, ApiToolDefinition, SseEvent,
};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";
/// OAuth beta flag embedded in the `anthropic-beta` header when combined with
/// the prompt-caching flag.  Used in `build_headers()` as the literal string
/// `"prompt-caching-2024-07-31,oauth-2025-04-20"` for OAuth sessions.
#[allow(dead_code)] // present for documentation; inlined in build_headers()
const OAUTH_BETA_FLAG: &str = "oauth-2025-04-20";
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Anthropic Claude provider.
///
/// Streams responses via SSE from the Messages API.
/// Supports configurable timeouts and retry with exponential backoff.
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    http_config: HttpConfig,
    models: Vec<ModelInfo>,
}

/// Safety: Debug impl redacts the API key to prevent accidental exposure in logs.
impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("http_config", &self.http_config)
            .finish()
    }
}

impl AnthropicProvider {
    /// Static fallback models — used when live discovery fails or at construction.
    /// These are replaced by live-discovered models via `discover_models()`.
    fn default_models() -> Vec<ModelInfo> {
        crate::model_registry::static_fallback_models("anthropic")
    }

    /// Create a new provider with the given API key and default HTTP config.
    ///
    /// Starts with static fallback models. Call `discover_models()` to upgrade
    /// to live-discovered models from the Anthropic API.
    pub fn new(api_key: String) -> Self {
        let http_config = HttpConfig::default();
        Self {
            client: http::build_client(&http_config),
            api_key,
            base_url: DEFAULT_BASE_URL.to_string(),
            http_config,
            models: Self::default_models(),
        }
    }

    /// Discover available models from the provider API.
    ///
    /// Anthropic doesn't have a standard /v1/models endpoint, so we use the
    /// static registry which is kept up-to-date with each Halcon release.
    /// The static_fallback_models already contains the correct capabilities.
    pub async fn discover_models(&mut self) {
        // Anthropic has no public model listing API — static registry is authoritative.
        // New models are discovered via Cenzontle gateway (/v1/llm/models) which
        // proxies Anthropic models with dynamic discovery.
        debug!("Anthropic: using static model registry (no live /v1/models API)");
    }

    /// Create a provider with a custom base URL (for testing / proxies).
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let http_config = HttpConfig::default();
        Self {
            client: http::build_client(&http_config),
            api_key,
            base_url,
            http_config,
            models: Self::default_models(),
        }
    }

    /// Create a provider with full HTTP configuration.
    pub fn with_config(api_key: String, base_url: Option<String>, http_config: HttpConfig) -> Self {
        Self {
            client: http::build_client(&http_config),
            api_key,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            http_config,
            models: Self::default_models(),
        }
    }

    /// Build the request body from a ModelRequest.
    ///
    /// ## Prompt caching (Q2 — Sprint 1)
    ///
    /// When the system prompt is non-empty, it is serialised as a **structured
    /// block array** with `cache_control: {type: "ephemeral"}` on the sole text
    /// block.  This instructs Anthropic's API to treat everything up to and
    /// including the system prompt as a cacheable KV prefix.
    ///
    /// Cache economics (as of anthropic-beta: prompt-caching-2024-07-31):
    /// - Cache write:  125 % of normal input cost
    /// - Cache read:    10 % of normal input cost
    /// - Minimum cacheable prefix: 1,024 tokens
    /// - TTL: 5 minutes (refreshed on every hit)
    ///
    /// Break-even: 2 cache hits. An average agent session makes 10-50 rounds →
    /// cost reduction ≈ 85-90 % on system prompt tokens.
    fn build_api_request(request: &ModelRequest) -> ApiRequest {
        let messages: Vec<ApiMessage> = request
            .messages
            .iter()
            .filter(|m| m.role != halcon_core::types::Role::System)
            .map(|m| ApiMessage {
                role: match m.role {
                    halcon_core::types::Role::User => "user".into(),
                    halcon_core::types::Role::Assistant => "assistant".into(),
                    halcon_core::types::Role::System => unreachable!(),
                },
                content: message_to_api_content(m),
            })
            .collect();

        let tools: Vec<ApiToolDefinition> = request
            .tools
            .iter()
            .map(|t| ApiToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect();

        // Attach cache_control to the system prompt when non-empty.
        // The API requires at least 1,024 tokens to create a cache entry; shorter
        // prompts still round-trip correctly — the cache_control directive is
        // simply ignored by the server.
        let system = request.system.as_deref().map(|text| {
            if text.is_empty() {
                ApiSystem::Text(text.to_string())
            } else {
                ApiSystem::Blocks(vec![ApiSystemBlock::cached(text)])
            }
        });

        ApiRequest {
            model: request.model.clone(),
            messages,
            max_tokens: request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
            temperature: request.temperature,
            system,
            stream: true,
            tools,
        }
    }

    /// Build HTTP headers for the Anthropic API.
    ///
    /// Uses `x-api-key` for API keys or `Authorization: Bearer` for OAuth tokens.
    /// Check if the credential is an API key (uses x-api-key header)
    /// vs an OAuth access token (uses Authorization: Bearer header).
    ///
    /// Anthropic key formats:
    /// - API keys:      `sk-ant-api*-...`  → x-api-key header
    /// - OAuth tokens:  `sk-ant-oat*-...`  → Authorization: Bearer header
    fn is_api_key(key: &str) -> bool {
        key.starts_with("sk-ant-api")
    }

    fn build_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if Self::is_api_key(&self.api_key) {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(&self.api_key)
                    .unwrap_or_else(|_| HeaderValue::from_static("")),
            );
            // Enable prompt-caching beta (Q2 — Sprint 1).
            // Break-even: 2 cache hits on the same system prompt prefix.
            // TTL: 5 min, refreshed on every hit. Min prefix: 1,024 tokens.
            headers.insert(
                "anthropic-beta",
                HeaderValue::from_static("prompt-caching-2024-07-31"),
            );
            debug!("auth: x-api-key + prompt-caching beta");
        } else {
            headers.insert(
                "Authorization",
                HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                    .unwrap_or_else(|_| HeaderValue::from_static("")),
            );
            // OAuth Bearer: combine both beta flags in a single header.
            // Comma-separated multi-value is supported by the Anthropic API.
            headers.insert(
                "anthropic-beta",
                HeaderValue::from_static("prompt-caching-2024-07-31,oauth-2025-04-20"),
            );
            debug!("auth: Bearer + prompt-caching + oauth beta flags");
        }
        headers.insert("anthropic-version", HeaderValue::from_static(API_VERSION));
        headers
    }

    /// Build a boxed SSE stream from a successful HTTP response.
    fn build_sse_stream(response: reqwest::Response) -> BoxStream<'static, Result<ModelChunk>> {
        use eventsource_stream::Eventsource as _;
        let byte_stream = response.bytes_stream();
        let sse_stream = byte_stream.eventsource();

        Box::pin(sse_stream.flat_map(move |sse_result| {
            let chunks: Vec<Result<ModelChunk>> = match sse_result {
                Ok(event) => {
                    if event.data.is_empty() || event.data == "[DONE]" {
                        vec![]
                    } else {
                        match serde_json::from_str::<SseEvent>(&event.data) {
                            Ok(sse_event) => AnthropicProvider::map_sse_event(&sse_event)
                                .into_iter()
                                .map(Ok)
                                .collect(),
                            Err(e) => {
                                warn!(data = %event.data, error = %e, "failed to parse SSE event");
                                vec![]
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "SSE stream error");
                    vec![Ok(ModelChunk::Error(format!("SSE error: {e}")))]
                }
            };
            futures::stream::iter(chunks)
        }))
    }

    /// Convert an SSE event into zero or more ModelChunks.
    fn map_sse_event(event: &SseEvent) -> Vec<ModelChunk> {
        match event {
            SseEvent::MessageStart { message } => {
                let mut chunks = Vec::new();
                if let Some(usage) = &message.usage {
                    chunks.push(ModelChunk::Usage(TokenUsage {
                        input_tokens: usage.input_tokens,
                        output_tokens: 0,
                        ..Default::default()
                    }));
                }
                chunks
            }
            SseEvent::ContentBlockDelta { index, delta } => match delta {
                types::DeltaData::TextDelta { text } => {
                    vec![ModelChunk::TextDelta(text.clone())]
                }
                types::DeltaData::InputJsonDelta { partial_json } => {
                    vec![ModelChunk::ToolUseDelta {
                        index: *index,
                        partial_json: partial_json.clone(),
                    }]
                }
            },
            SseEvent::MessageDelta { delta, usage } => {
                let mut chunks = Vec::new();
                if let Some(u) = usage {
                    chunks.push(ModelChunk::Usage(TokenUsage {
                        input_tokens: 0,
                        output_tokens: u.output_tokens,
                        ..Default::default()
                    }));
                }
                if let Some(reason) = &delta.stop_reason {
                    let stop = match reason.as_str() {
                        "end_turn" => StopReason::EndTurn,
                        "max_tokens" => StopReason::MaxTokens,
                        "tool_use" => StopReason::ToolUse,
                        "stop_sequence" => StopReason::StopSequence,
                        _ => StopReason::EndTurn,
                    };
                    chunks.push(ModelChunk::Done(stop));
                }
                chunks
            }
            SseEvent::MessageStop => vec![],
            SseEvent::Ping => vec![],
            SseEvent::ContentBlockStart {
                index,
                content_block,
            } => {
                if content_block.block_type == "tool_use" {
                    if let (Some(id), Some(name)) = (&content_block.id, &content_block.name) {
                        return vec![ModelChunk::ToolUseStart {
                            index: *index,
                            id: id.clone(),
                            name: name.clone(),
                        }];
                    }
                }
                vec![]
            }
            SseEvent::ContentBlockStop { .. } => vec![],
            SseEvent::Error { error } => {
                vec![ModelChunk::Error(format!(
                    "{}: {}",
                    error.error_type, error.message
                ))]
            }
        }
    }
}

/// Convert a ChatMessage's content into the Anthropic API format.
///
/// Plain text messages remain as strings. Messages with tool blocks
/// are converted to structured content arrays.
fn message_to_api_content(msg: &ChatMessage) -> ApiMessageContent {
    match &msg.content {
        MessageContent::Text(t) => ApiMessageContent::Text(t.clone()),
        MessageContent::Blocks(blocks) => {
            let api_blocks: Vec<ApiContentBlock> = blocks
                .iter()
                .map(|b| match b {
                    halcon_core::types::ContentBlock::Text { text } => {
                        ApiContentBlock::Text { text: text.clone() }
                    }
                    halcon_core::types::ContentBlock::ToolUse { id, name, input } => {
                        ApiContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        }
                    }
                    halcon_core::types::ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => ApiContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: content.clone(),
                        is_error: *is_error,
                    },
                    halcon_core::types::ContentBlock::Image { source } => {
                        use halcon_core::types::ImageSource;
                        match source {
                            ImageSource::Base64 { media_type, data } => ApiContentBlock::Image {
                                source: ApiImageSource::Base64 {
                                    media_type: media_type.as_mime_str().to_string(),
                                    data: data.clone(),
                                },
                            },
                            ImageSource::Url { url } => {
                                tracing::warn!(url = %url, "Anthropic does not support image URL; using text placeholder");
                                ApiContentBlock::Text {
                                    text: format!("[Image URL not supported by Anthropic: {url}]"),
                                }
                            }
                            ImageSource::LocalPath { path } => {
                                tracing::warn!(path = %path, "Unresolved LocalPath image; using text placeholder");
                                ApiContentBlock::Text {
                                    text: format!("[Unresolved local image: {path}]"),
                                }
                            }
                        }
                    }
                    halcon_core::types::ContentBlock::AudioTranscript { text, .. } => ApiContentBlock::Text {
                        text: format!("[Audio transcript]: {text}"),
                    },
                })
                .collect();
            ApiMessageContent::Blocks(api_blocks)
        }
    }
}

/// Extract plain text from a ChatMessage (for cost estimation).
fn message_to_text(msg: &ChatMessage) -> String {
    match &msg.content {
        MessageContent::Text(t) => t.clone(),
        MessageContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                halcon_core::types::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

impl AnthropicProvider {
    /// Wrap a raw SSE stream to record TTFT (time-to-first-token) on the
    /// current tracing span.
    ///
    /// TTFT fires the moment the first `TextDelta` or `ToolUseStart` chunk
    /// flows through the stream, recording:
    /// - `ttft_ms`: wall-clock ms from `invoke()` entry to first token
    /// - A debug log event with `ttft_ms` and `connect_ms` side-by-side
    ///
    /// Why TTFT matters: it is the dominant user-perceived latency metric.
    /// A prompt-cache hit (Anthropic prompt-caching beta, Q2) typically halves
    /// TTFT from ~800 ms to ~150 ms by reusing the system-prompt KV prefix.
    fn wrap_stream_with_ttft(
        stream: BoxStream<'static, Result<ModelChunk>>,
        connect_ms: u64,
    ) -> BoxStream<'static, Result<ModelChunk>> {
        use futures::StreamExt;
        let mut first_token = true;
        let ttft_start = std::time::Instant::now();
        let span = tracing::Span::current();

        Box::pin(stream.map(move |chunk| {
            if first_token {
                if let Ok(ModelChunk::TextDelta(_)) | Ok(ModelChunk::ToolUseStart { .. }) = &chunk {
                    first_token = false;
                    let ttft_ms = ttft_start.elapsed().as_millis() as u64;
                    span.record("ttft_ms", ttft_ms);
                    tracing::debug!(
                        parent: &span,
                        ttft_ms,
                        connect_ms,
                        "anthropic: first token received"
                    );
                }
            }
            chunk
        }))
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn supported_models(&self) -> &[ModelInfo] {
        &self.models
    }

    #[instrument(
        skip_all,
        fields(
            provider = "anthropic",
            model   = %request.model,
            msgs    = request.messages.len(),
            // Populated at runtime:
            ttft_ms,        // time-to-first-token (ms)
            total_ms,       // total invoke latency (ms)
            input_tokens,   // from usage chunk
            output_tokens,  // from usage chunk
            cache_hit,      // true when Anthropic signals a cache read
        )
    )]
    async fn invoke(
        &self,
        request: &ModelRequest,
    ) -> Result<BoxStream<'static, Result<ModelChunk>>> {
        let start = std::time::Instant::now();
        let api_request = Self::build_api_request(request);
        let url = format!("{}/v1/messages", self.base_url);
        let headers = self.build_headers();
        let msg_count = request.messages.len();

        debug!(
            model = %api_request.model,
            messages = msg_count,
            max_tokens = api_request.max_tokens,
            "anthropic: sending request"
        );

        // Serialize once, wrap in Bytes for O(1) clone on retries.
        let body: bytes::Bytes = serde_json::to_string(&api_request)
            .map_err(|e| HalconError::ApiError {
                message: format!("failed to serialize request: {e}"),
                status: None,
            })?
            .into();

        let request_timeout = Duration::from_secs(self.http_config.request_timeout_secs);
        let max_retries = self.http_config.max_retries;
        let base_delay = self.http_config.retry_base_delay_ms;

        // Retry loop for transient errors (429, 5xx).
        let mut last_error = None;
        for attempt in 0..=max_retries {
            if attempt > 0 {
                let delay = http::backoff_delay(base_delay, attempt - 1);
                debug!(attempt, delay_ms = delay.as_millis(), "retrying request");
                tokio::time::sleep(delay).await;
            }

            let send_fut = self
                .client
                .post(&url)
                .headers(headers.clone())
                .body(body.clone())
                .send();

            let response = match tokio::time::timeout(request_timeout, send_fut).await {
                Ok(Ok(resp)) => resp,
                Ok(Err(e)) => {
                    // Connection or request error.
                    if e.is_timeout() {
                        let err = HalconError::RequestTimeout {
                            provider: "anthropic".into(),
                            timeout_secs: self.http_config.request_timeout_secs,
                        };
                        if attempt < max_retries {
                            warn!(attempt, error = %e, "request timed out, will retry");
                            last_error = Some(err);
                            continue;
                        }
                        return Err(err);
                    }
                    if e.is_connect() {
                        let err = HalconError::ConnectionError {
                            provider: "anthropic".into(),
                            message: format!("{e}"),
                        };
                        if attempt < max_retries {
                            warn!(attempt, error = %e, "connection failed, will retry");
                            last_error = Some(err);
                            continue;
                        }
                        return Err(err);
                    }
                    return Err(HalconError::ApiError {
                        message: format!("HTTP request failed: {e}"),
                        status: e.status().map(|s| s.as_u16()),
                    });
                }
                Err(_elapsed) => {
                    let err = HalconError::RequestTimeout {
                        provider: "anthropic".into(),
                        timeout_secs: self.http_config.request_timeout_secs,
                    };
                    if attempt < max_retries {
                        warn!(attempt, "request timed out, will retry");
                        last_error = Some(err);
                        continue;
                    }
                    return Err(err);
                }
            };

            let status = response.status();
            if status.is_success() {
                let connect_ms = start.elapsed().as_millis() as u64;
                // Record connection latency on the span (TTFT recorded when
                // first token flows through the stream wrapper below).
                tracing::Span::current().record("total_ms", connect_ms);
                info!(
                    model = %api_request.model,
                    connect_ms,
                    attempts = attempt + 1,
                    "anthropic: stream established"
                );
                let raw_stream = Self::build_sse_stream(response);
                return Ok(Self::wrap_stream_with_ttft(raw_stream, connect_ms));
            }

            let status_code = status.as_u16();

            // Non-retryable auth error.
            if status_code == 401 {
                let body_text = response.text().await.unwrap_or_default();
                let msg = serde_json::from_str::<types::ApiErrorResponse>(&body_text)
                    .map(|e| e.error.message)
                    .unwrap_or_else(|_| body_text);
                return Err(HalconError::AuthFailed(msg));
            }

            // Check if retryable.
            if http::is_retryable_status(status_code) && attempt < max_retries {
                // Parse Retry-After header for 429.
                if status_code == 429 {
                    if let Some(retry_secs) = http::parse_retry_after(response.headers()) {
                        debug!(retry_secs, "got Retry-After header, waiting");
                        tokio::time::sleep(Duration::from_secs(retry_secs)).await;
                        continue;
                    }
                }
                let body_text = response.text().await.unwrap_or_default();
                warn!(
                    attempt,
                    status = status_code,
                    "retryable error: {body_text}"
                );
                last_error = Some(HalconError::ApiError {
                    message: format!("HTTP {status_code}: {body_text}"),
                    status: Some(status_code),
                });
                continue;
            }

            // Non-retryable or exhausted retries.
            // Clone headers NOW — before `response.text().await` consumes the response,
            // making headers inaccessible.  This was previously a bug: parsing Retry-After
            // from `reqwest::header::HeaderMap::new()` always yielded None, so all
            // rate-limited exhausted responses fell back to a hardcoded 30s default.
            let response_headers = response.headers().clone();
            let body_text = response.text().await.unwrap_or_default();
            if status_code == 429 {
                let retry_after = http::parse_retry_after(&response_headers).unwrap_or(30);
                return Err(HalconError::RateLimited {
                    provider: "anthropic".into(),
                    retry_after_secs: retry_after,
                });
            }

            if let Ok(err_resp) = serde_json::from_str::<types::ApiErrorResponse>(&body_text) {
                return Err(HalconError::ApiError {
                    message: format!("{}: {}", err_resp.error.error_type, err_resp.error.message),
                    status: Some(status_code),
                });
            }

            return Err(HalconError::ApiError {
                message: format!("HTTP {status_code}: {body_text}"),
                status: Some(status_code),
            });
        }

        // Should not reach here, but return last error if we do.
        Err(last_error.unwrap_or_else(|| HalconError::ApiError {
            message: "request failed after retries".into(),
            status: None,
        }))
    }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    fn estimate_cost(&self, request: &ModelRequest) -> TokenCost {
        // Rough estimate: ~4 chars per token.
        let input_chars: usize = request
            .messages
            .iter()
            .map(|m| message_to_text(m).len())
            .sum();
        let estimated_tokens = (input_chars / 4) as u32;

        // Find model cost or use Sonnet defaults.
        let cost_per_input = self
            .supported_models()
            .iter()
            .find(|m| m.id == request.model)
            .map(|m| m.cost_per_input_token)
            .unwrap_or(3.0 / 1_000_000.0);

        TokenCost {
            estimated_input_tokens: estimated_tokens,
            estimated_cost_usd: estimated_tokens as f64 * cost_per_input,
        }
    }

    fn tool_format(&self) -> halcon_core::types::ToolFormat {
        halcon_core::types::ToolFormat::AnthropicInputSchema
    }

    fn tokenizer_hint(&self) -> halcon_core::types::TokenizerHint {
        halcon_core::types::TokenizerHint::ClaudeBpe
    }
}

// --- Public helpers for cross-provider reuse (Bedrock, Vertex) ---
// These wrappers expose private methods so Bedrock/Vertex can reuse
// the Anthropic request format without duplicating serialization logic.
impl AnthropicProvider {
    /// Public wrapper for `build_api_request` — used by BedrockProvider.
    pub fn build_api_request_pub(request: &ModelRequest) -> ApiRequest {
        Self::build_api_request(request)
    }

    /// Public wrapper for `map_sse_event` — used by BedrockProvider.
    pub fn map_sse_event_pub(event: &SseEvent) -> Vec<ModelChunk> {
        Self::map_sse_event(event)
    }

    /// Public cost estimation without self — used by BedrockProvider.
    pub fn estimate_cost_pub(request: &ModelRequest) -> TokenCost {
        let input_chars: usize = request
            .messages
            .iter()
            .map(|m| message_to_text(m).len())
            .sum();
        let estimated_tokens = (input_chars / 4) as u32;
        TokenCost {
            estimated_input_tokens: estimated_tokens,
            estimated_cost_usd: estimated_tokens as f64 * (3.0 / 1_000_000.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_api_request_basic() {
        let req = ModelRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![ChatMessage {
                role: halcon_core::types::Role::User,
                content: MessageContent::Text("hello".into()),
            }],
            tools: vec![],
            max_tokens: Some(1024),
            temperature: Some(0.7),
            system: Some("You are helpful.".into()),
            stream: true,
        };

        let api_req = AnthropicProvider::build_api_request(&req);
        assert_eq!(api_req.model, "claude-sonnet-4-5-20250929");
        assert_eq!(api_req.messages.len(), 1);
        assert_eq!(api_req.messages[0].role, "user");
        assert!(matches!(
            &api_req.messages[0].content,
            types::ApiMessageContent::Text(t) if t == "hello"
        ));
        assert_eq!(api_req.max_tokens, 1024);
        assert_eq!(api_req.temperature, Some(0.7));
        // system is now ApiSystem::Blocks with cache_control (Q2 — Sprint 1)
        match &api_req.system {
            Some(types::ApiSystem::Blocks(blocks)) => {
                assert_eq!(blocks.len(), 1);
                assert_eq!(blocks[0].text, "You are helpful.");
                assert!(
                    blocks[0].cache_control.is_some(),
                    "non-empty system prompt must have cache_control"
                );
            }
            other => panic!("expected Blocks variant, got: {other:?}"),
        }
        assert!(api_req.stream);
        assert!(api_req.tools.is_empty());
    }

    #[test]
    fn build_api_request_defaults() {
        let req = ModelRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![ChatMessage {
                role: halcon_core::types::Role::User,
                content: MessageContent::Text("hi".into()),
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            stream: true,
        };

        let api_req = AnthropicProvider::build_api_request(&req);
        assert_eq!(api_req.max_tokens, DEFAULT_MAX_TOKENS);
        assert!(api_req.temperature.is_none());
        assert!(api_req.system.is_none());
    }

    #[test]
    fn build_api_request_filters_system_messages() {
        let req = ModelRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![
                ChatMessage {
                    role: halcon_core::types::Role::System,
                    content: MessageContent::Text("system msg".into()),
                },
                ChatMessage {
                    role: halcon_core::types::Role::User,
                    content: MessageContent::Text("hello".into()),
                },
            ],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            stream: true,
        };

        let api_req = AnthropicProvider::build_api_request(&req);
        assert_eq!(api_req.messages.len(), 1);
        assert_eq!(api_req.messages[0].role, "user");
    }

    #[test]
    fn map_message_start_with_usage() {
        let event = SseEvent::MessageStart {
            message: types::MessageStartData {
                id: "msg_01".into(),
                model: "claude-sonnet-4-5-20250929".into(),
                usage: Some(types::StartUsage { input_tokens: 50 }),
            },
        };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            ModelChunk::Usage(u) => assert_eq!(u.input_tokens, 50),
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn map_text_delta() {
        let event = SseEvent::ContentBlockDelta {
            index: 0,
            delta: types::DeltaData::TextDelta {
                text: "Hello world".into(),
            },
        };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            ModelChunk::TextDelta(t) => assert_eq!(t, "Hello world"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn map_message_delta_end_turn() {
        let event = SseEvent::MessageDelta {
            delta: types::MessageDeltaData {
                stop_reason: Some("end_turn".into()),
            },
            usage: Some(types::DeltaUsage { output_tokens: 42 }),
        };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert_eq!(chunks.len(), 2);
        match &chunks[0] {
            ModelChunk::Usage(u) => assert_eq!(u.output_tokens, 42),
            other => panic!("expected Usage, got {other:?}"),
        }
        assert!(matches!(&chunks[1], ModelChunk::Done(StopReason::EndTurn)));
    }

    #[test]
    fn map_message_delta_max_tokens() {
        let event = SseEvent::MessageDelta {
            delta: types::MessageDeltaData {
                stop_reason: Some("max_tokens".into()),
            },
            usage: None,
        };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            &chunks[0],
            ModelChunk::Done(StopReason::MaxTokens)
        ));
    }

    #[test]
    fn map_ping_returns_empty() {
        let chunks = AnthropicProvider::map_sse_event(&SseEvent::Ping);
        assert!(chunks.is_empty());
    }

    #[test]
    fn map_error_event() {
        let event = SseEvent::Error {
            error: types::ApiError {
                error_type: "overloaded_error".into(),
                message: "API is overloaded".into(),
            },
        };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            ModelChunk::Error(msg) => {
                assert!(msg.contains("overloaded_error"));
                assert!(msg.contains("API is overloaded"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn provider_name_and_models() {
        let provider = AnthropicProvider::new("test-key".into());
        assert_eq!(provider.name(), "anthropic");
        let models = provider.supported_models();
        assert!(models.len() >= 3);
        assert!(models.iter().any(|m| m.id.contains("sonnet")));
        assert!(models.iter().any(|m| m.id.contains("haiku")));
        assert!(models.iter().any(|m| m.id.contains("opus")));
    }

    #[tokio::test]
    async fn is_available_checks_api_key() {
        let with_key = AnthropicProvider::new("sk-test".into());
        assert!(with_key.is_available().await);

        let without_key = AnthropicProvider::new(String::new());
        assert!(!without_key.is_available().await);
    }

    #[test]
    fn estimate_cost_basic() {
        let provider = AnthropicProvider::new("test".into());
        let req = ModelRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![ChatMessage {
                role: halcon_core::types::Role::User,
                content: MessageContent::Text("hello world, this is a test".into()),
            }],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            stream: true,
        };
        let cost = provider.estimate_cost(&req);
        assert!(cost.estimated_input_tokens > 0);
        assert!(cost.estimated_cost_usd > 0.0);
    }

    #[test]
    fn message_to_text_from_blocks() {
        let msg = ChatMessage {
            role: halcon_core::types::Role::Assistant,
            content: MessageContent::Blocks(vec![
                halcon_core::types::ContentBlock::Text {
                    text: "Hello".into(),
                },
                halcon_core::types::ContentBlock::Text {
                    text: "World".into(),
                },
            ]),
        };
        assert_eq!(message_to_text(&msg), "Hello\nWorld");
    }

    #[test]
    fn debug_impl_redacts_api_key() {
        let provider = AnthropicProvider::new("sk-ant-secret-key-12345".into());
        let debug_output = format!("{:?}", provider);
        assert!(
            !debug_output.contains("sk-ant-secret-key-12345"),
            "Debug output must not contain the raw API key"
        );
        assert!(
            debug_output.contains("[REDACTED]"),
            "Debug output must show [REDACTED] for the API key"
        );
    }

    #[test]
    fn headers_contain_key_but_not_in_debug() {
        let provider = AnthropicProvider::new("sk-ant-api03-test-header-key".into());
        let headers = provider.build_headers();
        // API key (sk-ant-api*) uses x-api-key header.
        assert_eq!(
            headers.get("x-api-key").unwrap().to_str().unwrap(),
            "sk-ant-api03-test-header-key"
        );
        assert!(headers.get("Authorization").is_none());
        // But Debug doesn't leak it.
        let debug = format!("{:?}", provider);
        assert!(!debug.contains("sk-ant-api03-test-header-key"));
    }

    #[test]
    fn oauth_token_uses_bearer_header() {
        // OAuth access tokens start with sk-ant-oat (not sk-ant-api).
        let provider = AnthropicProvider::new("sk-ant-oat01-test-token-abc123".into());
        let headers = provider.build_headers();
        // OAuth token uses Authorization: Bearer header.
        assert_eq!(
            headers.get("Authorization").unwrap().to_str().unwrap(),
            "Bearer sk-ant-oat01-test-token-abc123"
        );
        assert!(headers.get("x-api-key").is_none());
        // OAuth Bearer requires both the OAuth flag AND the prompt-caching flag
        // (Q2 — Sprint 1): combined in a single comma-separated header value.
        let beta = headers.get("anthropic-beta").unwrap().to_str().unwrap();
        assert!(
            beta.contains("oauth-2025-04-20"),
            "OAuth beta flag must be present"
        );
        assert!(
            beta.contains("prompt-caching-2024-07-31"),
            "Prompt-caching beta flag must be present for OAuth too"
        );
    }

    #[test]
    fn is_api_key_detection() {
        assert!(AnthropicProvider::is_api_key("sk-ant-api03-abc123"));
        assert!(!AnthropicProvider::is_api_key("sk-ant-oat01-abc123"));
        assert!(!AnthropicProvider::is_api_key("some-other-token"));
    }

    #[test]
    fn map_tool_use_content_block_start() {
        let event = SseEvent::ContentBlockStart {
            index: 1,
            content_block: types::ContentBlockData {
                block_type: "tool_use".into(),
                text: None,
                id: Some("toolu_abc".into()),
                name: Some("file_read".into()),
            },
        };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            ModelChunk::ToolUseStart { index, id, name } => {
                assert_eq!(*index, 1);
                assert_eq!(id, "toolu_abc");
                assert_eq!(name, "file_read");
            }
            other => panic!("expected ToolUseStart, got {other:?}"),
        }
    }

    #[test]
    fn map_input_json_delta() {
        let event = SseEvent::ContentBlockDelta {
            index: 1,
            delta: types::DeltaData::InputJsonDelta {
                partial_json: "{\"path\":\"test.rs\"}".into(),
            },
        };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert_eq!(chunks.len(), 1);
        match &chunks[0] {
            ModelChunk::ToolUseDelta {
                index,
                partial_json,
            } => {
                assert_eq!(*index, 1);
                assert_eq!(partial_json, "{\"path\":\"test.rs\"}");
            }
            other => panic!("expected ToolUseDelta, got {other:?}"),
        }
    }

    #[test]
    fn map_tool_use_stop_reason() {
        let event = SseEvent::MessageDelta {
            delta: types::MessageDeltaData {
                stop_reason: Some("tool_use".into()),
            },
            usage: None,
        };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], ModelChunk::Done(StopReason::ToolUse)));
    }

    #[test]
    fn build_api_request_forwards_tools() {
        let req = ModelRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![ChatMessage {
                role: halcon_core::types::Role::User,
                content: MessageContent::Text("read test.rs".into()),
            }],
            tools: vec![halcon_core::types::ToolDefinition {
                name: "file_read".into(),
                description: "Read a file".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
            max_tokens: None,
            temperature: None,
            system: None,
            stream: true,
        };
        let api_req = AnthropicProvider::build_api_request(&req);
        assert_eq!(api_req.tools.len(), 1);
        assert_eq!(api_req.tools[0].name, "file_read");
    }

    #[test]
    fn build_api_request_converts_tool_result_blocks() {
        let req = ModelRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![
                ChatMessage {
                    role: halcon_core::types::Role::User,
                    content: MessageContent::Text("read test.rs".into()),
                },
                ChatMessage {
                    role: halcon_core::types::Role::Assistant,
                    content: MessageContent::Blocks(vec![
                        halcon_core::types::ContentBlock::ToolUse {
                            id: "toolu_123".into(),
                            name: "file_read".into(),
                            input: serde_json::json!({"path": "test.rs"}),
                        },
                    ]),
                },
                ChatMessage {
                    role: halcon_core::types::Role::User,
                    content: MessageContent::Blocks(vec![
                        halcon_core::types::ContentBlock::ToolResult {
                            tool_use_id: "toolu_123".into(),
                            content: "fn main() {}".into(),
                            is_error: false,
                        },
                    ]),
                },
            ],
            tools: vec![],
            max_tokens: None,
            temperature: None,
            system: None,
            stream: true,
        };
        let api_req = AnthropicProvider::build_api_request(&req);
        assert_eq!(api_req.messages.len(), 3);
        // The assistant message should be blocks.
        assert!(matches!(
            &api_req.messages[1].content,
            types::ApiMessageContent::Blocks(_)
        ));
        // The user message with tool_result should be blocks.
        assert!(matches!(
            &api_req.messages[2].content,
            types::ApiMessageContent::Blocks(_)
        ));
    }

    #[test]
    fn malformed_sse_produces_empty_not_panic() {
        // build_sse_stream already handles malformed events gracefully (warn + empty vec).
        // Verify map_sse_event doesn't panic on known event types.
        let event = SseEvent::Ping;
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert!(chunks.is_empty(), "Ping should produce no chunks");

        let event = SseEvent::MessageStop;
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert!(chunks.is_empty(), "MessageStop should produce no chunks");

        let event = SseEvent::ContentBlockStop { index: 0 };
        let chunks = AnthropicProvider::map_sse_event(&event);
        assert!(
            chunks.is_empty(),
            "ContentBlockStop should produce no chunks"
        );
    }
}
