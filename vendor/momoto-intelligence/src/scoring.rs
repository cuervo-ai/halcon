//! Scoring system for color combination quality.
//!
//! Provides deterministic scoring based on:
//! - Contrast compliance (WCAG, APCA)
//! - Perceptual quality (not too high, not too low)
//! - Context appropriateness

use crate::context::{ComplianceTarget, RecommendationContext, UsageContext};
use momoto_core::color::Color;
use momoto_core::perception::{ContrastMetric, PerceptualResult};
use momoto_metrics::apca::APCAMetric;
use momoto_metrics::wcag::WCAGMetric;

/// Score for a color combination (0.0 to 1.0).
///
/// - 1.0 = Perfect for the context
/// - 0.8-0.9 = Excellent
/// - 0.6-0.7 = Good
/// - 0.4-0.5 = Acceptable
/// - 0.0-0.3 = Poor/Fails requirements
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityScore {
    /// Overall quality score (0.0 to 1.0)
    pub overall: f64,

    /// Compliance score (0.0 = fails, 1.0 = exceeds)
    pub compliance: f64,

    /// Perceptual quality score (0.0 = poor, 1.0 = optimal)
    pub perceptual: f64,

    /// Context appropriateness score (0.0 = inappropriate, 1.0 = perfect fit)
    pub appropriateness: f64,
}

impl QualityScore {
    /// Create a new quality score.
    #[must_use]
    pub const fn new(overall: f64, compliance: f64, perceptual: f64, appropriateness: f64) -> Self {
        Self {
            overall,
            compliance,
            perceptual,
            appropriateness,
        }
    }

    /// Returns whether this score indicates the combination passes requirements.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::scoring::QualityScore;
    ///
    /// let good = QualityScore::new(0.8, 1.0, 0.8, 0.8);
    /// assert!(good.passes());
    ///
    /// let bad = QualityScore::new(0.3, 0.5, 0.6, 0.5);
    /// assert!(!bad.passes());
    /// ```
    #[must_use]
    pub fn passes(self) -> bool {
        self.compliance >= 1.0
    }

    /// Returns a qualitative assessment of the score.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::scoring::QualityScore;
    ///
    /// let excellent = QualityScore::new(0.95, 1.0, 0.95, 0.95);
    /// assert_eq!(excellent.assessment(), "Excellent");
    ///
    /// let poor = QualityScore::new(0.2, 0.3, 0.4, 0.3);
    /// assert_eq!(poor.assessment(), "Poor");
    /// ```
    #[must_use]
    pub fn assessment(self) -> &'static str {
        if self.overall >= 0.9 {
            "Excellent"
        } else if self.overall >= 0.7 {
            "Good"
        } else if self.overall >= 0.5 {
            "Acceptable"
        } else if self.overall >= 0.3 {
            "Marginal"
        } else {
            "Poor"
        }
    }
}

/// Scorer for evaluating color combination quality.
#[derive(Debug)]
pub struct QualityScorer {
    wcag: WCAGMetric,
    apca: APCAMetric,
}

