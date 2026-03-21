//! Hybrid router: local vs API, with content-addressed caching and observability.

pub mod cache;
pub mod detector;
pub mod policy;

use std::sync::Arc;

use crate::error::Result;
use crate::metrics::MultimodalMetrics;
use crate::provider::{MediaAnalysis, MultimodalProvider};
use crate::security::ValidatedMedia;
use cache::MediaCache;
use policy::{RoutingDecision, RoutingPolicy};

/// Routes media to the appropriate provider with caching and metrics.
///
/// Supports an optional fallback provider chain: if the primary API provider
/// returns a transient error (e.g., 429/503), the router retries with each
/// fallback provider in order before propagating the error.
pub struct HybridRouter {
    policy: RoutingPolicy,
    cache: Option<MediaCache>,
    api: Arc<dyn MultimodalProvider>,
    local: Option<Arc<dyn MultimodalProvider>>,
    metrics: Arc<MultimodalMetrics>,
    /// Secondary providers tried when the primary `api` returns an error.
    fallbacks: Vec<Arc<dyn MultimodalProvider>>,
}

impl std::fmt::Debug for HybridRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridRouter")
            .field("policy", &self.policy)
            .field("cache", &self.cache)
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl HybridRouter {
    pub fn new(
        policy: RoutingPolicy,
        cache: Option<MediaCache>,
        api: Arc<dyn MultimodalProvider>,
        local: Option<Arc<dyn MultimodalProvider>>,
        metrics: Arc<MultimodalMetrics>,
    ) -> Self {
        Self {
            policy,
            cache,
            api,
            local,
            metrics,
            fallbacks: vec![],
        }
    }

    /// Register additional providers to try if the primary API fails.
    pub fn with_fallbacks(mut self, providers: Vec<Arc<dyn MultimodalProvider>>) -> Self {
        self.fallbacks = providers;
        self
    }

    /// Clone of the primary API provider (for frame-level video analysis).
    pub fn api_provider(&self) -> Arc<dyn MultimodalProvider> {
        Arc::clone(&self.api)
    }

    /// Name of the primary (API) provider.
    pub fn provider_name(&self) -> &str {
        self.api.name()
    }

    /// Returns true if the primary provider supports audio transcription.
    pub fn supports_audio(&self) -> bool {
        self.api.supports_modality("audio")
    }

    /// Analyze media: check cache → route → store result → return.
    pub async fn analyze(
        &self,
        media: &ValidatedMedia,
        prompt: Option<&str>,
    ) -> Result<MediaAnalysis> {
        let t0 = std::time::Instant::now();
        self.metrics.inc_requests();
        self.metrics.add_bytes(media.original_size);

        // Cache lookup
        if let Some(cache) = &self.cache {
            if let Ok(Some(cached)) = cache.get(&media.data).await {
                self.metrics.inc_cache_hit();
                self.metrics.add_latency_ms(t0.elapsed().as_millis() as u64);
                return Ok(cached);
            }
        }
        self.metrics.inc_cache_miss();

        // Routing decision
        let primary_result = match self.policy.decide(media) {
            RoutingDecision::Local => {
                if let Some(local) = &self.local {
                    self.metrics.inc_local_request();
                    local.analyze(media, prompt).await
                } else {
                    self.metrics.inc_api_request();
                    self.api.analyze(media, prompt).await
                }
            }
            RoutingDecision::Api => {
                self.metrics.inc_api_request();
                self.api.analyze(media, prompt).await
            }
        };

        // Provider fallback chain: on primary error, try each fallback in order.
        let result = if let Err(initial_err) = primary_result {
            if self.fallbacks.is_empty() {
                return Err(initial_err);
            }
            let mut last_err = initial_err;
            let mut succeeded = None;
            for fb in &self.fallbacks {
                self.metrics.inc_api_request();
                match fb.analyze(media, prompt).await {
                    Ok(a) => {
                        tracing::info!(
                            fallback_provider = fb.name(),
                            "Primary provider failed; fallback succeeded"
                        );
                        succeeded = Some(a);
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            fallback_provider = fb.name(),
                            error = %e,
                            "Fallback provider also failed"
                        );
                        last_err = e;
                    }
                }
            }
            succeeded.map(Ok).unwrap_or(Err(last_err))
        } else {
            primary_result
        };

        let analysis = result.inspect_err(|_e| {
            self.metrics.inc_error();
        })?;

        // Per-modality counters
        if media.is_image() {
            self.metrics.record_image();
        } else if media.is_audio() {
            self.metrics.record_audio();
        } else if media.is_video() {
            self.metrics.record_video();
        }

        // Cache store (best-effort)
        if let Some(cache) = &self.cache {
            if let Err(e) = cache.set(&media.data, &analysis).await {
                tracing::warn!(err = %e, "failed to cache media analysis");
            }
        }

        self.metrics.add_latency_ms(t0.elapsed().as_millis() as u64);
        Ok(analysis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MultimodalError;
    use crate::provider::MockMultimodalProvider;
    use crate::security::{mime::DetectedMime, ValidatedMedia};

    /// A provider that always returns an error (for testing fallback).
    struct AlwaysErrorProvider;

    #[async_trait::async_trait]
    impl crate::provider::MultimodalProvider for AlwaysErrorProvider {
        fn name(&self) -> &str {
            "always-error"
        }
        fn supports_modality(&self, _: &str) -> bool {
            true
        }
        async fn analyze(
            &self,
            _: &ValidatedMedia,
            _: Option<&str>,
        ) -> crate::error::Result<crate::provider::MediaAnalysis> {
            Err(MultimodalError::Internal("simulated API failure".into()))
        }
    }

    fn jpeg_media() -> ValidatedMedia {
        ValidatedMedia {
            data: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mime: DetectedMime::ImageJpeg,
            original_size: 4,
        }
    }

    fn make_router() -> HybridRouter {
        HybridRouter::new(
            RoutingPolicy::default(),
            None,
            Arc::new(MockMultimodalProvider),
            None,
            MultimodalMetrics::new(),
        )
    }

    #[tokio::test]
    async fn router_returns_analysis() {
        let router = make_router();
        let media = jpeg_media();
        let result = router.analyze(&media, Some("describe")).await.unwrap();
        assert_eq!(result.modality, "image");
    }

    #[tokio::test]
    async fn metrics_increment_on_request() {
        let metrics = MultimodalMetrics::new();
        let router = HybridRouter::new(
            RoutingPolicy::default(),
            None,
            Arc::new(MockMultimodalProvider),
            None,
            Arc::clone(&metrics),
        );
        router.analyze(&jpeg_media(), None).await.unwrap();
        let snap = metrics.snapshot();
        assert_eq!(snap.requests_total, 1);
        assert_eq!(snap.images_analyzed, 1);
        // MockMultimodalProvider is the api slot → api_requests incremented
        assert_eq!(snap.api_requests, 1);
    }

    #[tokio::test]
    async fn fallback_chain_used_when_primary_fails() {
        let metrics = MultimodalMetrics::new();
        // Primary always errors; fallback is the mock (succeeds).
        let router = HybridRouter::new(
            RoutingPolicy::default(),
            None,
            Arc::new(AlwaysErrorProvider),
            None,
            Arc::clone(&metrics),
        )
        .with_fallbacks(vec![Arc::new(MockMultimodalProvider)]);

        let result = router.analyze(&jpeg_media(), None).await.unwrap();
        assert_eq!(
            result.modality, "image",
            "fallback should return image analysis"
        );
        // Primary + fallback = 2 api_requests incremented.
        assert_eq!(metrics.snapshot().api_requests, 2);
    }

    #[tokio::test]
    async fn error_propagated_when_all_providers_fail() {
        let router = HybridRouter::new(
            RoutingPolicy::default(),
            None,
            Arc::new(AlwaysErrorProvider),
            None,
            MultimodalMetrics::new(),
        )
        .with_fallbacks(vec![Arc::new(AlwaysErrorProvider)]);

        let err = router.analyze(&jpeg_media(), None).await.unwrap_err();
        assert!(
            err.to_string().contains("simulated") || err.to_string().contains("API"),
            "last provider error should propagate; got: {err}"
        );
    }

    #[tokio::test]
    async fn no_fallbacks_propagates_primary_error() {
        let router = HybridRouter::new(
            RoutingPolicy::default(),
            None,
            Arc::new(AlwaysErrorProvider),
            None,
            MultimodalMetrics::new(),
        );

        let err = router.analyze(&jpeg_media(), None).await.unwrap_err();
        assert!(err.to_string().contains("simulated"));
    }

    #[tokio::test]
    async fn api_provider_accessor_returns_same_provider() {
        let router = make_router();
        // The api_provider() clone should produce the same provider type.
        let provider = router.api_provider();
        let media = jpeg_media();
        let result = provider.analyze(&media, None).await.unwrap();
        assert_eq!(result.modality, "image");
    }
}
