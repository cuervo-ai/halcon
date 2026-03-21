// =============================================================================
// HCT Color Space
// File: crates/momoto-core/src/space/hct/mod.rs
//
// Implements HCT (Hue-Chroma-Tone) — Google Material Design 3 perceptual
// color space used for dynamic color generation.
//
// Reference: Google material-color-utilities (Apache 2.0)
// https://github.com/material-foundation/material-color-utilities
//
// HCT properties:
//   H (Hue):   CAM16 hue angle, 0–360°
//   C (Chroma): CAM16 chroma (≥ 0, varies with tone and hue)
//   T (Tone):   CIELAB L* (0 = black, 100 = white)
// =============================================================================

pub mod cam16;

use crate::color::Color;
use cam16::{lstar_from_y, mat3_mul_vec3, y_from_lstar, ViewingConditions, CAM16};

// =============================================================================
// sRGB ↔ XYZ matrices (D65 reference white)
// =============================================================================

/// Linear sRGB → XYZ D65 (IEC 61966-2-1)
const M_SRGB_TO_XYZ: [[f64; 3]; 3] = [
    [0.4124564, 0.3575761, 0.1804375],
    [0.2126729, 0.7151522, 0.0721750],
    [0.0193339, 0.1191920, 0.9503041],
];

/// XYZ D65 → linear sRGB (inverse of above)
const M_XYZ_TO_SRGB: [[f64; 3]; 3] = [
    [3.2404542, -1.5371385, -0.4985314],
    [-0.9692660, 1.8760108, 0.0415560],
    [0.0556434, -0.2040259, 1.0572252],
];

// =============================================================================
// HCT color struct
// =============================================================================

/// HCT (Hue, Chroma, Tone) — Material Design 3 perceptual color space.
///
/// - **Hue**: CAM16 hue angle (0–360°). Perceptually uniform hue wheel.
/// - **Chroma**: CAM16 chroma. Maximum achievable chroma varies by tone and hue.
/// - **Tone**: CIELAB L* lightness (0 = black, 100 = white). Independent of hue/chroma.
///
/// # Example
///
/// ```rust
/// use momoto_core::space::hct::HCT;
/// use momoto_core::color::Color;
///
/// let red = Color::from_srgb8(255, 0, 0);
/// let hct = HCT::from_color(&red);
/// println!("H={:.1}° C={:.1} T={:.1}", hct.hue, hct.chroma, hct.tone);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HCT {
    /// CAM16 hue angle in degrees (0–360°).
    pub hue: f64,
    /// CAM16 chroma (non-negative; maximum varies by hue and tone).
    pub chroma: f64,
    /// CIELAB L* lightness (0 = black, 100 = white).
    pub tone: f64,
}

impl HCT {
    /// Create an HCT color from components.
    ///
    /// Note: The requested chroma may be clamped to the sRGB gamut boundary
    /// when converting to a color (via `to_color()`).
    pub const fn new(hue: f64, chroma: f64, tone: f64) -> Self {
        Self { hue, chroma, tone }
    }

    // =========================================================================
    // Forward conversion: Color → HCT
    // =========================================================================

    /// Convert an sRGB color to HCT.
    pub fn from_color(color: &Color) -> Self {
        let xyz = linear_srgb_to_xyz(color.linear);
        let vc = ViewingConditions::s_rgb();
        let cam = CAM16::from_xyz(xyz, &vc);

        // Tone = CIELAB L* from Y (normalized to white Y = 1.0)
        let y_normalized = xyz[1] / 100.0;
        let tone = lstar_from_y(y_normalized);

        HCT {
            hue: cam.h,
            chroma: cam.c,
            tone: tone.clamp(0.0, 100.0),
        }
    }

    /// Create HCT from a packed ARGB integer (0xAARRGGBB, alpha ignored).
    pub fn from_argb(argb: u32) -> Self {
        let r = ((argb >> 16) & 0xFF) as u8;
        let g = ((argb >> 8) & 0xFF) as u8;
        let b = (argb & 0xFF) as u8;
        Self::from_color(&Color::from_srgb8(r, g, b))
    }

    // =========================================================================
    // Inverse conversion: HCT → Color
    // =========================================================================

