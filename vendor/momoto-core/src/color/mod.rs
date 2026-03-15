//! Color representation and conversion.
//!
//! This module provides the core `Color` type which represents colors
//! in both sRGB (gamma-corrected) and linear RGB spaces, with alpha channel support.

pub mod cvd;
mod operations;

/// sRGB gamma correction transfer functions.
///
/// This module provides the standard sRGB transfer functions for converting
/// between gamma-corrected sRGB and linear RGB values.
///
/// # Background
///
/// The sRGB color space uses a non-linear transfer function (gamma curve)
/// that approximates human visual perception. Displays emit light linearly,
/// but human vision perceives brightness logarithmically. The sRGB gamma
/// curve compensates for this, making perceptual steps appear uniform.
///
/// # Mathematical Specification (IEC 61966-2-1:1999)
///
/// ```text
/// sRGB → Linear:
///   if sRGB ≤ 0.04045:  linear = sRGB / 12.92
///   else:               linear = ((sRGB + 0.055) / 1.055)^2.4
///
/// Linear → sRGB:
///   if linear ≤ 0.0031308:  sRGB = linear × 12.92
///   else:                   sRGB = 1.055 × linear^(1/2.4) - 0.055
/// ```
///
/// # Example
///
/// ```rust
/// use momoto_core::gamma::{srgb_to_linear, linear_to_srgb};
///
/// // Mid gray in sRGB
/// let srgb = 0.5;
/// let linear = srgb_to_linear(srgb);
///
/// // Linear is NOT 0.5 - it's about 0.214
/// assert!((linear - 0.214).abs() < 0.01);
///
/// // Roundtrip is exact
/// let back = linear_to_srgb(linear);
/// assert!((back - srgb).abs() < 0.0001);
/// ```
pub mod gamma {
    /// Converts an sRGB channel value (0.0-1.0) to linear RGB.
    ///
    /// Uses the standard sRGB transfer function (IEC 61966-2-1:1999):
    /// - For dark values (≤ 0.04045): linear scaling (`channel / 12.92`)
    /// - For bright values: power curve with γ ≈ 2.4
    ///
    /// # Arguments
    ///
    /// * `channel` - sRGB channel value in [0.0, 1.0]
    ///
    /// # Returns
    ///
    /// Linear RGB channel value in [0.0, 1.0]
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::gamma::srgb_to_linear;
    ///
    /// // Black stays black
    /// assert_eq!(srgb_to_linear(0.0), 0.0);
    ///
    /// // White stays white
    /// assert!((srgb_to_linear(1.0) - 1.0).abs() < 0.0001);
    ///
    /// // Mid gray (sRGB 0.5) → linear ~0.214
    /// assert!((srgb_to_linear(0.5) - 0.214).abs() < 0.01);
    /// ```
    #[inline]
    #[must_use]
    pub fn srgb_to_linear(channel: f64) -> f64 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Converts a linear RGB channel value (0.0-1.0) to sRGB.
    ///
    /// Uses the inverse sRGB transfer function (IEC 61966-2-1:1999):
    /// - For dark values (≤ 0.0031308): linear scaling (`channel * 12.92`)
    /// - For bright values: power curve with γ ≈ 1/2.4
    ///
    /// # Arguments
    ///
    /// * `channel` - Linear RGB channel value in [0.0, 1.0]
    ///
    /// # Returns
    ///
    /// sRGB channel value in [0.0, 1.0]
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::gamma::linear_to_srgb;
    ///
    /// // Black stays black
    /// assert_eq!(linear_to_srgb(0.0), 0.0);
    ///
    /// // White stays white
    /// assert!((linear_to_srgb(1.0) - 1.0).abs() < 0.0001);
    ///
    /// // Linear 0.214 → sRGB ~0.5
    /// assert!((linear_to_srgb(0.214) - 0.5).abs() < 0.01);
    /// ```
    #[inline]
    #[must_use]
    pub fn linear_to_srgb(channel: f64) -> f64 {
        if channel <= 0.0031308 {
            channel * 12.92
        } else {
            1.055 * channel.powf(1.0 / 2.4) - 0.055
        }
    }
}

use core::fmt;

