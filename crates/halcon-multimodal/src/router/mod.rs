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
pub struct HybridRouter {
    policy:  RoutingPolicy,
    cache:   Option<MediaCache>,
    api:     Arc<dyn MultimodalProvider>,
    local:   Option<Arc<dyn MultimodalProvider>>,
    metrics: Arc<MultimodalMetrics>,
}

impl std::fmt::Debug for HybridRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridRouter")
            .field("policy",  &self.policy)
            .field("cache",   &self.cache)
            .field("metrics", &self.metrics)
            .finish()
    }
}

impl HybridRouter {
    pub fn new(
        policy:  RoutingPolicy,
        cache:   Option<MediaCache>,
        api:     Arc<dyn MultimodalProvider>,
        local:   Option<Arc<dyn MultimodalProvider>>,
        metrics: Arc<MultimodalMetrics>,
    ) -> Self {
        Self { policy, cache, api, local, metrics }
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
        let result = match self.policy.decide(media) {
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

        let analysis = result.map_err(|e| { self.metrics.inc_error(); e })?;

        // Per-modality counters
        if media.is_image()      { self.metrics.record_image(); }
        else if media.is_audio() { self.metrics.record_audio(); }
        else if media.is_video() { self.metrics.record_video(); }

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
    use crate::provider::MockMultimodalProvider;
    use crate::security::{ValidatedMedia, mime::DetectedMime};

    fn jpeg_media() -> ValidatedMedia {
        ValidatedMedia { data: vec![0xFF, 0xD8, 0xFF, 0xD9], mime: DetectedMime::ImageJpeg, original_size: 4 }
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
        let media  = jpeg_media();
        let result = router.analyze(&media, Some("describe")).await.unwrap();
        assert_eq!(result.modality, "image");
    }

    #[tokio::test]
    async fn metrics_increment_on_request() {
        let metrics = MultimodalMetrics::new();
        let router  = HybridRouter::new(
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
}
