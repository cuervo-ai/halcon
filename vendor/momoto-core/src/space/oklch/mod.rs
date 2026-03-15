//! OKLCH Color Space Implementation
//!
//! OKLCH is a perceptually uniform color space by Björn Ottosson.
//! It's a cylindrical representation of OKLab (L, a, b → L, C, H).
//!
//! # Properties
//!
//! - **L (Lightness)**: 0.0 (black) to 1.0 (white)
//! - **C (Chroma)**: 0.0 (gray) to ~0.4 (practical maximum, varies by hue)
//! - **H (Hue)**: 0.0 to 360.0 degrees
//!
//! # References
//!
//! - [Oklab Color Space](https://bottosson.github.io/posts/oklab/)
//! - [OKLCH Specification](https://www.w3.org/TR/css-color-4/#ok-lab)

use crate::color::Color;

// ============================================================================
// OKLab Transformation Matrices (from Björn Ottosson's paper)
// ============================================================================

/// RGB to LMS matrix (cone response).
///
/// Transforms linear RGB to LMS cone responses using Björn Ottosson's
/// optimized matrix coefficients for perceptually uniform color space.
///
/// # Matrix Structure
/// ```text
/// [L]   [0.4122  0.5363  0.0514] [R]
/// [M] = [0.2119  0.6807  0.1074] [G]
/// [S]   [0.0883  0.2817  0.6300] [B]
/// ```
pub const RGB_TO_LMS: [[f64; 3]; 3] = [
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
];

/// LMS to OKLab matrix.
///
/// Transforms cube-root LMS values to OKLab (L, a, b) coordinates.
/// The cube root non-linearity approximates the human visual system's
/// response to luminance.
///
/// # Matrix Structure
/// ```text
/// [L]   [0.2105   0.7936  -0.0041] [l^(1/3)]
/// [a] = [1.9780  -2.4286   0.4506] [m^(1/3)]
/// [b]   [0.0259   0.7828  -0.8087] [s^(1/3)]
/// ```
pub const LMS_TO_LAB: [[f64; 3]; 3] = [
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
];

/// OKLab to LMS matrix (inverse).
///
/// Transforms OKLab (L, a, b) back to cube-root LMS values.
/// After this transformation, cube each component to get linear LMS.
pub const LAB_TO_LMS: [[f64; 3]; 3] = [
    [1.0, 0.3963377774, 0.2158037573],
    [1.0, -0.1055613458, -0.0638541728],
    [1.0, -0.0894841775, -1.2914855480],
];

/// LMS to RGB matrix (inverse).
///
/// Transforms linear LMS cone responses back to linear RGB values.
/// This is the inverse of `RGB_TO_LMS`.
pub const LMS_TO_RGB: [[f64; 3]; 3] = [
    [4.0767416621, -3.3077115913, 0.2309699292],
    [-1.2684380046, 2.6097574011, -0.3413193965],
    [-0.0041960863, -0.7034186147, 1.7076147010],
];

/// Gamut boundary approximation coefficients for sRGB.
///
/// These coefficients enable fast estimation of the maximum achievable
/// chroma for any lightness/hue combination within the sRGB gamut.
///
/// # Format
///
/// Each entry is `(hue_degrees, (a, b))` where:
/// - `hue_degrees`: Hue angle in degrees (0-360)
/// - `a`, `b`: Parabolic coefficients
///
/// Maximum chroma is estimated as: `max_c ≈ a * L * (1 - L) + b`
///
/// # Example
///
/// ```rust
/// use momoto_core::gamut::GAMUT_COEFFICIENTS;
///
/// // Find coefficients for red hue (0°)
/// let red_coef = GAMUT_COEFFICIENTS.iter()
///     .find(|(h, _)| *h == 0)
///     .map(|(_, c)| *c)
///     .unwrap();
/// assert_eq!(red_coef, (0.28, 0.02));
/// ```
pub const GAMUT_COEFFICIENTS: [(u16, (f64, f64)); 12] = [
    (0, (0.28, 0.02)),   // Red
    (30, (0.30, 0.02)),  // Orange
    (60, (0.32, 0.02)),  // Yellow
    (90, (0.24, 0.02)),  // Yellow-Green
    (120, (0.22, 0.02)), // Green
    (150, (0.18, 0.02)), // Cyan-Green
    (180, (0.16, 0.02)), // Cyan
    (210, (0.14, 0.02)), // Blue-Cyan
    (240, (0.16, 0.02)), // Blue
    (270, (0.20, 0.02)), // Violet
    (300, (0.24, 0.02)), // Magenta
    (330, (0.26, 0.02)), // Red-Magenta
];

