//! MIME type detection via magic bytes (not file extension).
//!
//! Never trust client-supplied MIME types — always inspect magic bytes.

use crate::error::{MultimodalError, Result};
use halcon_core::types::ImageMediaType;

/// Detected media type after magic-byte inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetectedMime {
    ImageJpeg,
    ImagePng,
    ImageWebp,
    ImageGif,
    AudioMp3,
    AudioWav,
    AudioOgg,
    AudioFlac,
    VideoMp4,
    VideoWebm,
    VideoMkv,
    VideoMov,
    Pdf,
}

impl DetectedMime {
    /// Return the canonical MIME string.
    pub fn as_mime_str(&self) -> &'static str {
        match self {
            Self::ImageJpeg => "image/jpeg",
            Self::ImagePng => "image/png",
            Self::ImageWebp => "image/webp",
            Self::ImageGif => "image/gif",
            Self::AudioMp3 => "audio/mpeg",
            Self::AudioWav => "audio/wav",
            Self::AudioOgg => "audio/ogg",
            Self::AudioFlac => "audio/flac",
            Self::VideoMp4 => "video/mp4",
            Self::VideoWebm => "video/webm",
            Self::VideoMkv => "video/x-matroska",
            Self::VideoMov => "video/quicktime",
            Self::Pdf => "application/pdf",
        }
    }

    /// Convert image MIME to `ImageMediaType`; returns `None` for non-image types.
    pub fn to_image_media_type(&self) -> Option<ImageMediaType> {
        match self {
            Self::ImageJpeg => Some(ImageMediaType::Jpeg),
            Self::ImagePng => Some(ImageMediaType::Png),
            Self::ImageWebp => Some(ImageMediaType::Webp),
            Self::ImageGif => Some(ImageMediaType::Gif),
            _ => None,
        }
    }

    pub fn is_image(&self) -> bool {
        self.to_image_media_type().is_some()
    }
    pub fn is_audio(&self) -> bool {
        matches!(
            self,
            Self::AudioMp3 | Self::AudioWav | Self::AudioOgg | Self::AudioFlac
        )
    }
    pub fn is_video(&self) -> bool {
        matches!(
            self,
            Self::VideoMp4 | Self::VideoWebm | Self::VideoMkv | Self::VideoMov
        )
    }
}

/// Detect MIME type from magic bytes (requires ≥ 4 bytes).
pub fn detect_mime(bytes: &[u8]) -> Result<DetectedMime> {
    if bytes.len() < 4 {
        return Err(MultimodalError::UnsupportedMimeType(
            "file too short".into(),
        ));
    }

    // Images
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(DetectedMime::ImageJpeg);
    }
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Ok(DetectedMime::ImagePng);
    }
    if bytes.starts_with(b"GIF8") {
        return Ok(DetectedMime::ImageGif);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Ok(DetectedMime::ImageWebp);
    }

    // Audio
    if bytes.starts_with(&[0x49, 0x44, 0x33])
        || bytes.starts_with(&[0xFF, 0xFB])
        || bytes.starts_with(&[0xFF, 0xF3])
    {
        return Ok(DetectedMime::AudioMp3);
    }
    if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WAVE" {
        return Ok(DetectedMime::AudioWav);
    }
    if bytes.starts_with(b"OggS") {
        return Ok(DetectedMime::AudioOgg);
    }
    if bytes.starts_with(b"fLaC") {
        return Ok(DetectedMime::AudioFlac);
    }

    // Video
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..bytes.len().min(12)];
        if brand.starts_with(b"qt  ") || brand.starts_with(b"qt\0\0") {
            return Ok(DetectedMime::VideoMov);
        }
        return Ok(DetectedMime::VideoMp4);
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Ok(DetectedMime::VideoWebm);
    }

    // PDF
    if bytes.starts_with(b"%PDF") {
        return Ok(DetectedMime::Pdf);
    }

    Err(MultimodalError::UnsupportedMimeType(format!(
        "unknown magic {:02X?}",
        &bytes[..bytes.len().min(8)]
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_jpeg() {
        assert_eq!(
            detect_mime(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]).unwrap(),
            DetectedMime::ImageJpeg
        );
    }

    #[test]
    fn detect_png() {
        assert_eq!(
            detect_mime(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap(),
            DetectedMime::ImagePng
        );
    }

    #[test]
    fn detect_webp() {
        let mut m = vec![0u8; 12];
        m[..4].copy_from_slice(b"RIFF");
        m[4..8].copy_from_slice(&[0, 0, 1, 0]);
        m[8..12].copy_from_slice(b"WEBP");
        assert_eq!(detect_mime(&m).unwrap(), DetectedMime::ImageWebp);
    }

    #[test]
    fn detect_wav() {
        let mut m = vec![0u8; 12];
        m[..4].copy_from_slice(b"RIFF");
        m[8..12].copy_from_slice(b"WAVE");
        assert_eq!(detect_mime(&m).unwrap(), DetectedMime::AudioWav);
    }

    #[test]
    fn detect_ogg() {
        assert_eq!(
            detect_mime(b"OggS\x00\x02\x00").unwrap(),
            DetectedMime::AudioOgg
        );
    }

    #[test]
    fn detect_flac() {
        assert_eq!(
            detect_mime(b"fLaC\x00\x00\x00\x00").unwrap(),
            DetectedMime::AudioFlac
        );
    }

    #[test]
    fn detect_ebml_webm() {
        assert_eq!(
            detect_mime(&[0x1A, 0x45, 0xDF, 0xA3, 0x00]).unwrap(),
            DetectedMime::VideoWebm
        );
    }

    #[test]
    fn unknown_returns_err() {
        assert!(detect_mime(&[0x00, 0x11, 0x22, 0x33]).is_err());
    }

    #[test]
    fn too_short_returns_err() {
        assert!(detect_mime(&[0xFF]).is_err());
    }

    #[test]
    fn is_image_audio_video() {
        assert!(DetectedMime::ImagePng.is_image());
        assert!(DetectedMime::AudioFlac.is_audio());
        assert!(DetectedMime::VideoMp4.is_video());
        assert!(!DetectedMime::ImagePng.is_audio());
    }
}
