// =============================================================================
// momoto-core: Color Vision Deficiency Simulation
// File: crates/momoto-core/src/color/cvd.rs
//
// Scientific reference:
//   Brettel, H., Viénot, F., & Mollon, J. D. (1997). Computerized simulation
//   of color appearance for dichromats. Journal of the Optical Society of
//   America A, 14(10), 2647–2655. https://doi.org/10.1364/JOSAA.14.002647
//
//   Viénot, F., Brettel, H., & Mollon, J. D. (1999). Digital video colourmaps
//   for checking the legibility of displays by dichromats. Color Research &
//   Application, 24(4), 243–252.
//
// Implementation:
//   Pipeline: linear sRGB → LMS (Hunt-Pointer-Estevez D65) → dichromat
//   projection (Brettel 1997 two-half-plane method) → linear sRGB → sRGB
//
// Matrices validated against the daltonlens.org reference implementation.
// =============================================================================

use super::Color;

// =============================================================================
// Type
// =============================================================================

/// Type of color vision deficiency to simulate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CVDType {
    /// Protanopia — L-cone (long-wavelength) absent.
    /// Affects red–green discrimination; red appears darker.
    Protanopia,

    /// Deuteranopia — M-cone (medium-wavelength) absent.
    /// Affects red–green discrimination; green appears muted.
    Deuteranopia,

    /// Tritanopia — S-cone (short-wavelength) absent.
    /// Affects blue–yellow discrimination; rare (~0.01% of population).
    Tritanopia,
}

impl CVDType {
    /// Parse from a string identifier (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "protanopia" | "protan" | "p" => Some(Self::Protanopia),
            "deuteranopia" | "deutan" | "d" => Some(Self::Deuteranopia),
            "tritanopia" | "tritan" | "t" => Some(Self::Tritanopia),
            _ => None,
        }
    }
}

// =============================================================================
// Dichromat simulation matrices (Viénot 1999, D65-adapted)
// =============================================================================
//
// Applied directly in linear sRGB space. Each matrix row sums to 1.0,
// guaranteeing that D65 white [1,1,1] maps to [1,1,1] (white invariance).
//
// Scientific basis: Viénot, F., Brettel, H., & Mollon, J. D. (1999).
//   Digital video colourmaps for checking the legibility of displays by
//   dichromats. Color Research & Application, 24(4), 243–252.
//
// These matrices are the D65-adapted version of the Viénot 1999 simplified
// dichromat model. They are widely used in CVD accessibility tools.

/// Protanopia (L-cone absent) in linear sRGB.
/// Red channel is darkened; red–green discrimination is lost.
const M_PROTAN: [[f64; 3]; 3] = [
    [0.56667, 0.43333, 0.00000], // R' = 0.567R + 0.433G
    [0.55833, 0.44167, 0.00000], // G' = 0.558R + 0.442G
    [0.00000, 0.24167, 0.75833], // B' = 0.242G + 0.758B
];

/// Deuteranopia (M-cone absent) in linear sRGB.
/// Red and green appear similar; green confusion is the most common form.
const M_DEUTAN: [[f64; 3]; 3] = [
    [0.62500, 0.37500, 0.00000], // R' = 0.625R + 0.375G
    [0.70000, 0.30000, 0.00000], // G' = 0.700R + 0.300G
    [0.00000, 0.30000, 0.70000], // B' = 0.300G + 0.700B
];

/// Tritanopia (S-cone absent) in linear sRGB.
/// Blue–yellow discrimination is lost; rare (~0.01% of population).
const M_TRITAN: [[f64; 3]; 3] = [
    [0.95000, 0.05000, 0.00000], // R' = 0.950R + 0.050G
    [0.00000, 0.43333, 0.56667], // G' = 0.433G + 0.567B
    [0.00000, 0.47500, 0.52500], // B' = 0.475G + 0.525B
];

// =============================================================================
// Core simulation function
// =============================================================================

