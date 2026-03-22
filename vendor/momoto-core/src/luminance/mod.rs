//! Luminance calculations and gamma correction.
//!
//! Provides functions to calculate relative luminance from colors
//! according to various standards (sRGB, APCA, etc.).

use crate::color::Color;

/// Relative luminance (Y) of a color.
///
/// This is a newtype wrapper to ensure type safety and prevent
/// mixing luminance values with raw floats.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct RelativeLuminance(pub f64);

impl RelativeLuminance {
    /// Creates a new relative luminance value.
    ///
    /// # Panics
    ///
    /// Panics in debug mode if the value is not in [0.0, 1.0].
    #[inline]
    #[must_use]
    pub fn new(value: f64) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&value),
            "Luminance must be in [0.0, 1.0], got {}",
            value
        );
        Self(value)
    }

    /// Returns the luminance value as f64.
    #[inline]
    #[must_use]
    pub fn value(self) -> f64 {
        self.0
    }
}

/// Calculates relative luminance using sRGB coefficients.
///
/// Uses the ITU-R BT.709 coefficients:
/// - R: 0.2126
/// - G: 0.7152
/// - B: 0.0722
///
/// This is the standard formula used by WCAG 2.x.
///
/// # Examples
///
/// ```
/// use momoto_core::color::Color;
/// use momoto_core::luminance::relative_luminance_srgb;
///
/// let white = Color::from_srgb8(255, 255, 255);
/// let y = relative_luminance_srgb(&white);
/// assert_eq!(y.value(), 1.0);
/// ```
///
/// # References
///
/// - ITU-R BT.709
/// - WCAG 2.1 contrast formula
#[inline]
#[must_use]
pub fn relative_luminance_srgb(color: &Color) -> RelativeLuminance {
    const R_COEF: f64 = 0.2126;
    const G_COEF: f64 = 0.7152;
    const B_COEF: f64 = 0.0722;

    let y = R_COEF * color.linear[0] + G_COEF * color.linear[1] + B_COEF * color.linear[2];

    RelativeLuminance::new(y)
}

/// Calculates relative luminance using APCA-specific coefficients.
///
/// APCA uses slightly different coefficients for better perceptual accuracy:
/// - R: 0.2126729
/// - G: 0.7151522
/// - B: 0.0721750
///
/// # Examples
///
/// ```
/// use momoto_core::color::Color;
/// use momoto_core::luminance::relative_luminance_apca;
///
/// let color = Color::from_srgb8(128, 128, 128);
/// let y = relative_luminance_apca(&color);
/// ```
///
/// # References
///
/// - APCA-W3 0.1.9
#[inline]
#[must_use]
pub fn relative_luminance_apca(color: &Color) -> RelativeLuminance {
    const R_COEF: f64 = 0.2126729;
    const G_COEF: f64 = 0.7151522;
    const B_COEF: f64 = 0.0721750;

    let y = R_COEF * color.linear[0] + G_COEF * color.linear[1] + B_COEF * color.linear[2];

    RelativeLuminance::new(y)
}

/// Applies soft clamp for very dark colors.
///
/// This is used by APCA to prevent issues near absolute black.
/// Colors with Y < threshold get clamped using a power curve.
///
/// # Arguments
///
/// * `y` - Relative luminance value
/// * `threshold` - Clamp threshold (typically 0.022)
/// * `exponent` - Clamp exponent (typically 1.414)
///
/// # Examples
///
/// ```
/// use momoto_core::luminance::{RelativeLuminance, soft_clamp};
///
/// let y = RelativeLuminance::new(0.01);
/// let clamped = soft_clamp(y, 0.022, 1.414);
/// assert!(clamped.value() >= y.value());
/// ```
#[inline]
#[must_use]
pub fn soft_clamp(y: RelativeLuminance, threshold: f64, exponent: f64) -> RelativeLuminance {
    let value = if y.0 <= threshold {
        y.0 + (threshold - y.0).powf(exponent)
    } else {
        y.0
    };

    RelativeLuminance::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_luminance_black() {
        let black = Color::from_srgb8(0, 0, 0);
        let y = relative_luminance_srgb(&black);
        assert_eq!(y.value(), 0.0);
    }

    #[test]
    fn test_luminance_white() {
        let white = Color::from_srgb8(255, 255, 255);
        let y = relative_luminance_srgb(&white);
        assert_eq!(y.value(), 1.0);
    }

    #[test]
    fn test_luminance_red() {
        let red = Color::from_srgb8(255, 0, 0);
        let y = relative_luminance_srgb(&red);
        // Red contributes 21.26% to luminance
        assert!((y.value() - 0.2126).abs() < 0.01);
    }

    #[test]
    fn test_luminance_green() {
        let green = Color::from_srgb8(0, 255, 0);
        let y = relative_luminance_srgb(&green);
        // Green contributes 71.52% to luminance (dominant)
        assert!((y.value() - 0.7152).abs() < 0.01);
    }

    #[test]
    fn test_soft_clamp_above_threshold() {
        let y = RelativeLuminance::new(0.5);
        let clamped = soft_clamp(y, 0.022, 1.414);
        assert_eq!(clamped.value(), 0.5);
    }

    #[test]
    fn test_soft_clamp_below_threshold() {
        let y = RelativeLuminance::new(0.01);
        let clamped = soft_clamp(y, 0.022, 1.414);
        // Should be boosted above original
        assert!(clamped.value() > y.value());
    }

    #[test]
    fn test_apca_vs_srgb_coefficients() {
        let color = Color::from_srgb8(128, 128, 128);
        let y_srgb = relative_luminance_srgb(&color);
        let y_apca = relative_luminance_apca(&color);
        // Should be very close but not identical
        assert!((y_srgb.value() - y_apca.value()).abs() < 0.001);
    }
}
