//! Security validation for media inputs.
//!
//! All media MUST pass through `MediaValidator::validate()` before processing.
//! `ValidatedMedia` is a proof type — callers can trust all checks have passed.

pub mod exif;
pub mod limits;
pub mod mime;

use std::path::Path;

use halcon_core::types::ImageSource;

use crate::error::{MultimodalError, Result};
use limits::SecurityLimits;
use mime::{detect_mime, DetectedMime};

/// Proof type: media that has passed all security checks.
#[derive(Debug)]
pub struct ValidatedMedia {
    /// Raw bytes (EXIF-stripped when applicable).
    pub data: Vec<u8>,
    /// Detected MIME type (magic bytes, not extension).
    pub mime: DetectedMime,
    /// Original file size before any stripping.
    pub original_size: u64,
}

impl ValidatedMedia {
    pub fn is_image(&self) -> bool {
        self.mime.is_image()
    }
    pub fn is_audio(&self) -> bool {
        self.mime.is_audio()
    }
    pub fn is_video(&self) -> bool {
        self.mime.is_video()
    }

    /// Convert to `ImageSource::Base64` for provider ingestion.
    /// Fails if this is not an image MIME type.
    pub fn to_image_source(&self) -> Result<ImageSource> {
        use base64::Engine as _;
        let media_type = self.mime.to_image_media_type().ok_or_else(|| {
            MultimodalError::UnsupportedMimeType(format!(
                "{} is not an image",
                self.mime.as_mime_str()
            ))
        })?;
        let data = base64::engine::general_purpose::STANDARD.encode(&self.data);
        Ok(ImageSource::Base64 { media_type, data })
    }
}

/// Validates media inputs against security policies.
#[derive(Debug, Clone)]
pub struct MediaValidator {
    limits: SecurityLimits,
    strip_exif: bool,
    privacy_strict: bool,
}

impl MediaValidator {
    pub fn new(limits: SecurityLimits, strip_exif: bool, privacy_strict: bool) -> Self {
        Self {
            limits,
            strip_exif,
            privacy_strict,
        }
    }

    /// Validate raw bytes.
    ///
    /// Steps: (1) size check, (2) magic-byte MIME detection, (3) EXIF strip,
    ///        (4) decompression bomb check (PNG/JPEG dimension scan).
    pub fn validate_bytes(&self, data: Vec<u8>) -> Result<ValidatedMedia> {
        let original_size = data.len() as u64;
        self.limits.check_file_size(original_size)?;
        let mime = detect_mime(&data)?;
        let stripped = if self.strip_exif && mime.is_image() {
            exif::strip_exif(&data)?
        } else {
            data
        };

        // Decompression bomb guard: parse image dimensions from header bytes and
        // reject images claiming more than max_image_pixels decoded pixels.
        if mime.is_image() {
            match mime {
                DetectedMime::ImagePng => {
                    // PNG IHDR chunk: bytes 16-23 contain width (4 bytes BE) + height (4 bytes BE).
                    if stripped.len() >= 24 {
                        let w = u32::from_be_bytes([
                            stripped[16],
                            stripped[17],
                            stripped[18],
                            stripped[19],
                        ]);
                        let h = u32::from_be_bytes([
                            stripped[20],
                            stripped[21],
                            stripped[22],
                            stripped[23],
                        ]);
                        if w > 0 && h > 0 {
                            self.limits.check_image_dimensions(w, h)?;
                        }
                    }
                }
                DetectedMime::ImageJpeg => {
                    // JPEG: scan for SOF0/SOF1/SOF2/SOF3 marker (0xFF 0xCn) which contains
                    // height at bytes marker+5..+6 and width at bytes marker+7..+8.
                    let mut i = 2usize;
                    while i + 8 < stripped.len() {
                        if stripped[i] == 0xFF {
                            let marker = stripped[i + 1];
                            if matches!(marker, 0xC0..=0xC3) {
                                let h =
                                    u16::from_be_bytes([stripped[i + 5], stripped[i + 6]]) as u32;
                                let w =
                                    u16::from_be_bytes([stripped[i + 7], stripped[i + 8]]) as u32;
                                if w > 0 && h > 0 {
                                    self.limits.check_image_dimensions(w, h)?;
                                }
                                break;
                            }
                            // Skip marker: 2-byte marker + 2-byte length field (length includes itself).
                            if i + 3 < stripped.len() {
                                let len =
                                    u16::from_be_bytes([stripped[i + 2], stripped[i + 3]]) as usize;
                                i += 2 + len;
                            } else {
                                break;
                            }
                        } else {
                            i += 1;
                        }
                    }
                }
                _ => {} // GIF/WebP: lower attack surface, no dimension check.
            }
        }

        Ok(ValidatedMedia {
            data: stripped,
            mime,
            original_size,
        })
    }

