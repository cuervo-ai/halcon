//! HTTP client for Cenzontle's agent orchestration, MCP, and RAG APIs.
//!
//! Complements `CenzontleProvider` (which handles LLM chat via `/v1/llm/chat`)
//! by providing access to Cenzontle's higher-level capabilities:
//!
//! - Agent session management and task execution (with SSE streaming)
//! - MCP tool discovery and invocation
//! - RAG knowledge search
//!
//! # Construction
//!
//! ```ignore
//! // Share auth with existing CenzontleProvider:
//! let client = CenzontleAgentClient::new(access_token, base_url);
//!
//! // Or extract from provider:
//! let client = CenzontleAgentClient::from_provider(&provider);
//! ```

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::stream::BoxStream;
use futures::StreamExt;
use tracing::{debug, info, warn};
use uuid::Uuid;

use halcon_core::error::{HalconError, Result};

use super::agent_types::*;
use super::DEFAULT_BASE_URL;
use crate::http::{backoff_delay_with_jitter, is_retryable_status, parse_retry_after};

const CLIENT_NAME: &str = "halcon-cli";

/// Circuit breaker for agent API calls (separate from LLM circuit breaker).
#[derive(Debug, Default)]
struct AgentCircuitBreaker {
    consecutive_failures: AtomicU32,
    open_until_unix_ms: AtomicU64,
}

const CB_THRESHOLD: u32 = 5;
const CB_OPEN_MS: u64 = 60_000;

impl AgentCircuitBreaker {
    fn is_open(&self) -> bool {
        let until = self.open_until_unix_ms.load(Ordering::Relaxed);
        if until == 0 {
            return false;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        now_ms < until
    }

    fn record_failure(&self) {
        let n = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= CB_THRESHOLD {
            let until = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
                + CB_OPEN_MS;
            self.open_until_unix_ms.store(until, Ordering::Relaxed);
            warn!(
                failures = n,
                "Cenzontle agent API: circuit breaker opened for 60s"
            );
        }
    }

    fn record_success(&self) {
        let prev = self.consecutive_failures.swap(0, Ordering::Relaxed);
        if prev > 0 {
            self.open_until_unix_ms.store(0, Ordering::Relaxed);
            info!("Cenzontle agent API: circuit breaker reset");
        }
    }
}

/// Client for Cenzontle's agent orchestration, MCP, and RAG APIs.
pub struct CenzontleAgentClient {
    client: reqwest::Client,
    access_token: String,
    base_url: String,
    session_id: String,
    circuit_breaker: Arc<AgentCircuitBreaker>,
}

impl std::fmt::Debug for CenzontleAgentClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CenzontleAgentClient")
            .field("base_url", &self.base_url)
            .field("access_token", &"[REDACTED]")
            .finish()
    }
}

