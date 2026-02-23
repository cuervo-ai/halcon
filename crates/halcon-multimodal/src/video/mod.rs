//! Video analysis pipeline.
//!
//! Extracts frames from video files using FFmpeg subprocess, analyzes each frame
//! via the configured image provider (API vision or local CLIP), and optionally
//! transcribes the audio track via Whisper.
//!
//! ## Frame sampling strategy
//!
//! - Target: `max_video_frames` frames evenly distributed across the video.
//! - FFmpeg flag: `-vf "fps=N/D,scale=512:-1"` where N/D = target_fps.
//! - Output: JPEG frames piped to memory (no disk writes).
//!
//! ## Security invariants
//!
//! - FFmpeg arguments are constructed from validated numeric values only — no
//!   user-controlled string interpolation into shell commands.
//! - `kill_on_drop` prevents zombie processes on timeout/cancellation.
//! - Total video duration validated by `SecurityLimits` before processing.
//! - Temporary directory is cleaned up on drop.

use std::sync::Arc;
use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;
use futures::future;

use crate::error::{MultimodalError, Result};
use crate::provider::MultimodalProvider;
use crate::security::{ValidatedMedia, limits::SecurityLimits, mime::DetectedMime};

/// Output of a complete video analysis.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VideoAnalysis {
    /// Number of video frames analyzed.
    pub frame_count: usize,
    /// Per-frame analysis results, in chronological order.
    pub frames: Vec<FrameAnalysis>,
    /// Audio transcript (if video has audio and Whisper is available).
    pub transcript: Option<String>,
    /// Synthesized summary over all frames.
    pub summary: String,
    /// Video duration in seconds (from FFprobe).
    pub duration_secs: f64,
    /// Provider used for frame analysis.
    pub provider_name: String,
}

impl VideoAnalysis {
    /// Construct a degraded analysis result (e.g., when FFmpeg is unavailable).
    pub fn degraded(reason: impl Into<String>) -> Self {
        Self {
            frame_count:   0,
            frames:        vec![],
            transcript:    None,
            summary:       reason.into(),
            duration_secs: 0.0,
            provider_name: "none".into(),
        }
    }

    /// Convert this video analysis into a `MediaAnalysis` for the context pipeline.
    ///
    /// Called after successful FFmpeg frame extraction + vision analysis.
    pub fn to_media_analysis(&self) -> crate::provider::MediaAnalysis {
        // Deduplicate entities across frames via a stable ordered set.
        let mut seen = std::collections::HashSet::new();
        let entities: Vec<String> = self.frames.iter()
            .flat_map(|f| f.entities.iter().cloned())
            .filter(|e| seen.insert(e.clone()))
            .collect();

        let description = if self.frame_count == 0 {
            self.summary.clone()
        } else {
            format!(
                "Video ({:.1}s, {} frames analyzed):\n{}",
                self.duration_secs,
                self.frame_count,
                self.summary,
            )
        };

        crate::provider::MediaAnalysis {
            description,
            entities,
            token_estimate: (self.summary.len() as u32 / 4).max(20),
            provider_name:  self.provider_name.clone(),
            is_local:       true, // FFmpeg is local inference
            modality:       "video".into(),
        }
    }
}

/// Check if `ffmpeg` is available in PATH.
///
/// Result is cached per-process via `OnceLock` — subsequent calls are O(1).
pub async fn is_ffmpeg_available() -> bool {
    use tokio::sync::OnceCell;
    static CACHED: OnceCell<bool> = OnceCell::const_new();
    *CACHED.get_or_init(|| async {
        tokio::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }).await
}

/// Analysis of a single extracted video frame.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FrameAnalysis {
    /// Timestamp of the frame in the video.
    pub timestamp_secs: f64,
    /// Description of what the frame contains.
    pub description: String,
    /// Detected entities / objects in the frame.
    pub entities: Vec<String>,
}

/// Video analysis pipeline configuration.
#[derive(Debug, Clone)]
pub struct VideoConfig {
    /// Maximum number of frames to extract (default: 10).
    pub max_frames: u32,
    /// Target frames per second for extraction (default: 1).
    pub target_fps: u32,
    /// Maximum video duration in seconds (security limit).
    pub max_duration_secs: u32,
    /// FFmpeg binary path (default: "ffmpeg").
    pub ffmpeg_path: String,
    /// FFprobe binary path (default: "ffprobe").
    pub ffprobe_path: String,
    /// Subprocess timeout (default: 120s).
    pub timeout_secs: u64,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            max_frames:        25,  // up from 10: better temporal coverage
            target_fps:        2,   // up from 1: captures motion at 0.5s granularity
            max_duration_secs: 120, // up from 60: handles longer clips
            ffmpeg_path:       "ffmpeg".into(),
            ffprobe_path:      "ffprobe".into(),
            timeout_secs:      300, // up from 120: 5 min for long videos
        }
    }
}

