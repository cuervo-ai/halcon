//! Provider abstraction for multimodal inference.

pub mod api;
pub mod native;

use async_trait::async_trait;

use crate::error::Result;
use crate::security::ValidatedMedia;

/// Analysis result from a multimodal provider.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MediaAnalysis {
    /// Natural-language description of the media.
    pub description: String,
    /// Detected objects, entities, or transcription segments.
    pub entities: Vec<String>,
    /// Estimated token cost for context injection.
    pub token_estimate: u32,
    /// Provider name that produced this analysis.
    pub provider_name: String,
    /// Whether this result came from local inference (not API).
    pub is_local: bool,
    /// Modality: "image", "audio", or "video".
    pub modality: String,
}

/// Trait for multimodal inference providers.
#[async_trait]
pub trait MultimodalProvider: Send + Sync {
    /// Provider identifier (e.g., "anthropic-vision", "onnx-clip").
    fn name(&self) -> &str;
    /// True if this provider can analyze the given modality.
    fn supports_modality(&self, modality: &str) -> bool;
    /// Analyze validated media with an optional natural-language prompt.
    async fn analyze(&self, media: &ValidatedMedia, prompt: Option<&str>) -> Result<MediaAnalysis>;
}

// ── Test-only mock provider ───────────────────────────────────────────────────

/// Test-only mock provider that returns fixed `MediaAnalysis` without any API calls.
///
/// Use this in unit tests to avoid requiring real API keys.
#[cfg(test)]
pub struct MockMultimodalProvider;

#[cfg(test)]
#[async_trait]
impl MultimodalProvider for MockMultimodalProvider {
    fn name(&self) -> &str { "mock" }

    fn supports_modality(&self, _modality: &str) -> bool { true }

    async fn analyze(&self, media: &ValidatedMedia, _prompt: Option<&str>) -> Result<MediaAnalysis> {
        let modality = if media.is_image()      { "image" }
                       else if media.is_audio() { "audio" }
                       else                     { "video" };
        Ok(MediaAnalysis {
            description:   format!("Mock analysis: {} content detected", modality),
            entities:      vec!["mock-entity".into()],
            token_estimate: 10,
            provider_name: "mock".into(),
            is_local:      true,
            modality:      modality.into(),
        })
    }
}
