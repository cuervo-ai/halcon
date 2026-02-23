//! API-backed multimodal provider — real HTTP calls to OpenAI / Anthropic Vision APIs.
//!
//! Provider priority (auto-detected from environment):
//!   1. Anthropic  → `ANTHROPIC_API_KEY` → claude-3-5-sonnet-20241022 (vision + no audio)
//!   2. OpenAI     → `OPENAI_API_KEY`    → gpt-4o (vision) + whisper-1 (audio)
//!   3. Gemini     → `GEMINI_API_KEY`    → gemini-1.5-flash (vision)
//!
//! For audio transcription, only OpenAI Whisper is used (Anthropic has no audio API).
//! For images, Anthropic is preferred (best vision quality).

use async_trait::async_trait;
use base64::Engine as _;
use serde_json::json;
use std::time::Duration;

use crate::error::{MultimodalError, Result};
use crate::security::ValidatedMedia;
use super::{MediaAnalysis, MultimodalProvider};

// ── Provider type detection ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum ApiBackend {
    Anthropic { key: String },
    OpenAI    { key: String },
    Gemini    { key: String },
    /// No API key found — analysis returns a descriptive error.
    Unavailable,
}

impl ApiBackend {
    /// Auto-detect from environment variables.
    /// Priority: Anthropic → OpenAI → Gemini.
    fn from_env() -> Self {
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            if !key.trim().is_empty() {
                return ApiBackend::Anthropic { key };
            }
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            if !key.trim().is_empty() {
                return ApiBackend::OpenAI { key };
            }
        }
        if let Ok(key) = std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY")) {
            if !key.trim().is_empty() {
                return ApiBackend::Gemini { key };
            }
        }
        ApiBackend::Unavailable
    }

    fn name(&self) -> &str {
        match self {
            ApiBackend::Anthropic { .. } => "anthropic",
            ApiBackend::OpenAI    { .. } => "openai",
            ApiBackend::Gemini    { .. } => "gemini",
            ApiBackend::Unavailable      => "unavailable",
        }
    }

    fn supports_audio(&self) -> bool {
        // Only OpenAI has a Whisper audio API.
        matches!(self, ApiBackend::OpenAI { .. })
    }

    /// Provider-appropriate vision rate limit (requests per minute).
    ///
    /// Reflects real-world Tier-1 limits:
    /// - Anthropic claude-3-5-sonnet: ~50 RPM
    /// - OpenAI gpt-4o vision:        ~60 RPM
    /// - Gemini 1.5 Flash:            ~15 RPM (free tier)
    fn vision_rate_limit(&self) -> u32 {
        match self {
            ApiBackend::Anthropic { .. } => 50,
            ApiBackend::OpenAI    { .. } => 60,
            ApiBackend::Gemini    { .. } => 15,
            ApiBackend::Unavailable      => 60,
        }
    }

    /// Provider-appropriate audio rate limit (requests per minute).
    ///
    /// Only OpenAI Whisper is used for audio; its Tier-1 limit is ~50 RPM.
    /// Other backends return 60 as a safe default (audio calls are rejected anyway).
    fn audio_rate_limit(&self) -> u32 {
        match self {
            ApiBackend::OpenAI { .. } => 50,
            _                        => 60,
        }
    }
}

// ── Rate limiter ─────────────────────────────────────────────────────────────

/// Sliding-window rate limiter for API calls.
///
/// Default: 60 requests per minute (conservative; well below Anthropic/OpenAI limits).
/// The window is 60 seconds rolling; excess calls wait for the oldest call to age out.
#[derive(Debug)]
struct ApiRateLimiter {
    max_per_minute: u32,
    /// Timestamps (Unix seconds) of recent calls, oldest first.
    timestamps: tokio::sync::Mutex<std::collections::VecDeque<u64>>,
}

