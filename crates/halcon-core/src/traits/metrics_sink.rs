//! MetricsSink trait — domain-level write abstraction for observability.
//!
//! Defines the port in the hexagonal architecture; the storage crate
//! provides the infrastructure adapter that implements it.

/// Write-side abstraction for recording telemetry events.
///
/// Callers record events without knowing the backing store (SQLite, in-memory,
/// no-op for tests). The infrastructure layer implements this trait.
pub trait MetricsSink: Send + Sync {
    /// Record one model invocation.
    fn record_invocation(
        &self,
        provider:     &str,
        model:        &str,
        latency_ms:   u64,
        input_tokens: u32,
        output_tokens: u32,
        cost_usd:     f64,
        success:      bool,
        stop_reason:  &str,
        session_id:   Option<&str>,
    );

    /// Record one tool execution.
    fn record_tool_execution(
        &self,
        tool_name:   &str,
        duration_ms: u64,
        success:     bool,
        is_parallel: bool,
        session_id:  Option<&str>,
    );
}

/// No-op implementation for tests and contexts without a database.
pub struct NoopMetricsSink;

impl MetricsSink for NoopMetricsSink {
    fn record_invocation(
        &self,
        _provider:     &str,
        _model:        &str,
        _latency_ms:   u64,
        _input_tokens: u32,
        _output_tokens: u32,
        _cost_usd:     f64,
        _success:      bool,
        _stop_reason:  &str,
        _session_id:   Option<&str>,
    ) {}

    fn record_tool_execution(
        &self,
        _tool_name:   &str,
        _duration_ms: u64,
        _success:     bool,
        _is_parallel: bool,
        _session_id:  Option<&str>,
    ) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_sink_record_invocation_does_not_panic() {
        let sink = NoopMetricsSink;
        sink.record_invocation(
            "anthropic", "claude-sonnet", 500, 100, 50, 0.001, true, "end_turn", None,
        );
    }

    #[test]
    fn noop_sink_record_tool_execution_does_not_panic() {
        let sink = NoopMetricsSink;
        sink.record_tool_execution("bash", 42, true, false, Some("sess-1"));
    }

    #[test]
    fn noop_sink_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NoopMetricsSink>();
    }

    #[test]
    fn noop_sink_handles_all_optional_fields_none() {
        let sink = NoopMetricsSink;
        sink.record_invocation("p", "m", 0, 0, 0, 0.0, false, "", None);
        sink.record_tool_execution("t", 0, false, false, None);
    }

    #[test]
    fn noop_sink_handles_session_id_some() {
        let sink = NoopMetricsSink;
        sink.record_tool_execution("bash", 100, true, true, Some("session-xyz"));
    }
}
