//! `ManagedProcess` — resilient subprocess lifecycle manager.
//!
//! Wraps any `CliTransport` (real subprocess or mock) with:
//! - **Protocol state machine** (Idle → AwaitingResponse → Streaming → Draining → Recovering)
//! - **Drain before request** — prevents protocol desync after cancellation
//! - **Auto-respawn with exponential backoff** — up to `max_retries` attempts
//! - **Model switching** via `control_request` (no re-spawn needed)
//! - **Session-stable respawn** — preserves `session_id` across respawns when possible

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use tracing::{debug, info, warn};

use halcon_core::error::{HalconError, Result};

use super::protocol::{
    control_request_set_model, ndjson_chunk_to_model_chunks, NdjsonChunk,
};
use super::transport::CliTransport;
use halcon_core::types::ModelChunk;

// ─────────────────────────────────────────────────────────────────────────────
// Protocol state machine
// ─────────────────────────────────────────────────────────────────────────────

/// Internal state of the NDJSON protocol conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolState {
    /// No request in flight; ready for a new request.
    Idle,
    /// Request line sent, waiting for first response chunk.
    AwaitingResponse,
    /// Receiving response chunks.
    Streaming,
    /// Consuming remaining chunks from a cancelled/interrupted request.
    Draining,
    /// Transport is unhealthy; attempting respawn.
    Recovering,
}

// ─────────────────────────────────────────────────────────────────────────────
// RespawnFactory
// ─────────────────────────────────────────────────────────────────────────────

/// Async factory that creates a fresh `CliTransport` for (re-)spawning.
///
/// The closure is called with no arguments and must return a `BoxFuture`
/// resolving to `Result<Box<dyn CliTransport>>`.
///
/// **Production**: captures `SpawnConfig`, spawns a real subprocess.
/// **Tests**: captures a `MockTransport` builder, no subprocess.
pub type RespawnFactory =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Box<dyn CliTransport>>> + Send + Sync>;

// ─────────────────────────────────────────────────────────────────────────────
// ManagedProcess
// ─────────────────────────────────────────────────────────────────────────────

/// Resilient wrapper around a `CliTransport` with auto-respawn and drain support.
///
/// # Lifecycle
///
/// 1. Created with `transport = None` and a `factory`.
/// 2. First `ensure_healthy()` call spawns the transport via the factory.
/// 3. If the transport crashes, `ensure_healthy()` respawns it (up to `max_retries`).
/// 4. `drain_pending()` must be called before each new request to clear any
///    leftover chunks from a previously cancelled stream.
/// 5. `send_set_model()` switches models without a re-spawn.
/// 6. `execute_request()` sends one request and collects all response chunks.
pub struct ManagedProcess {
    transport: Option<Box<dyn CliTransport>>,
    factory: RespawnFactory,

    /// Current protocol state (for diagnostics and drain guard).
    pub state: ProtocolState,
    /// Whether there may be unconsumed chunks from the previous request.
    needs_drain: bool,

    /// Model name currently active in the subprocess.
    current_model: String,
    /// Monotonically increasing ID for `control_request` correlation.
    next_request_id: u64,

    /// Total number of times the subprocess was (re-)spawned.
    pub spawn_count: u32,
    /// Maximum re-spawn attempts before giving up.
    max_retries: u32,
    /// Timeout for draining a pending response.
    drain_timeout: Duration,

    /// System prompt that was passed at the most recent spawn.
    /// Used to detect when a re-spawn is required due to system prompt change.
    pub spawned_system_prompt: Option<String>,
}

impl ManagedProcess {
    /// Create a `ManagedProcess` with lazy spawning.
    ///
    /// `transport` starts as `None`; the factory is called on first `ensure_healthy()`.
    pub fn new(
        factory: RespawnFactory,
        initial_model: impl Into<String>,
        drain_timeout: Duration,
    ) -> Self {
        Self {
            transport: None,
            factory,
            state: ProtocolState::Idle,
            needs_drain: false,
            current_model: initial_model.into(),
            next_request_id: 0,
            spawn_count: 0,
            max_retries: 3,
            drain_timeout,
            spawned_system_prompt: None,
        }
    }