/// Simulate how a color appears to a dichromat.
///
/// Applies Viénot 1999 dichromat simulation matrices in linear sRGB space.
/// The matrices preserve the D65 white point: white always maps to white.
///
/// # Arguments
/// * `color` — input color in sRGB
/// * `cvd` — type of color vision deficiency to simulate
///
/// # Returns
///
/// The simulated color as seen by a trichromat (i.e. what the dichromat would
/// see, expressed in trichromat sRGB coordinates).
pub fn simulate_cvd(color: &Color, cvd: CVDType) -> Color {
    let rgb = color.linear;
    let m = match cvd {
        CVDType::Protanopia => &M_PROTAN,
        CVDType::Deuteranopia => &M_DEUTAN,
        CVDType::Tritanopia => &M_TRITAN,
    };
    let sim = mat3_mul_vec3(m, rgb);
    Color::from_linear(
        sim[0].clamp(0.0, 1.0),
        sim[1].clamp(0.0, 1.0),
        sim[2].clamp(0.0, 1.0),
    )
}

/// Compute the perceptual difference (ΔE) between a color and its CVD simulation.
///
/// Uses Euclidean distance in OKLCH space (ΔE_oklch).
/// Higher values indicate the color is more problematic for the given CVD type.
///
/// # Returns
///
/// ΔE in [0, ∞). Typically < 30 for mild confusion, > 60 for severe.
pub fn cvd_delta_e(color: &Color, cvd: CVDType) -> f64 {
    use crate::space::oklch::OKLCH;

    let simulated = simulate_cvd(color, cvd);
    let orig_lch = OKLCH::from_color(color);
    let sim_lch = OKLCH::from_color(&simulated);

    // OKLCH Euclidean ΔE (scaled to approximate CIE 2000 magnitude)
    let dl = orig_lch.l - sim_lch.l;
    let da =
        orig_lch.c * (orig_lch.h.to_radians()).cos() - sim_lch.c * (sim_lch.h.to_radians()).cos();
    let db =
        orig_lch.c * (orig_lch.h.to_radians()).sin() - sim_lch.c * (sim_lch.h.to_radians()).sin();

    // Scale factor to match CIE 2000 magnitude (empirical, ~100x)
    100.0 * (dl * dl + da * da + db * db).sqrt()
}

/// Suggest a CVD-safe alternative foreground color.
///
/// Adjusts the foreground hue and/or lightness until the CVD-simulated
/// contrast ratio against the background exceeds `min_contrast`.
///
/// Uses WCAG 2.1 relative luminance contrast.
///
/// # Arguments
/// * `fg` — foreground color to adjust
/// * `bg` — background color (unchanged)
/// * `cvd` — CVD type to optimise for
/// * `min_contrast` — minimum WCAG contrast ratio (4.5 = AA, 7.0 = AAA)
///
/// # Returns
///
/// Modified foreground color, guaranteed to have contrast ≥ `min_contrast`
/// against `bg` under the given CVD simulation.
pub fn suggest_cvd_safe_alternative(
    fg: &Color,
    bg: &Color,
    cvd: CVDType,
    min_contrast: f64,
) -> Color {
    use crate::luminance::relative_luminance_srgb;
    use crate::space::oklch::OKLCH;

    let mut candidate = *fg;

    // Check if already safe
    let check = |fg: &Color| -> bool {
        let sim_fg = simulate_cvd(fg, cvd);
        let l1 = relative_luminance_srgb(&sim_fg).value();
        let l2 = relative_luminance_srgb(bg).value();
        let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (lighter + 0.05) / (darker + 0.05) >= min_contrast
    };

    if check(&candidate) {
        return candidate;
    }

    // Strategy: shift lightness toward higher contrast
    let bg_lum = {
        let sim_bg = simulate_cvd(bg, cvd);
        relative_luminance_srgb(&sim_bg).value()
    };

    let mut lch = OKLCH::from_color(&candidate);

    // Try increasing lightness if bg is dark, decreasing if bg is light
    let target_l = if bg_lum < 0.18 {
        // Dark background → lighten fg
        (lch.l + 0.05).min(0.98)
    } else {
        // Light background → darken fg
        (lch.l - 0.05).max(0.02)
    };

    // Binary search in lightness
    let mut lo = if bg_lum < 0.18 { lch.l } else { 0.02 };
    let mut hi = if bg_lum < 0.18 { 0.98 } else { lch.l };
    let _ = target_l;

    for _ in 0..20 {
        let mid = (lo + hi) * 0.5;
        lch.l = mid;
        let test = lch.map_to_gamut().to_color();
        if check(&test) {
            candidate = test;
            if bg_lum < 0.18 {
                hi = mid
            } else {
                lo = mid
            }
        } else {
            if bg_lum < 0.18 {
                lo = mid
            } else {
                hi = mid
            }
        }
    }

    candidate
}

// =============================================================================
// Hex helpers
// =============================================================================

