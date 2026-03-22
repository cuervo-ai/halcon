//! Advanced scoring with impact, effort, and confidence metrics.
//!
//! This module extends the basic quality scoring with:
//! - Impact estimation (how much improvement this change provides)
//! - Effort estimation (how easy/hard the change is to implement)
//! - Confidence scoring (how certain we are about the recommendation)
//! - Priority calculation (impact/effort ratio for ranking)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::scoring::QualityScore;

/// Advanced score with impact, effort, and confidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedScore {
    /// Base quality overall score
    pub quality_overall: f64,
    /// Impact score (0.0 to 1.0): how much improvement this provides
    pub impact: f64,
    /// Effort score (0.0 to 1.0): 1.0 = trivial, 0.0 = very difficult
    pub effort: f64,
    /// Confidence score (0.0 to 1.0): certainty in the recommendation
    pub confidence: f64,
    /// Priority score (computed from impact/effort ratio)
    pub priority: f64,
    /// Breakdown of score components
    pub breakdown: ScoreBreakdown,
}

impl AdvancedScore {
    /// Create a new advanced score
    pub fn new(quality: QualityScore, impact: f64, effort: f64, confidence: f64) -> Self {
        let priority = Self::calculate_priority(impact, effort, confidence);
        Self {
            quality_overall: quality.overall,
            impact,
            effort,
            confidence,
            priority,
            breakdown: ScoreBreakdown::default(),
        }
    }

    /// Calculate priority from impact, effort, and confidence
    fn calculate_priority(impact: f64, effort: f64, confidence: f64) -> f64 {
        // Priority = (impact * confidence) / (1 - effort + 0.1)
        // Higher impact and confidence increase priority
        // Lower effort (higher ease) increases priority
        let ease = effort; // effort is already 0-1 where 1 = easy
        let denominator = (1.0 - ease + 0.1).max(0.1);
        (impact * confidence) / denominator
    }

    /// Create with detailed breakdown
    pub fn with_breakdown(mut self, breakdown: ScoreBreakdown) -> Self {
        self.breakdown = breakdown;
        self
    }

    /// Get overall recommendation strength (0.0 to 1.0)
    pub fn recommendation_strength(&self) -> f64 {
        (self.impact * self.confidence * self.effort).powf(1.0 / 3.0)
    }

    /// Check if this is a strong recommendation
    pub fn is_strong_recommendation(&self) -> bool {
        self.recommendation_strength() >= 0.7
    }

    /// Get priority assessment
    pub fn priority_assessment(&self) -> PriorityAssessment {
        if self.priority >= 2.0 {
            PriorityAssessment::Critical
        } else if self.priority >= 1.0 {
            PriorityAssessment::High
        } else if self.priority >= 0.5 {
            PriorityAssessment::Medium
        } else {
            PriorityAssessment::Low
        }
    }
}

/// Priority assessment levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityAssessment {
    /// Critical priority - should be addressed immediately
    Critical,
    /// High priority - address soon
    High,
    /// Medium priority - address when convenient
    Medium,
    /// Low priority - nice to have
    Low,
}

impl std::fmt::Display for PriorityAssessment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Critical => write!(f, "Critical"),
            Self::High => write!(f, "High"),
            Self::Medium => write!(f, "Medium"),
            Self::Low => write!(f, "Low"),
        }
    }
}

/// Detailed breakdown of score components
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    /// Impact components
    pub impact_components: Vec<ScoreComponent>,
    /// Effort components
    pub effort_components: Vec<ScoreComponent>,
    /// Confidence components
    pub confidence_components: Vec<ScoreComponent>,
}

impl ScoreBreakdown {
    /// Add impact component
    pub fn add_impact(mut self, name: impl Into<String>, value: f64, weight: f64) -> Self {
        self.impact_components.push(ScoreComponent {
            name: name.into(),
            value,
            weight,
            description: None,
        });
        self
    }

    /// Add effort component
    pub fn add_effort(mut self, name: impl Into<String>, value: f64, weight: f64) -> Self {
        self.effort_components.push(ScoreComponent {
            name: name.into(),
            value,
            weight,
            description: None,
        });
        self
    }