    /// Create a `ManagedProcess` with a pre-built transport (test constructor).
    ///
    /// - `initial_model`: set this to whatever model your test requests will use
    ///   so that `send_set_model` is not triggered (no `control_response` needed).
    /// - The factory always returns `ProviderUnavailable` so respawn is a no-op.
    pub fn with_transport(
        transport: Box<dyn CliTransport>,
        drain_timeout: Duration,
        initial_model: impl Into<String>,
    ) -> Self {
        let factory: RespawnFactory = Arc::new(|| {
            Box::pin(async {
                Err(HalconError::ProviderUnavailable {
                    provider: "claude-code-mock".into(),
                })
            })
        });
        Self {
            transport: Some(transport),
            factory,
            state: ProtocolState::Idle,
            needs_drain: false,
            current_model: initial_model.into(),
            next_request_id: 0,
            spawn_count: 1, // counts as "already spawned"
            max_retries: 0, // no retries for test transport
            drain_timeout,
            spawned_system_prompt: None,
        }
    }

    // ── Public accessors ─────────────────────────────────────────────────────

    /// `true` if the underlying transport reports itself alive.
    pub fn is_alive(&mut self) -> bool {
        self.transport
            .as_mut()
            .map(|t| t.is_alive())
            .unwrap_or(false)
    }

    /// Model currently active in the subprocess.
    pub fn current_model(&self) -> &str {
        &self.current_model
    }

    /// Override `current_model` without I/O (used after fresh spawn with `--model`).
    ///
    /// Call this after `ensure_healthy()` returns `true` (did_spawn) when the
    /// subprocess was started with `--model <model>`. This prevents a redundant
    /// `send_set_model` control_request on the very first turn.
    pub fn set_current_model(&mut self, model: &str) {
        if !model.is_empty() && model != "default" {
            self.current_model = model.to_string();
        }
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Ensure the transport is alive and ready for a new request.
    ///
    /// - If already healthy → no-op, returns `Ok(false)`.
    /// - If dead or absent → attempts respawn with exponential backoff.
    /// - Returns `Ok(true)` after a successful (re-)spawn.
    /// - Returns `Err` if all retries are exhausted.
    pub async fn ensure_healthy(&mut self) -> Result<bool> {
        let needs_spawn = self
            .transport
            .as_mut()
            .map(|t| !t.is_alive())
            .unwrap_or(true);

        if !needs_spawn {
            return Ok(false);
        }

        let is_respawn = self.transport.is_some();
        self.transport = None;
        self.state = ProtocolState::Recovering;
        self.needs_drain = false;

        if is_respawn {
            warn!("claude-code: subprocess died, attempting respawn");
        }

        let factory = self.factory.clone();
        let mut last_err =
            HalconError::ProviderUnavailable { provider: "claude_code".into() };

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let backoff_ms = (200u64 * (1u64 << (attempt - 1))).min(5_000);
                debug!(attempt, backoff_ms, "claude-code: respawn backoff");
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }

            match (factory)().await {
                Ok(transport) => {
                    info!(
                        attempt = attempt + 1,
                        spawn_count = self.spawn_count + 1,
                        is_respawn,
                        "claude-code: subprocess ready"
                    );
                    self.transport = Some(transport);
                    self.state = ProtocolState::Idle;
                    self.spawn_count += 1;
                    return Ok(true);
                }
                Err(e) => {
                    warn!(
                        attempt = attempt + 1,
                        max = self.max_retries + 1,
                        error = %e,
                        "claude-code: spawn attempt failed"
                    );
                    last_err = e;
                }
            }
        }

