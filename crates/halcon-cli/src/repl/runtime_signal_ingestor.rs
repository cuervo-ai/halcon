//! Phase 7: Runtime Signal Ingestor (OTEL-compatible)
//!
//! Collects runtime observability signals (metrics, spans, logs) from the
//! agent's execution environment and maps them to structured `RuntimeSignal`
//! values that the reward pipeline can consume.
//!
//! # Architecture
//! - `SignalKind` — what type of telemetry signal was received.
//! - `RuntimeSignal` — a timestamped signal value with metadata.
//! - `SignalBuffer` — bounded ring buffer of recent signals.
//! - `RuntimeMetrics` — aggregated window statistics (p50/p95/p99 latency, error rate, throughput).
//! - `RuntimeSignalIngestor` — async receiver that ingests signals and maintains a rolling window.
//!
//! # OTEL feature gate
//! When the `otel` Cargo feature is enabled, the `OtelSpanReceiver` backend
//! translates OTEL span events into `RuntimeSignal`s. Without the feature, only
//! the in-process `direct_ingest()` path is available (suitable for testing).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{broadcast, Mutex};

// ── Signal kinds ──────────────────────────────────────────────────────────────

/// The category of a runtime observability signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalKind {
    /// A completed operation span (name, duration_ms).
    Span,
    /// A gauge metric (name, value).
    Gauge,
    /// A counter increment (name, delta).
    Counter,
    /// An error event (message).
    Error,
    /// A log line at a specified severity.
    Log,
}

// ── RuntimeSignal ─────────────────────────────────────────────────────────────

/// A single telemetry observation from the runtime.
#[derive(Debug, Clone)]
pub struct RuntimeSignal {
    /// Wall-clock timestamp (seconds since UNIX epoch).
    pub timestamp_secs: u64,
    /// Signal category.
    pub kind: SignalKind,
    /// Operation or metric name.
    pub name: String,
    /// Numeric value (duration_ms for spans, gauge reading, counter delta).
    pub value: f64,
    /// Whether this signal represents an error outcome.
    pub is_error: bool,
    /// Free-form tags (key=value pairs for filtering).
    pub tags: Vec<(String, String)>,
}

impl RuntimeSignal {
    /// Create a span signal.
    pub fn span(name: impl Into<String>, duration_ms: f64, is_error: bool) -> Self {
        Self {
            timestamp_secs: now_secs(),
            kind: SignalKind::Span,
            name: name.into(),
            value: duration_ms,
            is_error,
            tags: vec![],
        }
    }

    /// Create a gauge metric signal.
    pub fn gauge(name: impl Into<String>, value: f64) -> Self {
        Self {
            timestamp_secs: now_secs(),
            kind: SignalKind::Gauge,
            name: name.into(),
            value,
            is_error: false,
            tags: vec![],
        }
    }

    /// Create a counter increment signal.
    pub fn counter(name: impl Into<String>, delta: f64) -> Self {
        Self {
            timestamp_secs: now_secs(),
            kind: SignalKind::Counter,
            name: name.into(),
            value: delta,
            is_error: false,
            tags: vec![],
        }
    }

    /// Create an error event signal.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            timestamp_secs: now_secs(),
            kind: SignalKind::Error,
            name: "error".to_string(),
            value: 1.0,
            is_error: true,
            tags: vec![("message".to_string(), message.into())],
        }
    }

    /// Add a tag to this signal (builder pattern).
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((key.into(), value.into()));
        self
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Aggregated metrics ────────────────────────────────────────────────────────

/// Rolling-window aggregated metrics derived from recent `RuntimeSignal`s.
#[derive(Debug, Clone, Default)]
pub struct RuntimeMetrics {
    /// Number of signals in the current window.
    pub sample_count: usize,
    /// Fraction of signals with `is_error = true` (0.0–1.0).
    pub error_rate: f64,
    /// Median span duration in milliseconds (p50).
    pub p50_ms: f64,
    /// 95th-percentile span duration in milliseconds.
    pub p95_ms: f64,
    /// 99th-percentile span duration in milliseconds.
    pub p99_ms: f64,
    /// Signals per second over the window duration.
    pub throughput_per_sec: f64,
    /// Sum of all counter deltas per counter name.
    pub counter_totals: Vec<(String, f64)>,
}

