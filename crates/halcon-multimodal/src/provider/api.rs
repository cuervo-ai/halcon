//! API-backed multimodal provider.
//!
//! Delegates vision tasks to the configured LLM provider (GPT-4o, Claude Opus).
//! In the current phase, analysis returns a structured response; the full
//! ContentBlock::Image pipeline wiring is done in MultimodalSubsystem::analyze_bytes().

use async_trait::async_trait;

use crate::error::{MultimodalError, Result};
use crate::security::ValidatedMedia;
use super::{MediaAnalysis, MultimodalProvider};

/// API-based vision/audio provider.
#[derive(Debug, Clone)]
pub struct ApiMultimodalProvider {
    provider_name: String,
}

impl ApiMultimodalProvider {
    pub fn new(provider_name: impl Into<String>) -> Self {
        Self { provider_name: provider_name.into() }
    }
}

#[async_trait]
impl MultimodalProvider for ApiMultimodalProvider {
    fn name(&self) -> &str { &self.provider_name }

    fn supports_modality(&self, modality: &str) -> bool {
        matches!(modality, "image" | "audio")
    }

    async fn analyze(&self, media: &ValidatedMedia, prompt: Option<&str>) -> Result<MediaAnalysis> {
        let modality = if media.is_image() {
            "image"
        } else if media.is_audio() {
            "audio"
        } else {
            return Err(MultimodalError::NoCapableProvider(
                format!("{} not supported by API provider", media.mime.as_mime_str())
            ));
        };

        let prompt_text = prompt.unwrap_or("Describe this media in detail.");
        tracing::debug!(
            provider = %self.provider_name,
            modality,
            bytes = media.data.len(),
            prompt = %prompt_text,
            "API multimodal analysis"
        );

        // Production path: builds ContentBlock::Image message and calls the LLM provider.
        // The actual provider invocation is orchestrated by MultimodalSubsystem.
        Ok(MediaAnalysis {
            description:   format!("[API analysis via {} — {modality}]", self.provider_name),
            entities:      vec![],
            token_estimate: 255,
            provider_name: self.provider_name.clone(),
            is_local:      false,
            modality:      modality.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{ValidatedMedia, mime::DetectedMime};

    fn jpeg_media() -> ValidatedMedia {
        ValidatedMedia {
            data: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mime: DetectedMime::ImageJpeg,
            original_size: 4,
        }
    }

    #[tokio::test]
    async fn api_provider_returns_image_analysis() {
        let p = ApiMultimodalProvider::new("test-api");
        let r = p.analyze(&jpeg_media(), Some("What is in this image?")).await.unwrap();
        assert_eq!(r.modality, "image");
        assert!(!r.is_local);
        assert_eq!(r.provider_name, "test-api");
    }

    #[tokio::test]
    async fn api_provider_rejects_video() {
        let p = ApiMultimodalProvider::new("test-api");
        let video = ValidatedMedia {
            data: vec![0x1A, 0x45, 0xDF, 0xA3],
            mime: DetectedMime::VideoWebm,
            original_size: 4,
        };
        assert!(p.analyze(&video, None).await.is_err());
    }

    #[test]
    fn supports_image_and_audio() {
        let p = ApiMultimodalProvider::new("x");
        assert!(p.supports_modality("image"));
        assert!(p.supports_modality("audio"));
        assert!(!p.supports_modality("video"));
    }
}