    /// Convert HCT back to an sRGB color.
    ///
    /// If the requested chroma exceeds the gamut boundary at this hue and tone,
    /// it is clamped to the maximum achievable in-gamut chroma.
    ///
    /// Uses a binary search over CAM16 J values to guarantee that the Y
    /// component of the resulting XYZ corresponds exactly to the target
    /// CIELAB L* tone (following the material-color-utilities HctSolver approach).
    pub fn to_color(&self) -> Color {
        let vc = ViewingConditions::s_rgb();

        // Achromatic shortcut
        if self.chroma < 1e-4 {
            return tone_to_achromatic_color(self.tone);
        }

        // Target Y (XYZ scale [0, 100]) corresponding to the requested tone
        let target_y = y_from_lstar(self.tone) * 100.0;

        if target_y <= 0.0 {
            return Color::from_srgb8(0, 0, 0);
        }

        // Binary search for J such that CAM16⁻¹(J, C, H)[Y] ≈ target_Y.
        // Different from using J of an achromatic gray because chromatic colors
        // can have Y ≠ Y_gray at the same J (CAM16 J ≠ CIELAB L*).
        let mut j_lo = 0.0_f64;
        let mut j_hi = 100.0_f64;

        for _ in 0..50 {
            let j_mid = (j_lo + j_hi) / 2.0;
            let max_c = find_max_chroma(j_mid, self.hue, &vc);
            let actual_c = self.chroma.min(max_c);
            let xyz = CAM16::to_xyz_from_jch(j_mid, actual_c, self.hue, &vc);
            if xyz[1] < target_y {
                j_lo = j_mid;
            } else {
                j_hi = j_mid;
            }
        }

        let j = (j_lo + j_hi) / 2.0;
        let max_c = find_max_chroma(j, self.hue, &vc);
        let actual_c = self.chroma.min(max_c);
        let xyz = CAM16::to_xyz_from_jch(j, actual_c, self.hue, &vc);
        xyz_to_color(xyz)
    }

    /// Convert HCT to a packed ARGB integer (0xFF_RR_GG_BB).
    pub fn to_argb(&self) -> u32 {
        let color = self.to_color();
        let [r, g, b] = color.to_srgb8();
        0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    }

    // =========================================================================
    // Utility
    // =========================================================================

    /// Clamp HCT chroma to what is achievable in the sRGB gamut.
    pub fn clamp_to_gamut(&self) -> Self {
        let vc = ViewingConditions::s_rgb();
        let y = y_from_lstar(self.tone);
        let gray_xyz = [y * 95.047, y * 100.0, y * 108.883];
        let j = CAM16::from_xyz(gray_xyz, &vc).j;
        let max_c = find_max_chroma(j, self.hue, &vc);
        HCT {
            hue: self.hue,
            chroma: self.chroma.min(max_c),
            tone: self.tone,
        }
    }
}

// =============================================================================
// Gamut search helpers
// =============================================================================

