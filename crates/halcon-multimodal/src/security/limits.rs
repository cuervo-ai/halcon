//! Security limits: file size, image dimensions, audio/video duration.

use crate::error::{MultimodalError, Result};

/// Security limits for media ingestion.
#[derive(Debug, Clone)]
pub struct SecurityLimits {
    /// Maximum raw file size in bytes (default: 20 MB).
    pub max_file_bytes: u64,
    /// Maximum decoded image width × height pixels (default: 8192 × 8192 = 67M).
    pub max_image_pixels: u32,
    /// Maximum audio duration in seconds (default: 300 s).
    pub max_audio_secs: u32,
    /// Maximum video duration in seconds (default: 60 s).
    pub max_video_secs: u32,
}

impl Default for SecurityLimits {
    fn default() -> Self {
        Self {
            max_file_bytes:   20 * 1024 * 1024,
            max_image_pixels: 8192 * 8192,
            max_audio_secs:   300,
            max_video_secs:   60,
        }
    }
}

impl SecurityLimits {
    pub fn check_file_size(&self, size: u64) -> Result<()> {
        if size > self.max_file_bytes {
            return Err(MultimodalError::FileTooLarge { size, limit: self.max_file_bytes });
        }
        Ok(())
    }

    pub fn check_image_dimensions(&self, width: u32, height: u32) -> Result<()> {
        let pixels = width.saturating_mul(height);
        if pixels > self.max_image_pixels {
            return Err(MultimodalError::DecompressionBomb { width, height });
        }
        Ok(())
    }

    pub fn check_audio_duration(&self, duration_secs: f32) -> Result<()> {
        if duration_secs > self.max_audio_secs as f32 {
            return Err(MultimodalError::AudioTooLong {
                duration_secs,
                limit_secs: self.max_audio_secs,
            });
        }
        Ok(())
    }

    pub fn check_video_duration(&self, duration_secs: f32) -> Result<()> {
        if duration_secs > self.max_video_secs as f32 {
            return Err(MultimodalError::VideoTooLong {
                duration_secs,
                limit_secs: self.max_video_secs,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_size_ok() {
        let l = SecurityLimits::default();
        assert!(l.check_file_size(1024).is_ok());
    }

    #[test]
    fn file_size_too_large() {
        let l = SecurityLimits::default();
        assert!(l.check_file_size(u64::MAX).is_err());
    }

    #[test]
    fn image_dimensions_ok() {
        let l = SecurityLimits::default();
        assert!(l.check_image_dimensions(1920, 1080).is_ok());
    }

    #[test]
    fn image_dimensions_bomb() {
        let l = SecurityLimits::default();
        assert!(l.check_image_dimensions(100_000, 100_000).is_err());
    }

    #[test]
    fn audio_ok_and_too_long() {
        let l = SecurityLimits::default();
        assert!(l.check_audio_duration(60.0).is_ok());
        assert!(l.check_audio_duration(999.0).is_err());
    }

    #[test]
    fn video_ok_and_too_long() {
        let l = SecurityLimits::default();
        assert!(l.check_video_duration(30.0).is_ok());
        assert!(l.check_video_duration(120.0).is_err());
    }

    #[test]
    fn decompression_bomb_100k_by_100k() {
        let l = SecurityLimits::default();
        let err = l.check_image_dimensions(100_000, 100_000).unwrap_err();
        assert!(matches!(err, MultimodalError::DecompressionBomb { .. }));
    }

    #[test]
    fn decompression_bomb_threshold_is_8192_squared() {
        let l = SecurityLimits::default();
        // Exactly at the default limit (8192 × 8192 = 67,108,864 pixels) — should pass.
        assert!(l.check_image_dimensions(8192, 8192).is_ok());
        // One pixel over — should fail.
        assert!(l.check_image_dimensions(8193, 8192).is_err());
    }
}
