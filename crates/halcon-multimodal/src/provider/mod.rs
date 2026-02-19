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