    /// Add confidence component
    pub fn add_confidence(mut self, name: impl Into<String>, value: f64, weight: f64) -> Self {
        self.confidence_components.push(ScoreComponent {
            name: name.into(),
            value,
            weight,
            description: None,
        });
        self
    }
}

/// A component of a score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreComponent {
    /// Component name
    pub name: String,
    /// Component value (0.0 to 1.0)
    pub value: f64,
    /// Weight of this component
    pub weight: f64,
    /// Optional description
    pub description: Option<String>,
}

impl ScoreComponent {
    /// Create a new score component
    pub fn new(name: impl Into<String>, value: f64, weight: f64) -> Self {
        Self {
            name: name.into(),
            value,
            weight,
            description: None,
        }
    }

    /// Add description
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Weighted contribution
    pub fn contribution(&self) -> f64 {
        self.value * self.weight
    }
}

/// Weights for impact calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactWeights {
    /// Weight for accessibility improvement
    pub accessibility: f64,
    /// Weight for perceptual quality improvement
    pub perceptual: f64,
    /// Weight for contrast improvement
    pub contrast: f64,
    /// Weight for color harmony
    pub harmony: f64,
    /// Weight for gamut compliance
    pub gamut: f64,
}

impl Default for ImpactWeights {
    fn default() -> Self {
        Self {
            accessibility: 0.35,
            perceptual: 0.25,
            contrast: 0.20,
            harmony: 0.10,
            gamut: 0.10,
        }
    }
}

/// Calculates impact of a color change
#[derive(Debug, Clone)]
pub struct ImpactCalculator {
    weights: ImpactWeights,
}

impl ImpactCalculator {
    /// Create a new impact calculator
    pub fn new(weights: ImpactWeights) -> Self {
        Self { weights }
    }

    /// Create with default weights
    pub fn with_defaults() -> Self {
        Self::new(ImpactWeights::default())
    }

    /// Calculate impact of changing from one score to another
    pub fn calculate_impact(
        &self,
        before: &QualityScore,
        after: &QualityScore,
    ) -> (f64, ScoreBreakdown) {
        let mut breakdown = ScoreBreakdown::default();

        // Accessibility improvement
        let accessibility_delta = after.compliance - before.compliance;
        let accessibility_impact = (accessibility_delta.max(0.0) * 2.0).min(1.0);
        breakdown = breakdown.add_impact(
            "accessibility",
            accessibility_impact,
            self.weights.accessibility,
        );

        // Perceptual quality improvement
        let perceptual_delta = after.perceptual - before.perceptual;
        let perceptual_impact = (perceptual_delta.max(0.0) * 2.0).min(1.0);
        breakdown = breakdown.add_impact("perceptual", perceptual_impact, self.weights.perceptual);

        // Overall improvement
        let overall_delta = after.overall - before.overall;
        let overall_impact = (overall_delta.max(0.0) * 2.0).min(1.0);
        breakdown = breakdown.add_impact("overall", overall_impact, self.weights.contrast);

        // Calculate weighted sum
        let total_weight =
            self.weights.accessibility + self.weights.perceptual + self.weights.contrast;
        let impact = (accessibility_impact * self.weights.accessibility
            + perceptual_impact * self.weights.perceptual
            + overall_impact * self.weights.contrast)
            / total_weight;

        (impact.clamp(0.0, 1.0), breakdown)
    }

    /// Calculate impact of a recommendation
    pub fn calculate_recommendation_impact(
        &self,
        current_passes: bool,
        recommended_passes: bool,
        quality_improvement: f64,
    ) -> f64 {
        let mut impact = 0.0;

        // Major impact if going from fail to pass
        if !current_passes && recommended_passes {
            impact += 0.5;
        }

        // Additional impact based on quality improvement
        impact += quality_improvement.max(0.0) * 0.5;

        impact.clamp(0.0, 1.0)
    }
}