// ============================================================================
// OKLab Intermediate Representation
// ============================================================================

/// OKLab color representation (Cartesian coordinates)
///
/// This is an intermediate format used for converting between RGB and OKLCH.
///
/// # Examples
///
/// ```
/// use momoto_core::space::oklch::OKLab;
/// use momoto_core::color::Color;
///
/// let red = Color::from_srgb8(255, 0, 0);
/// let lab = OKLab::from_color(&red);
/// assert!(lab.l > 0.6 && lab.l < 0.7);
/// assert!(lab.a > 0.2);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OKLab {
    /// Lightness (0.0 to 1.0)
    pub l: f64,
    /// Green-red axis
    pub a: f64,
    /// Blue-yellow axis
    pub b: f64,
}

impl OKLab {
    /// Create new OKLab color
    #[inline]
    #[must_use]
    pub const fn new(l: f64, a: f64, b: f64) -> Self {
        Self { l, a, b }
    }

    /// Convert from sRGB color
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLab;
    /// use momoto_core::color::Color;
    ///
    /// let white = Color::from_srgb8(255, 255, 255);
    /// let lab = OKLab::from_color(&white);
    /// assert!((lab.l - 1.0).abs() < 0.01);
    /// assert!(lab.a.abs() < 0.01);
    /// assert!(lab.b.abs() < 0.01);
    /// ```
    #[must_use]
    pub fn from_color(color: &Color) -> Self {
        // Use linear RGB from Color (already computed)
        let r = color.linear[0];
        let g = color.linear[1];
        let b = color.linear[2];

        // RGB to LMS (cone response)
        let l = RGB_TO_LMS[0][0] * r + RGB_TO_LMS[0][1] * g + RGB_TO_LMS[0][2] * b;
        let m = RGB_TO_LMS[1][0] * r + RGB_TO_LMS[1][1] * g + RGB_TO_LMS[1][2] * b;
        let s = RGB_TO_LMS[2][0] * r + RGB_TO_LMS[2][1] * g + RGB_TO_LMS[2][2] * b;

        // Apply cube root (perceptual transformation)
        let l_ = l.cbrt();
        let m_ = m.cbrt();
        let s_ = s.cbrt();

        // LMS to Lab
        Self {
            l: LMS_TO_LAB[0][0] * l_ + LMS_TO_LAB[0][1] * m_ + LMS_TO_LAB[0][2] * s_,
            a: LMS_TO_LAB[1][0] * l_ + LMS_TO_LAB[1][1] * m_ + LMS_TO_LAB[1][2] * s_,
            b: LMS_TO_LAB[2][0] * l_ + LMS_TO_LAB[2][1] * m_ + LMS_TO_LAB[2][2] * s_,
        }
    }

    /// Convert to sRGB color
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLab;
    /// use momoto_core::color::Color;
    ///
    /// let lab = OKLab::new(0.5, 0.1, 0.05);
    /// let color = lab.to_color();
    /// let [r, g, b] = color.to_srgb8();
    /// assert!(r > 0 && r < 255);
    /// ```
    #[must_use]
    pub fn to_color(&self) -> Color {
        // Lab to LMS (inverse)
        let l_ = LAB_TO_LMS[0][0] * self.l + LAB_TO_LMS[0][1] * self.a + LAB_TO_LMS[0][2] * self.b;
        let m_ = LAB_TO_LMS[1][0] * self.l + LAB_TO_LMS[1][1] * self.a + LAB_TO_LMS[1][2] * self.b;
        let s_ = LAB_TO_LMS[2][0] * self.l + LAB_TO_LMS[2][1] * self.a + LAB_TO_LMS[2][2] * self.b;

        // Cube (inverse of cube root)
        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        // LMS to RGB (linear)
        let r = LMS_TO_RGB[0][0] * l + LMS_TO_RGB[0][1] * m + LMS_TO_RGB[0][2] * s;
        let g = LMS_TO_RGB[1][0] * l + LMS_TO_RGB[1][1] * m + LMS_TO_RGB[1][2] * s;
        let b = LMS_TO_RGB[2][0] * l + LMS_TO_RGB[2][1] * m + LMS_TO_RGB[2][2] * s;

        Color::from_linear(r, g, b)
    }
}

// ============================================================================
// OKLCH Color Space
// ============================================================================

