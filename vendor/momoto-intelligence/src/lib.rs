//! # Momoto Intelligence
//!
//! Intelligence layer providing context-aware color recommendations.
//!
//! This crate implements a deterministic, rule-based recommendation system
//! for optimal color selection based on:
//! - Usage context (body text, headings, buttons, etc.)
//! - Compliance targets (WCAG AA/AAA, APCA)
//! - Perceptual quality scoring
//! - Explainable reasoning
//!
//! # Architecture
//!
//! The intelligence layer is built on three core components:
//!
//! 1. **Context**: Defines how colors will be used
//! 2. **Scoring**: Evaluates quality of color combinations
//! 3. **Recommendation**: Suggests optimal colors with explanations
//!
//! # Examples
//!
//! ## Recommend a foreground color for a background
//!
//! ```
//! use momoto_core::color::Color;
//! use momoto_intelligence::context::RecommendationContext;
//! use momoto_intelligence::recommendation::RecommendationEngine;
//!
//! let engine = RecommendationEngine::new();
//! let white_bg = Color::from_srgb8(255, 255, 255);
//! let context = RecommendationContext::body_text();
//!
//! let recommendation = engine.recommend_foreground(white_bg, context);
//!
//! println!("Recommended color: {:?}", recommendation.color);
//! println!("Quality score: {:.2}", recommendation.score.overall);
//! println!("Reason: {}", recommendation.reason);
//! ```
//!
//! ## Improve an existing color combination
//!
//! ```
//! use momoto_core::color::Color;
//! use momoto_intelligence::context::RecommendationContext;
//! use momoto_intelligence::recommendation::RecommendationEngine;
//!
//! let engine = RecommendationEngine::new();
//! let gray_fg = Color::from_srgb8(150, 150, 150);
//! let white_bg = Color::from_srgb8(255, 255, 255);
//! let context = RecommendationContext::body_text();
//!
//! let recommendation = engine.improve_foreground(gray_fg, white_bg, context);
//!
//! if let Some(modification) = &recommendation.modification {
//!     println!("Modification: {:?}", modification);
//! }
//! println!("Improvement: {}", recommendation.reason);
//! ```
//!
//! ## Score an existing combination
//!
//! ```
//! use momoto_core::color::Color;
//! use momoto_intelligence::context::RecommendationContext;
//! use momoto_intelligence::scoring::QualityScorer;
//!
//! let scorer = QualityScorer::new();
//! let black = Color::from_srgb8(0, 0, 0);
//! let white = Color::from_srgb8(255, 255, 255);
//! let context = RecommendationContext::body_text();
//!
//! let score = scorer.score(black, white, context);
//!
//! println!("Overall quality: {:.1}% ({})", score.overall * 100.0, score.assessment());
//! println!("Compliance: {:.0}%", score.compliance * 100.0);
//! println!("Perceptual: {:.0}%", score.perceptual * 100.0);
//! println!("Passes requirements: {}", score.passes());
//! ```
//!
//! # Design Principles
//!
//! ## Deterministic
//! No ML/AI black boxes - all decisions based on explicit, testable rules.
//!
//! ## Explainable
//! Every recommendation includes human-readable reasoning.
//!
//! ## Context-Aware
//! Different use cases have different requirements (body text vs decorative).
//!
//! ## Multi-Metric
//! Supports both WCAG 2.1 and APCA contrast algorithms.
//!
//! ## Perceptually-Informed
//! Uses OKLCH color space for perceptual uniformity.

#![warn(
    missing_docs,
    missing_debug_implementations,
    rust_2018_idioms,
    unreachable_pub
)]

pub mod adaptive;
pub mod advanced_scoring;
pub mod constraints;
pub mod context;
pub mod explanation;
pub mod harmony;
pub mod recommendation;
pub mod scoring;

// ============================================================================
// Convenient Re-exports
// ============================================================================

// Context types
pub use context::{ComplianceTarget, RecommendationContext, UsageContext};

// Scoring types
pub use scoring::{QualityScore, QualityScorer};

// Recommendation types
pub use recommendation::{Modification, Recommendation, RecommendationEngine};

// Advanced scoring types
pub use advanced_scoring::{
    AdvancedScore, AdvancedScorer, ConfidenceCalculator, EffortEstimator, EffortLevel,
    ImpactCalculator, ImpactWeights, PriorityAssessment, ScoreBreakdown, ScoreComponent,
};

// Explanation types
pub use explanation::{
    ExplanationBuilder, ExplanationGenerator, OklchChanges, ReasoningPoint,
    RecommendationExplanation, TechnicalDetails,
};

// Adaptive pipeline types
pub use adaptive::{
    BranchCondition, BranchEvaluator, ComparisonOp, ConvergenceConfig, ConvergenceDetector,
    ConvergenceStatus, CostEstimate, CostEstimator, StepRecommendation, StepSelector,
};

// Harmony types
pub use harmony::{
    design_system_palette, generate_palette, harmony_score, hex_to_oklch, oklch_to_hex, shades,
    temperature_palette, HarmonyType, Palette,
};

// Constraint solver types
pub use constraints::{
    ColorConstraint, ConstraintKind, ConstraintSolver, ConstraintViolation, SolverConfig,
    SolverResult,
};
