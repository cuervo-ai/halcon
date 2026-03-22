//! Observability counters for `ClaudeCodeProvider`.
//!
//! Tracks subprocess lifecycle, token usage, latency, and protocol events.

use std::time::Duration;

// ─────────────────────────────────────────────────────────────────────────────
// ProviderMetrics
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime metrics collected by a `ClaudeCodeProvider` instance.
///
/// All fields are u64/u32 counters that never reset; consumers can snapshot and diff.
#[derive(Debug, Default, Clone)]
pub struct ProviderMetrics {
    /// Total subprocess spawns (first spawn + all re-spawns).
    pub spawn_count: u32,
    /// Re-spawns due to crash/health failure (subset of `spawn_count - 1`).
    pub respawn_count: u32,
    /// Requests successfully dispatched.
    pub total_requests: u64,
    /// Requests that ended with an error result from the CLI.
    pub error_requests: u64,
    /// Cumulative input tokens reported by the CLI.
    pub total_input_tokens: u64,
    /// Cumulative output tokens reported by the CLI.
    pub total_output_tokens: u64,
    /// Time-to-first-token of the most recent successful request (milliseconds).
    pub last_ttft_ms: Option<u64>,
    /// Drain operations performed before requests.
    pub drain_count: u32,
    /// Model switch operations (`set_model` control requests).
    pub model_switch_count: u32,
    /// Drain operations that timed out.
    pub drain_timeout_count: u32,
}

impl ProviderMetrics {
    /// Create a new zero-initialized metrics instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a subprocess spawn event.
    pub fn record_spawn(&mut self, is_respawn: bool) {
        self.spawn_count += 1;
        if is_respawn {
            self.respawn_count += 1;
        }
    }

    /// Record completion of a request (success or error).
    pub fn record_request(
        &mut self,
        input_tokens: u32,
        output_tokens: u32,
        ttft: Duration,
        is_error: bool,
    ) {
        self.total_requests += 1;
        self.total_input_tokens += u64::from(input_tokens);
        self.total_output_tokens += u64::from(output_tokens);
        self.last_ttft_ms = Some(ttft.as_millis() as u64);
        if is_error {
            self.error_requests += 1;
        }
    }

    /// Record a drain operation.
    pub fn record_drain(&mut self, timed_out: bool) {
        self.drain_count += 1;
        if timed_out {
            self.drain_timeout_count += 1;
        }
    }

    /// Record a `set_model` control request.
    pub fn record_model_switch(&mut self) {
        self.model_switch_count += 1;
    }

    /// One-line human-readable summary for log output.
    pub fn summary(&self) -> String {
        format!(
            "spawns={} respawns={} requests={} errors={} tokens_in={} tokens_out={} drains={} model_switches={}",
            self.spawn_count,
            self.respawn_count,
            self.total_requests,
            self.error_requests,
            self.total_input_tokens,
            self.total_output_tokens,
            self.drain_count,
            self.model_switch_count,
        )
    }

    /// Error rate as a fraction [0.0, 1.0]. Returns 0.0 if no requests.
    pub fn error_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.error_requests as f64 / self.total_requests as f64
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_default_zero() {
        let m = ProviderMetrics::new();
        assert_eq!(m.spawn_count, 0);
        assert_eq!(m.respawn_count, 0);
        assert_eq!(m.total_requests, 0);
        assert_eq!(m.error_requests, 0);
    }

    #[test]
    fn record_spawn_first_not_respawn() {
        let mut m = ProviderMetrics::new();
        m.record_spawn(false);
        assert_eq!(m.spawn_count, 1);
        assert_eq!(m.respawn_count, 0);
    }

    #[test]
    fn record_spawn_respawn_increments_both() {
        let mut m = ProviderMetrics::new();
        m.record_spawn(false);
        m.record_spawn(true);
        assert_eq!(m.spawn_count, 2);
        assert_eq!(m.respawn_count, 1);
    }

    #[test]
    fn record_request_accumulates_tokens() {
        let mut m = ProviderMetrics::new();
        m.record_request(100, 50, Duration::from_millis(200), false);
        m.record_request(80, 30, Duration::from_millis(150), false);
        assert_eq!(m.total_input_tokens, 180);
        assert_eq!(m.total_output_tokens, 80);
        assert_eq!(m.total_requests, 2);
        assert_eq!(m.error_requests, 0);
    }

    #[test]
    fn record_request_error_counted() {
        let mut m = ProviderMetrics::new();
        m.record_request(10, 0, Duration::ZERO, true);
        assert_eq!(m.error_requests, 1);
        assert_eq!(m.total_requests, 1);
    }

    #[test]
    fn error_rate_zero_when_no_requests() {
        let m = ProviderMetrics::new();
        assert_eq!(m.error_rate(), 0.0);
    }

    #[test]
    fn error_rate_half() {
        let mut m = ProviderMetrics::new();
        m.record_request(10, 5, Duration::ZERO, false);
        m.record_request(10, 0, Duration::ZERO, true);
        assert!((m.error_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn record_drain_timeout_tracked() {
        let mut m = ProviderMetrics::new();
        m.record_drain(false);
        m.record_drain(true);
        assert_eq!(m.drain_count, 2);
        assert_eq!(m.drain_timeout_count, 1);
    }

    #[test]
    fn summary_not_empty() {
        let m = ProviderMetrics::new();
        assert!(!m.summary().is_empty());
    }

    #[test]
    fn last_ttft_updated_on_request() {
        let mut m = ProviderMetrics::new();
        assert!(m.last_ttft_ms.is_none());
        m.record_request(10, 5, Duration::from_millis(350), false);
        assert_eq!(m.last_ttft_ms, Some(350));
    }
}