impl Default for ImpactCalculator {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Effort levels for changes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortLevel {
    /// Trivial change (e.g., CSS variable update)
    Trivial,
    /// Easy change (single file modification)
    Easy,
    /// Moderate change (multiple files, but localized)
    Moderate,
    /// Significant change (cross-cutting concerns)
    Significant,
    /// Major change (architectural impact)
    Major,
}

impl EffortLevel {
    /// Convert to numeric score (1.0 = trivial, 0.0 = major)
    pub fn to_score(&self) -> f64 {
        match self {
            Self::Trivial => 1.0,
            Self::Easy => 0.8,
            Self::Moderate => 0.5,
            Self::Significant => 0.3,
            Self::Major => 0.1,
        }
    }
}

/// Estimates effort for implementing changes
#[derive(Debug, Clone, Default)]
pub struct EffortEstimator;

impl EffortEstimator {
    /// Create a new effort estimator
    pub fn new() -> Self {
        Self
    }

    /// Estimate effort for a color change
    pub fn estimate_color_change(
        &self,
        delta_l: f64, // Lightness change
        delta_c: f64, // Chroma change
        delta_h: f64, // Hue change
    ) -> (f64, EffortLevel, ScoreBreakdown) {
        let mut breakdown = ScoreBreakdown::default();

        // Larger changes are harder to evaluate for side effects
        let magnitude = (delta_l.abs() + delta_c.abs() * 0.5 + delta_h.abs() / 360.0) / 2.0;

        // Lightness-only changes are easiest
        let is_lightness_only = delta_c.abs() < 0.01 && delta_h.abs() < 1.0;

        // Hue changes are most noticeable/risky
        let hue_factor: f64 = if delta_h.abs() > 30.0 { 0.5 } else { 1.0 };

        let base_effort: f64 = if is_lightness_only {
            0.9 // Very easy - just brightness adjustment
        } else if magnitude < 0.1 {
            0.8 // Easy - minor adjustment
        } else if magnitude < 0.3 {
            0.6 // Moderate - noticeable change
        } else {
            0.3 // Significant - major color shift
        };

        let effort = (base_effort * hue_factor).clamp(0.0, 1.0);

        breakdown = breakdown.add_effort("magnitude", 1.0 - magnitude.min(1.0), 0.4);
        breakdown = breakdown.add_effort("hue_stability", hue_factor, 0.3);
        breakdown = breakdown.add_effort(
            "type_simplicity",
            if is_lightness_only { 1.0 } else { 0.5 },
            0.3,
        );

        let level = if effort >= 0.9 {
            EffortLevel::Trivial
        } else if effort >= 0.7 {
            EffortLevel::Easy
        } else if effort >= 0.5 {
            EffortLevel::Moderate
        } else if effort >= 0.3 {
            EffortLevel::Significant
        } else {
            EffortLevel::Major
        };

        (effort, level, breakdown)
    }

    /// Estimate effort for adopting a recommendation
    pub fn estimate_recommendation(
        &self,
        modification_type: &str,
        color_count: usize,
    ) -> (f64, EffortLevel) {
        // Base effort by modification type
        let type_effort = match modification_type {
            "lightness" => 0.9,
            "chroma" => 0.7,
            "hue" => 0.5,
            "combined" => 0.4,
            _ => 0.6,
        };

        // Scale factor for number of colors affected
        let scale = 1.0 / (1.0 + (color_count as f64 - 1.0) * 0.1);

        let effort = (type_effort * scale).clamp(0.0, 1.0);

        let level = if effort >= 0.9 {
            EffortLevel::Trivial
        } else if effort >= 0.7 {
            EffortLevel::Easy
        } else if effort >= 0.5 {
            EffortLevel::Moderate
        } else if effort >= 0.3 {
            EffortLevel::Significant
        } else {
            EffortLevel::Major
        };

        (effort, level)
    }
}

/// Historical outcome data for confidence calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalOutcome {
    /// Number of times this recommendation type was made
    pub count: usize,
    /// Number of times it was accepted
    pub accepted: usize,
    /// Number of times it achieved the predicted outcome
    pub successful: usize,
    /// Average accuracy of quality predictions
    pub accuracy: f64,
}

impl HistoricalOutcome {
    /// Create new outcome tracker
    pub fn new() -> Self {
        Self {
            count: 0,
            accepted: 0,
            successful: 0,
            accuracy: 0.5, // Default to 50%
        }
    }

