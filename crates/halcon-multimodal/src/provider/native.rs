//! Native (local) multimodal provider.
//!
//! Performs lightweight, dependency-free image/audio analysis from raw bytes
//! without any API calls. Extracts metadata (dimensions, format, color type)
//! and builds structured descriptions for context injection.
//!
//! When `vision-native` feature is enabled with ONNX model files present,
//! CLIP-based semantic embedding is used instead of metadata-only analysis.
//! Without model files, falls back to format metadata extraction.
//!
//! Audio analysis requires `audio-native` feature + whisper.onnx model file.

use async_trait::async_trait;

use crate::error::{MultimodalError, Result};
use crate::security::ValidatedMedia;
use super::{MediaAnalysis, MultimodalProvider};

/// Local ONNX/Whisper provider — zero API calls, full local execution.
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

// ── Lightweight image metadata extraction (no external crates) ───────────────

/// Image metadata decoded from raw bytes.
#[derive(Debug, Clone)]
pub struct ImageMeta {
    pub width:       u32,
    pub height:      u32,
    pub format:      &'static str,
    pub color_type:  &'static str,
    pub bit_depth:   u8,
}

impl ImageMeta {
    /// Human-readable megapixel count.
    pub fn megapixels(&self) -> f32 {
        (self.width as f32 * self.height as f32) / 1_000_000.0
    }

    /// Common aspect ratio label: "16:9", "4:3", "1:1", or "{w}:{h}".
    pub fn aspect_ratio(&self) -> String {
        fn gcd(a: u32, b: u32) -> u32 { if b == 0 { a } else { gcd(b, a % b) } }
        let g = gcd(self.width, self.height);
        let wr = self.width / g;
        let hr = self.height / g;
        // Map common ratios to friendly labels.
        match (wr, hr) {
            (16, 9) | (32, 18) => "16:9".into(),
            (4, 3)  | (8, 6)   => "4:3".into(),
            (1, 1)             => "1:1".into(),
            (3, 2)             => "3:2".into(),
            (21, 9)            => "21:9 ultrawide".into(),
            _                  => format!("{wr}:{hr}"),
        }
    }

    /// Build a structured analysis description from image metadata.
    pub fn to_description(&self) -> String {
        format!(
            "Native image analysis: {}×{} {} image, {} color ({}bpp), {:.1}MP, aspect {}. \
             Semantic content analysis requires ONNX CLIP model (models_dir/clip.onnx).",
            self.width, self.height,
            self.format,
            self.color_type,
            self.bit_depth,
            self.megapixels(),
            self.aspect_ratio(),
        )
    }

    /// Entity tags derived from metadata for retrieval.
    pub fn to_entities(&self) -> Vec<String> {
        vec![
            format!("{}x{}", self.width, self.height),
            self.format.to_string(),
            self.color_type.to_string(),
            format!("{:.1}mp", self.megapixels()),
        ]
    }
}