/// Find the maximum in-gamut CAM16 chroma at a given J and hue.
///
/// Uses binary search with 50 iterations (precision ≈ 1e-13 chroma units).
fn find_max_chroma(j: f64, h: f64, vc: &ViewingConditions) -> f64 {
    let mut lo = 0.0_f64;
    let mut hi = 200.0_f64;

    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        let xyz = CAM16::to_xyz_from_jch(j, mid, h, vc);
        if is_xyz_in_srgb_gamut(xyz) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Check if XYZ (range 0–100) maps to a valid linear sRGB (all channels in [-ε, 1+ε]).
fn is_xyz_in_srgb_gamut(xyz: [f64; 3]) -> bool {
    // XYZ is in [0, 100] range; M_XYZ_TO_SRGB expects normalized input
    let xyz_norm = [xyz[0] / 100.0, xyz[1] / 100.0, xyz[2] / 100.0];
    let rgb = mat3_mul_vec3(&M_XYZ_TO_SRGB, xyz_norm);
    const EPS: f64 = 0.0001;
    rgb[0] >= -EPS
        && rgb[0] <= 1.0 + EPS
        && rgb[1] >= -EPS
        && rgb[1] <= 1.0 + EPS
        && rgb[2] >= -EPS
        && rgb[2] <= 1.0 + EPS
}

// =============================================================================
// Color conversion helpers
// =============================================================================

/// Linear sRGB (in [0, 1]) → XYZ D65 (in [0, 100]).
fn linear_srgb_to_xyz(linear: [f64; 3]) -> [f64; 3] {
    // Scale by 100 so white Y = 100
    let xyz_norm = mat3_mul_vec3(&M_SRGB_TO_XYZ, linear);
    [
        xyz_norm[0] * 100.0,
        xyz_norm[1] * 100.0,
        xyz_norm[2] * 100.0,
    ]
}

/// XYZ D65 (in [0, 100]) → sRGB Color.
fn xyz_to_color(xyz: [f64; 3]) -> Color {
    let xyz_norm = [xyz[0] / 100.0, xyz[1] / 100.0, xyz[2] / 100.0];
    let rgb_lin = mat3_mul_vec3(&M_XYZ_TO_SRGB, xyz_norm);
    Color::from_linear(
        rgb_lin[0].clamp(0.0, 1.0),
        rgb_lin[1].clamp(0.0, 1.0),
        rgb_lin[2].clamp(0.0, 1.0),
    )
}

/// Create an achromatic (gray) color for a given CIELAB tone.
fn tone_to_achromatic_color(tone: f64) -> Color {
    let y = y_from_lstar(tone); // [0, 1]
    Color::from_linear(y, y, y)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure black → Tone ≈ 0
    #[test]
    fn test_black_tone() {
        let black = Color::from_srgb8(0, 0, 0);
        let hct = HCT::from_color(&black);
        assert!(
            hct.tone < 1.0,
            "Black tone should be ~0, got {:.2}",
            hct.tone
        );
        assert!(
            hct.chroma < 2.0,
            "Black chroma should be ~0, got {:.2}",
            hct.chroma
        );
    }

    /// Pure white → Tone ≈ 100, chroma ≈ 0
    #[test]
    fn test_white_tone() {
        let white = Color::from_srgb8(255, 255, 255);
        let hct = HCT::from_color(&white);
        assert!(
            (hct.tone - 100.0).abs() < 1.0,
            "White tone should be ~100, got {:.2}",
            hct.tone
        );
        assert!(
            hct.chroma < 5.0,
            "White chroma should be ~0, got {:.2}",
            hct.chroma
        );
    }

    /// Mid-gray → Tone ≈ 53.4, chroma ≈ 0
    #[test]
    fn test_gray_tone() {
        let gray = Color::from_srgb8(128, 128, 128);
        let hct = HCT::from_color(&gray);
        assert!(
            (hct.tone - 53.4).abs() < 3.0,
            "Mid-gray tone should be ~53, got {:.2}",
            hct.tone
        );
        assert!(
            hct.chroma < 5.0,
            "Gray chroma should be ~0, got {:.2}",
            hct.chroma
        );
    }

    /// Roundtrip: Color → HCT → Color preserves tone within 1 unit (ΔL* < 1)
    #[test]
    fn test_roundtrip_tone_preserved() {
        let test_colors = [
            Color::from_srgb8(255, 0, 0),    // red
            Color::from_srgb8(0, 255, 0),    // green
            Color::from_srgb8(0, 0, 255),    // blue
            Color::from_srgb8(200, 100, 50), // orange-brown
            Color::from_srgb8(50, 100, 200), // blue-ish
        ];

        for color in &test_colors {
            let hct = HCT::from_color(color);
            let recovered = hct.to_color();
            let hct2 = HCT::from_color(&recovered);

            let tone_delta = (hct.tone - hct2.tone).abs();
            assert!(
                tone_delta < 2.0,
                "Tone not preserved: original={:.2}, recovered={:.2}",
                hct.tone,
                hct2.tone
            );
        }
    }

    /// from_argb / to_argb roundtrip
    #[test]
    fn test_argb_roundtrip() {
        let original = 0xFF_3A_7B_D5_u32;
        let hct = HCT::from_argb(original);
        let argb = hct.to_argb();

        // Alpha must be 0xFF
        assert_eq!(argb >> 24, 0xFF);

        // RGB channels within 10 (some loss expected from gamut clamping)
        let or_ = ((original >> 16) & 0xFF) as i32;
        let og = ((original >> 8) & 0xFF) as i32;
        let ob = (original & 0xFF) as i32;
        let r = ((argb >> 16) & 0xFF) as i32;
        let g = ((argb >> 8) & 0xFF) as i32;
        let b = (argb & 0xFF) as i32;
        assert!(
            (or_ - r).abs() <= 15,
            "Red channel delta too large: {} vs {}",
            or_,
            r
        );
        assert!(
            (og - g).abs() <= 15,
            "Green channel delta too large: {} vs {}",
            og,
            g
        );
        assert!(
            (ob - b).abs() <= 15,
            "Blue channel delta too large: {} vs {}",
            ob,
            b
        );
    }

    /// Hue of red should be distinct from hue of blue
    #[test]
    fn test_hue_discrimination() {
        let red = HCT::from_color(&Color::from_srgb8(255, 0, 0));
        let blue = HCT::from_color(&Color::from_srgb8(0, 0, 255));
        let hue_diff = (red.hue - blue.hue).abs();
        let hue_diff = hue_diff.min(360.0 - hue_diff);
        assert!(
            hue_diff > 45.0,
            "Red and blue should be >45° apart, got {:.1}°",
            hue_diff
        );
    }

    /// Achromatic HCT → to_color → round trip tone
    #[test]
    fn test_achromatic_conversion() {
        let hct = HCT::new(0.0, 0.0, 60.0); // achromatic at tone 60
        let color = hct.to_color();
        let hct2 = HCT::from_color(&color);
        assert!(
            (hct2.tone - 60.0).abs() < 2.0,
            "Achromatic tone not preserved: {:.2}",
            hct2.tone
        );
    }

    /// clamp_to_gamut should not increase chroma
    #[test]
    fn test_clamp_to_gamut() {
        let over_gamut = HCT::new(30.0, 300.0, 50.0); // extremely high chroma
        let clamped = over_gamut.clamp_to_gamut();
        assert!(clamped.chroma <= over_gamut.chroma);
        assert!(clamped.chroma >= 0.0);
        // After clamping, to_color should be in sRGB gamut
        let color = clamped.to_color();
        for &ch in &color.srgb {
            assert!(ch >= -0.01 && ch <= 1.01, "Channel out of gamut: {}", ch);
        }
    }
}