    /// Record an outcome
    pub fn record(&mut self, accepted: bool, successful: bool, predicted: f64, actual: f64) {
        self.count += 1;
        if accepted {
            self.accepted += 1;
        }
        if successful {
            self.successful += 1;
        }

        // Update accuracy with exponential moving average
        let prediction_error = (predicted - actual).abs();
        let accuracy = 1.0 - prediction_error.min(1.0);
        self.accuracy = self.accuracy * 0.9 + accuracy * 0.1;
    }

    /// Calculate confidence based on history
    pub fn confidence(&self) -> f64 {
        if self.count == 0 {
            return 0.5; // No data, neutral confidence
        }

        // Confidence increases with more data and better outcomes
        let sample_factor = 1.0 - (1.0 / (1.0 + self.count as f64 * 0.1));
        let success_rate = self.successful as f64 / self.count as f64;

        (sample_factor * 0.3 + success_rate * 0.3 + self.accuracy * 0.4).clamp(0.0, 1.0)
    }
}

impl Default for HistoricalOutcome {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculates confidence in recommendations
#[derive(Debug, Clone)]
pub struct ConfidenceCalculator {
    /// Historical outcomes by recommendation category
    historical_data: HashMap<String, HistoricalOutcome>,
    /// Base confidence for recommendations without history
    base_confidence: f64,
}

impl ConfidenceCalculator {
    /// Create a new confidence calculator
    pub fn new() -> Self {
        Self {
            historical_data: HashMap::new(),
            base_confidence: 0.7,
        }
    }

    /// Set base confidence
    pub fn with_base_confidence(mut self, base: f64) -> Self {
        self.base_confidence = base;
        self
    }

    /// Calculate confidence for a recommendation
    pub fn calculate_confidence(
        &self,
        category: &str,
        quality_score: &QualityScore,
        modification_magnitude: f64,
    ) -> (f64, ScoreBreakdown) {
        let mut breakdown = ScoreBreakdown::default();

        // Historical confidence
        let historical_conf = self
            .historical_data
            .get(category)
            .map(|h| h.confidence())
            .unwrap_or(self.base_confidence);
        breakdown = breakdown.add_confidence("historical", historical_conf, 0.3);

        // Quality-based confidence (higher quality = more confident)
        let quality_conf = quality_score.overall;
        breakdown = breakdown.add_confidence("quality_based", quality_conf, 0.3);

        // Magnitude-based confidence (smaller changes = more confident)
        let magnitude_conf = 1.0 - modification_magnitude.min(1.0) * 0.5;
        breakdown = breakdown.add_confidence("magnitude", magnitude_conf, 0.2);

        // Compliance-based confidence (passing = more confident)
        let compliance_conf = if quality_score.passes() { 0.9 } else { 0.6 };
        breakdown = breakdown.add_confidence("compliance", compliance_conf, 0.2);

        let confidence = (historical_conf * 0.3
            + quality_conf * 0.3
            + magnitude_conf * 0.2
            + compliance_conf * 0.2)
            .clamp(0.0, 1.0);

        (confidence, breakdown)
    }

    /// Record an outcome for learning
    pub fn record_outcome(
        &mut self,
        category: &str,
        accepted: bool,
        successful: bool,
        predicted: f64,
        actual: f64,
    ) {
        let outcome = self
            .historical_data
            .entry(category.to_string())
            .or_default();
        outcome.record(accepted, successful, predicted, actual);
    }

    /// Get historical data for a category
    pub fn get_history(&self, category: &str) -> Option<&HistoricalOutcome> {
        self.historical_data.get(category)
    }

    /// Get all categories with history
    pub fn categories(&self) -> Vec<&String> {
        self.historical_data.keys().collect()
    }
}

impl Default for ConfidenceCalculator {
    fn default() -> Self {
        Self::new()
    }
}

/// Combines all scorers into an advanced scoring system
#[derive(Debug, Clone)]
pub struct AdvancedScorer {
    impact_calculator: ImpactCalculator,
    effort_estimator: EffortEstimator,
    confidence_calculator: ConfidenceCalculator,
}

impl AdvancedScorer {
    /// Create a new advanced scorer
    pub fn new() -> Self {
        Self {
            impact_calculator: ImpactCalculator::with_defaults(),
            effort_estimator: EffortEstimator::new(),
            confidence_calculator: ConfidenceCalculator::new(),
        }
    }

