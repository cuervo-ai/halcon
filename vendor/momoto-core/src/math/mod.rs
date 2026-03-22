//! Mathematical utilities and constants.
//!
//! Provides numerical constants and utility functions used throughout
//! the color perception calculations.

/// Mathematical and physical constants used in color calculations.
pub mod constants {
    /// Euler's number
    pub const E: f64 = core::f64::consts::E;

    /// Pi
    pub const PI: f64 = core::f64::consts::PI;

    /// Minimum floating point value considered "different from zero"
    /// for contrast calculations.
    pub const EPSILON: f64 = 1e-10;
}

/// Clamps a value between a minimum and maximum.
///
/// # Examples
///
/// ```
/// use momoto_core::math::clamp;
///
/// assert_eq!(clamp(0.5, 0.0, 1.0), 0.5);
/// assert_eq!(clamp(-0.5, 0.0, 1.0), 0.0);
/// assert_eq!(clamp(1.5, 0.0, 1.0), 1.0);
/// ```
#[inline]
#[must_use]
pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Linear interpolation between two values.
///
/// # Arguments
///
/// * `a` - Start value
/// * `b` - End value
/// * `t` - Interpolation factor (0.0 to 1.0)
///
/// # Examples
///
/// ```
/// use momoto_core::math::lerp;
///
/// assert_eq!(lerp(0.0, 100.0, 0.5), 50.0);
/// assert_eq!(lerp(0.0, 100.0, 0.0), 0.0);
/// assert_eq!(lerp(0.0, 100.0, 1.0), 100.0);
/// ```
#[inline]
#[must_use]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Inverse linear interpolation - finds t such that lerp(a, b, t) = value.
///
/// Returns 0.0 if a == b.
///
/// # Examples
///
/// ```
/// use momoto_core::math::inverse_lerp;
///
/// assert!((inverse_lerp(0.0, 100.0, 50.0) - 0.5).abs() < 1e-10);
/// assert!((inverse_lerp(0.0, 100.0, 25.0) - 0.25).abs() < 1e-10);
/// assert!((inverse_lerp(0.0, 100.0, 75.0) - 0.75).abs() < 1e-10);
/// ```
#[inline]
#[must_use]
pub fn inverse_lerp(a: f64, b: f64, value: f64) -> f64 {
    if (b - a).abs() < constants::EPSILON {
        0.0
    } else {
        (value - a) / (b - a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp() {
        assert_eq!(clamp(0.5, 0.0, 1.0), 0.5);
        assert_eq!(clamp(-0.5, 0.0, 1.0), 0.0);
        assert_eq!(clamp(1.5, 0.0, 1.0), 1.0);
    }

    #[test]
    fn test_lerp() {
        assert_eq!(lerp(0.0, 100.0, 0.0), 0.0);
        assert_eq!(lerp(0.0, 100.0, 0.5), 50.0);
        assert_eq!(lerp(0.0, 100.0, 1.0), 100.0);
    }

    #[test]
    fn test_inverse_lerp() {
        assert!((inverse_lerp(0.0, 100.0, 50.0) - 0.5).abs() < 1e-10);
        assert!((inverse_lerp(0.0, 100.0, 0.0) - 0.0).abs() < 1e-10);
        assert!((inverse_lerp(0.0, 100.0, 100.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_inverse_lerp_zero_range() {
        // When a == b, should return 0
        assert_eq!(inverse_lerp(5.0, 5.0, 5.0), 0.0);
    }
}