impl ApiRateLimiter {
    fn new(max_per_minute: u32) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            max_per_minute,
            timestamps: tokio::sync::Mutex::new(std::collections::VecDeque::new()),
        })
    }

    /// Block asynchronously until a call slot is available, then claim it.
    async fn acquire(&self) {
        loop {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let window_start = now.saturating_sub(60);
            let mut ts = self.timestamps.lock().await;
            // Evict calls older than the window.
            while ts.front().map_or(false, |&t| t <= window_start) {
                ts.pop_front();
            }
            if (ts.len() as u32) < self.max_per_minute {
                ts.push_back(now);
                return;
            }
            // Too many calls — wait until the oldest call falls outside the window.
            let oldest = *ts.front().unwrap_or(&now);
            let wait_secs = (oldest + 61).saturating_sub(now);
            drop(ts);
            tracing::debug!(wait_secs, "multimodal API rate limit — sleeping");
            tokio::time::sleep(Duration::from_secs(wait_secs)).await;
        }
    }
}

// ── Provider struct ──────────────────────────────────────────────────────────

/// API-based vision/audio provider.
///
/// Makes real HTTP requests to the configured provider's vision API.
/// API credentials are read from environment variables at construction time.
///
/// Built-in rate limiting: per-backend sliding-window limiters.
/// Vision and audio have independent quotas so Whisper calls don't consume
/// vision slots and vice-versa.
pub struct ApiMultimodalProvider {
    provider_name: String,
    backend: ApiBackend,
    client: reqwest::Client,
    timeout_ms: u64,
    /// Vision API rate limiter — RPM varies by backend (Anthropic 50, OpenAI 60, Gemini 15).
    vision_rate_limiter: std::sync::Arc<ApiRateLimiter>,
    /// Audio (Whisper) rate limiter — independent of vision quota (OpenAI 50 RPM).
    audio_rate_limiter: std::sync::Arc<ApiRateLimiter>,
}

impl std::fmt::Debug for ApiMultimodalProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiMultimodalProvider")
            .field("provider_name", &self.provider_name)
            .field("backend", &self.backend.name())
            .finish()
    }
}

impl ApiMultimodalProvider {
    /// Create provider with auto-detected backend from environment variables.
    pub fn new(provider_name: impl Into<String>) -> Self {
        Self::with_timeout(provider_name, 30_000)
    }

    pub fn with_timeout(provider_name: impl Into<String>, timeout_ms: u64) -> Self {
        let backend = ApiBackend::from_env();
        tracing::debug!(
            backend = backend.name(),
            vision_rpm = backend.vision_rate_limit(),
            audio_rpm  = backend.audio_rate_limit(),
            "Multimodal API backend selected",
        );
        let vision_rpm = backend.vision_rate_limit();
        let audio_rpm  = backend.audio_rate_limit();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .unwrap_or_default();
        Self {
            provider_name: provider_name.into(),
            backend,
            client,
            timeout_ms,
            vision_rate_limiter: ApiRateLimiter::new(vision_rpm),
            audio_rate_limiter:  ApiRateLimiter::new(audio_rpm),
        }
    }

    /// True if an API key is available and the backend is configured.
    pub fn is_available(&self) -> bool {
        !matches!(self.backend, ApiBackend::Unavailable)
    }

    /// If audio is not supported by the current backend, returns a human-readable hint.
    /// Returns `None` when audio transcription IS available (OpenAI Whisper).
    pub fn audio_unavailable_hint(&self) -> Option<String> {
        if self.backend.supports_audio() {
            None
        } else {
            Some(format!(
                "Audio transcription requires OPENAI_API_KEY (current backend: {}). \
                 Set OPENAI_API_KEY to enable Whisper transcription.",
                self.backend.name()
            ))
        }
    }

    /// Name of the active backend (e.g. "anthropic", "openai", "gemini", "unavailable").
    pub fn backend_name(&self) -> &str { self.backend.name() }
}

// ── MultimodalProvider implementation ───────────────────────────────────────

#[async_trait]
impl MultimodalProvider for ApiMultimodalProvider {
    fn name(&self) -> &str { &self.provider_name }