/// A color represented in both sRGB and linear RGB color spaces.
///
/// All channels are stored as `f64` in the range [0.0, 1.0].
///
/// # Design
///
/// We store both sRGB and linear representations to avoid repeated
/// gamma conversions. This is a space-time tradeoff optimized for
/// typical usage patterns where colors are created once and used many times.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// sRGB channels (gamma-corrected, 0.0-1.0)
    pub srgb: [f64; 3],
    /// Linear RGB channels (0.0-1.0)
    pub linear: [f64; 3],
    /// Alpha channel (0.0 = transparent, 1.0 = opaque)
    pub alpha: f64,
}

impl Color {
    /// Creates a color from 8-bit sRGB values (0-255).
    ///
    /// This is the most common constructor for colors from CSS, design tools, etc.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    ///
    /// let orange = Color::from_srgb8(255, 128, 0);
    /// ```
    #[inline]
    #[must_use]
    pub fn from_srgb8(r: u8, g: u8, b: u8) -> Self {
        let srgb = [
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
        ];

        let linear = [
            gamma::srgb_to_linear(srgb[0]),
            gamma::srgb_to_linear(srgb[1]),
            gamma::srgb_to_linear(srgb[2]),
        ];

        Self {
            srgb,
            linear,
            alpha: 1.0,
        }
    }

    /// Creates a color from normalized sRGB values (0.0-1.0).
    #[inline]
    #[must_use]
    pub fn from_srgb(r: f64, g: f64, b: f64) -> Self {
        let srgb = [r, g, b];
        let linear = [
            gamma::srgb_to_linear(r),
            gamma::srgb_to_linear(g),
            gamma::srgb_to_linear(b),
        ];

        Self {
            srgb,
            linear,
            alpha: 1.0,
        }
    }

    /// Creates a color from linear RGB values (0.0-1.0).
    #[inline]
    #[must_use]
    pub fn from_linear(r: f64, g: f64, b: f64) -> Self {
        let linear = [r, g, b];
        let srgb = [
            gamma::linear_to_srgb(r),
            gamma::linear_to_srgb(g),
            gamma::linear_to_srgb(b),
        ];

        Self {
            srgb,
            linear,
            alpha: 1.0,
        }
    }

    /// Returns the sRGB representation as 8-bit values (0-255).
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    ///
    /// let color = Color::from_srgb(0.5, 0.25, 0.75);
    /// let [r, g, b] = color.to_srgb8();
    /// assert_eq!(r, 128);
    /// assert_eq!(g, 64);
    /// assert_eq!(b, 191);
    /// ```
    #[inline]
    #[must_use]
    pub fn to_srgb8(&self) -> [u8; 3] {
        [
            (self.srgb[0] * 255.0).round() as u8,
            (self.srgb[1] * 255.0).round() as u8,
            (self.srgb[2] * 255.0).round() as u8,
        ]
    }

    /// Converts to OKLCH color space.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    ///
    /// let red = Color::from_srgb8(255, 0, 0);
    /// let oklch = red.to_oklch();
    /// assert!(oklch.l > 0.6); // Red is relatively bright
    /// assert!(oklch.c > 0.2); // Red is saturated
    /// ```
    #[must_use]
    pub fn to_oklch(&self) -> crate::space::oklch::OKLCH {
        crate::space::oklch::OKLCH::from_color(self)
    }

    /// Creates a color from OKLCH coordinates.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    ///
    /// let cyan = Color::from_oklch(0.7, 0.15, 180.0);
    /// let [r, g, b] = cyan.to_srgb8();
    /// // Cyan should have more blue and green than red
    /// assert!(b > r || g > r);
    /// ```
    #[must_use]
    pub fn from_oklch(l: f64, c: f64, h: f64) -> Self {
        let oklch = crate::space::oklch::OKLCH::new(l, c, h);
        let mut color = oklch.to_color();
        color.alpha = 1.0; // Default to opaque
        color
    }

