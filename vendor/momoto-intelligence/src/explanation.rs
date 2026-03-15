//! Explanation generator for AI recommendations.
//!
//! All recommendations include human-readable explanations that describe:
//! - The problem being addressed
//! - The reasoning behind the recommendation
//! - Expected benefits and trade-offs
//! - Technical details for developers

use serde::{Deserialize, Serialize};

use crate::advanced_scoring::{AdvancedScore, PriorityAssessment};
use crate::scoring::QualityScore;

/// A complete explanation for a recommendation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationExplanation {
    /// One-line summary
    pub summary: String,
    /// Detailed reasoning points
    pub reasoning: Vec<ReasoningPoint>,
    /// Problem being addressed
    pub problem_addressed: String,
    /// Expected benefits
    pub benefits: Vec<String>,
    /// Potential trade-offs
    pub trade_offs: Vec<String>,
    /// Technical details
    pub technical: TechnicalDetails,
}

impl RecommendationExplanation {
    /// Create a new explanation builder
    pub fn builder() -> ExplanationBuilder {
        ExplanationBuilder::new()
    }

    /// Get full markdown representation
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str(&format!("## {}\n\n", self.summary));
        md.push_str(&format!("**Problem:** {}\n\n", self.problem_addressed));

        if !self.reasoning.is_empty() {
            md.push_str("### Reasoning\n\n");
            for point in &self.reasoning {
                md.push_str(&format!(
                    "- **{}**: {}\n",
                    point.category, point.explanation
                ));
            }
            md.push('\n');
        }

        if !self.benefits.is_empty() {
            md.push_str("### Benefits\n\n");
            for benefit in &self.benefits {
                md.push_str(&format!("- {}\n", benefit));
            }
            md.push('\n');
        }

        if !self.trade_offs.is_empty() {
            md.push_str("### Trade-offs\n\n");
            for tradeoff in &self.trade_offs {
                md.push_str(&format!("- {}\n", tradeoff));
            }
            md.push('\n');
        }

        md.push_str("### Technical Details\n\n");
        md.push_str(&format!(
            "- Color change: `{}` → `{}`\n",
            self.technical.original_color, self.technical.recommended_color
        ));
        md.push_str(&format!(
            "- Contrast ratio: {:.2} → {:.2}\n",
            self.technical.original_contrast, self.technical.new_contrast
        ));
        md.push_str(&format!(
            "- Quality score: {:.0}% → {:.0}%\n",
            self.technical.original_quality * 100.0,
            self.technical.new_quality * 100.0
        ));

        md
    }
}

/// A point of reasoning in the explanation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningPoint {
    /// Category of reasoning (accessibility, perceptual, harmony)
    pub category: String,
    /// The explanation
    pub explanation: String,
    /// Importance level (1-5)
    pub importance: u8,
    /// Supporting evidence
    pub evidence: Option<String>,
}

impl ReasoningPoint {
    /// Create a new reasoning point
    pub fn new(category: impl Into<String>, explanation: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            explanation: explanation.into(),
            importance: 3,
            evidence: None,
        }
    }

    /// Set importance
    pub fn with_importance(mut self, importance: u8) -> Self {
        self.importance = importance.min(5);
        self
    }

    /// Add evidence
    pub fn with_evidence(mut self, evidence: impl Into<String>) -> Self {
        self.evidence = Some(evidence.into());
        self
    }
}

/// Technical details for an explanation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechnicalDetails {
    /// Original color (hex)
    pub original_color: String,
    /// Recommended color (hex)
    pub recommended_color: String,
    /// Original WCAG contrast ratio
    pub original_contrast: f64,
    /// New WCAG contrast ratio
    pub new_contrast: f64,
    /// Original APCA Lc value
    pub original_apca: f64,
    /// New APCA Lc value
    pub new_apca: f64,
    /// Original quality score
    pub original_quality: f64,
    /// New quality score
    pub new_quality: f64,
    /// Color difference (Delta E)
    pub delta_e: f64,
    /// Modification type
    pub modification_type: String,
    /// OKLCH components changed
    pub oklch_changes: OklchChanges,
}

impl Default for TechnicalDetails {
    fn default() -> Self {
        Self {
            original_color: "#000000".to_string(),
            recommended_color: "#000000".to_string(),
            original_contrast: 1.0,
            new_contrast: 1.0,
            original_apca: 0.0,
            new_apca: 0.0,
            original_quality: 0.0,
            new_quality: 0.0,
            delta_e: 0.0,
            modification_type: "none".to_string(),
            oklch_changes: OklchChanges::default(),
        }
    }
}