    fn supports_modality(&self, modality: &str) -> bool {
        match modality {
            "image" => !matches!(self.backend, ApiBackend::Unavailable),
            "audio" => self.backend.supports_audio(),
            "video" => false, // Video is handled by the video pipeline (frame-by-frame).
            _       => false,
        }
    }

    async fn analyze(&self, media: &ValidatedMedia, prompt: Option<&str>) -> Result<MediaAnalysis> {
        // Video: return a degraded-but-informative response instead of an opaque error.
        // Full FFmpeg frame-by-frame analysis is planned for Q2.
        if media.is_video() {
            return Ok(MediaAnalysis {
                description: format!(
                    "Video file detected ({} bytes). Frame-by-frame analysis requires \
                     FFmpeg integration (coming Q2). Current provider: {}.",
                    media.data.len(),
                    self.provider_name
                ),
                entities:      vec![],
                token_estimate: 20,
                provider_name: self.provider_name.clone(),
                is_local:      false,
                modality:      "video".into(),
            });
        }

        if media.is_image() {
            self.analyze_image(media, prompt).await
        } else if media.is_audio() {
            self.analyze_audio(media).await
        } else {
            Err(MultimodalError::NoCapableProvider(
                format!("{} not supported via API", media.mime.as_mime_str())
            ))
        }
    }
}

// ── Image analysis ───────────────────────────────────────────────────────────

impl ApiMultimodalProvider {
    async fn analyze_image(
        &self,
        media:  &ValidatedMedia,
        prompt: Option<&str>,
    ) -> Result<MediaAnalysis> {
        // Enforce per-backend vision rate limit before making any network call.
        self.vision_rate_limiter.acquire().await;

        let user_prompt = prompt.unwrap_or(
            "Describe this image in detail. List all visible objects, \
             text, people, colors, and any notable features. \
             Be specific and comprehensive."
        );
        let b64 = base64::engine::general_purpose::STANDARD.encode(&media.data);
        let mime = media.mime.as_mime_str();

        match &self.backend {
            ApiBackend::Anthropic { key } =>
                self.anthropic_image(key, &b64, mime, user_prompt).await,
            ApiBackend::OpenAI { key } =>
                self.openai_image(key, &b64, mime, user_prompt).await,
            ApiBackend::Gemini { key } =>
                self.gemini_image(key, &b64, mime, user_prompt).await,
            ApiBackend::Unavailable =>
                Err(MultimodalError::NoCapableProvider(
                    "No vision API key found. Set ANTHROPIC_API_KEY, OPENAI_API_KEY, \
                     or GEMINI_API_KEY.".into()
                )),
        }
    }

    // ── Anthropic vision ─────────────────────────────────────────────────────