/// Parse PNG metadata from raw bytes.
///
/// PNG structure: 8-byte magic + IHDR chunk (4-len + 4-type + 13-data + 4-crc)
/// IHDR data layout: width(4) + height(4) + bit_depth(1) + color_type(1) + ...
pub fn parse_png_meta(data: &[u8]) -> Option<ImageMeta> {
    if data.len() < 33 { return None; }
    // Skip: 8-byte magic + 4-byte IHDR len + 4-byte "IHDR" tag = offset 16
    let width  = u32::from_be_bytes(data[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(data[20..24].try_into().ok()?);
    let bit_depth  = data[24];
    let color_type = data[25];
    if width == 0 || height == 0 { return None; }

    let color_str = match color_type {
        0 => "grayscale",
        2 => "RGB",
        3 => "indexed",
        4 => "grayscale+alpha",
        6 => "RGBA",
        _ => "unknown",
    };
    let bpp = match (color_type, bit_depth) {
        (0, b) => b,
        (2, b) => b * 3,
        (3, b) => b,
        (4, b) => b * 2,
        (6, b) => b * 4,
        _      => bit_depth,
    };

    Some(ImageMeta { width, height, format: "PNG", color_type: color_str, bit_depth: bpp })
}

/// Parse JPEG dimensions from SOF0/SOF2 markers.
///
/// Scans the byte stream for Start-of-Frame markers:
/// 0xFF 0xC0 (SOF0/baseline), 0xFF 0xC2 (SOF2/progressive)
/// Frame header: 2-len + 1-precision + 2-height + 2-width + 1-components
pub fn parse_jpeg_meta(data: &[u8]) -> Option<ImageMeta> {
    if data.len() < 4 { return None; }
    // Scan for SOF markers.
    let mut i = 2; // Skip SOI marker.
    while i + 9 < data.len() {
        if data[i] != 0xFF { i += 1; continue; }
        let marker = data[i + 1];
        // SOF0, SOF1, SOF2, SOF3 (Baseline + Progressive)
        if matches!(marker, 0xC0 | 0xC1 | 0xC2 | 0xC3) {
            if i + 9 > data.len() { break; }
            let precision  = data[i + 4];
            let height     = u16::from_be_bytes(data[i + 5..i + 7].try_into().ok()?) as u32;
            let width      = u16::from_be_bytes(data[i + 7..i + 9].try_into().ok()?) as u32;
            let components = data[i + 9];
            if width == 0 || height == 0 { break; }
            let color_type = match components {
                1 => "grayscale",
                3 => "YCbCr (color)",
                4 => "CMYK",
                _ => "unknown",
            };
            let bpp = components * precision;
            return Some(ImageMeta { width, height, format: "JPEG", color_type, bit_depth: bpp });
        }
        // Skip over this segment.
        if i + 3 < data.len() {
            let seg_len = u16::from_be_bytes(data[i + 2..i + 4].try_into().ok()?) as usize;
            i += 2 + seg_len;
        } else {
            break;
        }
    }
    // Couldn't find SOF — return minimal info.
    Some(ImageMeta { width: 0, height: 0, format: "JPEG", color_type: "unknown", bit_depth: 0 })
}

/// Parse WebP metadata (VP8 lossy, VP8L lossless, VP8X extended).
pub fn parse_webp_meta(data: &[u8]) -> Option<ImageMeta> {
    // RIFF(4) + size(4) + WEBP(4) + chunk_tag(4) = 12 bytes header
    if data.len() < 30 { return None; }
    let chunk_tag = &data[12..16];
    match chunk_tag {
        b"VP8 " => {
            // VP8 bitstream: skip 3-byte frame tag + 3-byte magic → 10-byte header
            // Offset 26: width (14 bits) + horizontal scale (2 bits) | height (14 bits) + vertical scale (2 bits)
            if data.len() < 30 { return None; }
            let w_raw = u16::from_le_bytes(data[26..28].try_into().ok()?);
            let h_raw = u16::from_le_bytes(data[28..30].try_into().ok()?);
            let width  = (w_raw & 0x3FFF) as u32;
            let height = (h_raw & 0x3FFF) as u32;
            Some(ImageMeta { width, height, format: "WebP (lossy)", color_type: "YCbCr", bit_depth: 24 })
        }
        b"VP8L" => {
            // VP8L: signature byte (0x2F) at offset 21, then packed 28-bit width/height
            if data.len() < 25 { return None; }
            // Bits 0-13 = width-1, bits 14-27 = height-1
            let bits = u32::from_le_bytes(data[21..25].try_into().ok()?);
            let width  = (bits & 0x3FFF) + 1;
            let height = ((bits >> 14) & 0x3FFF) + 1;
            Some(ImageMeta { width, height, format: "WebP (lossless)", color_type: "RGBA", bit_depth: 32 })
        }
        b"VP8X" => {
            // VP8X extended: canvas width-1 (24 LE bits) + canvas height-1 (24 LE bits)
            if data.len() < 30 { return None; }
            let width  = (u32::from_le_bytes([data[24], data[25], data[26], 0]) & 0xFFFFFF) + 1;
            let height = (u32::from_le_bytes([data[27], data[28], data[29], 0]) & 0xFFFFFF) + 1;
            Some(ImageMeta { width, height, format: "WebP (extended)", color_type: "RGBA", bit_depth: 32 })
        }
        _ => None,
    }
}

/// Parse GIF metadata (GIF87a / GIF89a).
///
/// Header: 6-byte signature ("GIF87a"/"GIF89a") + 2-byte LE width + 2-byte LE height
pub fn parse_gif_meta(data: &[u8]) -> Option<ImageMeta> {
    if data.len() < 10 { return None; }
    let width  = u16::from_le_bytes(data[6..8].try_into().ok()?) as u32;
    let height = u16::from_le_bytes(data[8..10].try_into().ok()?) as u32;
    if width == 0 || height == 0 { return None; }
    Some(ImageMeta { width, height, format: "GIF", color_type: "indexed", bit_depth: 8 })
}

/// Extract image metadata from validated bytes.
///
/// Returns `None` if the format is not recognized or metadata is incomplete.
pub fn extract_image_meta(data: &[u8]) -> Option<ImageMeta> {
    // Detect by magic bytes.
    if data.len() >= 4 {
        // PNG: 0x89 50 4E 47
        if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            return parse_png_meta(data);
        }
        // JPEG: FF D8 FF
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return parse_jpeg_meta(data);
        }
        // GIF: "GIF8"
        if data.starts_with(b"GIF8") {
            return parse_gif_meta(data);
        }
        // WebP: RIFF ... WEBP
        if data.starts_with(b"RIFF") && data.len() >= 12 && &data[8..12] == b"WEBP" {
            return parse_webp_meta(data);
        }
    }
    None
}