impl RuntimeMetrics {
    /// Map metrics to a [0, 1] reward signal for the UCB1 pipeline.
    ///
    /// Formula: `(1 - error_rate) * latency_score`
    /// where `latency_score = 1.0 / (1.0 + p95_ms / 1000.0)` (sigmoid-like decay).
    /// Returns 0.5 when no span samples are available (neutral prior).
    pub fn as_reward(&self) -> f64 {
        if self.sample_count == 0 {
            return 0.5;
        }
        let health = 1.0 - self.error_rate.clamp(0.0, 1.0);
        let latency_score = if self.p95_ms > 0.0 {
            1.0 / (1.0 + self.p95_ms / 1000.0)
        } else {
            1.0 // no latency data → optimistic
        };
        (health * latency_score).clamp(0.0, 1.0)
    }
}

// ── Signal buffer ─────────────────────────────────────────────────────────────

/// Bounded ring buffer of recent `RuntimeSignal`s.
///
/// Older signals are evicted when `max_capacity` is exceeded.
struct SignalBuffer {
    signals: VecDeque<RuntimeSignal>,
    max_capacity: usize,
    /// Total signals ever ingested (for throughput calculation).
    total_ingested: u64,
    /// Timestamp of the first signal in the current window.
    window_start: Option<Instant>,
}

impl SignalBuffer {
    fn new(max_capacity: usize) -> Self {
        Self {
            signals: VecDeque::with_capacity(max_capacity),
            max_capacity,
            total_ingested: 0,
            window_start: None,
        }
    }

    fn push(&mut self, signal: RuntimeSignal) {
        if self.window_start.is_none() {
            self.window_start = Some(Instant::now());
        }
        if self.signals.len() >= self.max_capacity {
            self.signals.pop_front();
        }
        self.signals.push_back(signal);
        self.total_ingested += 1;
    }