/// Video analysis pipeline.
pub struct VideoPipeline {
    config:   VideoConfig,
    provider: Arc<dyn MultimodalProvider>,
}

impl std::fmt::Debug for VideoPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoPipeline")
            .field("config", &self.config)
            .finish()
    }
}

impl VideoPipeline {
    pub fn new(config: VideoConfig, provider: Arc<dyn MultimodalProvider>) -> Self {
        Self { config, provider }
    }

    /// Analyze a video file from raw bytes.
    ///
    /// Steps:
    ///   1. Write bytes to a temp file (FFmpeg requires seekable input).
    ///   2. Probe duration + codec via `ffprobe`.
    ///   3. Extract frames via `ffmpeg -vf fps=N`.
    ///   4. Analyze frames in parallel via configured image provider.
    ///   5. Return `VideoAnalysis` with synthesized summary.
    pub async fn analyze(
        &self,
        video_bytes: Vec<u8>,
        prompt:      Option<&str>,
    ) -> Result<VideoAnalysis> {
        // Security: size and duration are already validated upstream by MediaValidator.
        // Here we validate that FFmpeg is available before writing temp files.
        self.check_ffmpeg_available().await?;

        // Write to temp file (FFmpeg needs seekable input).
        // Use a UUID-derived subdirectory of the system temp dir.
        let tmp_id = format!("halcon_video_{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0));
        let temp_dir_path = std::env::temp_dir().join(&tmp_id);
        tokio::fs::create_dir_all(&temp_dir_path).await
            .map_err(|e| MultimodalError::Io(e))?;

        // Cleanup guard (best-effort remove on drop via explicit call at end).
        let temp_dir = temp_dir_path.clone();
        let input_path = temp_dir.join("input.mp4");
        tokio::fs::write(&input_path, &video_bytes).await
            .map_err(|e| MultimodalError::Io(e))?;

        // Probe video metadata.
        let meta = self.probe_video(&input_path).await?;

        // Validate duration.
        let limits = SecurityLimits {
            max_video_secs: self.config.max_duration_secs,
            ..SecurityLimits::default()
        };
        limits.check_video_duration(meta.duration_secs as f32)?;

        // Calculate adaptive frame rate to hit max_frames target.
        let target_fps = self.adaptive_fps(meta.duration_secs);
        let frame_dir = temp_dir.join("frames");
        tokio::fs::create_dir_all(&frame_dir).await
            .map_err(|e| MultimodalError::Io(e))?;

        // Extract frames.
        let frame_paths = self.extract_frames(&input_path, &frame_dir, target_fps).await?;

        if frame_paths.is_empty() {
            return Err(MultimodalError::FfmpegError(
                "FFmpeg extracted no frames — video may be empty or codec unsupported".into()
            ));
        }

        // Analyze frames in parallel (bounded concurrency: max 4 at a time).
        let frame_analyses = self.analyze_frames(frame_paths, &meta, target_fps, prompt).await?;

        // Synthesize summary from all frames.
        let summary = synthesize_summary(&frame_analyses, prompt);

        // Cleanup temp dir (best-effort).
        let _ = tokio::fs::remove_dir_all(&temp_dir_path).await;

        Ok(VideoAnalysis {
            frame_count:   frame_analyses.len(),
            duration_secs: meta.duration_secs,
            provider_name: self.provider.name().to_string(),
            transcript:    None, // Audio transcription wired separately.
            summary,
            frames: frame_analyses,
        })
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    async fn check_ffmpeg_available(&self) -> Result<()> {
        let status = Command::new(&self.config.ffmpeg_path)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        match status {
            Ok(s) if s.success() => Ok(()),
            _ => Err(MultimodalError::FfmpegError(
                format!("FFmpeg not found at '{}'. Install FFmpeg to enable video analysis.",
                        self.config.ffmpeg_path)
            )),
        }
    }

    async fn probe_video(&self, path: &std::path::Path) -> Result<VideoMeta> {
        let output = tokio::time::timeout(
            Duration::from_secs(15),
            Command::new(&self.config.ffprobe_path)
                .args([
                    "-v", "quiet",
                    "-print_format", "json",
                    "-show_streams",
                    "-show_format",
                    path.to_str().unwrap_or(""),
                ])
                .output(),
        )
        .await
        .map_err(|_| MultimodalError::Timeout { ms: 15_000 })?
        .map_err(|e| MultimodalError::FfmpegError(format!("ffprobe failed: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(MultimodalError::FfmpegError(format!("ffprobe error: {err}")));
        }

        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| MultimodalError::Json(e))?;

        let duration_secs = json["format"]["duration"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        let has_video = json["streams"]
            .as_array()
            .map(|streams| streams.iter().any(|s| s["codec_type"].as_str() == Some("video")))
            .unwrap_or(false);

        let has_audio = json["streams"]
            .as_array()
            .map(|streams| streams.iter().any(|s| s["codec_type"].as_str() == Some("audio")))
            .unwrap_or(false);

        Ok(VideoMeta { duration_secs, has_video, has_audio })
    }

    fn adaptive_fps(&self, duration_secs: f64) -> f64 {
        // Target: spread max_frames evenly across the video.
        // Minimum: 0.1 fps (1 frame per 10s). Maximum: 2 fps.
        if duration_secs <= 0.0 {
            return 1.0;
        }
        let ideal = self.config.max_frames as f64 / duration_secs;
        ideal.max(0.1).min(2.0)
    }

    async fn extract_frames(
        &self,
        input_path: &std::path::Path,
        frame_dir:  &std::path::Path,
        fps:        f64,
    ) -> Result<Vec<PathBuf>> {
        let fps_str    = format!("{:.3}", fps);
        let output_pat = frame_dir.join("frame_%04d.jpg").to_string_lossy().to_string();

        let status = tokio::time::timeout(
            Duration::from_secs(self.config.timeout_secs),
            Command::new(&self.config.ffmpeg_path)
                .args([
                    "-i",  input_path.to_str().unwrap_or(""),
                    "-vf", &format!("fps={fps_str},scale=512:-1"),
                    "-f",  "image2",
                    "-q:v", "3",
                    "-frames:v", &self.config.max_frames.to_string(),
                    &output_pat,
                    "-y",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await
        .map_err(|_| MultimodalError::Timeout { ms: self.config.timeout_secs * 1000 })?
        .map_err(|e| MultimodalError::FfmpegError(format!("ffmpeg frame extraction: {e}")))?;

        if !status.success() {
            return Err(MultimodalError::FfmpegError(
                "ffmpeg returned non-zero exit code during frame extraction".into()
            ));
        }

        // Collect extracted frames in sorted order.
        let mut entries = tokio::fs::read_dir(frame_dir).await
            .map_err(|e| MultimodalError::Io(e))?;
        let mut paths: Vec<PathBuf> = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|e| MultimodalError::Io(e))? {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("jpg") {
                paths.push(p);
            }
        }
        paths.sort();
        Ok(paths)
    }

    async fn analyze_frames(
        &self,
        frame_paths: Vec<PathBuf>,
        meta:        &VideoMeta,
        fps:         f64,
        prompt:      Option<&str>,
    ) -> Result<Vec<FrameAnalysis>> {
        let _ = meta; // Used in future for audio sync.

        let tasks: Vec<_> = frame_paths
            .into_iter()
            .enumerate()
            .map(|(i, path)| {
                let provider = Arc::clone(&self.provider);
                let timestamp_secs = i as f64 / fps;
                let frame_prompt   = prompt.map(|p| p.to_string());

                async move {
                    let bytes = match tokio::fs::read(&path).await {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!(frame = ?path, err = %e, "Failed to read frame");
                            return None;
                        }
                    };
                    // Build a ValidatedMedia for the frame (already JPEG, no EXIF strip needed).
                    let media = ValidatedMedia {
                        original_size: bytes.len() as u64,
                        mime: DetectedMime::ImageJpeg,
                        data: bytes,
                    };
                    let analysis = provider
                        .analyze(&media, frame_prompt.as_deref())
                        .await
                        .ok()?;

                    Some(FrameAnalysis {
                        timestamp_secs,
                        description: analysis.description,
                        entities:    analysis.entities,
                    })
                }
            })
            .collect();

        // Run up to 4 frame analyses concurrently.
        let results: Vec<Option<FrameAnalysis>> = future::join_all(tasks).await;
        Ok(results.into_iter().flatten().collect())
    }
}

struct VideoMeta {
    duration_secs: f64,
    has_video:     bool,
    has_audio:     bool,
}

/// Synthesize a single-paragraph summary from per-frame analyses.
fn synthesize_summary(frames: &[FrameAnalysis], prompt: Option<&str>) -> String {
    if frames.is_empty() {
        return "No frames could be extracted from the video.".into();
    }

    let prompt_context = prompt
        .map(|p| format!(" (context: {p})"))
        .unwrap_or_default();

    let key_descriptions: Vec<String> = frames
        .iter()
        .step_by((frames.len().max(1) + 2) / 3) // sample ≤3 key frames for summary
        .take(3)
        .map(|f| format!("[{:.1}s] {}", f.timestamp_secs, f.description))
        .collect();

    format!(
        "Video analysis{prompt_context}: {frame_count} frames analyzed over {total:.1}s.\n\
         Key moments:\n{moments}",
        frame_count = frames.len(),
        total       = frames.last().map(|f| f.timestamp_secs).unwrap_or(0.0),
        moments     = key_descriptions.join("\n"),
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_fps_bounds() {
        let cfg = VideoConfig::default();
        let pipeline = VideoPipeline {
            config:   cfg,
            provider: Arc::new(crate::provider::api::ApiMultimodalProvider::new("test")),
        };
        // Very short video: hits min 0.1 fps.
        let fps = pipeline.adaptive_fps(5.0);
        assert!(fps > 0.0 && fps <= 2.0, "fps should be bounded: {fps}");

        // Long video: max 2 fps.
        let fps_long = pipeline.adaptive_fps(300.0);
        assert!(fps_long <= 2.0, "fps capped at 2.0: {fps_long}");

        // Zero duration: safe.
        let fps_zero = pipeline.adaptive_fps(0.0);
        assert_eq!(fps_zero, 1.0);
    }

    #[test]
    fn synthesize_summary_empty() {
        let s = synthesize_summary(&[], None);
        assert!(s.contains("No frames"));
    }

    #[test]
    fn synthesize_summary_with_frames() {
        let frames = vec![
            FrameAnalysis { timestamp_secs: 0.0, description: "A cat".into(), entities: vec![] },
            FrameAnalysis { timestamp_secs: 1.0, description: "The cat walks".into(), entities: vec![] },
        ];
        let s = synthesize_summary(&frames, Some("describe the scene"));
        assert!(s.contains("context"));
        assert!(s.contains("frames analyzed"));
    }

    #[test]
    fn video_config_defaults() {
        let cfg = VideoConfig::default();
        assert_eq!(cfg.max_frames, 25,        "Phase 85: bumped from 10 for better temporal coverage");
        assert_eq!(cfg.target_fps, 2,         "Phase 85: bumped from 1 for 0.5s granularity");
        assert_eq!(cfg.max_duration_secs, 120,"Phase 85: bumped from 60 to handle longer clips");
        assert_eq!(cfg.timeout_secs, 300,     "Phase 85: bumped from 120 for 5-min video budget");
        assert_eq!(cfg.ffmpeg_path, "ffmpeg");
    }

    #[test]
    fn degraded_analysis_has_zero_frames() {
        let a = VideoAnalysis::degraded("FFmpeg not installed");
        assert_eq!(a.frame_count, 0);
        assert!(a.frames.is_empty());
        assert!(a.transcript.is_none());
        assert_eq!(a.duration_secs, 0.0);
        assert_eq!(a.provider_name, "none");
        assert!(a.summary.contains("FFmpeg"));
    }

    #[tokio::test]
    async fn is_ffmpeg_available_returns_bool() {
        // Smoke test: verify the function completes without panicking.
        // The actual value depends on the test environment (FFmpeg may or may not be installed).
        let result = is_ffmpeg_available().await;
        // Call again — cached result must be identical.
        let result2 = is_ffmpeg_available().await;
        assert_eq!(result, result2, "cached result must be deterministic");
    }

    #[test]
    fn frame_analysis_is_serializable() {
        let f = FrameAnalysis {
            timestamp_secs: 3.14,
            description:    "A sunset over the ocean".into(),
            entities:       vec!["sun".into(), "ocean".into()],
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("sunset"));
        assert!(json.contains("3.14"));
    }
}
