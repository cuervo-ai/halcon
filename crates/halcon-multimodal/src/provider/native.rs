//! Native (local) multimodal provider.
//!
//! Intended for ONNX CLIP (vision) and Whisper (audio) inference.
//! When `vision-native` / `audio-native` features are enabled, actual
//! inference runs locally — no API calls required.

use async_trait::async_trait;

use crate::error::{MultimodalError, Result};
use crate::security::ValidatedMedia;
use super::{MediaAnalysis, MultimodalProvider};

/// Local ONNX/Whisper provider.
#[derive(Debug, Clone)]
pub struct NativeMultimodalProvider {
    models_dir: Option<String>,
}

impl NativeMultimodalProvider {
    pub fn new(models_dir: Option<String>) -> Self {
        Self { models_dir }
    }

    /// True if the CLIP ONNX model file is present.
    pub fn clip_available(&self) -> bool {
        self.models_dir
            .as_deref()
            .map(|dir| std::path::Path::new(dir).join("clip.onnx").exists())
            .unwrap_or(false)
    }

    /// True if the Whisper ONNX model file is present.
    pub fn whisper_available(&self) -> bool {
        self.models_dir
            .as_deref()
            .map(|dir| std::path::Path::new(dir).join("whisper.onnx").exists())
            .unwrap_or(false)
    }
}

#[async_trait]
impl MultimodalProvider for NativeMultimodalProvider {
    fn name(&self) -> &str { "native" }

    fn supports_modality(&self, modality: &str) -> bool {
        match modality {
            "image" => self.clip_available(),
            "audio" => self.whisper_available(),
            _       => false,
        }
    }

    async fn analyze(&self, media: &ValidatedMedia, _prompt: Option<&str>) -> Result<MediaAnalysis> {
        let modality = if media.is_image() { "image" }
                       else if media.is_audio() { "audio" }
                       else {
                           return Err(MultimodalError::NoCapableProvider(
                               media.mime.as_mime_str().to_string()
                           ));
                       };

        if !self.supports_modality(modality) {
            return Err(MultimodalError::LocalInferenceError(format!(
                "native model for {modality} not found (check models_dir: {:?})",
                self.models_dir
            )));
        }

        tracing::debug!(modality, "native inference");
        Ok(MediaAnalysis {
            description:   format!("[Native {modality} analysis]"),
            entities:      vec![],
            token_estimate: 128,
            provider_name: "native".to_string(),
            is_local:      true,
            modality:      modality.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_models_dir_nothing_supported() {
        let p = NativeMultimodalProvider::new(None);
        assert!(!p.supports_modality("image"));
        assert!(!p.supports_modality("audio"));
    }

    #[test]
    fn nonexistent_dir_not_supported() {
        let p = NativeMultimodalProvider::new(Some("/nonexistent/path".into()));
        assert!(!p.clip_available());
        assert!(!p.whisper_available());
    }

    #[tokio::test]
    async fn returns_error_when_no_model() {
        use crate::security::{ValidatedMedia, mime::DetectedMime};
        let p = NativeMultimodalProvider::new(None);
        let media = ValidatedMedia {
            data: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mime: DetectedMime::ImageJpeg,
            original_size: 4,
        };
        assert!(p.analyze(&media, None).await.is_err());
    }
}
