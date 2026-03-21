//! SOTA Q2 multimodal stubs.
//!
//! Reserves namespace for next-quarter capabilities.
//! All functions are no-ops until the corresponding models/infrastructure land.

/// Florence-2 zero-shot spatial grounding (Q2).
pub mod florence2 {
    /// Spatial grounding bounding box.
    #[derive(Debug, Clone)]
    pub struct BoundingBox {
        pub x: f32,
        pub y: f32,
        pub w: f32,
        pub h: f32,
        pub label: String,
    }

    /// STUB: activated when Florence-2 ONNX weights are available.
    pub async fn ground(_image_bytes: &[u8], _prompt: &str) -> Vec<BoundingBox> {
        vec![]
    }
}

/// Moshi real-time audio dialogue (Q2).
pub mod moshi {
    /// STUB: will stream audio tokens via WebSocket.
    pub async fn transcribe_streaming(_audio_chunk: &[u8]) -> Option<String> {
        None
    }
}

/// VideoRAG temporal retrieval (Q2).
pub mod videorag {
    /// A temporal clip with start/end timestamps.
    #[derive(Debug, Clone)]
    pub struct VideoClip {
        pub start_secs: f32,
        pub end_secs: f32,
        pub description: String,
    }

    /// STUB: will extract and index video frames at 1 FPS.
    pub async fn extract_clips(_video_bytes: &[u8]) -> Vec<VideoClip> {
        vec![]
    }
}

/// Streaming progressive analysis (Q2).
pub mod streaming {
    /// STUB: progressive chunk-by-chunk analysis for large files.
    pub async fn analyze_stream() -> &'static str {
        "[streaming analysis not yet implemented]"
    }
}

/// ImageBind cross-modal alignment (Q2).
pub mod imagebind {
    /// Cross-modal embedding (image ↔ audio ↔ text).
    #[derive(Debug, Clone)]
    pub struct CrossModalEmbedding {
        pub embedding: Vec<f32>,
        pub modality: String,
    }

    /// STUB: will align embeddings across modalities using ImageBind.
    pub async fn embed(_data: &[u8], _modality: &str) -> Option<CrossModalEmbedding> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stubs_return_empty() {
        assert!(florence2::ground(b"", "anything").await.is_empty());
        assert!(moshi::transcribe_streaming(b"").await.is_none());
        assert!(videorag::extract_clips(b"").await.is_empty());
        assert!(imagebind::embed(b"", "image").await.is_none());
    }
}
