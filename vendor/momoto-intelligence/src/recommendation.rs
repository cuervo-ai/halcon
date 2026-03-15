//! Recommendation engine for optimal color selection.
//!
//! Provides intelligent color recommendations based on:
//! - Current color and context
//! - Quality scoring
//! - Perceptual adjustments

use crate::context::RecommendationContext;
use crate::scoring::{QualityScore, QualityScorer};
use momoto_core::color::Color;
use momoto_core::space::oklch::OKLCH;

/// A color recommendation with explanation.
#[derive(Debug, Clone)]
pub struct Recommendation {
    /// The recommended color
    pub color: Color,

    /// Quality score for this recommendation
    pub score: QualityScore,

    /// Confidence in this recommendation (0.0 to 1.0)
    pub confidence: f64,

    /// Human-readable reason for this recommendation
    pub reason: String,

    /// What was changed from the original (if applicable)
    pub modification: Option<Modification>,
}

/// Description of how a color was modified.
#[derive(Debug, Clone, PartialEq)]
pub enum Modification {
    /// Lightness was adjusted
    Lightness {
        /// Original lightness value
        from: f64,
        /// New lightness value
        to: f64,
        /// Change amount
        delta: f64,
    },

    /// Chroma was adjusted
    Chroma {
        /// Original chroma value
        from: f64,
        /// New chroma value
        to: f64,
        /// Change amount
        delta: f64,
    },

    /// Hue was adjusted
    Hue {
        /// Original hue value
        from: f64,
        /// New hue value
        to: f64,
        /// Change amount
        delta: f64,
    },

    /// Multiple adjustments
    Combined(Vec<Modification>),

    /// No modification needed
    None,
}

/// Recommendation engine for intelligent color suggestions.
#[derive(Debug)]
pub struct RecommendationEngine {
    scorer: QualityScorer,
}