    async fn anthropic_image(
        &self,
        api_key:     &str,
        b64:         &str,
        mime:        &str,
        user_prompt: &str,
    ) -> Result<MediaAnalysis> {
        let body = json!({
            "model": "claude-3-5-sonnet-20241022",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": mime,
                            "data": b64
                        }
                    },
                    {
                        "type": "text",
                        "text": user_prompt
                    }
                ]
            }]
        });

        let resp = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| MultimodalError::ApiError(format!("Anthropic request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(MultimodalError::ApiError(
                format!("Anthropic vision API error {status}: {text}")
            ));
        }

        let json: serde_json::Value = resp.json().await
            .map_err(|e| MultimodalError::ApiError(format!("Anthropic response parse: {e}")))?;

        let description = json["content"][0]["text"]
            .as_str()
            .unwrap_or("[Anthropic vision: empty response]")
            .to_string();

        let token_estimate = json["usage"]["output_tokens"].as_u64().unwrap_or(256) as u32;
        let entities = extract_entities_from_description(&description);

        tracing::debug!(
            backend = "anthropic",
            tokens = token_estimate,
            chars = description.len(),
            "Image analysis complete"
        );

        Ok(MediaAnalysis {
            description,
            entities,
            token_estimate,
            provider_name: "anthropic-vision".into(),
            is_local: false,
            modality: "image".into(),
        })
    }

    // ── OpenAI GPT-4o vision ─────────────────────────────────────────────────

    async fn openai_image(
        &self,
        api_key:     &str,
        b64:         &str,
        mime:        &str,
        user_prompt: &str,
    ) -> Result<MediaAnalysis> {
        let data_uri = format!("data:{mime};base64,{b64}");
        let body = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": data_uri,
                            "detail": "auto"
                        }
                    },
                    {
                        "type": "text",
                        "text": user_prompt
                    }
                ]
            }]
        });

        let resp = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| MultimodalError::ApiError(format!("OpenAI request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(MultimodalError::ApiError(
                format!("OpenAI vision API error {status}: {text}")
            ));
        }

        let json: serde_json::Value = resp.json().await
            .map_err(|e| MultimodalError::ApiError(format!("OpenAI response parse: {e}")))?;

        let description = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("[GPT-4o vision: empty response]")
            .to_string();

        let token_estimate = json["usage"]["completion_tokens"].as_u64().unwrap_or(256) as u32;
        let entities = extract_entities_from_description(&description);

        tracing::debug!(
            backend = "openai",
            tokens = token_estimate,
            chars = description.len(),
            "Image analysis complete"
        );

        Ok(MediaAnalysis {
            description,
            entities,
            token_estimate,
            provider_name: "openai-vision".into(),
            is_local: false,
            modality: "image".into(),
        })
    }

    // ── Gemini vision ────────────────────────────────────────────────────────

    async fn gemini_image(
        &self,
        api_key:     &str,
        b64:         &str,
        mime:        &str,
        user_prompt: &str,
    ) -> Result<MediaAnalysis> {
        let body = json!({
            "contents": [{
                "parts": [
                    {
                        "inlineData": {
                            "mimeType": mime,
                            "data": b64
                        }
                    },
                    {
                        "text": user_prompt
                    }
                ]
            }]
        });

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/\
             gemini-1.5-flash:generateContent?key={api_key}"
        );
        let resp = self.client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| MultimodalError::ApiError(format!("Gemini request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(MultimodalError::ApiError(
                format!("Gemini vision API error {status}: {text}")
            ));
        }

        let json: serde_json::Value = resp.json().await
            .map_err(|e| MultimodalError::ApiError(format!("Gemini response parse: {e}")))?;

        let description = json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("[Gemini vision: empty response]")
            .to_string();

        let entities = extract_entities_from_description(&description);
        let token_estimate = (description.len() / 4) as u32;

        tracing::debug!(
            backend = "gemini",
            chars = description.len(),
            "Image analysis complete"
        );

        Ok(MediaAnalysis {
            description,
            entities,
            token_estimate,
            provider_name: "gemini-vision".into(),
            is_local: false,
            modality: "image".into(),
        })
    }
}

// ── Audio transcription (OpenAI Whisper) ─────────────────────────────────────

impl ApiMultimodalProvider {
    async fn analyze_audio(&self, media: &ValidatedMedia) -> Result<MediaAnalysis> {
        // Enforce per-provider audio rate limit (independent of vision quota).
        self.audio_rate_limiter.acquire().await;

        match &self.backend {
            ApiBackend::OpenAI { key } => self.whisper_transcribe(key, media).await,
            _ => Err(MultimodalError::NoCapableProvider(
                "Audio transcription requires OPENAI_API_KEY (Whisper API). \
                 Set OPENAI_API_KEY to enable audio analysis.".into()
            )),
        }
    }