/// OKLCH color representation (Cylindrical coordinates)
///
/// Perceptually uniform color space with intuitive controls:
/// - Lightness: how light/dark
/// - Chroma: how colorful/gray
/// - Hue: which color
///
/// # Examples
///
/// ```
/// use momoto_core::space::oklch::OKLCH;
/// use momoto_core::color::Color;
///
/// // Create from LCH values
/// let color = OKLCH::new(0.5, 0.1, 180.0);
///
/// // Convert from RGB
/// let rgb = Color::from_srgb8(255, 0, 0);
/// let oklch = OKLCH::from_color(&rgb);
///
/// // Modify
/// let lighter = oklch.lighten(0.1);
/// let more_saturated = oklch.saturate(1.5);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OKLCH {
    /// Lightness: 0.0 (black) to 1.0 (white)
    pub l: f64,
    /// Chroma: 0.0 (gray) to ~0.4 (practical max)
    pub c: f64,
    /// Hue: 0.0 to 360.0 degrees
    pub h: f64,
}

impl OKLCH {
    /// Create new OKLCH color
    ///
    /// Values are clamped/normalized:
    /// - L: clamped to [0.0, 1.0]
    /// - C: clamped to [0.0, ∞) (but practical max ~0.4)
    /// - H: normalized to [0.0, 360.0)
    #[must_use]
    pub fn new(l: f64, c: f64, h: f64) -> Self {
        Self {
            l: l.clamp(0.0, 1.0),
            c: c.max(0.0),
            h: normalize_hue(h),
        }
    }

    /// Convert from sRGB color
    #[must_use]
    pub fn from_color(color: &Color) -> Self {
        let lab = OKLab::from_color(color);

        // Cartesian to polar
        let c = (lab.a * lab.a + lab.b * lab.b).sqrt();
        let h = lab.b.atan2(lab.a).to_degrees();

        Self::new(lab.l, c, h)
    }

    /// Convert to OKLab
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLCH;
    ///
    /// let oklch = OKLCH::new(0.7, 0.15, 180.0);
    /// let lab = oklch.to_oklab();
    /// assert_eq!(lab.l, 0.7);
    /// assert!(lab.a < 0.0); // Cyan has negative a
    /// assert!(lab.b.abs() < 0.01); // Near zero b
    /// ```
    #[must_use]
    pub fn to_oklab(&self) -> OKLab {
        let h_rad = self.h.to_radians();
        OKLab {
            l: self.l,
            a: self.c * h_rad.cos(),
            b: self.c * h_rad.sin(),
        }
    }

    /// Convert to sRGB color
    ///
    /// May produce out-of-gamut values. Use `map_to_gamut()` first if needed.
    #[must_use]
    pub fn to_color(&self) -> Color {
        self.to_oklab().to_color()
    }

    // ============================================
    // Transformations (Immutable)
    // ============================================

    /// Create new OKLCH with modified lightness
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLCH;
    ///
    /// let color = OKLCH::new(0.5, 0.1, 180.0);
    /// let lighter = color.with_lightness(0.8);
    /// assert_eq!(lighter.l, 0.8);
    /// assert_eq!(lighter.c, 0.1);
    /// assert_eq!(lighter.h, 180.0);
    /// ```
    #[inline]
    #[must_use]
    pub fn with_lightness(self, l: f64) -> Self {
        Self::new(l, self.c, self.h)
    }

    /// Create new OKLCH with modified chroma
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLCH;
    ///
    /// let color = OKLCH::new(0.5, 0.1, 180.0);
    /// let saturated = color.with_chroma(0.2);
    /// assert_eq!(saturated.l, 0.5);
    /// assert_eq!(saturated.c, 0.2);
    /// assert_eq!(saturated.h, 180.0);
    /// ```
    #[inline]
    #[must_use]
    pub fn with_chroma(self, c: f64) -> Self {
        Self::new(self.l, c, self.h)
    }

    /// Create new OKLCH with modified hue
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLCH;
    ///
    /// let color = OKLCH::new(0.5, 0.1, 180.0);
    /// let rotated = color.with_hue(270.0);
    /// assert_eq!(rotated.l, 0.5);
    /// assert_eq!(rotated.c, 0.1);
    /// assert_eq!(rotated.h, 270.0);
    /// ```
    #[inline]
    #[must_use]
    pub fn with_hue(self, h: f64) -> Self {
        Self::new(self.l, self.c, h)
    }

    /// Lighten by delta (additive)
    #[inline]
    #[must_use]
    pub fn lighten(self, delta: f64) -> Self {
        Self::new(self.l + delta, self.c, self.h)
    }

