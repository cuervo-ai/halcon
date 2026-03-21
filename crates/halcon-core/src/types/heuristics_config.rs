//! `HeuristicsConfig` — configuration for all hardcoded heuristic thresholds.
//!
//! Phase 3 addition. Centralizes values that were previously baked into source code
//! across multiple modules. Default values match the existing hardcoded values exactly,
//! so behavior is identical when no config is specified.
//!
//! # Usage
//! ```rust,no_run
//! let config = halcon_core::types::HeuristicsConfig::default();
//! // identical to existing hardcoded behavior
//! assert_eq!(config.default_context_window, 64_000);
//! ```

use serde::{Deserialize, Serialize};

/// Configuration for the `ModelRouter` — which models to use for each tier.
///
/// Replaces `ModelRouter::deepseek_defaults()` with a config-driven constructor.
/// Default values match the existing deepseek-based defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRouterConfig {
    /// Model to use for the `Fast` routing tier.
    pub fast_model: String,
    /// Model to use for the `Balanced` routing tier.
    pub balanced_model: String,
    /// Model to use for the `Deep` routing tier.
    pub deep_model: String,
}

impl Default for ModelRouterConfig {
    fn default() -> Self {
        Self {
            fast_model: "deepseek-chat".into(),
            balanced_model: "deepseek-chat".into(),
            deep_model: "deepseek-reasoner".into(),
        }
    }
}

/// Confidence weight formula parameters for `IntentScorer`.
///
/// Weights must sum to 1.0. Default values match the existing formula:
/// `0.30*scope + 0.25*depth + 0.25*type + 0.10*lang + 0.10*(1-ambiguity)`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceWeights {
    /// Weight for the scope confidence component.
    pub scope: f64,
    /// Weight for the analysis depth component.
    pub depth: f64,
    /// Weight for the task type component.
    pub task_type: f64,
    /// Weight for the programming language confidence.
    pub language: f64,
    /// Weight for the (1 - ambiguity) component.
    pub clarity: f64,
}

impl Default for ConfidenceWeights {
    fn default() -> Self {
        // Must sum to 1.0 — matches existing hardcoded values exactly.
        Self {
            scope: 0.30,
            depth: 0.25,
            task_type: 0.25,
            language: 0.10,
            clarity: 0.10,
        }
    }
}

impl ConfidenceWeights {
    /// Validate that weights sum to approximately 1.0 (±0.01 tolerance).
    pub fn is_valid(&self) -> bool {
        let sum = self.scope + self.depth + self.task_type + self.language + self.clarity;
        (sum - 1.0).abs() < 0.01
    }
}

/// Scope confidence values per task scope.
///
/// Each value is the prior confidence when the scorer assigns a given scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeConfidences {
    pub conversational: f64,
    pub single_artifact: f64,
    pub local_context: f64,
    pub project_wide: f64,
    pub system_wide: f64,
}

impl Default for ScopeConfidences {
    fn default() -> Self {
        // Matches existing hardcoded values in intent_scorer.rs:180-186.
        Self {
            conversational: 0.90,
            single_artifact: 0.75,
            local_context: 0.70,
            project_wide: 0.80,
            system_wide: 0.65,
        }
    }
}

/// Word-count thresholds for automatic scope classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordCountThresholds {
    /// Messages with ≤ this many words are classified as `SingleArtifact`.
    pub single_artifact_max: usize,
    /// Messages with > `single_artifact_max` and ≤ this many words → `LocalContext`.
    /// Messages above this threshold → `ProjectWide`.
    pub local_context_max: usize,
    /// Messages with ≤ this many words are eligible for `Conversational` scope
    /// (in addition to requiring a conversational keyword match).
    pub conversational_max: usize,
}

impl Default for WordCountThresholds {
    fn default() -> Self {
        // Matches intent_scorer.rs:310-322 and line 310 conversational check.
        Self {
            single_artifact_max: 10,
            local_context_max: 25,
            conversational_max: 12,
        }
    }
}

/// Phi coherence thresholds for the metacognitive health monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhiCoherenceThresholds {
    /// Phi ≥ this value → system is healthy.
    pub healthy: f32,
    /// Phi ≥ this value (and < healthy) → system is degraded.
    pub degraded: f32,
    // Phi < degraded → critical.
}

impl Default for PhiCoherenceThresholds {
    fn default() -> Self {
        // Matches convergence_phase.rs:204-210.
        Self {
            healthy: 0.7,
            degraded: 0.5,
        }
    }
}

/// Default context window size when the provider does not report one.
pub const DEFAULT_CONTEXT_WINDOW_TOKENS: u32 = 64_000;

/// How often (in rounds) the metacognitive loop runs a full health cycle.
pub const DEFAULT_METACOGNITIVE_CYCLE_ROUNDS: u32 = 10;

/// Configuration for the `LoopGuard` oscillation health scoring.
///
/// Used to calculate loop guard health as:
/// `1.0 - (consecutive_rounds / divisor).min(1.0)`
pub const DEFAULT_LOOP_GUARD_HEALTH_DIVISOR: f32 = 10.0;