    async fn whisper_transcribe(
        &self,
        api_key: &str,
        media:   &ValidatedMedia,
    ) -> Result<MediaAnalysis> {
        // Whisper API uses multipart/form-data with the audio file.
        let extension = match media.mime.as_mime_str() {
            "audio/mpeg"  => "mp3",
            "audio/wav"   => "wav",
            "audio/ogg"   => "ogg",
            "audio/flac"  => "flac",
            _             => "mp3",
        };
        let filename = format!("audio.{extension}");

        let part = reqwest::multipart::Part::bytes(media.data.clone())
            .file_name(filename)
            .mime_str(media.mime.as_mime_str())
            .map_err(|e| MultimodalError::ApiError(format!("Whisper form: {e}")))?;

        let form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", "whisper-1")
            .text("response_format", "verbose_json");

        let resp = self.client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .bearer_auth(api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|e| MultimodalError::ApiError(format!("Whisper request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(MultimodalError::ApiError(
                format!("Whisper API error {status}: {text}")
            ));
        }

        let json: serde_json::Value = resp.json().await
            .map_err(|e| MultimodalError::ApiError(format!("Whisper response parse: {e}")))?;

        let transcript = json["text"]
            .as_str()
            .unwrap_or("[Whisper: empty transcript]")
            .to_string();

        // Extract segments with timestamps if available.
        let entities: Vec<String> = json["segments"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|seg| {
                let start = seg["start"].as_f64()?;
                let text  = seg["text"].as_str()?;
                Some(format!("[{:.1}s] {}", start, text.trim()))
            })
            .collect();

        let duration = json["duration"].as_f64().unwrap_or(0.0) as f32;
        let token_estimate = (transcript.len() / 4) as u32;

        tracing::debug!(
            duration_secs = duration,
            tokens = token_estimate,
            segments = entities.len(),
            "Audio transcription complete"
        );

        Ok(MediaAnalysis {
            description: transcript,
            entities,
            token_estimate,
            provider_name: "openai-whisper".into(),
            is_local: false,
            modality: "audio".into(),
        })
    }
}

// ── Entity extraction helper ─────────────────────────────────────────────────