    /// Darken by delta (subtractive)
    #[inline]
    #[must_use]
    pub fn darken(self, delta: f64) -> Self {
        Self::new(self.l - delta, self.c, self.h)
    }

    /// Saturate by factor (multiplicative)
    #[inline]
    #[must_use]
    pub fn saturate(self, factor: f64) -> Self {
        Self::new(self.l, self.c * factor, self.h)
    }

    /// Desaturate by factor (divisive)
    #[inline]
    #[must_use]
    pub fn desaturate(self, factor: f64) -> Self {
        Self::new(self.l, self.c / factor, self.h)
    }

    /// Rotate hue by degrees
    #[inline]
    #[must_use]
    pub fn rotate_hue(self, degrees: f64) -> Self {
        Self::new(self.l, self.c, self.h + degrees)
    }

    // ============================================
    // Gamut Operations
    // ============================================

    /// Estimate maximum chroma for current L and H in sRGB gamut
    ///
    /// Uses interpolated coefficients for speed.
    /// Parabolic approximation: max_c ≈ a * L * (1 - L) + b
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLCH;
    ///
    /// let color = OKLCH::new(0.5, 0.1, 180.0);
    /// let max_c = color.estimate_max_chroma();
    /// assert!(max_c > 0.05 && max_c < 0.15);
    /// ```
    #[must_use]
    pub fn estimate_max_chroma(&self) -> f64 {
        // Find surrounding hue coefficients
        let mut lower_h = 0;
        let mut upper_h = 360;

        for &(h, _) in &GAMUT_COEFFICIENTS {
            let h = f64::from(h);
            if h <= self.h && h > f64::from(lower_h) {
                lower_h = h as u16;
            }
            if h > self.h && h < f64::from(upper_h) {
                upper_h = h as u16;
            }
        }

        if upper_h == 360 {
            upper_h = 0;
        }

        // Get coefficients
        let lower_coef = GAMUT_COEFFICIENTS
            .iter()
            .find(|(h, _)| *h == lower_h)
            .map(|(_, c)| *c)
            .unwrap_or((0.2, 0.02));

        let upper_coef = GAMUT_COEFFICIENTS
            .iter()
            .find(|(h, _)| *h == upper_h)
            .map(|(_, c)| *c)
            .unwrap_or((0.2, 0.02));

        // Interpolate
        let t = if upper_h != lower_h {
            (self.h - f64::from(lower_h)) / (f64::from(upper_h) - f64::from(lower_h))
        } else {
            0.0
        };

        let a = lower_coef.0 + t * (upper_coef.0 - lower_coef.0);
        let b = lower_coef.1 + t * (upper_coef.1 - lower_coef.1);

        // Parabolic approximation
        a * self.l * (1.0 - self.l) + b
    }

    /// Check if color is approximately within sRGB gamut
    ///
    /// Uses fast estimation. For precise checking, convert to RGB.
    #[must_use]
    pub fn is_in_gamut(&self) -> bool {
        if self.l <= 0.0 || self.l >= 1.0 {
            return self.c < 0.001;
        }
        self.c <= self.estimate_max_chroma() * 1.1 // 10% tolerance
    }

    /// Clamp to sRGB gamut by reducing chroma
    ///
    /// Preserves hue and lightness.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLCH;
    ///
    /// let out_of_gamut = OKLCH::new(0.5, 0.5, 180.0);
    /// let clamped = out_of_gamut.clamp_to_gamut();
    /// assert!(clamped.c < 0.5);
    /// assert_eq!(clamped.l, 0.5);
    /// assert_eq!(clamped.h, 180.0);
    /// ```
    #[must_use]
    pub fn clamp_to_gamut(&self) -> Self {
        let max_c = self.estimate_max_chroma();
        if self.c <= max_c {
            *self
        } else {
            Self::new(self.l, max_c, self.h)
        }
    }

    /// Map to gamut using chroma reduction
    ///
    /// Uses binary search to find maximum in-gamut chroma.
    /// More accurate than `clamp_to_gamut()`.
    #[must_use]
    pub fn map_to_gamut(&self) -> Self {
        // Check if already in gamut
        let rgb = self.to_color();

        // Check linear RGB values (before clamping to u8)
        if is_in_gamut_rgb(&rgb.linear) {
            return *self;
        }

        // Binary search for maximum in-gamut chroma
        let mut low = 0.0;
        let mut high = self.c;
        let mut result = Self::new(self.l, 0.0, self.h);

        for _ in 0..15 {
            // 15 iterations for precision
            let mid = (low + high) / 2.0;
            let test = Self::new(self.l, mid, self.h);
            let test_rgb = test.to_color();

            if is_in_gamut_rgb(&test_rgb.linear) {
                result = test;
                low = mid;
            } else {
                high = mid;
            }
        }

        result
    }