        Err(last_err)
    }

    // ── Drain ─────────────────────────────────────────────────────────────────

    /// Drain any pending response from a cancelled or interrupted request.
    ///
    /// Reads lines until a `result` / `error` chunk or `drain_timeout` elapses.
    /// Leaves the protocol in `Idle` state regardless of outcome.
    ///
    /// Returns `((), timed_out)` — callers can record drain metrics.
    pub async fn drain_pending(&mut self) -> ((), bool) {
        if !self.needs_drain {
            return ((), false);
        }

        let transport = match self.transport.as_mut() {
            Some(t) => t,
            None => {
                self.needs_drain = false;
                return ((), false);
            }
        };

        self.state = ProtocolState::Draining;
        debug!("claude-code: draining pending response (timeout={}s)", self.drain_timeout.as_secs());

        let deadline = Instant::now() + self.drain_timeout;
        let mut timed_out = false;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                warn!("claude-code: drain timeout, resetting state");
                timed_out = true;
                break;
            }

            match tokio::time::timeout(remaining, transport.recv_line()).await {
                Ok(Ok(None)) => break, // EOF
                Ok(Ok(Some(line))) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(chunk) = serde_json::from_str::<NdjsonChunk>(&line) {
                        if matches!(chunk, NdjsonChunk::Result { .. }) {
                            break; // clean drain
                        }
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    // stream error or timeout — stop draining
                    timed_out = true;
                    break;
                }
            }
        }

        self.needs_drain = false;
        self.state = ProtocolState::Idle;
        ((), timed_out)
    }

    // ── Model switching ───────────────────────────────────────────────────────

    /// Switch the active model via a `control_request` (no re-spawn).
    ///
    /// No-op if `model` equals the current model, is empty, or is `"default"`.
    /// Returns `Err` if the CLI rejects the switch or the response times out.
    pub async fn send_set_model(&mut self, model: &str) -> Result<()> {
        if model.is_empty() || model == "default" || model == self.current_model {
            return Ok(());
        }

        let req_id = self.next_id();
        let line = control_request_set_model(model, req_id);

        let transport = self
            .transport
            .as_mut()
            .ok_or_else(|| HalconError::ProviderUnavailable { provider: "claude_code".into() })?;

        transport.send_line(&line).await?;

        let timeout_dur = Duration::from_secs(10);
        let deadline = Instant::now() + timeout_dur;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(HalconError::RequestTimeout {
                    provider: "claude_code".into(),
                    timeout_secs: 10,
                });
            }

            match tokio::time::timeout(remaining, transport.recv_line()).await {
                Ok(Ok(Some(line))) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(NdjsonChunk::ControlResponse { response }) =
                        serde_json::from_str::<NdjsonChunk>(&line)
                    {
                        if response.request_id == req_id.to_string() {
                            return if response.subtype == "success" {
                                debug!(model, "claude-code: model switched");
                                self.current_model = model.to_string();
                                Ok(())
                            } else {
                                Err(HalconError::ApiError {
                                    message: format!(
                                        "set_model '{}' failed: {}",
                                        model,
                                        response.error.unwrap_or_else(|| "unknown".into())
                                    ),
                                    status: None,
                                })
                            };
                        }
                        // Different request_id — keep reading
                    }
                    // Other chunk types during model switch are silently ignored
                }
                Ok(Ok(None)) => {
                    return Err(HalconError::StreamError(
                        "EOF while waiting for set_model response".into(),
                    ));
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(HalconError::RequestTimeout {
                        provider: "claude_code".into(),
                        timeout_secs: 10,
                    });
                }
            }
        }
    }

    // ── Request execution ─────────────────────────────────────────────────────

    /// Send one NDJSON request line and collect all response chunks until `result`.
    ///
    /// Returns the raw `NdjsonChunk` list including the terminal `Result` chunk.
    /// Also returns the `ModelChunk` list ready for the stream.
    ///
    /// On timeout: marks `needs_drain = true` so the next call drains first.
    /// On transport error: clears the transport so `ensure_healthy` respawns.
    pub async fn execute_request(
        &mut self,
        ndjson: &str,
        request_timeout: Duration,
    ) -> Result<Vec<ModelChunk>> {
        let transport = self
            .transport
            .as_mut()
            .ok_or_else(|| HalconError::ProviderUnavailable { provider: "claude_code".into() })?;

        transport.send_line(ndjson).await?;
        self.state = ProtocolState::AwaitingResponse;
        self.needs_drain = true;

        let mut model_chunks = Vec::new();
        let deadline = Instant::now() + request_timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // Leave needs_drain = true so next call drains first.
                return Err(HalconError::RequestTimeout {
                    provider: "claude_code".into(),
                    timeout_secs: request_timeout.as_secs(),
                });
            }

            let line = match tokio::time::timeout(remaining, transport.recv_line()).await {
                Ok(Ok(Some(l))) => l,
                Ok(Ok(None)) => {
                    // EOF — subprocess exited before sending the `result` chunk.
                    // This usually means Claude Code attempted to use tools
                    // (sending `tool_use` events on stdout) and waited for
                    // `tool_result` events on stdin that never arrived, then
                    // hit its own internal 30-second timeout and exited.
                    warn!("claude-code: stdout EOF before result chunk — subprocess exited unexpectedly");
                    self.needs_drain = false;
                    self.transport = None; // force respawn next call
                    return Err(HalconError::StreamError(
                        "claude-code subprocess exited before completing the response (EOF)".into(),
                    ));
                }
                Ok(Err(e)) => {
                    self.needs_drain = false;
                    self.transport = None;
                    return Err(e);
                }
                Err(_elapsed) => {
                    // needs_drain stays true
                    return Err(HalconError::RequestTimeout {
                        provider: "claude_code".into(),
                        timeout_secs: request_timeout.as_secs(),
                    });
                }
            };

            if line.trim().is_empty() {
                continue;
            }

            self.state = ProtocolState::Streaming;

            let chunk: NdjsonChunk = match serde_json::from_str(&line) {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, line = %line, "claude-code: unparseable NDJSON line (skipped)");
                    continue;
                }
            };

            let is_terminal = matches!(chunk, NdjsonChunk::Result { .. });
            model_chunks.extend(ndjson_chunk_to_model_chunks(chunk));

            if is_terminal {
                self.needs_drain = false;
                self.state = ProtocolState::Idle;
                break;
            }
        }

        Ok(model_chunks)
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn next_id(&mut self) -> u64 {
        self.next_request_id += 1;
        self.next_request_id
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_code::transport::{
        mock_error_response, mock_set_model_ok, mock_success_response, MockTransport,
    };
    use halcon_core::types::StopReason;

    fn managed_with_mock(mock: MockTransport) -> ManagedProcess {
        // "claude-opus-4-6" matches the model used in test requests → no model-switch triggered.
        ManagedProcess::with_transport(Box::new(mock), Duration::from_secs(5), "claude-opus-4-6")
    }

    // ── Basic request execution ───────────────────────────────────────────────

    #[tokio::test]
    async fn execute_request_success() {
        let mut mock = MockTransport::new();
        mock.queue_response(mock_success_response("hello world"));

        let mut mgd = managed_with_mock(mock);
        let chunks = mgd
            .execute_request(r#"{"type":"user"}"#, Duration::from_secs(5))
            .await
            .unwrap();

        // Expect: TextDelta + Usage + Done
        let has_text = chunks.iter().any(|c| matches!(c, ModelChunk::TextDelta(t) if t.contains("hello world")));
        let has_done = chunks.iter().any(|c| matches!(c, ModelChunk::Done(StopReason::EndTurn)));
        assert!(has_text, "missing TextDelta");
        assert!(has_done, "missing Done");
    }

    #[tokio::test]
    async fn execute_request_error_result() {
        let mut mock = MockTransport::new();
        mock.queue_response(mock_error_response("auth failed"));

        let mut mgd = managed_with_mock(mock);
        let chunks = mgd
            .execute_request("req", Duration::from_secs(5))
            .await
            .unwrap();

        assert!(chunks.iter().any(|c| matches!(c, ModelChunk::Error(e) if e.contains("auth failed"))));
    }

    // ── Multiple requests ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn multiple_requests_sequential() {
        let mut mock = MockTransport::new();
        mock.queue_response(mock_success_response("first response"));
        mock.queue_response(mock_success_response("second response"));

        let mut mgd = managed_with_mock(mock);

        let r1 = mgd.execute_request("req1", Duration::from_secs(5)).await.unwrap();
        assert!(r1.iter().any(|c| matches!(c, ModelChunk::TextDelta(t) if t.contains("first"))));

        let r2 = mgd.execute_request("req2", Duration::from_secs(5)).await.unwrap();
        assert!(r2.iter().any(|c| matches!(c, ModelChunk::TextDelta(t) if t.contains("second"))));
    }

    // ── Drain ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn drain_clears_needs_drain_flag() {
        let mut mock = MockTransport::new();
        mock.queue_response(mock_success_response("will be drained"));
        mock.queue_response(mock_success_response("second"));

        let mut mgd = managed_with_mock(mock);
        // Send request but simulate cancellation: don't consume chunks.
        // Mark needs_drain manually (simulate interrupted stream).
        mgd.needs_drain = true;
        mgd.state = ProtocolState::Streaming;
        // Queue a second response so recv_line can find the result.
        // (In the mock, send_line already activated the first response.)
        // Drain is called before second execute_request.
        let ((), timed_out) = mgd.drain_pending().await;
        assert!(!timed_out);
        assert!(!mgd.needs_drain);
        assert_eq!(mgd.state, ProtocolState::Idle);
    }

    #[tokio::test]
    async fn drain_noop_when_no_pending() {
        let mock = MockTransport::new();
        let mut mgd = managed_with_mock(mock);
        assert!(!mgd.needs_drain);
        let ((), timed_out) = mgd.drain_pending().await;
        assert!(!timed_out);
    }

    // ── Model switching ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn send_set_model_success() {
        let mut mock = MockTransport::new();
        mock.queue_response(mock_set_model_ok(1));

        let mut mgd = managed_with_mock(mock);
        mgd.current_model = "claude-sonnet-4-6".into();

        mgd.send_set_model("claude-opus-4-6").await.unwrap();
        assert_eq!(mgd.current_model, "claude-opus-4-6");
    }

    #[tokio::test]
    async fn send_set_model_noop_same_model() {
        let mock = MockTransport::new();
        let mut mgd = managed_with_mock(mock);
        mgd.current_model = "claude-opus-4-6".into();

        // No request_id consumed, no lines needed in mock
        mgd.send_set_model("claude-opus-4-6").await.unwrap();
        // request_id still at 0 (no control_request sent)
        assert_eq!(mgd.next_request_id, 0);
    }

    #[tokio::test]
    async fn send_set_model_noop_default() {
        let mock = MockTransport::new();
        let mut mgd = managed_with_mock(mock);
        mgd.send_set_model("default").await.unwrap();
        assert_eq!(mgd.next_request_id, 0);
    }

    // ── State machine transitions ─────────────────────────────────────────────

    #[tokio::test]
    async fn state_transitions_during_request() {
        let mut mock = MockTransport::new();
        mock.queue_response(mock_success_response("data"));
        let mut mgd = managed_with_mock(mock);

        assert_eq!(mgd.state, ProtocolState::Idle);
        mgd.execute_request("req", Duration::from_secs(5)).await.unwrap();
        assert_eq!(mgd.state, ProtocolState::Idle); // back to Idle after result
        assert!(!mgd.needs_drain);
    }

    // ── ensure_healthy ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ensure_healthy_noop_when_alive() {
        let mock = MockTransport::new();
        let mut mgd = managed_with_mock(mock);
        let did_spawn = mgd.ensure_healthy().await.unwrap();
        assert!(!did_spawn); // already alive, no respawn
    }

    #[tokio::test]
    async fn ensure_healthy_err_when_factory_fails() {
        // Factory always fails, transport starts dead.
        let factory: RespawnFactory = Arc::new(|| {
            Box::pin(async {
                Err(HalconError::ProviderUnavailable { provider: "claude_code".into() })
            })
        });
        let mut mgd = ManagedProcess::new(factory, "default", Duration::from_secs(1));
        let err = mgd.ensure_healthy().await.unwrap_err();
        assert!(matches!(err, HalconError::ProviderUnavailable { .. }));
    }

    // ── is_alive ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn is_alive_with_live_mock() {
        let mock = MockTransport::new();
        let mut mgd = managed_with_mock(mock);
        assert!(mgd.is_alive());
    }

    #[tokio::test]
    async fn is_alive_false_after_crash() {
        let mut mock = MockTransport::new();
        mock.crash();
        let mut mgd = managed_with_mock(mock);
        assert!(!mgd.is_alive());
    }

    // ── spawn_count ───────────────────────────────────────────────────────────

    #[test]
    fn spawn_count_starts_at_one_with_transport() {
        let mock = MockTransport::new();
        let mgd = ManagedProcess::with_transport(Box::new(mock), Duration::from_secs(5), "default");
        assert_eq!(mgd.spawn_count, 1);
    }

    #[test]
    fn spawn_count_starts_at_zero_lazy() {
        let factory: RespawnFactory = Arc::new(|| {
            Box::pin(async {
                Err(HalconError::ProviderUnavailable { provider: "claude_code".into() })
            })
        });
        let mgd = ManagedProcess::new(factory, "model", Duration::from_secs(5));
        assert_eq!(mgd.spawn_count, 0);
    }
}