impl CenzontleAgentClient {
    /// Create a new agent client with explicit token and base URL.
    pub fn new(access_token: String, base_url: Option<String>) -> Self {
        let base_url = base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        // Force HTTP/1.1 for SSE streaming (same reason as CenzontleProvider).
        let client = reqwest::Client::builder()
            .http1_only()
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(4)
            .user_agent(format!("halcon-cli/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build HTTP client for Cenzontle agent API");

        Self {
            client,
            access_token,
            base_url,
            session_id: Uuid::new_v4().to_string(),
            circuit_breaker: Arc::new(AgentCircuitBreaker::default()),
        }
    }

    /// Create a client that shares auth credentials with a `CenzontleProvider`.
    pub fn from_provider(provider: &super::CenzontleProvider) -> Self {
        Self::new(
            provider.access_token().to_string(),
            Some(provider.base_url().to_string()),
        )
    }

    /// Base URL of the Cenzontle instance.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // ── Agent Sessions ──────────────────────────────────────────────────────

    /// Create a new agent session.
    pub async fn create_session(&self, req: &CreateSessionRequest) -> Result<AgentSession> {
        let url = format!("{}/v1/agents/sessions", self.base_url);
        self.post_json(&url, req).await
    }

    /// Get the current state of an agent session.
    pub async fn get_session(&self, session_id: &str) -> Result<AgentSession> {
        let url = format!("{}/v1/agents/sessions/{}", self.base_url, session_id);
        self.get_json(&url).await
    }

    // ── Task Execution ──────────────────────────────────────────────────────

    /// Submit a task and stream execution events via SSE.
    ///
    /// Returns a stream of `TaskEvent` items. The stream ends when the task
    /// completes or errors.
    pub async fn submit_task(
        &self,
        session_id: &str,
        req: &SubmitTaskRequest,
    ) -> Result<BoxStream<'static, Result<TaskEvent>>> {
        self.check_circuit_breaker()?;

        let url = format!("{}/v1/agents/sessions/{}/tasks", self.base_url, session_id);

        let halcon_ctx = serde_json::json!({
            "client": CLIENT_NAME,
            "session_id": self.session_id,
        })
        .to_string();

        let request_id = Uuid::new_v4().to_string();

        debug!(
            url = %url,
            request_id = %request_id,
            "Cenzontle: submitting agent task (SSE streaming)"
        );

        // Retry once on connection error (SSE connect is not retried in the loop).
        let response = {
            let send_request = || {
                self.client
                    .post(&url)
                    .bearer_auth(&self.access_token)
                    .header("x-halcon-context", &halcon_ctx)
                    .header("x-request-id", &request_id)
                    .header("accept", "text/event-stream")
                    .timeout(Duration::from_secs(30))
                    .json(req)
                    .send()
            };

            match send_request().await {
                Ok(r) => r,
                Err(e) if e.is_connect() => {
                    // Single retry on connection failure.
                    warn!(error = %e, "Cenzontle agent task: connection error, retrying once");
                    let delay = backoff_delay_with_jitter(1000, 1);
                    tokio::time::sleep(delay).await;
                    send_request()
                        .await
                        .map_err(|e| HalconError::ConnectionError {
                            provider: "cenzontle-agent".to_string(),
                            message: format!("Cannot reach Cenzontle agent API: {e}"),
                        })?
                }
                Err(e) => {
                    return Err(HalconError::ConnectionError {
                        provider: "cenzontle-agent".to_string(),
                        message: format!("Cannot reach Cenzontle agent API: {e}"),
                    });
                }
            }
        };

        let status = response.status();
        if !status.is_success() {
            self.circuit_breaker.record_failure();
            let code = status.as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(HalconError::ApiError {
                message: format!("Cenzontle agent task HTTP {code}: {body}"),
                status: Some(code),
            });
        }

        self.circuit_breaker.record_success();

        // Detect response type: JSON (synchronous result) vs SSE (streaming).
        // The Cenzontle task endpoint returns JSON when the task completes
        // synchronously (e.g., no agents available, instant result) and SSE
        // when agents execute asynchronously with streaming output.
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("application/json") {
            // Synchronous JSON response — parse as typed TaskSyncResponse.
            let sync_resp: TaskSyncResponse =
                response.json().await.map_err(|e| HalconError::ApiError {
                    message: format!("Failed to parse task JSON response: {e}"),
                    status: None,
                })?;

            let mut events = Vec::new();

            // Extract token usage from agent results.
            let tokens_used = sync_resp
                .agent_results
                .first()
                .and_then(|r| r.token_usage.as_ref())
                .map(|u| u.total_tokens);

            if sync_resp.all_succeeded {
                events.push(Ok(TaskEvent::Completed {
                    output: sync_resp.combined_output,
                    tokens_used,
                }));
            } else {
                let error_code = sync_resp
                    .agent_results
                    .first()
                    .and_then(|r| r.error.clone())
                    .unwrap_or_else(|| "unknown".to_string());

                let message = if !sync_resp.combined_output.is_empty() {
                    sync_resp.combined_output
                } else {
                    format!("Task failed: {}", error_code)
                };

                events.push(Ok(TaskEvent::Error {
                    message,
                    code: Some(error_code),
                }));
            }

            return Ok(Box::pin(futures::stream::iter(events)));
        }

        // SSE streaming response — parse events incrementally.
        let cb = Arc::clone(&self.circuit_breaker);

        struct SseState {
            byte_stream: futures::stream::BoxStream<
                'static,
                std::result::Result<bytes::Bytes, reqwest::Error>,
            >,
            /// Raw byte buffer — avoids UTF-8 corruption at chunk boundaries.
            raw_buffer: Vec<u8>,
            pending_events: std::collections::VecDeque<Result<TaskEvent>>,
            done: bool,
            cb: Arc<AgentCircuitBreaker>,
        }

        /// Max SSE buffer size (4 MB) to prevent OOM on malformed streams.
        const MAX_SSE_BUFFER: usize = 4 * 1024 * 1024;
        /// Idle timeout per SSE chunk (5 minutes).
        const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

        let byte_stream: futures::stream::BoxStream<'static, _> = Box::pin(response.bytes_stream());
        let initial = SseState {
            byte_stream,
            raw_buffer: Vec::new(),
            pending_events: std::collections::VecDeque::new(),
            done: false,
            cb,
        };

        let stream = futures::stream::unfold(initial, |mut state| async move {
            // Drain pending parsed events first (FIFO order).
            if let Some(event) = state.pending_events.pop_front() {
                return Some((event, state));
            }
            if state.done {
                return None;
            }

            loop {
                // Idle timeout: if no data arrives within 5 minutes, abort.
                let next_chunk =
                    tokio::time::timeout(SSE_IDLE_TIMEOUT, state.byte_stream.next()).await;

                let chunk_result = match next_chunk {
                    Ok(Some(r)) => r,
                    Ok(None) => return None, // Stream ended cleanly.
                    Err(_) => {
                        state.done = true;
                        return Some((
                            Err(HalconError::ApiError {
                                message: "SSE stream idle timeout (5 min) — aborting".to_string(),
                                status: None,
                            }),
                            state,
                        ));
                    }
                };

                match chunk_result {
                    Err(e) => {
                        state.cb.record_failure();
                        state.done = true;
                        return Some((
                            Err(HalconError::ApiError {
                                message: format!("SSE stream error: {e}"),
                                status: None,
                            }),
                            state,
                        ));
                    }
                    Ok(chunk) => {
                        state.raw_buffer.extend_from_slice(&chunk);

                        // Guard against unbounded buffer growth.
                        if state.raw_buffer.len() > MAX_SSE_BUFFER {
                            state.done = true;
                            return Some((
                                Err(HalconError::ApiError {
                                    message: "SSE buffer exceeded 4 MB — aborting stream"
                                        .to_string(),
                                    status: None,
                                }),
                                state,
                            ));
                        }

                        // Convert raw bytes to string, keeping only valid UTF-8 prefix.
                        // This avoids corruption at multi-byte char boundaries.
                        let valid_len = match std::str::from_utf8(&state.raw_buffer) {
                            Ok(_) => state.raw_buffer.len(),
                            Err(e) => e.valid_up_to(),
                        };
                        if valid_len == 0 {
                            continue; // Wait for more bytes.
                        }

                        // Normalize \r\n → \n for cross-platform SSE parsing.
                        let text = std::str::from_utf8(&state.raw_buffer[..valid_len])
                            .unwrap_or("")
                            .replace("\r\n", "\n");
                        // Keep incomplete UTF-8 bytes in the buffer.
                        state.raw_buffer = state.raw_buffer[valid_len..].to_vec();

                        // Process complete SSE events (double-newline delimited).
                        // We need a persistent text buffer across chunks.
                        // Use a simple approach: prepend any leftover text.
                        let mut work = text;

                        while let Some(pos) = work.find("\n\n") {
                            let event_text = &work[..pos];

                            for line in event_text.lines() {
                                let line = line.trim();
                                if let Some(data) = line.strip_prefix("data: ") {
                                    if data == "[DONE]" {
                                        state.done = true;
                                        if let Some(ev) = state.pending_events.pop_front() {
                                            return Some((ev, state));
                                        }
                                        return None;
                                    }
                                    match serde_json::from_str::<TaskEvent>(data) {
                                        Ok(event) => {
                                            state.pending_events.push_back(Ok(event));
                                        }
                                        Err(_e) => {
                                            // Skip unparseable SSE events (forward compatibility).
                                        }
                                    }
                                }
                            }

                            work = work[pos + 2..].to_string();
                        }

                        // Put remaining incomplete text back as raw bytes.
                        if !work.is_empty() {
                            let mut remaining = work.into_bytes();
                            remaining.extend_from_slice(&state.raw_buffer);
                            state.raw_buffer = remaining;
                        }

                        // Return first pending event if any (FIFO).
                        if let Some(event) = state.pending_events.pop_front() {
                            return Some((event, state));
                        }
                    }
                }
            }
        });

        Ok(Box::pin(stream))
    }