impl QualityScorer {
    /// Create a new quality scorer.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::scoring::QualityScorer;
    ///
    /// let scorer = QualityScorer::new();
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self {
            wcag: WCAGMetric,
            apca: APCAMetric,
        }
    }

    /// Score a color combination for a given context.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::color::Color;
    /// use momoto_intelligence::context::RecommendationContext;
    /// use momoto_intelligence::scoring::QualityScorer;
    ///
    /// let scorer = QualityScorer::new();
    /// let black = Color::from_srgb8(0, 0, 0);
    /// let white = Color::from_srgb8(255, 255, 255);
    /// let context = RecommendationContext::body_text();
    ///
    /// let score = scorer.score(black, white, context);
    /// assert!(score.passes());
    /// assert!(score.overall > 0.8);
    /// ```
    #[must_use]
    pub fn score(
        &self,
        foreground: Color,
        background: Color,
        context: RecommendationContext,
    ) -> QualityScore {
        // Evaluate metrics
        let wcag_result = self.wcag.evaluate(foreground, background);
        let apca_result = self.apca.evaluate(foreground, background);

        // Calculate sub-scores
        let compliance = self.score_compliance(&wcag_result, &apca_result, context);
        let perceptual = self.score_perceptual_quality(&wcag_result, &apca_result, context);
        let appropriateness = self.score_appropriateness(&wcag_result, &apca_result, context);

        // Overall score is weighted average
        // Compliance is most important (40%), perceptual quality (35%), appropriateness (25%)
        let overall = compliance * 0.4 + perceptual * 0.35 + appropriateness * 0.25;

        QualityScore::new(overall, compliance, perceptual, appropriateness)
    }

    /// Score compliance with requirements.
    fn score_compliance(
        &self,
        wcag_result: &PerceptualResult,
        apca_result: &PerceptualResult,
        context: RecommendationContext,
    ) -> f64 {
        match context.target {
            ComplianceTarget::WCAG_AA => {
                let required = context.usage.min_wcag_aa();
                if required == 0.0 {
                    return 1.0; // No requirement
                }
                Self::compliance_score(wcag_result.value, required)
            }
            ComplianceTarget::WCAG_AAA => {
                let required = context.usage.min_wcag_aaa();
                if required == 0.0 {
                    return 1.0;
                }
                Self::compliance_score(wcag_result.value, required)
            }
            ComplianceTarget::APCA => {
                let required = context.usage.min_apca_lc();
                if required == 0.0 {
                    return 1.0;
                }
                Self::compliance_score(apca_result.value.abs(), required)
            }
            ComplianceTarget::Hybrid => {
                let wcag_score = {
                    let required = context.usage.min_wcag_aa();
                    if required == 0.0 {
                        1.0
                    } else {
                        Self::compliance_score(wcag_result.value, required)
                    }
                };
                let apca_score = {
                    let required = context.usage.min_apca_lc();
                    if required == 0.0 {
                        1.0
                    } else {
                        Self::compliance_score(apca_result.value.abs(), required)
                    }
                };
                // Both must pass for hybrid
                wcag_score.min(apca_score)
            }
        }
    }

    /// Calculate compliance score for a value vs requirement.
    fn compliance_score(actual: f64, required: f64) -> f64 {
        if actual >= required * 1.5 {
            1.0 // Significantly exceeds
        } else if actual >= required * 1.2 {
            0.95 // Comfortably exceeds
        } else if actual >= required {
            0.9 // Meets requirement
        } else if actual >= required * 0.9 {
            0.7 // Close but fails
        } else if actual >= required * 0.7 {
            0.4 // Notably below
        } else {
            0.0 // Far below
        }
    }

    /// Score perceptual quality (avoiding extremes).
    fn score_perceptual_quality(
        &self,
        wcag_result: &PerceptualResult,
        _apca_result: &PerceptualResult,
        context: RecommendationContext,
    ) -> f64 {
        // For most contexts, we want good contrast but not maximum
        // Maximum contrast (21:1) can be harsh on the eyes
        let ratio = wcag_result.value;

        match context.usage {
            UsageContext::BodyText | UsageContext::Interactive => {
                // Optimal range: 7:1 to 15:1
                if (7.0..=15.0).contains(&ratio) {
                    1.0
                } else if ratio > 15.0 {
                    // Too high - reduce score gradually
                    1.0 - ((ratio - 15.0) / 10.0).min(0.3)
                } else if ratio >= 4.5 {
                    // Between minimum and optimal
                    0.7 + (ratio - 4.5) / (7.0 - 4.5) * 0.3
                } else {
                    // Below minimum
                    ratio / 4.5 * 0.5
                }
            }
            UsageContext::LargeText => {
                // Optimal range: 4.5:1 to 12:1
                if (4.5..=12.0).contains(&ratio) {
                    1.0
                } else if ratio > 12.0 {
                    1.0 - ((ratio - 12.0) / 10.0).min(0.3)
                } else {
                    ratio / 4.5 * 0.7
                }
            }
            UsageContext::Decorative | UsageContext::Disabled => {
                // Lower contrast is acceptable
                if (2.0..=10.0).contains(&ratio) {
                    1.0
                } else if ratio > 10.0 {
                    0.8
                } else {
                    ratio / 2.0 * 0.8
                }
            }
            UsageContext::IconsGraphics => {
                // Similar to large text
                if (3.0..=10.0).contains(&ratio) {
                    1.0
                } else if ratio > 10.0 {
                    0.9
                } else {
                    ratio / 3.0 * 0.7
                }
            }
        }
    }

    /// Score appropriateness for context.
    fn score_appropriateness(
        &self,
        wcag_result: &PerceptualResult,
        apca_result: &PerceptualResult,
        context: RecommendationContext,
    ) -> f64 {
        let ratio = wcag_result.value;
        let lc = apca_result.value.abs();

        // Check if contrast level is appropriate for the use case
        match context.usage {
            UsageContext::BodyText => {
                // Body text needs consistent, moderate-high contrast
                if ratio >= 7.0 && lc >= 60.0 {
                    1.0
                } else if ratio >= 4.5 {
                    0.8
                } else {
                    0.5
                }
            }
            UsageContext::LargeText => {
                // Large text can work with lower contrast
                if ratio >= 4.5 {
                    1.0
                } else if ratio >= 3.0 {
                    0.9
                } else {
                    0.6
                }
            }
            UsageContext::Interactive => {
                // Interactive elements need clear visibility
                if ratio >= 7.0 {
                    1.0
                } else if ratio >= 4.5 {
                    0.85
                } else {
                    0.5
                }
            }
            UsageContext::Decorative => {
                // Decorative elements have flexibility
                if ratio >= 1.5 {
                    1.0
                } else {
                    0.8
                }
            }
            UsageContext::Disabled => {
                // Disabled elements should have lower contrast
                if (2.0..=5.0).contains(&ratio) {
                    1.0
                } else if ratio > 5.0 {
                    0.7 // Too prominent for disabled
                } else {
                    0.8
                }
            }
            UsageContext::IconsGraphics => {
                // Icons need clear recognition
                if ratio >= 4.5 {
                    1.0
                } else if ratio >= 3.0 {
                    0.9
                } else {
                    0.6
                }
            }
        }
    }
}

