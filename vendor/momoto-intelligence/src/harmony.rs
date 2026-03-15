// =============================================================================
// momoto-intelligence: Color Harmony Engine
// File: crates/momoto-intelligence/src/harmony.rs
//
// Scientific references:
//   Itten, J. (1961). The Art of Color. Reinhold Publishing.
//   Munsell, A. (1905). A Color Notation. Boston: G.H. Ellis.
//
// All operations in OKLCH (perceptually uniform) color space.
// Every generated color is mapped to the sRGB gamut before returning.
// =============================================================================

use momoto_core::color::Color;
use momoto_core::space::oklch::OKLCH;

// =============================================================================
// HarmonyType
// =============================================================================

/// Chromatic harmony model based on hue-wheel geometry (Itten 1961).
///
/// All angle offsets are in the OKLCH hue wheel (0–360°).
/// The wheel is perceptually uniform but NOT linearly spaced in wavelength.
#[derive(Debug, Clone, PartialEq)]
pub enum HarmonyType {
    /// Two hues 180° apart — maximum contrast, classic complementary.
    Complementary,

    /// Seed + two hues at ±150° (or +150° and +210°).
    /// Less harsh than complementary, retains visual interest.
    SplitComplementary,

    /// Hues evenly distributed every 120° around the wheel.
    Triadic,

    /// Four hues at 90° intervals — rich, complex palette.
    Tetradic,

    /// Hues within a narrow arc of ±`spread` degrees around the seed.
    Analogous {
        /// Angular spread in degrees (30° = tight, 60° = wide).
        spread: f64,
    },

    /// Single hue, varying lightness only — classic Munsell scale.
    Monochromatic {
        /// Number of tonal steps to generate.
        steps: u8,
    },

    /// Warm (hue 0–60°) or cool (hue 180–270°) tonal range.
    Temperature {
        /// `true` = warm palette, `false` = cool palette.
        warm: bool,
    },

    /// Custom set of hue offsets in degrees from the seed hue.
    Custom(Vec<f64>),
}

// =============================================================================
// Palette
// =============================================================================

/// A generated color palette with harmony metadata.
#[derive(Debug, Clone)]
pub struct Palette {
    /// Constituent colors in OKLCH, all gamut-safe.
    pub colors: Vec<OKLCH>,

    /// The harmony model used to generate this palette.
    pub harmony: HarmonyType,

    /// Harmony quality score in [0, 1]. Higher is more coherent.
    pub score: f64,
}

// =============================================================================
// Core functions
// =============================================================================

/// Generate a color palette from a seed color using the given harmony model.
///
/// All output colors are gamut-mapped into sRGB before return.
///
/// # Arguments
/// * `seed` — base color in OKLCH
/// * `harmony` — harmony model
///
/// # Returns
///
/// A `Palette` whose colors satisfy the chosen chromatic relationship.
pub fn generate_palette(seed: OKLCH, harmony: HarmonyType) -> Palette {
    let colors = match &harmony {
        HarmonyType::Complementary => {
            vec![gamut_safe(seed), gamut_safe(rotate_hue(seed, 180.0))]
        }

        HarmonyType::SplitComplementary => {
            vec![
                gamut_safe(seed),
                gamut_safe(rotate_hue(seed, 150.0)),
                gamut_safe(rotate_hue(seed, 210.0)),
            ]
        }

        HarmonyType::Triadic => {
            vec![
                gamut_safe(seed),
                gamut_safe(rotate_hue(seed, 120.0)),
                gamut_safe(rotate_hue(seed, 240.0)),
            ]
        }

        HarmonyType::Tetradic => {
            vec![
                gamut_safe(seed),
                gamut_safe(rotate_hue(seed, 90.0)),
                gamut_safe(rotate_hue(seed, 180.0)),
                gamut_safe(rotate_hue(seed, 270.0)),
            ]
        }

        HarmonyType::Analogous { spread } => {
            let s = spread.abs().clamp(5.0, 90.0);
            let n = 5usize;
            let step = (2.0 * s) / (n - 1) as f64;
            (0..n)
                .map(|i| {
                    let offset = -s + step * i as f64;
                    gamut_safe(rotate_hue(seed, offset))
                })
                .collect()
        }

        HarmonyType::Monochromatic { steps } => {
            let n = (*steps as usize).max(2).min(12);
            shades(seed, n as u8)
        }

        HarmonyType::Temperature { warm } => temperature_palette(*warm, 5),

        HarmonyType::Custom(offsets) => {
            let mut colors = vec![gamut_safe(seed)];
            for &offset in offsets {
                colors.push(gamut_safe(rotate_hue(seed, offset)));
            }
            colors
        }
    };

    let score = harmony_score(&colors);
    Palette {
        colors,
        harmony,
        score,
    }
}