    fn compute_metrics(&self) -> RuntimeMetrics {
        if self.signals.is_empty() {
            return RuntimeMetrics::default();
        }

        let sample_count = self.signals.len();
        let error_count = self.signals.iter().filter(|s| s.is_error).count();
        let error_rate = error_count as f64 / sample_count as f64;

        // Collect span durations for percentile calculation.
        let mut durations: Vec<f64> = self
            .signals
            .iter()
            .filter(|s| s.kind == SignalKind::Span)
            .map(|s| s.value)
            .collect();
        durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let p50_ms = percentile(&durations, 0.50);
        let p95_ms = percentile(&durations, 0.95);
        let p99_ms = percentile(&durations, 0.99);

        // Counter totals.
        let mut counter_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
        for sig in &self.signals {
            if sig.kind == SignalKind::Counter {
                *counter_map.entry(sig.name.clone()).or_default() += sig.value;
            }
        }
        let mut counter_totals: Vec<(String, f64)> = counter_map.into_iter().collect();
        counter_totals.sort_by(|a, b| a.0.cmp(&b.0));

        // Throughput.
        let throughput_per_sec = self
            .window_start
            .map(|start| {
                let elapsed = start.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    self.total_ingested as f64 / elapsed
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);

        RuntimeMetrics {
            sample_count,
            error_rate,
            p50_ms,
            p95_ms,
            p99_ms,
            throughput_per_sec,
            counter_totals,
        }
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// ── Ingestor ──────────────────────────────────────────────────────────────────

/// Async runtime signal ingestor with rolling-window metrics.
///
/// # Usage
/// ```no_run
/// # async fn example() {
/// use halcon_cli::repl::runtime_signal_ingestor::{RuntimeSignalIngestor, RuntimeSignal};
///
/// let ingestor = RuntimeSignalIngestor::new(1000);
/// let mut rx = ingestor.subscribe();
///
/// // Inject a span signal directly (without OTEL).
/// ingestor.ingest(RuntimeSignal::span("agent.round", 342.0, false)).await;
/// let metrics = ingestor.metrics().await;
/// println!("p95 latency: {}ms", metrics.p95_ms);
/// # }
/// ```
pub struct RuntimeSignalIngestor {
    buffer: Arc<Mutex<SignalBuffer>>,
    tx: broadcast::Sender<RuntimeSignal>,
}

impl RuntimeSignalIngestor {
    /// Create a new ingestor with the given buffer capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.min(1024));
        Self {
            buffer: Arc::new(Mutex::new(SignalBuffer::new(capacity))),
            tx,
        }
    }

    /// Subscribe to the raw signal stream for real-time consumers.
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeSignal> {
        self.tx.subscribe()
    }

    /// Ingest a single signal directly (no OTEL pipeline required).
    pub async fn ingest(&self, signal: RuntimeSignal) {
        let _ = self.tx.send(signal.clone());
        self.buffer.lock().await.push(signal);
    }

    /// Compute aggregated metrics from the current rolling window.
    pub async fn metrics(&self) -> RuntimeMetrics {
        self.buffer.lock().await.compute_metrics()
    }

    /// Drain all signals matching `predicate` from the buffer.
    ///
    /// Returns the number of signals removed.
    pub async fn drain_where<F>(&self, predicate: F) -> usize
    where
        F: Fn(&RuntimeSignal) -> bool,
    {
        let mut guard = self.buffer.lock().await;
        let before = guard.signals.len();
        guard.signals.retain(|s| !predicate(s));
        before - guard.signals.len()
    }

    /// Return the number of signals currently buffered.
    pub async fn buffered_count(&self) -> usize {
        self.buffer.lock().await.signals.len()
    }

    /// Spawn a background task that reads from `rx` and ingests each signal.
    ///
    /// Useful for bridging an external signal source (e.g. a UDP/HTTP OTEL
    /// receiver) into the ingestor's rolling window.
    pub fn start_relay(
        self: Arc<Self>,
        mut rx: broadcast::Receiver<RuntimeSignal>,
        stop: Arc<tokio::sync::Notify>,
    ) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.notified() => break,
                    result = rx.recv() => {
                        match result {
                            Ok(signal) => self.ingest(signal).await,
                            Err(broadcast::error::RecvError::Closed) => break,
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(dropped = n, "RuntimeSignalIngestor relay lagged");
                            }
                        }
                    }
                }
            }
        });
    }
}

// ── Metric snapshot helper ────────────────────────────────────────────────────

