//! Modality detection helpers.

use crate::error::Result;
use crate::security::mime::DetectedMime;

/// Return the modality string for a detected MIME type.
pub fn modality_of(mime: &DetectedMime) -> &'static str {
    if mime.is_image()      { "image" }
    else if mime.is_audio() { "audio" }
    else if mime.is_video() { "video" }
    else                    { "unknown" }
}

/// Detect the modality of a local file (reads only the first 16 bytes).
pub async fn detect_modality_from_path(path: &str) -> Result<&'static str> {
    use tokio::io::AsyncReadExt as _;
    let mut buf = vec![0u8; 16];
    let mut f = tokio::fs::File::open(path).await?;
    let n = f.read(&mut buf).await?;
    let mime = crate::security::mime::detect_mime(&buf[..n])?;
    Ok(modality_of(&mime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modality_of_image() { assert_eq!(modality_of(&DetectedMime::ImagePng), "image"); }
    #[test]
    fn modality_of_audio() { assert_eq!(modality_of(&DetectedMime::AudioWav), "audio"); }
    #[test]
    fn modality_of_video() { assert_eq!(modality_of(&DetectedMime::VideoMp4), "video"); }
    #[test]
    fn modality_of_pdf()   { assert_eq!(modality_of(&DetectedMime::Pdf), "unknown"); }
}
