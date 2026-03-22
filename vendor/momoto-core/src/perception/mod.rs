//! Perceptual primitives and result types.
//!
//! This module defines the core abstractions for contrast metrics
//! and perceptual results.

use crate::color::Color;

/// Result of a perceptual contrast calculation.
///
/// This is a generic result type that can be used by any contrast metric.
/// Different metrics may have different value ranges and interpretations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerceptualResult {
    /// The raw contrast value.
    ///
    /// Interpretation depends on the metric:
    /// - APCA: Lc value (-108.0 to 106.0)
    /// - WCAG 2.x: Contrast ratio (1.0 to 21.0)
    pub value: f64,

    /// Polarity of the contrast.
    ///
    /// Some metrics (like APCA) are polarity-aware:
    /// - `Positive`: Dark text on light background
    /// - `Negative`: Light text on dark background
    /// - `None`: Metric is not polarity-aware
    pub polarity: Option<Polarity>,

    /// Additional metadata specific to the metric.
    pub metadata: Option<&'static str>,
}

/// Polarity of a contrast calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// Dark text on light background (positive)
    DarkOnLight,
    /// Light text on dark background (negative)
    LightOnDark,
}

impl PerceptualResult {
    /// Creates a new perceptual result.
    #[inline]
    #[must_use]
    pub fn new(value: f64) -> Self {
        Self {
            value,
            polarity: None,
            metadata: None,
        }
    }

    /// Creates a new perceptual result with polarity.
    #[inline]
    #[must_use]
    pub fn with_polarity(value: f64, polarity: Polarity) -> Self {
        Self {
            value,
            polarity: Some(polarity),
            metadata: None,
        }
    }

    /// Returns the absolute value of the contrast.
    #[inline]
    #[must_use]
    pub fn abs(&self) -> f64 {
        self.value.abs()
    }
}

/// Trait for contrast metrics.
///
/// This is the core abstraction that allows different contrast algorithms
/// (APCA, WCAG, SAPC, etc.) to be used interchangeably.
///
/// # Design Principles
///
/// 1. **Batch-first**: `evaluate_batch` is the primary method
/// 2. **Allocation-aware**: Batch methods take slices and return `Vec`
/// 3. **SIMD-ready**: Implementations should be vectorizable
/// 4. **Deterministic**: Same inputs always produce same outputs
///
/// # Examples
///
/// ```
/// use momoto_core::color::Color;
/// use momoto_core::perception::ContrastMetric;
///
/// fn check_contrast<M: ContrastMetric>(metric: &M, fg: Color, bg: Color) {
///     let result = metric.evaluate(fg, bg);
///     println!("Contrast: {}", result.value);
/// }
/// ```
pub trait ContrastMetric {
    /// Evaluates contrast between a single foreground and background.
    ///
    /// This is a convenience method. For performance-critical code,
    /// use `evaluate_batch` instead.
    fn evaluate(&self, foreground: Color, background: Color) -> PerceptualResult;

    /// Evaluates contrast for multiple foreground/background pairs.
    ///
    /// This is the primary method for performance-critical code.
    /// Implementations should optimize this for batch processing.
    ///
    /// # Arguments
    ///
    /// * `foregrounds` - Slice of foreground colors
    /// * `backgrounds` - Slice of background colors
    ///
    /// # Panics
    ///
    /// Panics if the slices have different lengths.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let fgs = vec![Color::from_srgb8(0, 0, 0); 100];
    /// let bgs = vec![Color::from_srgb8(255, 255, 255); 100];
    /// let results = metric.evaluate_batch(&fgs, &bgs);
    /// ```
    fn evaluate_batch(
        &self,
        foregrounds: &[Color],
        backgrounds: &[Color],
    ) -> Vec<PerceptualResult> {
        assert_eq!(
            foregrounds.len(),
            backgrounds.len(),
            "Foreground and background slices must have the same length"
        );

        foregrounds
            .iter()
            .zip(backgrounds.iter())
            .map(|(&fg, &bg)| self.evaluate(fg, bg))
            .collect()
    }

    /// Returns the name of this metric.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use momoto_core::perception::ContrastMetric;
    /// use momoto_metrics::wcag::WCAGMetric;
    ///
    /// let metric = WCAGMetric::new();
    /// assert_eq!(metric.name(), "WCAG2");
    /// ```
    fn name(&self) -> &'static str;

    /// Returns the version of this metric implementation.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use momoto_core::perception::ContrastMetric;
    /// use momoto_metrics::apca::APCAMetric;
    ///
    /// let metric = APCAMetric::new();
    /// assert_eq!(metric.version(), "0.1.9");
    /// ```
    fn version(&self) -> &'static str {
        "1.0.0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perceptual_result_abs() {
        let positive = PerceptualResult::new(50.0);
        assert_eq!(positive.abs(), 50.0);

        let negative = PerceptualResult::new(-50.0);
        assert_eq!(negative.abs(), 50.0);
    }

    #[test]
    fn test_polarity() {
        let dark_on_light = PerceptualResult::with_polarity(50.0, Polarity::DarkOnLight);
        assert_eq!(dark_on_light.polarity, Some(Polarity::DarkOnLight));

        let light_on_dark = PerceptualResult::with_polarity(-50.0, Polarity::LightOnDark);
        assert_eq!(light_on_dark.polarity, Some(Polarity::LightOnDark));
    }
}