    /// Score a recommendation
    pub fn score_recommendation(
        &self,
        category: &str,
        before: &QualityScore,
        after: &QualityScore,
        delta_l: f64,
        delta_c: f64,
        delta_h: f64,
    ) -> AdvancedScore {
        // Calculate impact
        let (impact, impact_breakdown) = self.impact_calculator.calculate_impact(before, after);

        // Calculate effort
        let (effort, _level, effort_breakdown) = self
            .effort_estimator
            .estimate_color_change(delta_l, delta_c, delta_h);

        // Calculate confidence
        let modification_magnitude = (delta_l.abs() + delta_c.abs() + delta_h.abs() / 360.0) / 3.0;
        let (confidence, conf_breakdown) = self.confidence_calculator.calculate_confidence(
            category,
            after,
            modification_magnitude,
        );

        // Combine breakdowns
        let breakdown = ScoreBreakdown {
            impact_components: impact_breakdown.impact_components,
            effort_components: effort_breakdown.effort_components,
            confidence_components: conf_breakdown.confidence_components,
        };

        AdvancedScore::new(*after, impact, effort, confidence).with_breakdown(breakdown)
    }

    /// Get the confidence calculator for recording outcomes
    pub fn confidence_calculator_mut(&mut self) -> &mut ConfidenceCalculator {
        &mut self.confidence_calculator
    }
}

impl Default for AdvancedScorer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_quality_score(overall: f64, compliance: f64, perceptual: f64) -> QualityScore {
        QualityScore {
            overall,
            compliance,
            perceptual,
            appropriateness: 0.8,
        }
    }

    #[test]
    fn test_advanced_score_priority() {
        let quality = mock_quality_score(0.9, 0.95, 0.85);
        let score = AdvancedScore::new(quality, 0.8, 0.9, 0.95);

        // High impact, low effort, high confidence should give high priority
        assert!(score.priority > 1.0);
        assert!(score.is_strong_recommendation());
    }

    #[test]
    fn test_impact_calculator() {
        let calculator = ImpactCalculator::with_defaults();
        let before = mock_quality_score(0.5, 0.4, 0.6);
        let after = mock_quality_score(0.9, 0.95, 0.85);

        let (impact, _) = calculator.calculate_impact(&before, &after);
        assert!(impact > 0.5); // Significant improvement
    }

    #[test]
    fn test_effort_estimator() {
        let estimator = EffortEstimator::new();

        // Lightness-only change should be easy
        let (effort, level, _) = estimator.estimate_color_change(0.1, 0.0, 0.0);
        assert!(effort > 0.8);
        assert!(matches!(level, EffortLevel::Trivial | EffortLevel::Easy));

        // Large hue change should be harder
        let (effort2, level2, _) = estimator.estimate_color_change(0.1, 0.1, 45.0);
        assert!(effort2 < effort);
        assert!(matches!(
            level2,
            EffortLevel::Moderate | EffortLevel::Significant
        ));
    }

    #[test]
    fn test_confidence_calculator() {
        let calculator = ConfidenceCalculator::new();
        let quality = mock_quality_score(0.9, 0.95, 0.85);

        let (confidence, _) = calculator.calculate_confidence("color_improvement", &quality, 0.1);
        assert!(confidence > 0.5);
    }

    #[test]
    fn test_historical_outcome() {
        let mut outcome = HistoricalOutcome::new();

        // Record some successes
        outcome.record(true, true, 0.8, 0.85);
        outcome.record(true, true, 0.7, 0.75);
        outcome.record(true, false, 0.9, 0.6);

        assert!(outcome.confidence() > 0.5);
        assert_eq!(outcome.count, 3);
        assert_eq!(outcome.successful, 2);
    }

    #[test]
    fn test_priority_assessment() {
        let quality = mock_quality_score(0.9, 0.95, 0.85);

        let critical = AdvancedScore::new(quality.clone(), 0.95, 0.95, 0.95);
        assert_eq!(critical.priority_assessment(), PriorityAssessment::Critical);

        let low = AdvancedScore::new(quality, 0.2, 0.3, 0.5);
        assert_eq!(low.priority_assessment(), PriorityAssessment::Low);
    }
}
