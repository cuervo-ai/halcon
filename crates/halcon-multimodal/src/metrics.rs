//! Observability counters for the multimodal subsystem.
//!
//! All counters are `AtomicU64` — safe for concurrent access from multiple tasks.
//! Call `snapshot()` to get a serializable point-in-time view.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::Serialize;

/// Shared atomic counters for the multimodal pipeline.
#[derive(Debug, Default)]
pub struct MultimodalMetrics {
    /// Total analysis requests dispatched (image + audio + video).
    pub requests_total: AtomicU64,
    /// Requests served from the media cache (cache hits).
    pub cache_hits: AtomicU64,
    /// Requests that required actual inference (cache misses).
    pub cache_misses: AtomicU64,
    /// Requests routed to API providers.
    pub api_requests: AtomicU64,
    /// Requests routed to local (ONNX/Whisper) inference.
    pub local_requests: AtomicU64,
    /// Total errors (any category).
    pub errors_total: AtomicU64,
    /// Security rejections (file-too-large, bad MIME, decompression bomb).
    pub security_rejections: AtomicU64,
    /// Images analyzed.
    pub images_analyzed: AtomicU64,
    /// Audio files transcribed.
    pub audio_transcribed: AtomicU64,
    /// Video files analyzed.
    pub videos_analyzed: AtomicU64,
    /// Total raw bytes processed (for throughput monitoring).
    pub bytes_processed: AtomicU64,
    /// Cumulative latency in milliseconds (for p50/p95 approximation).
    pub latency_ms_total: AtomicU64,
}

impl MultimodalMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn inc_requests(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_api_request(&self) {
        self.api_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_local_request(&self) {
        self.local_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_error(&self) {
        self.errors_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_security_rejection(&self) {
        self.security_rejections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_image(&self) {
        self.images_analyzed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_audio(&self) {
        self.audio_transcribed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_video(&self) {
        self.videos_analyzed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_bytes(&self, n: u64) {
        self.bytes_processed.fetch_add(n, Ordering::Relaxed);
    }

    pub fn add_latency_ms(&self, ms: u64) {
        self.latency_ms_total.fetch_add(ms, Ordering::Relaxed);
    }

    /// Return a serializable point-in-time snapshot.
    pub fn snapshot(&self) -> MetricsSnapshot {
        let req = self.requests_total.load(Ordering::Relaxed);
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let api = self.api_requests.load(Ordering::Relaxed);
        let local = self.local_requests.load(Ordering::Relaxed);
        let lat = self.latency_ms_total.load(Ordering::Relaxed);
        MetricsSnapshot {
            requests_total: req,
            cache_hits: hits,
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            cache_hit_rate: if req > 0 {
                hits as f64 / req as f64
            } else {
                0.0
            },
            api_requests: api,
            local_requests: local,
            errors_total: self.errors_total.load(Ordering::Relaxed),
            security_rejections: self.security_rejections.load(Ordering::Relaxed),
            images_analyzed: self.images_analyzed.load(Ordering::Relaxed),
            audio_transcribed: self.audio_transcribed.load(Ordering::Relaxed),
            videos_analyzed: self.videos_analyzed.load(Ordering::Relaxed),
            bytes_processed: self.bytes_processed.load(Ordering::Relaxed),
            avg_latency_ms: if req > 0 { lat / req } else { 0 },
        }
    }
}

/// Serializable snapshot of multimodal metrics.
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub requests_total: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    /// Cache hit rate (0.0–1.0). Never NaN (0.0 when requests_total == 0).
    pub cache_hit_rate: f64,
    pub api_requests: u64,
    pub local_requests: u64,
    pub errors_total: u64,
    pub security_rejections: u64,
    pub images_analyzed: u64,
    pub audio_transcribed: u64,
    pub videos_analyzed: u64,
    pub bytes_processed: u64,
    pub avg_latency_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_no_requests_no_nan() {
        let m = MultimodalMetrics::new();
        let s = m.snapshot();
        assert_eq!(s.requests_total, 0);
        // cache_hit_rate must NOT be NaN when requests = 0
        assert!(s.cache_hit_rate.is_finite());
        assert_eq!(s.cache_hit_rate, 0.0);
        assert_eq!(s.avg_latency_ms, 0);
    }

    #[test]
    fn snapshot_accurate_counts() {
        let m = MultimodalMetrics::new();
        m.inc_requests();
        m.inc_requests();
        m.inc_cache_hit();
        m.inc_api_request();
        m.inc_local_request();
        m.inc_error();
        m.record_image();
        m.record_audio();
        m.add_bytes(1024);
        m.add_latency_ms(100);

        let s = m.snapshot();
        assert_eq!(s.requests_total, 2);
        assert_eq!(s.cache_hits, 1);
        assert_eq!(s.api_requests, 1);
        assert_eq!(s.local_requests, 1);
        assert_eq!(s.errors_total, 1);
        assert_eq!(s.images_analyzed, 1);
        assert_eq!(s.audio_transcribed, 1);
        assert_eq!(s.bytes_processed, 1024);
        assert_eq!(s.avg_latency_ms, 50); // 100ms / 2 requests
    }

    #[test]
    fn snapshot_cache_hit_rate_correct() {
        let m = MultimodalMetrics::new();
        for _ in 0..4 {
            m.inc_requests();
        }
        m.inc_cache_hit();
        m.inc_cache_hit();
        let s = m.snapshot();
        assert!((s.cache_hit_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn snapshot_is_json_serializable() {
        let m = MultimodalMetrics::new();
        m.inc_requests();
        m.record_video();
        let s = m.snapshot();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("requests_total"));
        assert!(json.contains("cache_hit_rate"));
        // Verify no NaN in JSON (JSON cannot represent NaN)
        assert!(!json.contains("NaN"));
    }

    #[test]
    fn concurrent_increments_are_safe() {
        use std::thread;
        let m = Arc::new(MultimodalMetrics::default());
        let mut handles = Vec::new();
        for _ in 0..10 {
            let m2 = m.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    m2.inc_requests();
                    m2.inc_cache_hit();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(m.requests_total.load(Ordering::SeqCst), 1000);
        assert_eq!(m.cache_hits.load(Ordering::SeqCst), 1000);
    }
}
