//! Color space transformations.
//!
//! Currently implemented:
//! - **OKLCH** (Oklab with cylindrical coordinates) — Perceptually uniform color space
//! - **HCT** (Hue, Chroma, Tone) — Material Design 3 perceptual color space
//!   (includes CAM16 color appearance model)

pub mod oklch;

/// HCT (Hue, Chroma, Tone) color space from Google Material Design 3.
///
/// HCT combines:
/// - **Hue** from CAM16 (Color Appearance Model 2016)
/// - **Chroma** from CAM16
/// - **Tone** from CIELAB L* (lightness)
///
/// Reference: material-color-utilities (Google), Li et al. 2017.
pub mod hct;