/// Combined heuristics configuration.
///
/// All fields have `Default` implementations that match the current hardcoded
/// values — no behavior change when using `HeuristicsConfig::default()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeuristicsConfig {
    /// Model tier → model ID mapping for routing.
    pub model_router: ModelRouterConfig,
    /// Confidence weight formula parameters.
    pub confidence_weights: ConfidenceWeights,
    /// Scope confidence priors.
    pub scope_confidences: ScopeConfidences,
    /// Word-count-based scope classification thresholds.
    pub word_count: WordCountThresholds,
    /// Metacognitive health monitor thresholds.
    pub phi_coherence: PhiCoherenceThresholds,
    /// Default context window when provider reports none.
    #[serde(default = "default_ctx_window")]
    pub default_context_window: u32,
    /// Metacognitive full-cycle frequency in rounds.
    #[serde(default = "default_metacog_cycle")]
    pub metacognitive_cycle_rounds: u32,
}

impl Default for HeuristicsConfig {
    // BUG-S4-PRE: #[derive(Default)] sets serde-defaulted fields to 0 (u32 Default),
    // but the test expects them to equal the named constants. This custom impl ensures
    // HeuristicsConfig::default() matches what serde deserialization produces.
    fn default() -> Self {
        Self {
            model_router: ModelRouterConfig::default(),
            confidence_weights: ConfidenceWeights::default(),
            scope_confidences: ScopeConfidences::default(),
            word_count: WordCountThresholds::default(),
            phi_coherence: PhiCoherenceThresholds::default(),
            default_context_window: DEFAULT_CONTEXT_WINDOW_TOKENS,
            metacognitive_cycle_rounds: DEFAULT_METACOGNITIVE_CYCLE_ROUNDS,
        }
    }
}

fn default_ctx_window() -> u32 {
    DEFAULT_CONTEXT_WINDOW_TOKENS
}
fn default_metacog_cycle() -> u32 {
    DEFAULT_METACOGNITIVE_CYCLE_ROUNDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_confidence_weights_sum_to_one() {
        let w = ConfidenceWeights::default();
        assert!(
            w.is_valid(),
            "default weights must sum to 1.0, got {}",
            w.scope + w.depth + w.task_type + w.language + w.clarity
        );
    }

    #[test]
    fn default_heuristics_matches_hardcoded_values() {
        let cfg = HeuristicsConfig::default();
        // Model router defaults
        assert_eq!(cfg.model_router.fast_model, "deepseek-chat");
        assert_eq!(cfg.model_router.balanced_model, "deepseek-chat");
        assert_eq!(cfg.model_router.deep_model, "deepseek-reasoner");
        // Intent scorer thresholds
        assert_eq!(cfg.word_count.single_artifact_max, 10);
        assert_eq!(cfg.word_count.local_context_max, 25);
        assert_eq!(cfg.word_count.conversational_max, 12);
        // Confidence weights
        assert!((cfg.confidence_weights.scope - 0.30).abs() < 1e-6);
        assert!((cfg.confidence_weights.depth - 0.25).abs() < 1e-6);
        // Convergence thresholds
        assert!((cfg.phi_coherence.healthy - 0.7).abs() < 1e-6);
        assert!((cfg.phi_coherence.degraded - 0.5).abs() < 1e-6);
        // Context window default
        assert_eq!(cfg.default_context_window, DEFAULT_CONTEXT_WINDOW_TOKENS);
        assert_eq!(
            cfg.metacognitive_cycle_rounds,
            DEFAULT_METACOGNITIVE_CYCLE_ROUNDS
        );
    }

    #[test]
    fn heuristics_config_serde_roundtrip() {
        let cfg = HeuristicsConfig::default();
        let json = serde_json::to_string(&cfg).expect("serialize");
        let parsed: HeuristicsConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.model_router.fast_model, cfg.model_router.fast_model);
        assert_eq!(
            parsed.word_count.single_artifact_max,
            cfg.word_count.single_artifact_max
        );
        assert_eq!(parsed.default_context_window, cfg.default_context_window);
    }

    #[test]
    fn custom_confidence_weights_validation() {
        let valid = ConfidenceWeights {
            scope: 0.5,
            depth: 0.2,
            task_type: 0.2,
            language: 0.05,
            clarity: 0.05,
        };
        assert!(valid.is_valid());

        let invalid = ConfidenceWeights {
            scope: 0.5,
            depth: 0.5,
            task_type: 0.5,
            language: 0.5,
            clarity: 0.5,
        };
        assert!(!invalid.is_valid());
    }

    #[test]
    fn scope_confidences_all_in_unit_interval() {
        let sc = ScopeConfidences::default();
        for val in [
            sc.conversational,
            sc.single_artifact,
            sc.local_context,
            sc.project_wide,
            sc.system_wide,
        ] {
            assert!(
                val >= 0.0 && val <= 1.0,
                "scope confidence out of [0,1]: {val}"
            );
        }
    }

    #[test]
    fn default_context_window_constant() {
        assert_eq!(DEFAULT_CONTEXT_WINDOW_TOKENS, 64_000);
    }

    #[test]
    fn metacognitive_cycle_constant() {
        assert_eq!(DEFAULT_METACOGNITIVE_CYCLE_ROUNDS, 10);
    }
}