/// Compute a harmony quality score for an existing palette.
///
/// score = hue_coherence × 0.4 + chroma_balance × 0.3 + lightness_spread × 0.3
///
/// All components in [0, 1]. Higher is more harmonious.
pub fn harmony_score(palette: &[OKLCH]) -> f64 {
    if palette.len() < 2 {
        return 1.0;
    }

    let hue_coherence = compute_hue_coherence(palette);
    let chroma_balance = compute_chroma_balance(palette);
    let lightness_spread = compute_lightness_spread(palette);

    (hue_coherence * 0.4 + chroma_balance * 0.3 + lightness_spread * 0.3).clamp(0.0, 1.0)
}

/// Generate an 11-step tonal scale (50–950) for the given base color.
///
/// Follows Tailwind / Material Design convention:
/// - L ranges from 0.97 (50) to 0.12 (950)
/// - Same hue throughout
/// - Chroma reduced at extremes (dark/light ends) for naturalism
///
/// # Arguments
/// * `base` — seed color; its hue and chroma are used
/// * `count` — number of shades to generate (2–11)
pub fn shades(base: OKLCH, count: u8) -> Vec<OKLCH> {
    let n = (count as usize).clamp(2, 11);
    let hue = base.h;

    // Lightness from ~0.97 to ~0.12 (inclusive)
    let l_max = 0.97f64;
    let l_min = 0.12f64;

    (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1) as f64; // 0 → 1
            let l = l_max - (l_max - l_min) * t;

            // Chroma: bell-curve peak near the base lightness, reduced at extremes
            let chroma_factor = 4.0 * t * (1.0 - t); // peaks at t=0.5
            let c = (base.c * chroma_factor).clamp(0.0, base.c);

            gamut_safe(OKLCH::new(l, c, hue))
        })
        .collect()
}

/// Generate a warm or cool tonal palette.
///
/// - Warm: hue range 0–60° (reds, oranges, yellows)
/// - Cool: hue range 180–270° (cyans, blues, violets)
///
/// Lightness and chroma are held near a perceptually comfortable mid-range.
///
/// # Arguments
/// * `warm` — `true` = warm palette, `false` = cool
/// * `count` — number of colors (2–8)
pub fn temperature_palette(warm: bool, count: u8) -> Vec<OKLCH> {
    let n = (count as usize).clamp(2, 8);

    let (h_start, h_end) = if warm {
        (0.0f64, 60.0f64) // red → yellow
    } else {
        (180.0f64, 270.0f64) // cyan → violet
    };

    let base_l = 0.65f64;
    let base_c = 0.14f64;

    (0..n)
        .map(|i| {
            let t = i as f64 / (n - 1) as f64;
            let hue = h_start + (h_end - h_start) * t;
            // Slight lightness variation for variety
            let l = base_l + 0.1 * (t - 0.5);
            gamut_safe(OKLCH::new(l, base_c, hue))
        })
        .collect()
}

// =============================================================================
// Extended palette utilities
// =============================================================================