impl RecommendationEngine {
    /// Create a new recommendation engine.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::recommendation::RecommendationEngine;
    ///
    /// let engine = RecommendationEngine::new();
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scorer: QualityScorer::new(),
        }
    }

    /// Recommend an optimal foreground color for the given background and context.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    /// use momoto_intelligence::context::RecommendationContext;
    /// use momoto_intelligence::recommendation::RecommendationEngine;
    ///
    /// let engine = RecommendationEngine::new();
    /// let white_bg = Color::from_srgb8(255, 255, 255);
    /// let context = RecommendationContext::body_text();
    ///
    /// let recommendation = engine.recommend_foreground(white_bg, context);
    /// assert!(recommendation.score.passes());
    /// assert!(recommendation.confidence > 0.8);
    /// ```
    #[must_use]
    pub fn recommend_foreground(
        &self,
        background: Color,
        context: RecommendationContext,
    ) -> Recommendation {
        // Convert background to OKLCH for perceptual manipulation
        let bg_oklch = OKLCH::from_color(&background);

        // Determine if background is light or dark
        let is_light_bg = bg_oklch.l > 0.5;

        // Generate candidate foreground color
        let candidate = if is_light_bg {
            // Dark foreground for light background
            self.generate_dark_foreground(bg_oklch, context)
        } else {
            // Light foreground for dark background
            self.generate_light_foreground(bg_oklch, context)
        };

        // Score the candidate
        let score = self.scorer.score(candidate, background, context);

        // Build recommendation
        let reason = self.build_reason(&score, is_light_bg, context);
        let confidence = self.calculate_confidence(&score);

        Recommendation {
            color: candidate,
            score,
            confidence,
            reason,
            modification: Some(Modification::None), // Calculated from scratch
        }
    }

    /// Recommend an improved version of the given foreground color.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    /// use momoto_intelligence::context::RecommendationContext;
    /// use momoto_intelligence::recommendation::RecommendationEngine;
    ///
    /// let engine = RecommendationEngine::new();
    /// let gray_fg = Color::from_srgb8(150, 150, 150);
    /// let white_bg = Color::from_srgb8(255, 255, 255);
    /// let context = RecommendationContext::body_text();
    ///
    /// let recommendation = engine.improve_foreground(gray_fg, white_bg, context);
    /// // Should recommend darker gray for better contrast
    /// assert!(recommendation.score.overall > 0.5);
    /// ```
    #[must_use]
    pub fn improve_foreground(
        &self,
        foreground: Color,
        background: Color,
        context: RecommendationContext,
    ) -> Recommendation {
        // Check if current combination is already good
        let current_score = self.scorer.score(foreground, background, context);

        if current_score.overall >= 0.9 {
            // Already excellent, return as-is
            return Recommendation {
                color: foreground,
                score: current_score,
                confidence: 1.0,
                reason: "Current color is already optimal".to_string(),
                modification: Some(Modification::None),
            };
        }

        // Convert to OKLCH for perceptual adjustments
        let fg_oklch = OKLCH::from_color(&foreground);
        let bg_oklch = OKLCH::from_color(&background);

        // Determine adjustment strategy
        let improved = self.adjust_for_better_contrast(fg_oklch, bg_oklch, context);

        // Score the improved version
        let improved_color = improved.to_color();
        let new_score = self.scorer.score(improved_color, background, context);

        // Calculate modification details
        let modification = self.calculate_modification(fg_oklch, improved);

        // Build reason
        let reason = self.build_improvement_reason(&modification, &current_score, &new_score);

        // Confidence based on improvement
        let improvement = new_score.overall - current_score.overall;
        let confidence = if improvement > 0.3 {
            1.0
        } else if improvement > 0.1 {
            0.8
        } else {
            0.6
        };

        Recommendation {
            color: improved_color,
            score: new_score,
            confidence,
            reason,
            modification: Some(modification),
        }
    }

    /// Generate a dark foreground for a light background.
    fn generate_dark_foreground(&self, bg: OKLCH, context: RecommendationContext) -> Color {
        // Start with a dark color (L=0.2) with low chroma to ensure readability
        let target_l = match context.usage {
            crate::context::UsageContext::BodyText => 0.15,
            crate::context::UsageContext::LargeText => 0.20,
            crate::context::UsageContext::Interactive => 0.15,
            _ => 0.25,
        };

        let fg_oklch = OKLCH::new(target_l, 0.02, bg.h);
        fg_oklch.to_color()
    }

    /// Generate a light foreground for a dark background.
    fn generate_light_foreground(&self, bg: OKLCH, context: RecommendationContext) -> Color {
        // Start with a light color (L=0.9) with low chroma
        let target_l = match context.usage {
            crate::context::UsageContext::BodyText => 0.95,
            crate::context::UsageContext::LargeText => 0.90,
            crate::context::UsageContext::Interactive => 0.95,
            _ => 0.85,
        };

        let fg_oklch = OKLCH::new(target_l, 0.02, bg.h);
        fg_oklch.to_color()
    }

    /// Adjust a foreground color for better contrast.
    fn adjust_for_better_contrast(
        &self,
        fg: OKLCH,
        bg: OKLCH,
        _context: RecommendationContext,
    ) -> OKLCH {
        // Determine if we need to go lighter or darker
        let l_diff = (fg.l - bg.l).abs();

        let target_l = if fg.l > bg.l {
            // Foreground is lighter, push it even lighter
            (fg.l + 0.3).min(0.98)
        } else {
            // Foreground is darker, push it even darker
            (fg.l - 0.3).max(0.05)
        };

        // Reduce chroma for better readability
        let target_c = if l_diff < 0.3 {
            // Low contrast, reduce chroma significantly
            (fg.c * 0.3).max(0.01)
        } else {
            // Adequate contrast, slightly reduce chroma
            (fg.c * 0.7).max(0.02)
        };

        OKLCH::new(target_l, target_c, fg.h)
    }

    /// Calculate what modifications were made.
    fn calculate_modification(&self, original: OKLCH, modified: OKLCH) -> Modification {
        let l_delta = modified.l - original.l;
        let c_delta = modified.c - original.c;
        let h_delta = modified.h - original.h;

        let mut mods = Vec::new();

        if l_delta.abs() > 0.01 {
            mods.push(Modification::Lightness {
                from: original.l,
                to: modified.l,
                delta: l_delta,
            });
        }

        if c_delta.abs() > 0.01 {
            mods.push(Modification::Chroma {
                from: original.c,
                to: modified.c,
                delta: c_delta,
            });
        }

        if h_delta.abs() > 1.0 {
            mods.push(Modification::Hue {
                from: original.h,
                to: modified.h,
                delta: h_delta,
            });
        }

        match mods.len() {
            0 => Modification::None,
            1 => mods.into_iter().next().unwrap(),
            _ => Modification::Combined(mods),
        }
    }

    /// Build explanation for the recommendation.
    fn build_reason(
        &self,
        score: &QualityScore,
        is_light_bg: bool,
        context: RecommendationContext,
    ) -> String {
        if score.passes() {
            format!(
                "Recommended {} foreground for {:?} context. Compliance: {:.0}%, Perceptual quality: {:.0}%",
                if is_light_bg { "dark" } else { "light" },
                context.usage,
                score.compliance * 100.0,
                score.perceptual * 100.0
            )
        } else {
            format!(
                "Could not find optimal color for {:?} context with current constraints",
                context.usage
            )
        }
    }

    /// Build explanation for an improvement.
    fn build_improvement_reason(
        &self,
        modification: &Modification,
        old_score: &QualityScore,
        new_score: &QualityScore,
    ) -> String {
        let improvement = ((new_score.overall - old_score.overall) * 100.0) as i32;

        match modification {
            Modification::Lightness { delta, .. } => {
                format!(
                    "Adjusted lightness by {:.0}% to improve contrast. Quality improved by {}%",
                    delta * 100.0,
                    improvement
                )
            }
            Modification::Chroma { delta, .. } => {
                format!(
                    "Reduced chroma by {:.0}% for better readability. Quality improved by {}%",
                    delta.abs() * 100.0,
                    improvement
                )
            }
            Modification::Combined(mods) => {
                format!(
                    "Made {} adjustments to optimize contrast. Quality improved by {}%",
                    mods.len(),
                    improvement
                )
            }
            Modification::None => "No adjustments needed".to_string(),
            Modification::Hue { .. } => {
                format!(
                    "Adjusted hue to improve perception. Quality improved by {}%",
                    improvement
                )
            }
        }
    }

    /// Calculate confidence in the recommendation.
    fn calculate_confidence(&self, score: &QualityScore) -> f64 {
        // Confidence is based on how well the recommendation meets requirements
        if score.overall >= 0.9 {
            1.0
        } else if score.overall >= 0.7 {
            0.9
        } else if score.overall >= 0.5 {
            0.7
        } else {
            0.5
        }
    }
}