/// OKLCH component changes
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OklchChanges {
    /// Lightness change (delta L)
    pub delta_l: f64,
    /// Chroma change (delta C)
    pub delta_c: f64,
    /// Hue change (delta H in degrees)
    pub delta_h: f64,
}

impl OklchChanges {
    /// Create new changes
    pub fn new(delta_l: f64, delta_c: f64, delta_h: f64) -> Self {
        Self {
            delta_l,
            delta_c,
            delta_h,
        }
    }

    /// Describe the primary change
    pub fn describe(&self) -> String {
        let l_change = if self.delta_l.abs() > 0.01 {
            if self.delta_l > 0.0 {
                "lighter"
            } else {
                "darker"
            }
        } else {
            ""
        };

        let c_change = if self.delta_c.abs() > 0.01 {
            if self.delta_c > 0.0 {
                "more saturated"
            } else {
                "less saturated"
            }
        } else {
            ""
        };

        let h_change = if self.delta_h.abs() > 5.0 {
            "hue shifted"
        } else {
            ""
        };

        let changes: Vec<&str> = [l_change, c_change, h_change]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();

        if changes.is_empty() {
            "minimal change".to_string()
        } else {
            changes.join(", ")
        }
    }
}

/// Builder for recommendation explanations
#[derive(Debug, Clone, Default)]
pub struct ExplanationBuilder {
    summary: Option<String>,
    reasoning: Vec<ReasoningPoint>,
    problem: Option<String>,
    benefits: Vec<String>,
    trade_offs: Vec<String>,
    technical: TechnicalDetails,
}

impl ExplanationBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set summary
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Add reasoning point
    pub fn reasoning(mut self, point: ReasoningPoint) -> Self {
        self.reasoning.push(point);
        self
    }

    /// Set problem addressed
    pub fn problem(mut self, problem: impl Into<String>) -> Self {
        self.problem = Some(problem.into());
        self
    }

    /// Add benefit
    pub fn benefit(mut self, benefit: impl Into<String>) -> Self {
        self.benefits.push(benefit.into());
        self
    }

    /// Add trade-off
    pub fn trade_off(mut self, trade_off: impl Into<String>) -> Self {
        self.trade_offs.push(trade_off.into());
        self
    }

    /// Set technical details
    pub fn technical(mut self, technical: TechnicalDetails) -> Self {
        self.technical = technical;
        self
    }

    /// Build the explanation
    pub fn build(self) -> RecommendationExplanation {
        RecommendationExplanation {
            summary: self
                .summary
                .unwrap_or_else(|| "Color recommendation".to_string()),
            reasoning: self.reasoning,
            problem_addressed: self.problem.unwrap_or_default(),
            benefits: self.benefits,
            trade_offs: self.trade_offs,
            technical: self.technical,
        }
    }
}

/// Generates explanations for recommendations
#[derive(Debug, Clone, Default)]
pub struct ExplanationGenerator;

impl ExplanationGenerator {
    /// Create a new explanation generator
    pub fn new() -> Self {
        Self
    }