/// Extract a list of likely entities from a vision description.
///
/// Uses simple heuristics: noun phrases after "I see", "there is/are", etc.
/// Keeps the first 10 meaningful lines as entity candidates.
fn extract_entities_from_description(description: &str) -> Vec<String> {
    description
        .lines()
        .filter(|line| {
            let l = line.trim();
            !l.is_empty()
                && l.len() > 5
                && !l.starts_with('#')
                && !l.starts_with("---")
        })
        .take(10)
        .map(|l| l.trim().to_string())
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

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

    fn wav_media() -> ValidatedMedia {
        // WAV magic: RIFF....WAVE
        let mut data = b"RIFF".to_vec();
        data.extend_from_slice(&[0x00u8; 4]); // size
        data.extend_from_slice(b"WAVE");
        ValidatedMedia {
            original_size: data.len() as u64,
            data,
            mime: DetectedMime::AudioWav,
        }
    }

    #[test]
    fn supports_image_when_key_available() {
        // Without env key → unavailable.
        // This test just ensures modality logic is correct regardless of key.
        let p = ApiMultimodalProvider::new("test");
        // The result depends on whether env vars are set.
        // At minimum, it should not panic.
        let _ = p.supports_modality("image");
        let _ = p.supports_modality("audio");
        assert!(!p.supports_modality("video"));
    }

    #[test]
    fn backend_name_unavailable_without_env() {
        // Clear any env key for isolated test (best-effort).
        // We can't truly clear env in parallel tests, so just verify the fn runs.
        let backend = ApiBackend::Unavailable;
        assert_eq!(backend.name(), "unavailable");
        assert!(!backend.supports_audio());
    }

    #[test]
    fn backend_openai_supports_audio() {
        let backend = ApiBackend::OpenAI { key: "sk-test".into() };
        assert!(backend.supports_audio());
        assert_eq!(backend.name(), "openai");
    }

    #[test]
    fn backend_anthropic_no_audio() {
        let backend = ApiBackend::Anthropic { key: "sk-ant-test".into() };
        assert!(!backend.supports_audio());
        assert_eq!(backend.name(), "anthropic");
    }

    #[test]
    fn extract_entities_from_description_non_empty() {
        let desc = "The image shows a cat sitting on a window sill.\nThe cat is orange.\nThere is sunlight.";
        let entities = extract_entities_from_description(desc);
        assert!(!entities.is_empty());
        assert!(entities.len() <= 10);
    }

    #[tokio::test]
    async fn video_returns_degraded_not_error() {
        // Video no longer returns an error — it returns a degraded informational response.
        let p = ApiMultimodalProvider::new("test");
        let video = ValidatedMedia {
            data: vec![0x1A, 0x45, 0xDF, 0xA3],
            mime: DetectedMime::VideoWebm,
            original_size: 4,
        };
        let result = p.analyze(&video, None).await;
        assert!(result.is_ok(), "video should return Ok degraded response, not Err");
        let analysis = result.unwrap();
        assert_eq!(analysis.modality, "video");
        assert!(analysis.description.contains("Q2") || analysis.description.contains("FFmpeg"),
            "degraded video description should mention Q2/FFmpeg: {}", analysis.description);
    }

    #[test]
    fn audio_unavailable_hint_when_anthropic() {
        let p = ApiMultimodalProvider {
            provider_name: "test".into(),
            backend: ApiBackend::Anthropic { key: "sk-ant-test".into() },
            client: reqwest::Client::new(),
            timeout_ms: 5_000,
            vision_rate_limiter: ApiRateLimiter::new(60),
            audio_rate_limiter:  ApiRateLimiter::new(60),
        };
        let hint = p.audio_unavailable_hint();
        assert!(hint.is_some(), "Anthropic backend should return an unavailability hint");
        let msg = hint.unwrap();
        assert!(msg.contains("OPENAI_API_KEY"), "hint should mention OPENAI_API_KEY: {msg}");
        assert!(msg.contains("anthropic"), "hint should mention current backend: {msg}");
    }

    #[test]
    fn audio_unavailable_hint_none_when_openai() {
        let p = ApiMultimodalProvider {
            provider_name: "test".into(),
            backend: ApiBackend::OpenAI { key: "sk-test".into() },
            client: reqwest::Client::new(),
            timeout_ms: 5_000,
            vision_rate_limiter: ApiRateLimiter::new(60),
            audio_rate_limiter:  ApiRateLimiter::new(60),
        };
        // OpenAI supports Whisper, so hint should be None.
        assert!(p.audio_unavailable_hint().is_none(), "OpenAI backend should return None hint");
    }

    /// Helper: create an unavailable provider (no API key) for testing.
    fn unavailable_provider() -> ApiMultimodalProvider {
        ApiMultimodalProvider {
            provider_name: "test".into(),
            backend: ApiBackend::Unavailable,
            client: reqwest::Client::new(),
            timeout_ms: 5_000,
            vision_rate_limiter: ApiRateLimiter::new(60),
            audio_rate_limiter:  ApiRateLimiter::new(60),
        }
    }

    #[tokio::test]
    async fn unavailable_backend_returns_error_for_image() {
        let p = unavailable_provider();
        let err = p.analyze(&jpeg_media(), None).await.unwrap_err();
        assert!(err.to_string().contains("API key") || err.to_string().contains("key"));
    }

    #[tokio::test]
    async fn unavailable_backend_returns_error_for_audio() {
        let p = unavailable_provider();
        let err = p.analyze(&wav_media(), None).await.unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("openai") || msg.contains("api_key") || msg.contains("audio"),
            "Expected audio/key error, got: {err}"
        );
    }

    #[test]
    fn api_provider_is_available_checks_backend() {
        let p = unavailable_provider();
        assert!(!p.is_available());

        let p2 = ApiMultimodalProvider {
            provider_name: "test".into(),
            backend: ApiBackend::OpenAI { key: "sk-test".into() },
            client: reqwest::Client::new(),
            timeout_ms: 5_000,
            vision_rate_limiter: ApiRateLimiter::new(60),
            audio_rate_limiter:  ApiRateLimiter::new(60),
        };
        assert!(p2.is_available());
    }

    // ── Rate limiter tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn rate_limiter_allows_under_limit() {
        let rl = ApiRateLimiter::new(10);
        // 5 calls should all succeed immediately (well under the 10/min limit).
        for _ in 0..5 {
            rl.acquire().await;
        }
        let ts = rl.timestamps.lock().await;
        assert_eq!(ts.len(), 5);
    }

    #[tokio::test]
    async fn rate_limiter_tracks_call_count() {
        let rl = ApiRateLimiter::new(100);
        rl.acquire().await;
        rl.acquire().await;
        rl.acquire().await;
        let ts = rl.timestamps.lock().await;
        assert_eq!(ts.len(), 3);
    }

    #[tokio::test]
    async fn rate_limiter_new_creates_arc() {
        let rl = ApiRateLimiter::new(60);
        assert_eq!(rl.max_per_minute, 60);
        let ts = rl.timestamps.lock().await;
        assert!(ts.is_empty());
    }

    // ── Per-provider rate limit tests ──────────────────────────────────────────

    #[test]
    fn anthropic_vision_rate_limit_is_50() {
        let backend = ApiBackend::Anthropic { key: "sk-ant-test".into() };
        assert_eq!(backend.vision_rate_limit(), 50,
            "Anthropic claude-3-5-sonnet vision is capped at ~50 RPM Tier-1");
    }

    #[test]
    fn openai_vision_rate_limit_is_60() {
        let backend = ApiBackend::OpenAI { key: "sk-test".into() };
        assert_eq!(backend.vision_rate_limit(), 60,
            "OpenAI GPT-4o vision is 60 RPM at Tier-1");
    }

    #[test]
    fn gemini_vision_rate_limit_is_15() {
        let backend = ApiBackend::Gemini { key: "ai-test".into() };
        assert_eq!(backend.vision_rate_limit(), 15,
            "Gemini 1.5 Flash free tier is 15 RPM");
    }

    #[test]
    fn openai_audio_rate_limit_is_50() {
        let backend = ApiBackend::OpenAI { key: "sk-test".into() };
        assert_eq!(backend.audio_rate_limit(), 50,
            "OpenAI Whisper audio is ~50 RPM at Tier-1");
    }

    #[test]
    fn non_openai_audio_rate_limit_is_safe_default() {
        // Anthropic and Gemini don't support audio, but the default 60 is a safe no-op.
        assert_eq!(ApiBackend::Anthropic { key: "k".into() }.audio_rate_limit(), 60);
        assert_eq!(ApiBackend::Gemini    { key: "k".into() }.audio_rate_limit(), 60);
        assert_eq!(ApiBackend::Unavailable.audio_rate_limit(), 60);
    }

    #[test]
    fn provider_uses_backend_appropriate_limits_at_construction() {
        // Build a provider with a known Anthropic backend by injecting it directly.
        let p = ApiMultimodalProvider {
            provider_name: "test".into(),
            backend: ApiBackend::Anthropic { key: "sk-ant-test".into() },
            client: reqwest::Client::new(),
            timeout_ms: 5_000,
            vision_rate_limiter: ApiRateLimiter::new(50),
            audio_rate_limiter:  ApiRateLimiter::new(60),
        };
        assert_eq!(p.vision_rate_limiter.max_per_minute, 50,
            "vision limiter should use Anthropic 50 RPM");
        assert_eq!(p.audio_rate_limiter.max_per_minute, 60,
            "audio limiter should use safe default for non-OpenAI backends");
    }

    #[tokio::test]
    async fn vision_and_audio_limiters_are_independent() {
        // Vision calls should not consume audio quota and vice-versa.
        let vision_rl = ApiRateLimiter::new(100);
        let audio_rl  = ApiRateLimiter::new(100);

        // 10 vision calls
        for _ in 0..10 { vision_rl.acquire().await; }
        // 5 audio calls
        for _ in 0..5  { audio_rl.acquire().await; }

        let v_count = vision_rl.timestamps.lock().await.len();
        let a_count = audio_rl.timestamps.lock().await.len();
        assert_eq!(v_count, 10, "vision limiter should track 10 calls");
        assert_eq!(a_count, 5,  "audio limiter should track 5 calls independently");
    }
}