    /// Creates a color from a hex string (e.g., "#FF8000" or "FF8000").
    ///
    /// Accepts 6-character hex strings with or without the leading `#`.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    ///
    /// let orange = Color::from_hex("#FF8000").unwrap();
    /// assert_eq!(orange.to_srgb8(), [255, 128, 0]);
    ///
    /// let blue = Color::from_hex("0080FF").unwrap();
    /// assert_eq!(blue.to_srgb8(), [0, 128, 255]);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The string (after stripping `#`) is not exactly 6 characters
    /// - The string contains non-hexadecimal characters
    pub fn from_hex(hex: &str) -> Result<Self, String> {
        let hex = hex.trim_start_matches('#');

        if hex.len() != 6 {
            return Err(format!("Hex color must be 6 characters, got {}", hex.len()));
        }

        let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| {
            format!(
                "Invalid hex color: could not parse red channel '{}'",
                &hex[0..2]
            )
        })?;
        let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| {
            format!(
                "Invalid hex color: could not parse green channel '{}'",
                &hex[2..4]
            )
        })?;
        let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| {
            format!(
                "Invalid hex color: could not parse blue channel '{}'",
                &hex[4..6]
            )
        })?;

        Ok(Self::from_srgb8(r, g, b))
    }

    /// Converts the color to a hex string (e.g., "#FF8000").
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    ///
    /// let orange = Color::from_srgb8(255, 128, 0);
    /// assert_eq!(orange.to_hex(), "#FF8000");
    /// ```
    #[must_use]
    pub fn to_hex(&self) -> String {
        let [r, g, b] = self.to_srgb8();
        format!("#{:02X}{:02X}{:02X}", r, g, b)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.to_srgb8();
        write!(f, "rgb({}, {}, {})", r, g, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_black() {
        let black = Color::from_srgb8(0, 0, 0);
        assert_eq!(black.srgb, [0.0, 0.0, 0.0]);
        assert_eq!(black.linear, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_white() {
        let white = Color::from_srgb8(255, 255, 255);
        assert_eq!(white.srgb, [1.0, 1.0, 1.0]);
        assert_eq!(white.linear, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn test_roundtrip_srgb8() {
        let original = [128, 64, 192];
        let color = Color::from_srgb8(original[0], original[1], original[2]);
        let roundtrip = color.to_srgb8();
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn test_gamma_correction() {
        let mid_gray = Color::from_srgb8(128, 128, 128);
        // Mid gray (128) in sRGB is NOT 0.5 in linear space
        // It's approximately 0.2158 due to gamma correction
        assert!((mid_gray.linear[0] - 0.2158).abs() < 0.01);
    }

    #[test]
    fn test_from_hex_with_hash() {
        let color = Color::from_hex("#FF8000").unwrap();
        assert_eq!(color.to_srgb8(), [255, 128, 0]);
    }

    #[test]
    fn test_from_hex_without_hash() {
        let color = Color::from_hex("FF8000").unwrap();
        assert_eq!(color.to_srgb8(), [255, 128, 0]);
    }

    #[test]
    fn test_from_hex_lowercase() {
        let color = Color::from_hex("ff8000").unwrap();
        assert_eq!(color.to_srgb8(), [255, 128, 0]);
    }

    #[test]
    fn test_from_hex_invalid_length() {
        assert!(Color::from_hex("FF80").is_err());
        assert!(Color::from_hex("#FF800").is_err());
        assert!(Color::from_hex("FF80000").is_err());
    }

    #[test]
    fn test_from_hex_invalid_chars() {
        assert!(Color::from_hex("GGGGGG").is_err());
        assert!(Color::from_hex("ZZZZZZ").is_err());
    }

    #[test]
    fn test_to_hex() {
        let color = Color::from_srgb8(255, 128, 0);
        assert_eq!(color.to_hex(), "#FF8000");
    }

    #[test]
    fn test_hex_roundtrip() {
        let original = Color::from_srgb8(100, 150, 200);
        let hex = original.to_hex();
        let restored = Color::from_hex(&hex).unwrap();
        assert_eq!(original.to_srgb8(), restored.to_srgb8());
    }

    #[test]
    fn test_hex_black_white() {
        let black = Color::from_hex("#000000").unwrap();
        assert_eq!(black.to_srgb8(), [0, 0, 0]);

        let white = Color::from_hex("#FFFFFF").unwrap();
        assert_eq!(white.to_srgb8(), [255, 255, 255]);
    }
}