/// Summarise `RuntimeMetrics` as a short Markdown block for system prompt injection.
pub fn metrics_to_markdown(m: &RuntimeMetrics) -> String {
    if m.sample_count == 0 {
        return String::new();
    }
    let mut out = String::from("## Runtime Signals\n");
    out.push_str(&format!("- Samples: {}\n", m.sample_count));
    out.push_str(&format!("- Error rate: {:.1}%\n", m.error_rate * 100.0));
    if m.p50_ms > 0.0 {
        out.push_str(&format!(
            "- Latency (p50/p95/p99): {:.1}ms / {:.1}ms / {:.1}ms\n",
            m.p50_ms, m.p95_ms, m.p99_ms
        ));
    }
    out.push_str(&format!("- Throughput: {:.1} sig/s\n", m.throughput_per_sec));
    for (name, total) in &m.counter_totals {
        out.push_str(&format!("- Counter `{name}`: {total:.0}\n"));
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── RuntimeSignal constructors ────────────────────────────────────────────

    #[test]
    fn span_signal_has_correct_kind() {
        let s = RuntimeSignal::span("op", 100.0, false);
        assert_eq!(s.kind, SignalKind::Span);
        assert!((s.value - 100.0).abs() < 1e-9);
        assert!(!s.is_error);
    }

    #[test]
    fn error_signal_is_marked() {
        let s = RuntimeSignal::error("something broke");
        assert!(s.is_error);
        assert_eq!(s.kind, SignalKind::Error);
    }

    #[test]
    fn gauge_signal_kind() {
        let s = RuntimeSignal::gauge("memory_mb", 256.0);
        assert_eq!(s.kind, SignalKind::Gauge);
        assert!((s.value - 256.0).abs() < 1e-9);
    }

    #[test]
    fn counter_signal_kind() {
        let s = RuntimeSignal::counter("requests", 1.0);
        assert_eq!(s.kind, SignalKind::Counter);
    }

    #[test]
    fn with_tag_adds_tag() {
        let s = RuntimeSignal::span("op", 10.0, false).with_tag("env", "prod");
        assert_eq!(s.tags, vec![("env".to_string(), "prod".to_string())]);
    }

    // ── SignalBuffer percentiles ──────────────────────────────────────────────

    #[test]
    fn percentile_empty_returns_zero() {
        assert_eq!(percentile(&[], 0.95), 0.0);
    }

    #[test]
    fn percentile_single_element() {
        assert!((percentile(&[42.0], 0.50) - 42.0).abs() < 1e-9);
        assert!((percentile(&[42.0], 0.95) - 42.0).abs() < 1e-9);
    }

    #[test]
    fn percentile_p50_of_sorted_values() {
        let vals = [10.0, 20.0, 30.0, 40.0, 50.0];
        // p50 at index 2 → 30.0
        assert!((percentile(&vals, 0.50) - 30.0).abs() < 1e-9);
    }

    #[test]
    fn percentile_p95_uses_upper_end() {
        let vals: Vec<f64> = (1..=20).map(|i| i as f64 * 10.0).collect();
        // 19th index (0-based): val = 190 (p95 of 20 items)
        let p95 = percentile(&vals, 0.95);
        assert!(p95 >= 150.0, "p95={p95}");
    }

    // ── RuntimeMetrics::as_reward ─────────────────────────────────────────────

    #[test]
    fn reward_zero_samples_is_neutral() {
        let m = RuntimeMetrics::default();
        assert!((m.as_reward() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn reward_all_errors_is_low() {
        let m = RuntimeMetrics {
            sample_count: 10,
            error_rate: 1.0,
            p95_ms: 0.0,
            ..Default::default()
        };
        assert!((m.as_reward() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn reward_no_errors_low_latency_is_high() {
        let m = RuntimeMetrics {
            sample_count: 10,
            error_rate: 0.0,
            p95_ms: 10.0, // very low
            ..Default::default()
        };
        let r = m.as_reward();
        assert!(r > 0.9, "reward={r}");
    }

    #[test]
    fn reward_high_latency_degrades() {
        let m_fast = RuntimeMetrics {
            sample_count: 1,
            error_rate: 0.0,
            p95_ms: 50.0,
            ..Default::default()
        };
        let m_slow = RuntimeMetrics {
            sample_count: 1,
            error_rate: 0.0,
            p95_ms: 5000.0,
            ..Default::default()
        };
        assert!(m_fast.as_reward() > m_slow.as_reward());
    }

    #[test]
    fn reward_is_in_unit_interval() {
        let cases = [
            RuntimeMetrics { sample_count: 5, error_rate: 0.4, p95_ms: 200.0, ..Default::default() },
            RuntimeMetrics { sample_count: 0, ..Default::default() },
            RuntimeMetrics { sample_count: 1, error_rate: 0.0, p95_ms: 0.0, ..Default::default() },
        ];
        for m in &cases {
            let r = m.as_reward();
            assert!((0.0..=1.0).contains(&r), "reward out of range: {r}");
        }
    }

    // ── RuntimeSignalIngestor ─────────────────────────────────────────────────

    #[tokio::test]
    async fn ingest_increments_buffer() {
        let ingestor = RuntimeSignalIngestor::new(64);
        assert_eq!(ingestor.buffered_count().await, 0);
        ingestor.ingest(RuntimeSignal::span("op", 100.0, false)).await;
        assert_eq!(ingestor.buffered_count().await, 1);
    }

    #[tokio::test]
    async fn ingest_multiple_computes_metrics() {
        let ingestor = RuntimeSignalIngestor::new(64);
        for ms in [100.0, 200.0, 300.0, 400.0, 500.0] {
            ingestor.ingest(RuntimeSignal::span("op", ms, false)).await;
        }
        let m = ingestor.metrics().await;
        assert_eq!(m.sample_count, 5);
        assert_eq!(m.error_rate, 0.0);
        // p50 of [100,200,300,400,500] = 300
        assert!((m.p50_ms - 300.0).abs() < 1e-6, "p50={}", m.p50_ms);
    }

    #[tokio::test]
    async fn error_rate_computed_correctly() {
        let ingestor = RuntimeSignalIngestor::new(64);
        ingestor.ingest(RuntimeSignal::span("ok", 50.0, false)).await;
        ingestor.ingest(RuntimeSignal::span("err", 50.0, true)).await;
        ingestor.ingest(RuntimeSignal::error("boom")).await;
        let m = ingestor.metrics().await;
        // 2/3 signals are errors
        assert!((m.error_rate - 2.0 / 3.0).abs() < 1e-6, "error_rate={}", m.error_rate);
    }

    #[tokio::test]
    async fn buffer_evicts_oldest_when_full() {
        let ingestor = RuntimeSignalIngestor::new(3);
        for i in 0..5u64 {
            ingestor
                .ingest(RuntimeSignal::gauge("g", i as f64))
                .await;
        }
        // Buffer holds max 3 — should contain the 3 newest.
        assert_eq!(ingestor.buffered_count().await, 3);
    }

    #[tokio::test]
    async fn counter_totals_accumulated() {
        let ingestor = RuntimeSignalIngestor::new(64);
        ingestor.ingest(RuntimeSignal::counter("req", 1.0)).await;
        ingestor.ingest(RuntimeSignal::counter("req", 3.0)).await;
        ingestor.ingest(RuntimeSignal::counter("err", 2.0)).await;
        let m = ingestor.metrics().await;
        let req_total = m
            .counter_totals
            .iter()
            .find(|(k, _)| k == "req")
            .map(|(_, v)| *v);
        assert_eq!(req_total, Some(4.0));
    }

    #[tokio::test]
    async fn drain_where_removes_errors() {
        let ingestor = RuntimeSignalIngestor::new(64);
        ingestor.ingest(RuntimeSignal::span("ok", 10.0, false)).await;
        ingestor.ingest(RuntimeSignal::error("e1")).await;
        ingestor.ingest(RuntimeSignal::error("e2")).await;
        let removed = ingestor.drain_where(|s| s.is_error).await;
        assert_eq!(removed, 2);
        assert_eq!(ingestor.buffered_count().await, 1);
    }

    #[tokio::test]
    async fn subscribe_receives_ingested_signals() {
        let ingestor = RuntimeSignalIngestor::new(64);
        let mut rx = ingestor.subscribe();
        ingestor.ingest(RuntimeSignal::span("ping", 1.0, false)).await;
        let sig = rx.try_recv().expect("should have received a signal");
        assert_eq!(sig.name, "ping");
    }

    // ── metrics_to_markdown ───────────────────────────────────────────────────

    #[tokio::test]
    async fn metrics_to_markdown_empty_when_no_samples() {
        let ingestor = RuntimeSignalIngestor::new(64);
        let m = ingestor.metrics().await;
        assert!(metrics_to_markdown(&m).is_empty());
    }

    #[tokio::test]
    async fn metrics_to_markdown_includes_stats() {
        let ingestor = RuntimeSignalIngestor::new(64);
        ingestor.ingest(RuntimeSignal::span("op", 150.0, false)).await;
        ingestor.ingest(RuntimeSignal::counter("hits", 5.0)).await;
        let m = ingestor.metrics().await;
        let md = metrics_to_markdown(&m);
        assert!(md.contains("Samples"));
        assert!(md.contains("hits"));
    }
}