impl Default for RecommendationEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recommend_foreground_light_bg() {
        let engine = RecommendationEngine::new();
        let white = Color::from_srgb8(255, 255, 255);
        let context = RecommendationContext::body_text();

        let rec = engine.recommend_foreground(white, context);

        // Should recommend dark foreground
        assert!(rec.score.passes());
        assert!(rec.confidence > 0.8);
        assert!(!rec.reason.is_empty());
    }

    #[test]
    fn test_recommend_foreground_dark_bg() {
        let engine = RecommendationEngine::new();
        let black = Color::from_srgb8(0, 0, 0);
        let context = RecommendationContext::body_text();

        let rec = engine.recommend_foreground(black, context);

        // Should recommend light foreground
        assert!(rec.score.passes());
        assert!(rec.confidence > 0.8);
    }

    #[test]
    fn test_improve_already_good() {
        let engine = RecommendationEngine::new();
        let black = Color::from_srgb8(0, 0, 0);
        let white = Color::from_srgb8(255, 255, 255);
        let context = RecommendationContext::body_text();

        let rec = engine.improve_foreground(black, white, context);

        // Black on white is excellent - should recognize as already optimal
        // or return a very high score
        assert!(rec.score.overall >= 0.8); // High quality score
        assert!(rec.score.passes()); // Passes compliance
        assert!(rec.confidence >= 0.6); // Good confidence
    }

    #[test]
    fn test_improve_poor_contrast() {
        let engine = RecommendationEngine::new();
        let light_gray = Color::from_srgb8(200, 200, 200);
        let white = Color::from_srgb8(255, 255, 255);
        let context = RecommendationContext::body_text();

        let rec = engine.improve_foreground(light_gray, white, context);

        // Should improve the color
        assert!(rec.score.overall > 0.5);
        assert!(rec.modification.is_some());
    }

    #[test]
    fn test_modification_detection() {
        let engine = RecommendationEngine::new();
        let original = OKLCH::new(0.7, 0.1, 180.0);
        let modified = OKLCH::new(0.5, 0.05, 180.0);

        let modification = engine.calculate_modification(original, modified);

        match modification {
            Modification::Combined(mods) => {
                assert_eq!(mods.len(), 2); // Lightness and chroma changed
            }
            _ => panic!("Expected combined modification"),
        }
    }
}