impl Default for QualityScorer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ComplianceTarget;

    #[test]
    fn test_perfect_score() {
        let scorer = QualityScorer::new();
        let black = Color::from_srgb8(0, 0, 0);
        let white = Color::from_srgb8(255, 255, 255);
        let context = RecommendationContext::body_text();

        let score = scorer.score(black, white, context);

        assert!(score.passes());
        assert!(score.overall > 0.8);
        assert_eq!(score.compliance, 1.0);
    }

    #[test]
    fn test_failing_score() {
        let scorer = QualityScorer::new();
        let light_gray = Color::from_srgb8(200, 200, 200);
        let white = Color::from_srgb8(255, 255, 255);
        let context = RecommendationContext::body_text();

        let score = scorer.score(light_gray, white, context);

        assert!(!score.passes());
        assert!(score.overall < 0.5);
    }

    #[test]
    fn test_assessment_categories() {
        let excellent = QualityScore::new(0.95, 1.0, 0.95, 0.95);
        assert_eq!(excellent.assessment(), "Excellent");

        let good = QualityScore::new(0.75, 1.0, 0.7, 0.7);
        assert_eq!(good.assessment(), "Good");

        let acceptable = QualityScore::new(0.55, 0.9, 0.6, 0.5);
        assert_eq!(acceptable.assessment(), "Acceptable");

        let poor = QualityScore::new(0.2, 0.3, 0.4, 0.3);
        assert_eq!(poor.assessment(), "Poor");
    }

    #[test]
    fn test_context_specific_scoring() {
        let scorer = QualityScorer::new();
        let fg = Color::from_srgb8(100, 100, 100);
        let bg = Color::from_srgb8(255, 255, 255);

        // Same colors, different contexts = different scores
        let body_ctx = RecommendationContext::body_text();
        let large_ctx = RecommendationContext::large_text();
        let deco_ctx =
            RecommendationContext::new(UsageContext::Decorative, ComplianceTarget::WCAG_AA);

        let body_score = scorer.score(fg, bg, body_ctx);
        let large_score = scorer.score(fg, bg, large_ctx);
        let deco_score = scorer.score(fg, bg, deco_ctx);

        // Different contexts produce different scores
        // Body text is most strict, large text less strict, decorative most lenient
        // However, overall scores can vary based on perceptual quality component
        assert!(body_score.compliance <= large_score.compliance);
        assert!(large_score.compliance <= deco_score.compliance);
    }

    #[test]
    fn test_hybrid_compliance() {
        let scorer = QualityScorer::new();
        let black = Color::from_srgb8(0, 0, 0);
        let white = Color::from_srgb8(255, 255, 255);

        let hybrid_ctx =
            RecommendationContext::new(UsageContext::BodyText, ComplianceTarget::Hybrid);

        let score = scorer.score(black, white, hybrid_ctx);

        // Black on white should pass both WCAG and APCA
        assert!(score.passes());
        assert_eq!(score.compliance, 1.0);
    }
}