    /// Generate explanation for a contrast improvement
    pub fn generate_contrast_improvement(
        &self,
        original_color: &str,
        recommended_color: &str,
        background: &str,
        original_ratio: f64,
        new_ratio: f64,
        target_ratio: f64,
        oklch_changes: OklchChanges,
    ) -> RecommendationExplanation {
        let change_desc = oklch_changes.describe();
        let ratio_improvement = new_ratio - original_ratio;

        let summary = if new_ratio >= target_ratio && original_ratio < target_ratio {
            format!(
                "Adjust color to achieve {} contrast compliance",
                if target_ratio >= 7.0 {
                    "WCAG AAA"
                } else {
                    "WCAG AA"
                }
            )
        } else {
            format!("Improve contrast ratio by {:.1}:1", ratio_improvement)
        };

        let problem = format!(
            "The current color {} on {} has a contrast ratio of {:.2}:1, \
             which is below the required {:.1}:1 for accessibility compliance.",
            original_color, background, original_ratio, target_ratio
        );

        let mut builder = ExplanationBuilder::new()
            .summary(&summary)
            .problem(&problem);

        // Add reasoning
        builder = builder.reasoning(
            ReasoningPoint::new(
                "Accessibility",
                format!(
                    "WCAG requires a minimum contrast ratio of {:.1}:1 for this content type. \
                     The recommended color achieves {:.2}:1.",
                    target_ratio, new_ratio
                ),
            )
            .with_importance(5),
        );

        if oklch_changes.delta_l.abs() > 0.01 {
            let direction = if oklch_changes.delta_l > 0.0 {
                "Increasing"
            } else {
                "Decreasing"
            };
            builder = builder.reasoning(
                ReasoningPoint::new(
                    "Perceptual",
                    format!(
                        "{} lightness by {:.0}% improves contrast while maintaining color identity.",
                        direction, oklch_changes.delta_l.abs() * 100.0
                    )
                ).with_importance(4)
            );
        }

        // Add benefits
        builder = builder
            .benefit("Meets accessibility requirements")
            .benefit(format!(
                "Improves readability with {:.1}:1 contrast",
                new_ratio
            ));

        if oklch_changes.delta_h.abs() < 5.0 {
            builder = builder.benefit("Preserves original color identity (hue unchanged)");
        }

        // Add trade-offs
        if oklch_changes.delta_l.abs() > 0.15 {
            builder = builder.trade_off(format!(
                "Noticeable {} change may affect visual hierarchy",
                if oklch_changes.delta_l > 0.0 {
                    "lightening"
                } else {
                    "darkening"
                }
            ));
        }

        if oklch_changes.delta_c.abs() > 0.05 {
            builder = builder.trade_off(format!(
                "Saturation {} may affect brand consistency",
                if oklch_changes.delta_c > 0.0 {
                    "increase"
                } else {
                    "decrease"
                }
            ));
        }

        // Technical details
        let technical = TechnicalDetails {
            original_color: original_color.to_string(),
            recommended_color: recommended_color.to_string(),
            original_contrast: original_ratio,
            new_contrast: new_ratio,
            modification_type: change_desc,
            oklch_changes,
            ..Default::default()
        };

        builder.technical(technical).build()
    }

    /// Generate explanation for a quality improvement
    pub fn generate_quality_improvement(
        &self,
        original_color: &str,
        recommended_color: &str,
        before_score: &QualityScore,
        after_score: &QualityScore,
        oklch_changes: OklchChanges,
    ) -> RecommendationExplanation {
        let improvement = after_score.overall - before_score.overall;
        let summary = format!("Improve color quality by {:.0}%", improvement * 100.0);

        let problem = format!(
            "The current color {} has a quality score of {:.0}%, \
             which could be improved for better perceptual qualities.",
            original_color,
            before_score.overall * 100.0
        );

        let mut builder = ExplanationBuilder::new()
            .summary(&summary)
            .problem(&problem);

        // Add reasoning based on what improved
        if after_score.compliance > before_score.compliance {
            builder = builder.reasoning(
                ReasoningPoint::new(
                    "Compliance",
                    format!(
                        "Compliance score improves from {:.0}% to {:.0}%",
                        before_score.compliance * 100.0,
                        after_score.compliance * 100.0
                    ),
                )
                .with_importance(5),
            );
        }

        if after_score.perceptual > before_score.perceptual {
            builder = builder.reasoning(
                ReasoningPoint::new(
                    "Perceptual Quality",
                    format!(
                        "Perceptual quality improves from {:.0}% to {:.0}%",
                        before_score.perceptual * 100.0,
                        after_score.perceptual * 100.0
                    ),
                )
                .with_importance(4),
            );
        }

        // Benefits
        builder = builder
            .benefit(format!(
                "Overall quality: {:.0}% → {:.0}%",
                before_score.overall * 100.0,
                after_score.overall * 100.0
            ))
            .benefit(format!(
                "Assessment: {} → {}",
                before_score.assessment(),
                after_score.assessment()
            ));

        // Technical details
        let technical = TechnicalDetails {
            original_color: original_color.to_string(),
            recommended_color: recommended_color.to_string(),
            original_quality: before_score.overall,
            new_quality: after_score.overall,
            modification_type: oklch_changes.describe(),
            oklch_changes,
            ..Default::default()
        };

        builder.technical(technical).build()
    }