    /// Map to gamut using high-precision binary search (Lindbloom boundary projection).
    ///
    /// More accurate than `map_to_gamut()` — uses 25 iterations (precision ~1e-8 in chroma).
    /// Preserves lightness and hue exactly; only reduces chroma.
    ///
    /// Use this when accuracy is critical (e.g. gamut mapping for print/display profiles).
    /// For interactive use where performance matters, prefer `map_to_gamut()`.
    ///
    /// # Guarantees
    /// - `|L_out - L_in| < 1e-10`
    /// - `|H_out - H_in| < 1e-10`
    /// - `C_out ≤ C_in`
    /// - Result is in sRGB gamut: all channels in [0, 1]
    #[must_use]
    pub fn map_to_gamut_precise(&self) -> Self {
        // Fast path: already in gamut
        if is_in_gamut_rgb(&self.to_color().linear) {
            return *self;
        }

        // Achromatic edge case
        if self.c < 1e-10 {
            return Self::new(self.l.clamp(0.0, 1.0), 0.0, self.h);
        }

        // Binary search in chroma [0, self.c] — 25 iterations ≈ 1e-8 precision
        let mut low = 0.0f64;
        let mut high = self.c;
        let mut result = Self::new(self.l, 0.0, self.h);

        for _ in 0..25 {
            let mid = (low + high) * 0.5;
            let test = Self::new(self.l, mid, self.h);
            if is_in_gamut_rgb(&test.to_color().linear) {
                result = test;
                low = mid;
            } else {
                high = mid;
            }
        }

        result
    }

    // ============================================
    // Analysis
    // ============================================

    /// Calculate perceptual difference (Delta E in OKLCH)
    ///
    /// Lower values indicate more similar colors.
    #[must_use]
    pub fn delta_e(&self, other: &Self) -> f64 {
        let dl = self.l - other.l;
        let dc = self.c - other.c;

        // Hue difference (handling wraparound)
        let mut dh = other.h - self.h;
        if dh > 180.0 {
            dh -= 360.0;
        }
        if dh < -180.0 {
            dh += 360.0;
        }

        // Convert hue difference to Cartesian
        let dh_rad = dh.to_radians();
        let dh_cart = 2.0 * (self.c * other.c).sqrt() * (dh_rad / 2.0).sin();

        (dl * dl + dc * dc + dh_cart * dh_cart).sqrt()
    }

    /// Check if colors are perceptually similar
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::space::oklch::OKLCH;
    ///
    /// let color1 = OKLCH::new(0.5, 0.1, 180.0);
    /// let color2 = OKLCH::new(0.51, 0.1, 180.0);
    /// assert!(color1.is_similar_to(&color2, 0.05));
    /// assert!(!color1.is_similar_to(&color2, 0.001));
    /// ```
    #[inline]
    #[must_use]
    pub fn is_similar_to(&self, other: &Self, threshold: f64) -> bool {
        self.delta_e(other) < threshold
    }

    // ============================================
    // Static Utilities
    // ============================================

    /// Interpolate between two OKLCH colors
    ///
    /// # Arguments
    ///
    /// * `a` - Start color
    /// * `b` - End color
    /// * `t` - Interpolation factor (0.0 to 1.0)
    /// * `hue_path` - How to interpolate hue ('shorter' or 'longer')
    #[must_use]
    pub fn interpolate(a: &Self, b: &Self, t: f64, hue_path: HuePath) -> Self {
        // Linear interpolation for L and C
        let l = a.l + t * (b.l - a.l);
        let c = a.c + t * (b.c - a.c);

        // Hue interpolation with path selection
        let mut h_diff = b.h - a.h;

        // Normalize difference
        if h_diff > 180.0 {
            h_diff -= 360.0;
        }
        if h_diff < -180.0 {
            h_diff += 360.0;
        }

        let h = match hue_path {
            HuePath::Shorter => a.h + t * h_diff,
            HuePath::Longer => {
                let adjusted_diff = if h_diff > 0.0 {
                    h_diff - 360.0
                } else {
                    h_diff + 360.0
                };
                a.h + t * adjusted_diff
            }
        };

        Self::new(l, c, h)
    }
}