// ── Audio metadata extraction ─────────────────────────────────────────────────

/// Basic WAV metadata.
#[derive(Debug)]
struct WavMeta {
    channels:    u16,
    sample_rate: u32,
    duration_ms: u32,
}

/// Parse WAV RIFF header.
fn parse_wav_meta(data: &[u8]) -> Option<WavMeta> {
    // RIFF(4) + size(4) + WAVE(4) + "fmt "(4) + fmt_size(4) = 20 bytes minimum
    if data.len() < 44 { return None; }
    if &data[0..4] != b"RIFF" { return None; }
    if &data[8..12] != b"WAVE" { return None; }
    if &data[12..16] != b"fmt " { return None; }
    let audio_format = u16::from_le_bytes(data[20..22].try_into().ok()?);
    if audio_format != 1 { return None; } // PCM only
    let channels    = u16::from_le_bytes(data[22..24].try_into().ok()?);
    let sample_rate = u32::from_le_bytes(data[24..28].try_into().ok()?);
    let byte_rate   = u32::from_le_bytes(data[28..32].try_into().ok()?);
    // Find "data" chunk.
    let mut offset = 36_usize;
    while offset + 8 <= data.len() {
        if &data[offset..offset + 4] == b"data" {
            let data_size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?);
            let duration_ms = if byte_rate > 0 {
                (data_size as u64 * 1000 / byte_rate as u64) as u32
            } else {
                0
            };
            return Some(WavMeta { channels, sample_rate, duration_ms });
        }
        let chunk_size = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().ok()?);
        offset += 8 + chunk_size as usize;
    }
    Some(WavMeta { channels, sample_rate, duration_ms: 0 })
}