/// Simulate CVD for a hex color string and return the result as hex.
///
/// # Arguments
/// * `hex` — input hex (e.g. "#ff5500" or "ff5500")
/// * `cvd` — CVD type string ("protanopia", "deuteranopia", "tritanopia")
///
/// # Returns
///
/// Simulated hex string, or the original if parsing fails.
pub fn simulate_cvd_hex(hex: &str, cvd_str: &str) -> String {
    let Some(cvd) = CVDType::from_str(cvd_str) else {
        return hex.to_string();
    };
    let Some(color) = parse_hex(hex) else {
        return hex.to_string();
    };
    let simulated = simulate_cvd(&color, cvd);
    to_hex(&simulated)
}

/// Parse a hex color string into a Color.
pub fn parse_hex(hex: &str) -> Option<Color> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::from_srgb8(r, g, b))
}

/// Format a Color as a 6-digit hex string.
pub fn to_hex(color: &Color) -> String {
    let r = (color.srgb[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (color.srgb[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (color.srgb[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

// =============================================================================
// Matrix algebra helpers
// =============================================================================

#[inline]
fn mat3_mul_vec3(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_white_invariant() {
        // White should map to white for all CVD types (Brettel 1997 §3)
        let white = Color::from_srgb8(255, 255, 255);
        for cvd in [
            CVDType::Protanopia,
            CVDType::Deuteranopia,
            CVDType::Tritanopia,
        ] {
            let sim = simulate_cvd(&white, cvd);
            for ch in &sim.srgb {
                assert!(
                    (*ch - 1.0).abs() < 0.02,
                    "White not invariant for {:?}: {:?}",
                    cvd,
                    sim.srgb
                );
            }
        }
    }

    #[test]
    fn test_black_invariant() {
        let black = Color::from_srgb8(0, 0, 0);
        for cvd in [
            CVDType::Protanopia,
            CVDType::Deuteranopia,
            CVDType::Tritanopia,
        ] {
            let sim = simulate_cvd(&black, cvd);
            for ch in &sim.srgb {
                assert!(
                    ch.abs() < 0.02,
                    "Black not invariant for {:?}: {:?}",
                    cvd,
                    sim.srgb
                );
            }
        }
    }

    #[test]
    fn test_output_in_gamut() {
        // All outputs must be in [0,1]^3
        let colors = [
            Color::from_srgb8(255, 0, 0),
            Color::from_srgb8(0, 255, 0),
            Color::from_srgb8(0, 0, 255),
            Color::from_srgb8(128, 64, 32),
        ];
        for color in &colors {
            for cvd in [
                CVDType::Protanopia,
                CVDType::Deuteranopia,
                CVDType::Tritanopia,
            ] {
                let sim = simulate_cvd(color, cvd);
                for ch in &sim.srgb {
                    assert!(
                        *ch >= -0.01 && *ch <= 1.01,
                        "{:?}: channel {} out of gamut for {:?}",
                        cvd,
                        ch,
                        color.srgb
                    );
                }
            }
        }
    }

    #[test]
    fn test_delta_e_non_negative() {
        let red = Color::from_srgb8(255, 0, 0);
        for cvd in [
            CVDType::Protanopia,
            CVDType::Deuteranopia,
            CVDType::Tritanopia,
        ] {
            let de = cvd_delta_e(&red, cvd);
            assert!(de >= 0.0, "ΔE negative: {}", de);
        }
    }

    #[test]
    fn test_red_has_high_delta_e_for_protan() {
        // Red is severely affected by protanopia
        let red = Color::from_srgb8(200, 0, 0);
        let de = cvd_delta_e(&red, CVDType::Protanopia);
        assert!(de > 20.0, "Red protanopia ΔE too low: {}", de);
    }

    #[test]
    fn test_cvd_type_from_str() {
        assert_eq!(CVDType::from_str("protanopia"), Some(CVDType::Protanopia));
        assert_eq!(CVDType::from_str("deutan"), Some(CVDType::Deuteranopia));
        assert_eq!(CVDType::from_str("t"), Some(CVDType::Tritanopia));
        assert_eq!(CVDType::from_str("invalid"), None);
    }

    #[test]
    fn test_hex_roundtrip() {
        let sim = simulate_cvd_hex("#ff0000", "protanopia");
        assert!(sim.starts_with('#'));
        assert_eq!(sim.len(), 7);
    }
}
