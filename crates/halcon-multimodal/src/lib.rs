//! Multimodal subsystem for HALCÓN CLI.
//!
//! Provides production-ready image, audio, and video analysis with:
//! - **P0 security**: EXIF stripping, magic-byte MIME detection, size limits,
//!   decompression bomb detection.
//! - **Hybrid routing**: local ONNX/Whisper vs API provider.
//! - **Content-addressed caching** (M28 `media_cache`).
//! - **CLIP embedding index** for cross-modal retrieval (M29 `media_index`).
//! - **Observability** via lock-free atomic counters (`MetricsSnapshot`).
//! - **Context pipeline integration** via `MediaContextSource`.
//!
//! # Activation
//!
//! The subsystem is created by `MultimodalSubsystem::init()` when
//! `--full` is passed to `halcon chat`.

pub mod context;
pub mod error;
pub mod index;
pub mod metrics;
pub mod provider;
pub mod router;
pub mod security;
pub mod sota;
pub mod worker;

use std::sync::Arc;

use halcon_core::types::MultimodalConfig;
use halcon_storage::AsyncDatabase;

use error::Result;
use index::MediaIndex;
use metrics::MultimodalMetrics;
use provider::api::ApiMultimodalProvider;
use provider::native::NativeMultimodalProvider;
use router::{
    cache::MediaCache,
    policy::RoutingPolicy,
    HybridRouter,
};
use security::{limits::SecurityLimits, MediaValidator};
use worker::MediaWorkerPool;

/// Multimodal subsystem facade.
///
/// Encapsulates security validation, routing, caching, indexing, and metrics.
/// Created via [`MultimodalSubsystem::init()`] and held by the Repl when `--full` is active.
#[derive(Debug)]
pub struct MultimodalSubsystem {
    pub router:    HybridRouter,
    pub validator: MediaValidator,
    pub index:     Arc<MediaIndex>,
    pub metrics:   Arc<MultimodalMetrics>,
    pub workers:   Arc<MediaWorkerPool>,
}

impl MultimodalSubsystem {
    /// Initialize the multimodal subsystem from config and a shared database.
    pub fn init(config: &MultimodalConfig, db: Arc<AsyncDatabase>) -> Result<Self> {
        let metrics = MultimodalMetrics::new();

        // Worker pool for CPU-bound ONNX / Whisper inference.
        let workers = MediaWorkerPool::new(0)?; // 0 = rayon default (CPU count)

        // Security
        let limits = SecurityLimits {
            max_file_bytes:   config.max_file_size_bytes,
            max_audio_secs:   config.max_audio_duration_secs,
            max_video_secs:   config.max_video_duration_secs,
            ..SecurityLimits::default()
        };
        let validator = MediaValidator::new(limits, config.strip_exif, config.privacy_strict);

        // Providers
        let native = NativeMultimodalProvider::new(config.models_dir.clone());
        let native_available = native.clip_available();
        let api_provider: Arc<dyn provider::MultimodalProvider> =
            Arc::new(ApiMultimodalProvider::new("halcon-api"));
        let local_provider: Option<Arc<dyn provider::MultimodalProvider>> = if native_available {
            Some(Arc::new(native))
        } else {
            None
        };

        // Cache
        let cache = if config.cache_enabled {
            Some(MediaCache::new(Arc::clone(&db), config.cache_ttl_secs))
        } else {
            None
        };

        // Router
        let policy = RoutingPolicy {
            local_threshold_bytes: config.local_threshold_bytes,
            native_available,
        };
        let router = HybridRouter::new(
            policy,
            cache,
            api_provider,
            local_provider,
            Arc::clone(&metrics),
        );

        let index = Arc::new(MediaIndex::new(db));

        Ok(Self { router, validator, index, metrics, workers })
    }

    /// Validate raw bytes and run inference on the result.
    pub async fn analyze_bytes(
        &self,
        data:   Vec<u8>,
        prompt: Option<&str>,
    ) -> Result<provider::MediaAnalysis> {
        let validated = self.validator.validate_bytes(data)?;
        self.router.analyze(&validated, prompt).await
    }

    /// Return a point-in-time metrics snapshot.
    pub fn metrics_snapshot(&self) -> metrics::MetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Build a `MediaContextSource` for the context assembly pipeline.
    pub fn context_source(&self) -> context::MediaContextSource {
        context::MediaContextSource::new(Arc::clone(&self.index), 5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_storage::Database;

    fn test_config() -> MultimodalConfig {
        MultimodalConfig {
            enabled:                  true,
            mode:                     "api".into(),
            max_file_size_bytes:      20 * 1024 * 1024,
            local_threshold_bytes:    2 * 1024 * 1024,
            strip_exif:               true,
            privacy_strict:           false,
            max_audio_duration_secs:  300,
            max_video_duration_secs:  60,
            video_sample_fps:         1,
            max_video_frames:         10,
            cache_enabled:            true,
            cache_ttl_secs:           3600,
            models_dir:               None,
            api_timeout_ms:           30_000,
        }
    }

    fn test_db() -> Arc<AsyncDatabase> {
        Arc::new(AsyncDatabase::new(Arc::new(Database::open_in_memory().unwrap())))
    }

    #[test]
    fn init_succeeds() {
        let sys = MultimodalSubsystem::init(&test_config(), test_db()).unwrap();
        let snap = sys.metrics_snapshot();
        assert_eq!(snap.requests_total, 0);
    }

    #[tokio::test]
    async fn analyze_jpeg_bytes() {
        let sys  = MultimodalSubsystem::init(&test_config(), test_db()).unwrap();
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9]; // minimal JPEG
        let result = sys.analyze_bytes(data, Some("describe")).await.unwrap();
        assert_eq!(result.modality, "image");
    }

    #[tokio::test]
    async fn metrics_increment_after_analysis() {
        let sys  = MultimodalSubsystem::init(&test_config(), test_db()).unwrap();
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9];
        sys.analyze_bytes(data, None).await.unwrap();
        let snap = sys.metrics_snapshot();
        assert_eq!(snap.requests_total, 1);
        assert_eq!(snap.images_analyzed, 1);
    }

    #[tokio::test]
    async fn second_call_hits_cache() {
        let sys  = MultimodalSubsystem::init(&test_config(), test_db()).unwrap();
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9];
        sys.analyze_bytes(data.clone(), None).await.unwrap();
        sys.analyze_bytes(data, None).await.unwrap();
        let snap = sys.metrics_snapshot();
        assert_eq!(snap.requests_total, 2);
        assert_eq!(snap.cache_hits, 1);
        assert_eq!(snap.cache_misses, 1);
    }

    #[test]
    fn context_source_name() {
        use halcon_core::traits::ContextSource;
        let sys = MultimodalSubsystem::init(&test_config(), test_db()).unwrap();
        let src = sys.context_source();
        assert_eq!(src.name(), "media_index");
    }
}
