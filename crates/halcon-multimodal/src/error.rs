//! Error types for the multimodal subsystem.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MultimodalError {
    // Security errors (P0)
    #[error("file too large: {size} bytes exceeds limit {limit}")]
    FileTooLarge { size: u64, limit: u64 },

    #[error("unsupported MIME type: {0}")]
    UnsupportedMimeType(String),

    #[error("decompression bomb detected: {width}x{height} exceeds safe dimensions")]
    DecompressionBomb { width: u32, height: u32 },

    #[error("local path not permitted in privacy-strict mode: {0}")]
    PrivacyViolation(String),

    #[error("EXIF strip failed: {0}")]
    ExifStripFailed(String),

    // Media processing errors
    #[error("FFmpeg error: {0}")]
    FfmpegError(String),

    #[error("audio duration {duration_secs}s exceeds limit {limit_secs}s")]
    AudioTooLong { duration_secs: f32, limit_secs: u32 },

    #[error("video duration {duration_secs}s exceeds limit {limit_secs}s")]
    VideoTooLong { duration_secs: f32, limit_secs: u32 },

    // Provider / routing errors
    #[error("no capable provider for modality: {0}")]
    NoCapableProvider(String),

    #[error("API provider error: {0}")]
    ApiError(String),

    #[error("local inference error: {0}")]
    LocalInferenceError(String),

    #[error("native ONNX model not available: {0}")]
    NativeModelNotAvailable(String),

    #[error("provider timeout after {ms}ms")]
    Timeout { ms: u64 },

    // Storage errors
    #[error("cache error: {0}")]
    CacheError(String),

    #[error("index error: {0}")]
    IndexError(String),

    // IO errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    // Serialization errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    // Internal errors
    #[error("internal error: {0}")]
    Internal(String),

    // Worker pool errors
    #[error("worker pool error: {0}")]
    WorkerError(String),
}

pub type Result<T> = std::result::Result<T, MultimodalError>;

impl From<halcon_core::error::HalconError> for MultimodalError {
    fn from(e: halcon_core::error::HalconError) -> Self {
        MultimodalError::Internal(e.to_string())
    }
}