// ============================================================================
// Helper Types
// ============================================================================

/// Hue interpolation path
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HuePath {
    /// Take the shorter path around the hue circle
    Shorter,
    /// Take the longer path around the hue circle
    Longer,
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Normalize hue to [0.0, 360.0)
#[inline]
fn normalize_hue(h: f64) -> f64 {
    ((h % 360.0) + 360.0) % 360.0
}

/// Check if linear RGB values are in valid gamut [0.0, 1.0]
#[inline]
fn is_in_gamut_rgb(linear_rgb: &[f64; 3]) -> bool {
    linear_rgb[0] >= 0.0
        && linear_rgb[0] <= 1.0
        && linear_rgb[1] >= 0.0
        && linear_rgb[1] <= 1.0
        && linear_rgb[2] >= 0.0
        && linear_rgb[2] <= 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_red_to_oklch() {
        let red = Color::from_srgb8(255, 0, 0);
        let oklch = OKLCH::from_color(&red);

        // Red should have:
        // - High lightness (~0.6)
        // - High chroma (~0.3)
        // - Hue around 30° (red-orange)
        assert!(oklch.l > 0.5 && oklch.l < 0.7, "Red L: {}", oklch.l);
        assert!(oklch.c > 0.2, "Red C: {}", oklch.c);
        assert!(oklch.h > 20.0 && oklch.h < 40.0, "Red H: {}", oklch.h);
    }

    #[test]
    fn test_roundtrip() {
        let original = Color::from_srgb8(128, 64, 192);
        let oklch = OKLCH::from_color(&original);
        let back = oklch.to_color();
        let [r, g, b] = back.to_srgb8();

        // Should be very close (within 1-2 due to rounding)
        assert!((r as i16 - 128).abs() <= 2);
        assert!((g as i16 - 64).abs() <= 2);
        assert!((b as i16 - 192).abs() <= 2);
    }

    #[test]
    fn test_lighten_darken() {
        let color = OKLCH::new(0.5, 0.1, 180.0);

        let lighter = color.lighten(0.1);
        assert!((lighter.l - 0.6).abs() < 0.001);

        let darker = color.darken(0.1);
        assert!((darker.l - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_saturate_desaturate() {
        let color = OKLCH::new(0.5, 0.1, 180.0);

        let saturated = color.saturate(2.0);
        assert!((saturated.c - 0.2).abs() < 0.001);

        let desaturated = color.desaturate(2.0);
        assert!((desaturated.c - 0.05).abs() < 0.001);
    }

    #[test]
    fn test_hue_normalization() {
        assert!((normalize_hue(390.0) - 30.0).abs() < 0.001);
        assert!((normalize_hue(-30.0) - 330.0).abs() < 0.001);
        assert!((normalize_hue(720.0) - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_interpolation() {
        let a = OKLCH::new(0.3, 0.1, 0.0);
        let b = OKLCH::new(0.7, 0.2, 180.0);

        let mid = OKLCH::interpolate(&a, &b, 0.5, HuePath::Shorter);

        assert!((mid.l - 0.5).abs() < 0.001);
        assert!((mid.c - 0.15).abs() < 0.001);
        assert!((mid.h - 90.0).abs() < 0.001);
    }

    #[test]
    fn test_delta_e() {
        let a = OKLCH::new(0.5, 0.1, 0.0);
        let b = OKLCH::new(0.5, 0.1, 0.0);

        // Same color = zero distance
        assert!(a.delta_e(&b) < 0.001);

        let c = OKLCH::new(0.6, 0.1, 0.0);
        // Different lightness
        assert!(a.delta_e(&c) > 0.05);
    }

    #[test]
    fn test_gamut_estimation() {
        // Red at mid-lightness (L=0.5, H=0)
        // max_c = 0.28 * 0.5 * 0.5 + 0.02 = 0.09
        let red = OKLCH::new(0.5, 0.1, 0.0);
        let max_c = red.estimate_max_chroma();
        assert!(
            max_c > 0.08 && max_c < 0.11,
            "Expected ~0.09, got {}",
            max_c
        );

        // Yellow at mid-lightness (L=0.5, H=60)
        // max_c = 0.32 * 0.5 * 0.5 + 0.02 = 0.10
        let yellow = OKLCH::new(0.5, 0.1, 60.0);
        let max_c_yellow = yellow.estimate_max_chroma();
        assert!(
            max_c_yellow > 0.09 && max_c_yellow < 0.12,
            "Expected ~0.10, got {}",
            max_c_yellow
        );

        // Cyan at mid-lightness (L=0.5, H=180) - lower chroma
        // max_c = 0.16 * 0.5 * 0.5 + 0.02 = 0.06
        let cyan = OKLCH::new(0.5, 0.05, 180.0);
        let max_c_cyan = cyan.estimate_max_chroma();
        assert!(
            max_c_cyan > 0.05 && max_c_cyan < 0.08,
            "Expected ~0.06, got {}",
            max_c_cyan
        );
    }

    // ============================================
    // Golden Vector Tests
    // Validated against TypeScript implementation
    // ============================================

    #[test]
    fn golden_white_black() {
        // White: RGB(255, 255, 255) → OKLCH(1.0, 0, *)
        let white = Color::from_srgb8(255, 255, 255);
        let oklch_white = OKLCH::from_color(&white);
        assert!(
            (oklch_white.l - 1.0).abs() < 0.01,
            "White L: {}",
            oklch_white.l
        );
        assert!(oklch_white.c < 0.01, "White C: {}", oklch_white.c);

        // Black: RGB(0, 0, 0) → OKLCH(0.0, 0, *)
        let black = Color::from_srgb8(0, 0, 0);
        let oklch_black = OKLCH::from_color(&black);
        assert!(oklch_black.l < 0.01, "Black L: {}", oklch_black.l);
        assert!(oklch_black.c < 0.01, "Black C: {}", oklch_black.c);
    }

    #[test]
    fn golden_primary_colors() {
        // Pure Red: RGB(255, 0, 0)
        let red = Color::from_srgb8(255, 0, 0);
        let oklch_red = OKLCH::from_color(&red);
        assert!((oklch_red.l - 0.628).abs() < 0.01, "Red L: {}", oklch_red.l);
        assert!((oklch_red.c - 0.257).abs() < 0.01, "Red C: {}", oklch_red.c);
        assert!((oklch_red.h - 29.2).abs() < 2.0, "Red H: {}", oklch_red.h);

        // Pure Green: RGB(0, 255, 0)
        let green = Color::from_srgb8(0, 255, 0);
        let oklch_green = OKLCH::from_color(&green);
        assert!(
            (oklch_green.l - 0.866).abs() < 0.01,
            "Green L: {}",
            oklch_green.l
        );
        assert!(
            (oklch_green.c - 0.295).abs() < 0.02,
            "Green C: {}",
            oklch_green.c
        );
        assert!(
            (oklch_green.h - 142.5).abs() < 5.0,
            "Green H: {}",
            oklch_green.h
        );

        // Pure Blue: RGB(0, 0, 255)
        let blue = Color::from_srgb8(0, 0, 255);
        let oklch_blue = OKLCH::from_color(&blue);
        assert!(
            (oklch_blue.l - 0.452).abs() < 0.01,
            "Blue L: {}",
            oklch_blue.l
        );
        assert!(
            (oklch_blue.c - 0.313).abs() < 0.02,
            "Blue C: {}",
            oklch_blue.c
        );
        assert!(
            (oklch_blue.h - 264.0).abs() < 5.0,
            "Blue H: {}",
            oklch_blue.h
        );
    }

    #[test]
    fn golden_grayscale() {
        // Mid gray: RGB(128, 128, 128)
        let gray = Color::from_srgb8(128, 128, 128);
        let oklch_gray = OKLCH::from_color(&gray);
        assert!(
            (oklch_gray.l - 0.600).abs() < 0.02,
            "Gray L: {}",
            oklch_gray.l
        );
        assert!(
            oklch_gray.c < 0.01,
            "Gray should have near-zero chroma: {}",
            oklch_gray.c
        );

        // Dark gray: RGB(64, 64, 64)
        let dark_gray = Color::from_srgb8(64, 64, 64);
        let oklch_dark = OKLCH::from_color(&dark_gray);
        assert!(
            (oklch_dark.l - 0.380).abs() < 0.02,
            "Dark gray L: {}",
            oklch_dark.l
        );
        assert!(oklch_dark.c < 0.01, "Dark gray chroma: {}", oklch_dark.c);
    }

    #[test]
    fn golden_map_to_gamut() {
        // Out of gamut color: very high chroma
        let out_of_gamut = OKLCH::new(0.5, 0.5, 180.0);
        let mapped = out_of_gamut.map_to_gamut();

        // Chroma should be reduced
        assert!(mapped.c < 0.5, "Mapped chroma: {}", mapped.c);
        assert!(
            mapped.c < 0.15,
            "Chroma should be well under limit: {}",
            mapped.c
        );

        // Lightness and hue should be preserved (approximately)
        assert!(
            (mapped.l - 0.5).abs() < 0.02,
            "Lightness preserved: {}",
            mapped.l
        );
        assert!(
            (mapped.h - 180.0).abs() < 5.0,
            "Hue preserved: {}",
            mapped.h
        );
    }

    #[test]
    fn golden_interpolation_shorter() {
        // Interpolate red (h=350) to yellow (h=10)
        let a = OKLCH::new(0.5, 0.1, 350.0);
        let b = OKLCH::new(0.5, 0.1, 10.0);

        let mid = OKLCH::interpolate(&a, &b, 0.5, HuePath::Shorter);

        // Shorter path goes through 0 (360)
        assert!(
            mid.h < 5.0 || mid.h > 355.0,
            "Midpoint should be near 0°: {}",
            mid.h
        );
    }

    #[test]
    fn golden_interpolation_longer() {
        // Interpolate close hues the long way
        let a = OKLCH::new(0.5, 0.1, 10.0);
        let b = OKLCH::new(0.5, 0.1, 50.0);

        let mid = OKLCH::interpolate(&a, &b, 0.5, HuePath::Longer);

        // Longer path goes through 180°
        assert!(
            mid.h > 150.0 && mid.h < 250.0,
            "Long path midpoint should be opposite: {}",
            mid.h
        );
    }

    #[test]
    fn golden_extreme_lightness() {
        // Very dark colors have low max chroma
        let dark = OKLCH::new(0.1, 0.1, 180.0);
        let max_c_dark = dark.estimate_max_chroma();
        assert!(max_c_dark < 0.05, "Dark max chroma: {}", max_c_dark);

        // Very light colors have low max chroma
        let light = OKLCH::new(0.9, 0.1, 180.0);
        let max_c_light = light.estimate_max_chroma();
        assert!(max_c_light < 0.05, "Light max chroma: {}", max_c_light);

        // Mid-lightness has higher max chroma
        let mid = OKLCH::new(0.5, 0.1, 180.0);
        let max_c_mid = mid.estimate_max_chroma();
        assert!(max_c_mid > max_c_dark && max_c_mid > max_c_light);
    }

    #[test]
    fn golden_hue_wraparound() {
        // Test hue normalization edge cases
        let over = OKLCH::new(0.5, 0.1, 450.0);
        assert!((over.h - 90.0).abs() < 0.001, "450° → 90°");

        let negative = OKLCH::new(0.5, 0.1, -90.0);
        assert!((negative.h - 270.0).abs() < 0.001, "-90° → 270°");

        let double = OKLCH::new(0.5, 0.1, 720.0);
        assert!((double.h - 0.0).abs() < 0.001, "720° → 0°");
    }

    #[test]
    fn golden_cyan_conversion() {
        // Cyan: H=180, negative a, ~0 b
        let cyan = OKLCH::new(0.7, 0.15, 180.0);
        let lab = cyan.to_oklab();

        assert_eq!(lab.l, 0.7);
        assert!(lab.a < 0.0, "Cyan should have negative a: {}", lab.a);
        assert!(
            lab.b.abs() < 0.01,
            "Cyan should have near-zero b: {}",
            lab.b
        );
    }

    #[test]
    fn golden_roundtrip_precision() {
        // Test various colors for roundtrip precision
        let colors = [
            (255, 0, 0),     // Red
            (0, 255, 0),     // Green
            (0, 0, 255),     // Blue
            (255, 255, 0),   // Yellow
            (255, 0, 255),   // Magenta
            (0, 255, 255),   // Cyan
            (128, 128, 128), // Gray
            (59, 130, 246),  // Blue-500
        ];

        for (r, g, b) in colors {
            let original = Color::from_srgb8(r, g, b);
            let oklch = OKLCH::from_color(&original);
            let back = oklch.to_color();
            let [r2, g2, b2] = back.to_srgb8();

            // Allow 2-unit tolerance for rounding
            assert!(
                (r2 as i16 - r as i16).abs() <= 2,
                "R mismatch: {} → {} (via OKLCH)",
                r,
                r2
            );
            assert!(
                (g2 as i16 - g as i16).abs() <= 2,
                "G mismatch: {} → {} (via OKLCH)",
                g,
                g2
            );
            assert!(
                (b2 as i16 - b as i16).abs() <= 2,
                "B mismatch: {} → {} (via OKLCH)",
                b,
                b2
            );
        }
    }
}