    /// Validate a local file path.
    ///
    /// Rejected in privacy-strict mode.
    pub async fn validate_path(&self, path: &str) -> Result<ValidatedMedia> {
        if self.privacy_strict {
            return Err(MultimodalError::PrivacyViolation(path.to_string()));
        }
        let data = tokio::fs::read(Path::new(path)).await?;
        self.validate_bytes(data)
    }

    /// Resolve an `ImageSource` to a `ValidatedMedia`.
    ///
    /// URL sources must be pre-fetched by the caller before reaching here.
    pub async fn validate_image_source(&self, source: &ImageSource) -> Result<ValidatedMedia> {
        use base64::Engine as _;
        match source {
            ImageSource::Base64 { data, .. } => {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(data.as_bytes())
                    .map_err(|e| MultimodalError::Internal(e.to_string()))?;
                self.validate_bytes(raw)
            }
            ImageSource::LocalPath { path } => self.validate_path(path).await,
            ImageSource::Url { url } => Err(MultimodalError::UnsupportedMimeType(format!(
                "URL images must be pre-fetched before validation: {url}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validator() -> MediaValidator {
        MediaValidator::new(SecurityLimits::default(), true, false)
    }

    #[test]
    fn validates_jpeg_magic() {
        // Minimal JPEG: SOI + EOI
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9];
        let vm = validator().validate_bytes(data).unwrap();
        assert!(vm.is_image());
        assert_eq!(vm.mime, DetectedMime::ImageJpeg);
    }

    #[test]
    fn rejects_unknown_mime() {
        let data = vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        assert!(validator().validate_bytes(data).is_err());
    }

    #[test]
    fn rejects_too_large() {
        let limits = SecurityLimits {
            max_file_bytes: 4,
            ..Default::default()
        };
        let v = MediaValidator::new(limits, false, false);
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9, 0x00]; // 5 bytes
        assert!(v.validate_bytes(data).is_err());
    }

    #[tokio::test]
    async fn privacy_strict_rejects_local_path() {
        let v = MediaValidator::new(SecurityLimits::default(), false, true);
        let err = v.validate_path("/tmp/test.jpg").await.unwrap_err();
        assert!(matches!(err, MultimodalError::PrivacyViolation(_)));
    }

    #[test]
    fn to_image_source_encodes_base64() {
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9];
        let vm = validator().validate_bytes(data).unwrap();
        let src = vm.to_image_source().unwrap();
        assert!(matches!(src, ImageSource::Base64 { .. }));
    }

    #[test]
    fn audio_validated_media_rejects_to_image_source() {
        // WAV magic: RIFF....WAVE
        let mut data = vec![0x52, 0x49, 0x46, 0x46]; // RIFF
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // chunk size
        data.extend_from_slice(b"WAVE"); // format
        data.extend_from_slice(&[0x66, 0x6D, 0x74, 0x20]); // "fmt " subchunk
        data.extend_from_slice(&[0x10, 0x00, 0x00, 0x00]); // subchunk size = 16
                                                           // PCM format
        data.extend_from_slice(&[0x01, 0x00]); // audio format = PCM
        data.extend_from_slice(&[0x01, 0x00]); // num channels = 1
        data.extend_from_slice(&[0x44, 0xAC, 0x00, 0x00]); // sample rate = 44100
        data.extend_from_slice(&[0x88, 0x58, 0x01, 0x00]); // byte rate
        data.extend_from_slice(&[0x02, 0x00]); // block align
        data.extend_from_slice(&[0x10, 0x00]); // bits per sample = 16
        let vm = validator().validate_bytes(data).unwrap();
        assert!(vm.is_audio(), "WAV should be detected as audio");
        let err = vm.to_image_source().unwrap_err();
        assert!(
            err.to_string().contains("not an image"),
            "Audio media should fail to_image_source; got: {err}"
        );
    }

    #[test]
    fn exif_stripping_enabled_removes_app1() {
        // Minimal JPEG with an APP1 marker (0xFF 0xE1) followed by data
        let data = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xE1, // APP1 marker
            0x00, 0x08, // length = 8 (includes length field)
            0x45, 0x78, 0x69, 0x66, // "Exif"
            0x00, 0x00, // Exif null padding
            0xFF, 0xD9, // EOI
        ];
        let v = MediaValidator::new(SecurityLimits::default(), true, false);
        let vm = v.validate_bytes(data).unwrap();
        // SOI + EOI should remain; APP1 should be stripped
        assert!(vm.data.starts_with(&[0xFF, 0xD8]), "SOI preserved");
        assert!(vm.data.ends_with(&[0xFF, 0xD9]), "EOI preserved");
        // APP1 marker should be gone
        let has_app1 = vm.data.windows(2).any(|w| w == [0xFF, 0xE1]);
        assert!(!has_app1, "APP1 EXIF marker stripped");
    }