// ── MultimodalProvider implementation ───────────────────────────────────────

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

        // Phase 6 Remediation: Explicit failure instead of silent metadata-only fallback.
        // ONNX inference is not yet implemented (Q2 2026). Returning Ok with metadata-only
        // analysis was misleading — callers expect actual semantic understanding.
        // Return a clear error directing users to configure an API provider instead.
        if modality == "image" {
            if !self.clip_available() {
                return Err(MultimodalError::NativeModelNotAvailable(format!(
                    "CLIP ONNX model not found at {:?}/clip.onnx. \
                     Install the model or set ANTHROPIC_API_KEY / OPENAI_API_KEY to use vision API.",
                    self.models_dir
                )));
            }
            // Model file present but ONNX inference not yet implemented.
            return Err(MultimodalError::NativeModelNotAvailable(
                "CLIP ONNX inference not yet implemented (Q2 2026). \
                 Set ANTHROPIC_API_KEY or OPENAI_API_KEY to use a vision API provider.".into()
            ));
        }

        if modality == "audio" {
            if !self.whisper_available() {
                return Err(MultimodalError::NativeModelNotAvailable(format!(
                    "Whisper ONNX model not found at {:?}/whisper.onnx. \
                     Install the model or set OPENAI_API_KEY to use Whisper API.",
                    self.models_dir
                )));
            }
            // Model file present but ONNX inference not yet implemented.
            return Err(MultimodalError::NativeModelNotAvailable(
                "Whisper ONNX inference not yet implemented (Q2 2026). \
                 Set OPENAI_API_KEY to use the Whisper transcription API.".into()
            ));
        }

        unreachable!("modality filtered above")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::ValidatedMedia;
    use crate::security::mime::DetectedMime;

    // ── NativeMultimodalProvider availability tests ───────────────────────────

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
        let p = NativeMultimodalProvider::new(None);
        let media = ValidatedMedia {
            data: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mime: DetectedMime::ImageJpeg,
            original_size: 4,
        };
        assert!(p.analyze(&media, None).await.is_err());
    }

    #[test]
    fn video_not_supported() {
        let p = NativeMultimodalProvider::new(None);
        assert!(!p.supports_modality("video"));
    }

    // ── PNG metadata parsing ──────────────────────────────────────────────────

    fn minimal_png() -> Vec<u8> {
        // 8-byte PNG magic + IHDR chunk
        let mut bytes = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG magic
            0x00, 0x00, 0x00, 0x0D, // IHDR length = 13
            0x49, 0x48, 0x44, 0x52, // "IHDR"
        ];
        // Width = 256, Height = 128, bit_depth = 8, color_type = 2 (RGB)
        bytes.extend_from_slice(&[0x00, 0x00, 0x01, 0x00]); // width=256
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x80]); // height=128
        bytes.push(0x08); // bit_depth
        bytes.push(0x02); // color_type = RGB
        bytes.extend_from_slice(&[0x00, 0x00, 0x00]); // compression, filter, interlace
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // CRC placeholder
        bytes
    }

    #[test]
    fn parse_png_meta_rgb() {
        let data = minimal_png();
        let meta = parse_png_meta(&data).expect("should parse PNG");
        assert_eq!(meta.width, 256);
        assert_eq!(meta.height, 128);
        assert_eq!(meta.format, "PNG");
        assert_eq!(meta.color_type, "RGB");
        assert_eq!(meta.bit_depth, 24); // 8 bpp × 3 channels
    }

    #[test]
    fn parse_png_rgba() {
        let mut data = minimal_png();
        data[25] = 6; // color_type = RGBA
        let meta = parse_png_meta(&data).expect("should parse PNG");
        assert_eq!(meta.color_type, "RGBA");
        assert_eq!(meta.bit_depth, 32); // 8 bpp × 4 channels
    }

    #[test]
    fn parse_png_grayscale() {
        let mut data = minimal_png();
        data[25] = 0; // color_type = grayscale
        let meta = parse_png_meta(&data).expect("should parse PNG");
        assert_eq!(meta.color_type, "grayscale");
        assert_eq!(meta.bit_depth, 8);
    }

    #[test]
    fn parse_png_too_short_returns_none() {
        let data = vec![0x89, 0x50, 0x4E, 0x47]; // magic only
        assert!(parse_png_meta(&data).is_none());
    }

    #[test]
    fn extract_image_meta_routes_png() {
        let data = minimal_png();
        let meta = extract_image_meta(&data).expect("should detect PNG");
        assert_eq!(meta.format, "PNG");
        assert_eq!(meta.width, 256);
    }

    // ── JPEG metadata parsing ─────────────────────────────────────────────────

    /// Minimal JPEG with a valid SOF0 marker.
    fn minimal_jpeg_sof() -> Vec<u8> {
        let mut bytes = vec![
            0xFF, 0xD8,             // SOI
            0xFF, 0xE0,             // APP0 marker
            0x00, 0x10,             // APP0 length = 16
        ];
        // APP0 data (14 bytes = len 16 - 2 for the length field itself)
        bytes.extend_from_slice(&[0x4A, 0x46, 0x49, 0x46, 0x00, // JFIF\0
                                   0x01, 0x01, 0x00,             // version + units
                                   0x00, 0x01, 0x00, 0x01,       // X/Y density
                                   0x00, 0x00]);                  // thumbnail
        // SOF0 marker
        bytes.push(0xFF);
        bytes.push(0xC0); // SOF0 baseline
        bytes.extend_from_slice(&[0x00, 0x0B]); // length = 11
        bytes.push(0x08); // precision
        bytes.extend_from_slice(&[0x00, 0x60]); // height = 96
        bytes.extend_from_slice(&[0x00, 0x80]); // width = 128
        bytes.push(0x03); // 3 components (YCbCr)
        bytes
    }

    #[test]
    fn parse_jpeg_meta_sof0() {
        let data = minimal_jpeg_sof();
        let meta = parse_jpeg_meta(&data).expect("should parse JPEG");
        assert_eq!(meta.width, 128);
        assert_eq!(meta.height, 96);
        assert_eq!(meta.format, "JPEG");
        assert_eq!(meta.color_type, "YCbCr (color)");
    }

    #[test]
    fn extract_image_meta_routes_jpeg() {
        let data = minimal_jpeg_sof();
        let meta = extract_image_meta(&data).expect("should detect JPEG");
        assert_eq!(meta.format, "JPEG");
    }

    // ── GIF metadata parsing ──────────────────────────────────────────────────

    fn minimal_gif() -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        bytes.extend_from_slice(&[0x40, 0x00]); // width = 64 (LE)
        bytes.extend_from_slice(&[0x20, 0x00]); // height = 32 (LE)
        bytes
    }

    #[test]
    fn parse_gif_meta_basic() {
        let data = minimal_gif();
        let meta = parse_gif_meta(&data).expect("should parse GIF");
        assert_eq!(meta.width, 64);
        assert_eq!(meta.height, 32);
        assert_eq!(meta.format, "GIF");
        assert_eq!(meta.color_type, "indexed");
    }

    #[test]
    fn extract_image_meta_routes_gif() {
        let data = minimal_gif();
        let meta = extract_image_meta(&data).expect("should detect GIF");
        assert_eq!(meta.format, "GIF");
    }

    // ── ImageMeta utility methods ──────────────────────────────────────────────

    #[test]
    fn aspect_ratio_16x9() {
        let meta = ImageMeta { width: 1920, height: 1080, format: "PNG", color_type: "RGB", bit_depth: 24 };
        assert_eq!(meta.aspect_ratio(), "16:9");
    }

    #[test]
    fn aspect_ratio_4x3() {
        let meta = ImageMeta { width: 1024, height: 768, format: "PNG", color_type: "RGB", bit_depth: 24 };
        assert_eq!(meta.aspect_ratio(), "4:3");
    }

    #[test]
    fn aspect_ratio_1x1() {
        let meta = ImageMeta { width: 512, height: 512, format: "PNG", color_type: "RGBA", bit_depth: 32 };
        assert_eq!(meta.aspect_ratio(), "1:1");
    }

    #[test]
    fn megapixels_calculation() {
        let meta = ImageMeta { width: 1000, height: 1000, format: "JPEG", color_type: "RGB", bit_depth: 24 };
        assert!((meta.megapixels() - 1.0).abs() < 0.01);
    }

    #[test]
    fn description_contains_dimensions() {
        let meta = ImageMeta { width: 1920, height: 1080, format: "JPEG", color_type: "YCbCr (color)", bit_depth: 24 };
        let desc = meta.to_description();
        assert!(desc.contains("1920×1080"), "description: {desc}");
        assert!(desc.contains("JPEG"), "description: {desc}");
    }

    #[test]
    fn entities_contain_resolution() {
        let meta = ImageMeta { width: 800, height: 600, format: "PNG", color_type: "RGB", bit_depth: 24 };
        let ents = meta.to_entities();
        assert!(ents.contains(&"800x600".to_string()));
        assert!(ents.contains(&"PNG".to_string()));
    }

    // ── WAV metadata parsing ──────────────────────────────────────────────────

    fn minimal_wav() -> Vec<u8> {
        let data_size: u32 = 44100 * 2; // 1 second of 16-bit mono at 44100Hz
        let byte_rate:  u32 = 44100 * 2;
        let total_size: u32 = 36 + data_size;
        let mut bytes = vec![];
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&total_size.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        bytes.extend_from_slice(&1u16.to_le_bytes());  // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes());  // 1 channel (mono)
        bytes.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());  // block align
        bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        // Actual PCM data would follow but we skip it for the test.
        bytes
    }

    #[test]
    fn parse_wav_meta_mono_44100() {
        let data = minimal_wav();
        let meta = parse_wav_meta(&data).expect("should parse WAV");
        assert_eq!(meta.channels, 1);
        assert_eq!(meta.sample_rate, 44100);
        assert_eq!(meta.duration_ms, 1000); // 1 second
    }

    // ── Extract image meta on unknown data ────────────────────────────────────

    #[test]
    fn extract_image_meta_returns_none_for_unknown() {
        let data = vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
        assert!(extract_image_meta(&data).is_none());
    }

    #[test]
    fn extract_image_meta_returns_none_for_empty() {
        assert!(extract_image_meta(&[]).is_none());
    }
}