    /// Generate explanation from advanced score
    pub fn generate_from_advanced_score(
        &self,
        original_color: &str,
        recommended_color: &str,
        score: &AdvancedScore,
        context: &str,
    ) -> RecommendationExplanation {
        let priority = score.priority_assessment();

        let summary = match priority {
            PriorityAssessment::Critical => {
                format!("Critical: Improve {} for {}", original_color, context)
            }
            PriorityAssessment::High => {
                format!(
                    "Recommended: Adjust {} for better {}",
                    original_color, context
                )
            }
            PriorityAssessment::Medium => {
                format!(
                    "Suggestion: Consider adjusting {} for {}",
                    original_color, context
                )
            }
            PriorityAssessment::Low => {
                format!(
                    "Optional: Minor improvement available for {}",
                    original_color
                )
            }
        };

        let problem = format!(
            "Analysis identified an opportunity to improve color quality for {} context.",
            context
        );

        let mut builder = ExplanationBuilder::new()
            .summary(&summary)
            .problem(&problem);

        // Add reasoning from score components
        for component in &score.breakdown.impact_components {
            if component.value > 0.3 {
                builder = builder.reasoning(
                    ReasoningPoint::new(
                        &component.category(),
                        format!("{} impact: {:.0}%", component.name, component.value * 100.0),
                    )
                    .with_importance((component.value * 5.0) as u8),
                );
            }
        }

        // Benefits
        builder = builder
            .benefit(format!("Impact: {:.0}%", score.impact * 100.0))
            .benefit(format!("Confidence: {:.0}%", score.confidence * 100.0))
            .benefit(format!("Priority: {}", priority));

        // Effort-based trade-offs
        if score.effort < 0.7 {
            builder = builder.trade_off("Implementation requires more than trivial changes");
        }

        let technical = TechnicalDetails {
            original_color: original_color.to_string(),
            recommended_color: recommended_color.to_string(),
            original_quality: 0.0, // Would need before score
            new_quality: score.quality_overall,
            ..Default::default()
        };

        builder.technical(technical).build()
    }
}

// Helper trait for ScoreComponent
trait ComponentExt {
    fn category(&self) -> String;
}

impl ComponentExt for crate::advanced_scoring::ScoreComponent {
    fn category(&self) -> String {
        // Convert name to category
        self.name.replace('_', " ").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_explanation_builder() {
        let explanation = ExplanationBuilder::new()
            .summary("Test recommendation")
            .problem("Test problem")
            .reasoning(ReasoningPoint::new("test", "explanation"))
            .benefit("Test benefit")
            .trade_off("Test trade-off")
            .build();

        assert_eq!(explanation.summary, "Test recommendation");
        assert!(!explanation.reasoning.is_empty());
        assert!(!explanation.benefits.is_empty());
        assert!(!explanation.trade_offs.is_empty());
    }

    #[test]
    fn test_oklch_changes_describe() {
        let changes = OklchChanges::new(0.1, 0.0, 0.0);
        assert!(changes.describe().contains("lighter"));

        let changes = OklchChanges::new(-0.1, 0.05, 0.0);
        assert!(changes.describe().contains("darker"));
        assert!(changes.describe().contains("saturated"));
    }

    #[test]
    fn test_contrast_improvement_explanation() {
        let generator = ExplanationGenerator::new();
        let changes = OklchChanges::new(-0.15, 0.0, 0.0);

        let explanation = generator
            .generate_contrast_improvement("#888888", "#5a5a5a", "#ffffff", 3.5, 7.2, 7.0, changes);

        assert!(explanation.summary.contains("AAA"));
        assert!(!explanation.reasoning.is_empty());
        assert!(!explanation.benefits.is_empty());
    }

    #[test]
    fn test_markdown_output() {
        let explanation = ExplanationBuilder::new()
            .summary("Improve contrast")
            .problem("Low contrast")
            .reasoning(ReasoningPoint::new("Accessibility", "WCAG requires 4.5:1"))
            .benefit("Better readability")
            .technical(TechnicalDetails {
                original_color: "#888".to_string(),
                recommended_color: "#555".to_string(),
                original_contrast: 3.5,
                new_contrast: 7.0,
                original_quality: 0.5,
                new_quality: 0.9,
                ..Default::default()
            })
            .build();

        let md = explanation.to_markdown();
        assert!(md.contains("## Improve contrast"));
        assert!(md.contains("WCAG"));
        assert!(md.contains("3.50"));
        assert!(md.contains("7.00"));
    }
}
