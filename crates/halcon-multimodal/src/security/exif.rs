//! EXIF metadata stripping.
//!
//! Removes location, device, and personal metadata from images before API upload.
//! Pure-Rust byte manipulation — no C library dependency.

use crate::error::{MultimodalError, Result};

/// Strip EXIF/metadata segments from JPEG data.
///
/// JPEG structure: SOI (FF D8) + N×segments + compressed image data.
/// We drop APP1 (FF E1 — Exif) and APP2 (FF E2 — ICC/XMP).
/// All other segments and compressed data are preserved unchanged.
pub fn strip_exif_jpeg(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 4 || data[..2] != [0xFF, 0xD8] {
        return Err(MultimodalError::ExifStripFailed("not a JPEG".into()));
    }

    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&[0xFF, 0xD8]); // preserve SOI

    let mut pos = 2usize;
    while pos < data.len() {
        // Skip fill bytes
        if data[pos] == 0xFF {
            pos += 1;
            continue;
        }
        if pos == 0 {
            break;
        } // shouldn't happen

        // data[pos-1] is 0xFF, data[pos] is the marker byte
        let marker = data[pos];
        pos += 1; // advance past marker byte

        // Standalone markers (no length field): SOI, EOI, RST0-RST7, TEM
        if matches!(marker, 0x00 | 0x01 | 0xD0..=0xD9) {
            out.extend_from_slice(&[0xFF, marker]);
            if marker == 0xD9 {
                break;
            } // EOI
            continue;
        }

        // SOS: start of scan — compressed data follows, copy remainder verbatim
        if marker == 0xDA {
            out.extend_from_slice(&[0xFF, marker]);
            if pos + 1 < data.len() {
                out.extend_from_slice(&data[pos..]);
            }
            break;
        }

        // Normal segment: 2-byte length (includes the 2 length bytes themselves)
        if pos + 1 >= data.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        if seg_len < 2 {
            break;
        }
        let seg_end = (pos + seg_len).min(data.len());

        // Drop: APP1 (0xE1 = Exif/XMP) and APP2 (0xE2 = ICC/extended XMP)
        let drop = matches!(marker, 0xE1 | 0xE2);
        if !drop {
            out.push(0xFF);
            out.push(marker);
            out.extend_from_slice(&data[pos..seg_end]);
        }
        pos = seg_end;
    }

    Ok(out)
}

/// Strip metadata chunks from PNG data.
///
/// Drops: iTXt, tEXt, zTXt (text metadata), tIME (timestamp), eXIf (Exif).
/// All other chunks (IHDR, IDAT, IEND, ...) are preserved.
pub fn strip_exif_png(data: &[u8]) -> Result<Vec<u8>> {
    const SIG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if data.len() < 8 || &data[..8] != SIG {
        return Err(MultimodalError::ExifStripFailed("not a PNG".into()));
    }

    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(SIG);
    let mut pos = 8usize;

    while pos + 12 <= data.len() {
        let chunk_len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let chunk_type = &data[pos + 4..pos + 8];
        let total = 4 + 4 + chunk_len + 4; // length + type + data + CRC

        // Drop metadata-bearing chunks
        let drop = matches!(chunk_type, b"iTXt" | b"tEXt" | b"zTXt" | b"tIME" | b"eXIf");

        if !drop {
            let end = (pos + total).min(data.len());
            out.extend_from_slice(&data[pos..end]);
        }
        pos += total;
    }

    Ok(out)
}

/// Dispatch EXIF stripping by format; passthrough for unsupported formats.
pub fn strip_exif(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() >= 2 && data[..2] == [0xFF, 0xD8] {
        return strip_exif_jpeg(data);
    }
    if data.len() >= 8 && data[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] {
        return strip_exif_png(data);
    }
    // WebP, GIF — no standard EXIF embedding; return as-is.
    Ok(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_soi_eoi_preserved() {
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9];
        let out = strip_exif_jpeg(&data).unwrap();
        assert!(out.starts_with(&[0xFF, 0xD8]));
        assert!(out.ends_with(&[0xFF, 0xD9]));
    }

    #[test]
    fn jpeg_app1_removed() {
        // SOI + APP1 (E1) with 6 bytes of data + EOI
        let payload = b"Exif\0\0";
        let seg_len = (2u16 + payload.len() as u16).to_be_bytes();
        let mut data = vec![0xFF, 0xD8]; // SOI
        data.extend_from_slice(&[0xFF, 0xE1]); // APP1 marker
        data.extend_from_slice(&seg_len);
        data.extend_from_slice(payload);
        data.extend_from_slice(&[0xFF, 0xD9]); // EOI
        let out = strip_exif_jpeg(&data).unwrap();
        assert!(out.starts_with(&[0xFF, 0xD8]));
        // APP1 marker (E1) must not appear in output
        assert!(!out.windows(2).any(|w| w == [0xFF, 0xE1]));
    }

    #[test]
    fn not_jpeg_returns_err() {
        assert!(strip_exif_jpeg(b"PNG...").is_err());
    }

    #[test]
    fn png_minimal_preserved() {
        const SIG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut data = SIG.to_vec();
        // IHDR: 13 bytes data
        data.extend_from_slice(&13u32.to_be_bytes());
        data.extend_from_slice(b"IHDR");
        data.extend_from_slice(&[0u8; 13]);
        data.extend_from_slice(&[0u8; 4]); // CRC
        let out = strip_exif_png(&data).unwrap();
        assert!(out.starts_with(SIG));
        assert!(out.windows(4).any(|w| w == b"IHDR"));
    }

    #[test]
    fn png_text_chunk_removed() {
        const SIG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let mut data = SIG.to_vec();
        // tEXt chunk: "Comment\0hello"
        let payload = b"Comment\0hello";
        data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        data.extend_from_slice(b"tEXt");
        data.extend_from_slice(payload);
        data.extend_from_slice(&[0u8; 4]); // CRC
        let out = strip_exif_png(&data).unwrap();
        assert!(!out.windows(4).any(|w| w == b"tEXt"));
    }

    #[test]
    fn dispatch_jpeg() {
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xD9];
        assert!(strip_exif(&jpeg).is_ok());
    }

    #[test]
    fn dispatch_unknown_passthrough() {
        let gif = b"GIF89a\x00\x00".to_vec();
        let out = strip_exif(&gif).unwrap();
        assert_eq!(out, gif);
    }
}