/// Build a full design-system palette: shades + complementary accent + neutrals.
///
/// Returns colors in order:
/// `[shade_0, ..., shade_N, accent, neutral_light, neutral_mid, neutral_dark]`
pub fn design_system_palette(seed: OKLCH, shade_count: u8) -> Vec<OKLCH> {
    let mut colors = shades(seed, shade_count);

    // Complementary accent
    let accent = gamut_safe(rotate_hue(seed, 180.0));
    colors.push(accent);

    // Neutrals: desaturated variants of the seed hue
    let neutral_l_values = [0.95f64, 0.5, 0.15];
    for l in neutral_l_values {
        colors.push(gamut_safe(OKLCH::new(l, 0.02, seed.h)));
    }

    colors
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Rotate a color's hue by `delta` degrees, wrapping at 360°.
#[inline]
fn rotate_hue(c: OKLCH, delta: f64) -> OKLCH {
    OKLCH::new(c.l, c.c, (c.h + delta).rem_euclid(360.0))
}

/// Map an OKLCH color to the sRGB gamut, preserving L and H.
#[inline]
fn gamut_safe(c: OKLCH) -> OKLCH {
    c.map_to_gamut()
}

/// Compute hue coherence: how well the hue distribution matches the expected
/// harmonic pattern (variance-based).
///
/// Returns 1.0 for a perfectly uniform distribution, lower for random distributions.
fn compute_hue_coherence(palette: &[OKLCH]) -> f64 {
    if palette.len() < 2 {
        return 1.0;
    }

    let n = palette.len() as f64;
    let expected_step = 360.0 / n;

    // Sort hues
    let mut hues: Vec<f64> = palette.iter().map(|c| c.h).collect();
    hues.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Compute circular differences
    let mut gaps: Vec<f64> = Vec::with_capacity(hues.len());
    for i in 0..hues.len() {
        let next = hues[(i + 1) % hues.len()];
        let curr = hues[i];
        let gap = if next > curr {
            next - curr
        } else {
            next + 360.0 - curr
        };
        gaps.push(gap);
    }

    // Coefficient of variation of gaps vs expected
    let mean_gap = gaps.iter().sum::<f64>() / gaps.len() as f64;
    if mean_gap < 1e-10 {
        return 0.0;
    }

    let variance = gaps.iter().map(|g| (g - mean_gap).powi(2)).sum::<f64>() / gaps.len() as f64;
    let std_dev = variance.sqrt();
    let cv = std_dev / expected_step;

    // High CV = random hues = low coherence
    (1.0 - cv.min(1.0)).max(0.0)
}

/// Compute chroma balance: 1 - (std / mean) of chroma values.
///
/// A balanced palette has similar chroma across colors.
fn compute_chroma_balance(palette: &[OKLCH]) -> f64 {
    let chromas: Vec<f64> = palette.iter().map(|c| c.c).collect();
    let n = chromas.len() as f64;
    if n < 2.0 {
        return 1.0;
    }

    let mean = chromas.iter().sum::<f64>() / n;
    if mean < 1e-10 {
        return 1.0; // all grays → trivially balanced
    }

    let variance = chromas.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / n;
    let cv = variance.sqrt() / mean;

    (1.0 - cv.min(1.0)).max(0.0)
}

/// Compute lightness spread: normalised standard deviation of L values.
///
/// A wider spread is better for palettes (more tonal range = higher score).
fn compute_lightness_spread(palette: &[OKLCH]) -> f64 {
    let ls: Vec<f64> = palette.iter().map(|c| c.l).collect();
    let n = ls.len() as f64;
    if n < 2.0 {
        return 0.0;
    }

    let mean = ls.iter().sum::<f64>() / n;
    let variance = ls.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();

    // Normalise: std_dev of 0.3 across [0,1] is "perfect" spread
    (std_dev / 0.3).min(1.0)
}

// =============================================================================
// Conversion helpers (used by WASM layer)
// =============================================================================

/// Convert a hex string (e.g. "#ff5500") to OKLCH.
///
/// Returns `None` if the string cannot be parsed.
pub fn hex_to_oklch(hex: &str) -> Option<OKLCH> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

    let color = Color::from_srgb8(r, g, b);
    Some(OKLCH::from_color(&color))
}