    /// Submit a task and collect all events into a `TaskResult`.
    ///
    /// Convenience method that consumes the SSE stream and accumulates results.
    pub async fn submit_task_blocking(
        &self,
        session_id: &str,
        req: &SubmitTaskRequest,
    ) -> Result<TaskResult> {
        let mut stream = self.submit_task(session_id, req).await?;
        let mut result = TaskResult::default();

        while let Some(event) = stream.next().await {
            match event? {
                TaskEvent::Content { content } => result.output.push_str(&content),
                TaskEvent::Thinking { content } => result.thinking.push_str(&content),
                TaskEvent::ToolCall { name, .. } => result.tool_calls.push(name),
                TaskEvent::Completed {
                    output,
                    tokens_used,
                } => {
                    if result.output.is_empty() {
                        result.output = output;
                    }
                    result.tokens_used = tokens_used.unwrap_or(0);
                    result.success = true;
                }
                TaskEvent::Error { message, .. } => {
                    result.error = Some(message);
                    result.success = false;
                }
                _ => {}
            }
        }

        Ok(result)
    }

    // ── Agent Listing ───────────────────────────────────────────────────────

    /// List all registered agents.
    ///
    /// The Cenzontle `/v1/agents` endpoint returns a bare JSON array `[...]`,
    /// not a wrapper object.
    pub async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let url = format!("{}/v1/agents", self.base_url);
        self.get_json(&url).await
    }

    // ── MCP Tools ───────────────────────────────────────────────────────────

    /// List available MCP tools.
    pub async fn list_mcp_tools(&self) -> Result<Vec<McpToolDef>> {
        let url = format!("{}/v1/mcp/tools", self.base_url);
        let resp: McpToolListResponse = self.get_json(&url).await?;
        Ok(resp.tools)
    }

    /// Call an MCP tool.
    pub async fn call_mcp_tool(&self, req: &McpToolCallRequest) -> Result<McpToolCallResponse> {
        let url = format!("{}/v1/mcp/tools/call", self.base_url);
        self.post_json(&url, req).await
    }

    // ── Knowledge Search (RAG) ──────────────────────────────────────────────

    /// Search the knowledge base via RAG.
    ///
    /// Routes through the MCP `knowledge_search` tool since Cenzontle's RAG
    /// is exposed via the MCP tool layer, not a dedicated REST endpoint.
    pub async fn knowledge_search(
        &self,
        req: &KnowledgeSearchRequest,
    ) -> Result<KnowledgeSearchResponse> {
        let mcp_args = serde_json::json!({
            "query": req.query,
            "botId": req.bot_id,
            "topK": req.top_k.unwrap_or(5),
        });

        let mcp_req = McpToolCallRequest {
            name: "knowledge_search".to_string(),
            arguments: mcp_args,
        };

        let resp = self.call_mcp_tool(&mcp_req).await?;

        if resp.is_error {
            return Err(HalconError::ApiError {
                message: format!("Knowledge search failed: {}", resp.text()),
                status: None,
            });
        }

        // Parse the MCP tool response content as KnowledgeSearchResponse.
        // If it doesn't parse as structured data, wrap as a single chunk.
        let text = resp.text();
        match serde_json::from_str::<KnowledgeSearchResponse>(&text) {
            Ok(parsed) => Ok(parsed),
            Err(_) => {
                // MCP tool returned plain text — wrap as a single result chunk.
                Ok(KnowledgeSearchResponse {
                    chunks: vec![KnowledgeChunk {
                        content: text,
                        score: 1.0,
                        source: None,
                        metadata: serde_json::Value::Null,
                    }],
                })
            }
        }
    }

    // ── Internal Helpers ────────────────────────────────────────────────────

    fn check_circuit_breaker(&self) -> Result<()> {
        if self.circuit_breaker.is_open() {
            return Err(HalconError::ApiError {
                message: "Cenzontle agent API: circuit breaker open — backend is degraded"
                    .to_string(),
                status: None,
            });
        }
        Ok(())
    }

    /// GET request with JSON response, retry, and circuit breaker.
    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        self.request_json(reqwest::Method::GET, url, None::<&()>)
            .await
    }

    /// POST request with JSON body/response, retry, and circuit breaker.
    async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        self.request_json(reqwest::Method::POST, url, Some(body))
            .await
    }

    /// Unified request method with retry, circuit breaker, and no double-sleep.
    async fn request_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<&B>,
    ) -> Result<T> {
        self.check_circuit_breaker()?;

        let max_retries = 2u32;
        let timeout_secs = if method == reqwest::Method::GET {
            15
        } else {
            30
        };
        // Track whether the previous iteration already slept for a retryable error
        // to avoid double-sleeping (loop-top delay + retryable delay).
        let mut already_delayed = false;

        for attempt in 0..=max_retries {
            if attempt > 0 && !already_delayed {
                let delay = backoff_delay_with_jitter(1000, attempt);
                tokio::time::sleep(delay).await;
            }
            already_delayed = false;

            let mut req = self
                .client
                .request(method.clone(), url)
                .bearer_auth(&self.access_token)
                .header("x-halcon-context", &self.halcon_context())
                .timeout(Duration::from_secs(timeout_secs));

            if let Some(b) = body {
                req = req.json(b);
            }

            let response = match req.send().await {
                Ok(r) => r,
                Err(e) if e.is_connect() && attempt < max_retries => {
                    self.circuit_breaker.record_failure();
                    warn!(attempt = attempt + 1, error = %e, "Cenzontle agent API: retry");
                    continue;
                }
                Err(e) => {
                    self.circuit_breaker.record_failure();
                    return Err(HalconError::ConnectionError {
                        provider: "cenzontle-agent".to_string(),
                        message: format!("Cannot reach Cenzontle: {e}"),
                    });
                }
            };

            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(HalconError::ApiError {
                    message: "Cenzontle: token expired. Run `halcon login cenzontle`.".to_string(),
                    status: Some(401),
                });
            }

            if is_retryable_status(status.as_u16()) && attempt < max_retries {
                self.circuit_breaker.record_failure();
                let delay = parse_retry_after(response.headers())
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| backoff_delay_with_jitter(1000, attempt));
                tokio::time::sleep(delay).await;
                already_delayed = true; // Skip loop-top delay on next iteration.
                continue;
            }

            if !status.is_success() {
                let code = status.as_u16();
                let body_text = response.text().await.unwrap_or_default();
                return Err(HalconError::ApiError {
                    message: format!("Cenzontle agent API HTTP {code}: {body_text}"),
                    status: Some(code),
                });
            }

            self.circuit_breaker.record_success();
            let parsed: T = response.json().await.map_err(|e| HalconError::ApiError {
                message: format!("Failed to parse Cenzontle response: {e}"),
                status: None,
            })?;
            return Ok(parsed);
        }

        Err(HalconError::ApiError {
            message: "Cenzontle agent API: all retries exhausted".to_string(),
            status: None,
        })
    }

    fn halcon_context(&self) -> String {
        serde_json::json!({
            "client": CLIENT_NAME,
            "session_id": self.session_id,
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_construction() {
        let client = CenzontleAgentClient::new("test-token".into(), None);
        assert_eq!(client.base_url(), DEFAULT_BASE_URL);
    }

    #[test]
    fn client_custom_base_url() {
        let client = CenzontleAgentClient::new("tok".into(), Some("http://localhost:3001".into()));
        assert_eq!(client.base_url(), "http://localhost:3001");
    }

    #[test]
    fn client_debug_redacts_token() {
        let client = CenzontleAgentClient::new("secret-token".into(), None);
        let debug = format!("{:?}", client);
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("secret-token"));
    }

    #[test]
    fn circuit_breaker_initially_closed() {
        let cb = AgentCircuitBreaker::default();
        assert!(!cb.is_open());
    }

    #[test]
    fn circuit_breaker_opens_after_threshold() {
        let cb = AgentCircuitBreaker::default();
        for _ in 0..CB_THRESHOLD {
            cb.record_failure();
        }
        assert!(cb.is_open());
    }

    #[test]
    fn circuit_breaker_resets_on_success() {
        let cb = AgentCircuitBreaker::default();
        for _ in 0..3 {
            cb.record_failure();
        }
        cb.record_success();
        assert!(!cb.is_open());
        assert_eq!(cb.consecutive_failures.load(Ordering::Relaxed), 0);
    }
}