    #[test]
    fn no_strip_exif_preserves_app1() {
        let data = vec![
            0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x08, 0x45, 0x78, 0x69, 0x66, 0x00, 0x00, 0xFF, 0xD9,
        ];
        let v = MediaValidator::new(SecurityLimits::default(), false, false);
        let vm = v.validate_bytes(data.clone()).unwrap();
        assert_eq!(vm.data, data, "Without EXIF stripping data unchanged");
    }

    #[test]
    fn corrupt_jpeg_truncated_passes_mime_check() {
        // Truncated JPEG (SOI + EOI only, no SOF marker) — should validate as JPEG.
        // No dimensions found means no bomb check fires, so it passes.
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9]; // SOI + EOI
        let result = validator().validate_bytes(data);
        assert!(result.is_ok(), "Minimal JPEG without SOF should pass");
        assert_eq!(result.unwrap().mime, DetectedMime::ImageJpeg);
    }

    #[test]
    fn png_bomb_rejected() {
        // Craft a PNG header claiming 100000×100000 pixels (decompression bomb).
        let mut data = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG magic (8 bytes)
            0x00, 0x00, 0x00, 0x0D, // IHDR chunk length = 13
            0x49, 0x48, 0x44, 0x52, // "IHDR" chunk type
        ];
        // width = 100000 (0x000186A0)
        data.extend_from_slice(&[0x00, 0x01, 0x86, 0xA0]);
        // height = 100000 (0x000186A0)
        data.extend_from_slice(&[0x00, 0x01, 0x86, 0xA0]);
        // bit depth, color type, compression, filter, interlace + CRC placeholder
        data.extend_from_slice(&[0x08, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        let err = validator().validate_bytes(data).unwrap_err();
        assert!(
            matches!(err, MultimodalError::DecompressionBomb { .. }),
            "100000×100000 PNG must be rejected as decompression bomb; got: {err}"
        );
    }

    #[test]
    fn valid_small_png_dimensions_accepted() {
        // PNG header with 100×100 dimensions — well within limits.
        let mut data = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG magic
            0x00, 0x00, 0x00, 0x0D, // IHDR length = 13
            0x49, 0x48, 0x44, 0x52, // "IHDR"
        ];
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x64]); // width = 100
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x64]); // height = 100
        data.extend_from_slice(&[0x08, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        // Should pass all checks (100×100 = 10,000 pixels, well under 67M limit).
        let result = validator().validate_bytes(data);
        assert!(result.is_ok(), "Small 100×100 PNG should be accepted");
    }
}