/// Convert OKLCH to a hex string (e.g. "#ff5500").
pub fn oklch_to_hex(c: OKLCH) -> String {
    let color = c.to_color();
    let r = (color.srgb[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (color.srgb[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (color.srgb[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_seed() -> OKLCH {
        OKLCH::new(0.65, 0.15, 30.0)
    }

    #[test]
    fn test_complementary_angle() {
        let seed = make_seed();
        let palette = generate_palette(seed, HarmonyType::Complementary);
        assert_eq!(palette.colors.len(), 2);

        let h0 = palette.colors[0].h;
        let h1 = palette.colors[1].h;
        let delta = (h1 - h0).rem_euclid(360.0);
        assert!(
            (delta - 180.0).abs() < 0.01,
            "Complementary delta: {}",
            delta
        );
    }

    #[test]
    fn test_triadic_angles() {
        let seed = make_seed();
        let palette = generate_palette(seed, HarmonyType::Triadic);
        assert_eq!(palette.colors.len(), 3);

        let h0 = palette.colors[0].h;
        let h1 = palette.colors[1].h;
        let h2 = palette.colors[2].h;

        let d1 = (h1 - h0).rem_euclid(360.0);
        let d2 = (h2 - h0).rem_euclid(360.0);

        assert!((d1 - 120.0).abs() < 0.01, "Triadic d1: {}", d1);
        assert!((d2 - 240.0).abs() < 0.01, "Triadic d2: {}", d2);
    }

    #[test]
    fn test_tetradic_angles() {
        let seed = make_seed();
        let palette = generate_palette(seed, HarmonyType::Tetradic);
        assert_eq!(palette.colors.len(), 4);

        let hues: Vec<f64> = palette.colors.iter().map(|c| c.h).collect();
        for i in 1..4 {
            let delta = (hues[i] - hues[0]).rem_euclid(360.0);
            let expected = 90.0 * i as f64;
            assert!(
                (delta - expected).abs() < 0.01,
                "Tetradic color {}: delta {} vs expected {}",
                i,
                delta,
                expected
            );
        }
    }

    #[test]
    fn test_harmony_score_triadic_better_than_random() {
        let seed = make_seed();
        let triadic = generate_palette(seed, HarmonyType::Triadic);

        // Random-ish palette
        let random = vec![
            OKLCH::new(0.5, 0.15, 10.0),
            OKLCH::new(0.5, 0.15, 45.0),
            OKLCH::new(0.5, 0.15, 300.0),
        ];

        assert!(
            triadic.score >= harmony_score(&random),
            "Triadic score {} should be >= random score {}",
            triadic.score,
            harmony_score(&random)
        );
    }

    #[test]
    fn test_shades_monotone_lightness() {
        let seed = make_seed();
        let shds = shades(seed, 10);
        assert_eq!(shds.len(), 10);

        // Lightness should decrease monotonically
        for i in 1..shds.len() {
            assert!(
                shds[i].l <= shds[i - 1].l + 1e-10,
                "Shade lightness not monotone at index {}: {} > {}",
                i,
                shds[i].l,
                shds[i - 1].l
            );
        }
    }

    #[test]
    fn test_all_colors_in_gamut() {
        let seed = make_seed();
        for harmony in [
            HarmonyType::Complementary,
            HarmonyType::Triadic,
            HarmonyType::Tetradic,
            HarmonyType::Analogous { spread: 30.0 },
        ] {
            let palette = generate_palette(seed, harmony);
            for c in &palette.colors {
                let color = c.to_color();
                for ch in &color.srgb {
                    assert!(
                        *ch >= -0.01 && *ch <= 1.01,
                        "Color out of sRGB gamut: {:?}",
                        color.srgb
                    );
                }
            }
        }
    }

    #[test]
    fn test_hex_roundtrip() {
        let hex = "#3a7bd5";
        let oklch = hex_to_oklch(hex).unwrap();
        let back = oklch_to_hex(oklch);
        // Should be very close (small rounding error from 8-bit quantisation)
        let r1 = u8::from_str_radix(&hex[1..3], 16).unwrap();
        let r2 = u8::from_str_radix(&back[1..3], 16).unwrap();
        assert!(
            (r1 as i32 - r2 as i32).abs() <= 1,
            "Hex roundtrip error: {} vs {}",
            hex,
            back
        );
    }
}
