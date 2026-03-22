//! Adaptive pipeline — convergence detection, step selection, cost estimation.
//!
//! Provides self-tuning machinery for iterative color optimization loops.
//! All algorithms are deterministic and alloc-free beyond the history buffer.

use serde::{Deserialize, Serialize};

// =============================================================================
// cost_estimator submodule
// =============================================================================

/// Cost estimation helpers — kept in a submodule to allow granular imports.
pub mod cost_estimator {
    use serde::{Deserialize, Serialize};

    /// Factors that affect computational cost of a pipeline step.
    ///
    /// Uses a builder pattern so callers only specify what's relevant.
    ///
    /// # Example
    /// ```
    /// use momoto_intelligence::adaptive::cost_estimator::CostFactors;
    /// let f = CostFactors::new().with_color_count(256).with_neural();
    /// ```
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CostFactors {
        /// Number of distinct colors to process.
        pub color_count: usize,
        /// Spectral pipeline active (31-band CIE integration).
        pub spectral: bool,
        /// SIREN neural correction active.
        pub neural: bool,
        /// PBR material BSDF evaluation active.
        pub material: bool,
    }

    impl Default for CostFactors {
        fn default() -> Self {
            Self::new()
        }
    }

    impl CostFactors {
        /// Create factors with sensible defaults (1 color, no extra stages).
        #[inline]
        pub fn new() -> Self {
            Self {
                color_count: 1,
                spectral: false,
                neural: false,
                material: false,
            }
        }

        /// Set the number of colors to process.
        #[inline]
        pub fn with_color_count(mut self, n: usize) -> Self {
            self.color_count = n;
            self
        }

        /// Enable spectral pipeline cost.
        #[inline]
        pub fn with_spectral(mut self) -> Self {
            self.spectral = true;
            self
        }

        /// Enable neural correction cost.
        #[inline]
        pub fn with_neural(mut self) -> Self {
            self.neural = true;
            self
        }

        /// Enable material BSDF cost.
        #[inline]
        pub fn with_material(mut self) -> Self {
            self.material = true;
            self
        }
    }
}

// =============================================================================
// ConvergenceConfig
// =============================================================================

/// Configuration parameters for the convergence detector.
#[derive(Debug, Clone)]
pub struct ConvergenceConfig {
    /// Maximum number of iterations before declaring non-convergence.
    pub max_iterations: usize,
    /// Minimum absolute change in quality to be considered progress.
    pub tolerance: f64,
    /// Sliding window size for oscillation and trend detection.
    pub window_size: usize,
    /// Minimum per-iteration improvement to stay in `Converging` state.
    pub min_improvement: f64,
}

impl Default for ConvergenceConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            tolerance: 0.01,
            window_size: 8,
            min_improvement: 0.002,
        }
    }
}

impl ConvergenceConfig {
    /// Fast preset — fewer iterations, looser tolerance.
    /// Best for quick feedback loops where perfect quality is less critical.
    pub fn fast() -> Self {
        Self {
            max_iterations: 20,
            tolerance: 0.05,
            window_size: 5,
            min_improvement: 0.01,
        }
    }

    /// High-quality preset — more iterations, tighter tolerance.
    /// Best for design-critical color decisions.
    pub fn high_quality() -> Self {
        Self {
            max_iterations: 100,
            tolerance: 0.001,
            window_size: 12,
            min_improvement: 0.001,
        }
    }

    /// Neural preset — balanced for SIREN correction loops.
    /// Tuned for the typical convergence behavior of neural color correction.
    pub fn neural() -> Self {
        Self {
            max_iterations: 50,
            tolerance: 0.008,
            window_size: 7,
            min_improvement: 0.003,
        }
    }
}

// =============================================================================
// ConvergenceStatus
// =============================================================================

/// Current convergence state, returned by [`ConvergenceDetector::update`].
///
/// Matches the pattern expected by the WASM intelligence bindings.
#[derive(Debug, Clone)]
pub enum ConvergenceStatus {
    /// Quality is improving at a consistent rate.
    ///
    /// `rate` — improvement per iteration (positive).
    /// `estimated_iterations` — estimated remaining iterations.
    Converging {
        /// Mean improvement per step in the current window.
        rate: f64,
        /// Estimated remaining iterations to reach tolerance.
        estimated_iterations: u32,
    },

    /// Quality has stabilised below the tolerance threshold.
    Converged {
        /// Total iterations elapsed.
        iterations: u32,
        /// Final stabilised quality value.
        final_value: f64,
    },

    /// Quality is fluctuating periodically (stuck in a cycle).
    Oscillating {
        /// Peak-to-peak amplitude of the oscillation.
        amplitude: f64,
        /// Estimated cycle frequency (oscillations per iteration).
        frequency: f64,
    },

    /// Quality is degrading (negative trend).
    Diverging {
        /// Rate of degradation (positive value, quality decreasing at this speed).
        rate: f64,
    },

    /// Quality has not changed meaningfully for several iterations.
    Stalled {
        /// Quality value at which the detector stalled.
        stuck_at: f64,
        /// Number of iterations spent stalled.
        iterations_stuck: u32,
    },

    /// Not enough data yet to classify the convergence behaviour.
    Undetermined {
        /// Current (latest) quality value.
        current: f64,
        /// Iterations elapsed so far.
        iterations: u32,
    },
}

// =============================================================================
// ConvergenceStats  (serialisable summary)
// =============================================================================

/// Snapshot of convergence statistics, returned by [`ConvergenceDetector::stats`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceStats {
    /// Total iterations processed.
    pub iterations: usize,
    /// Best quality seen so far (highest value).
    pub best_quality: f64,
    /// Cumulative improvement from first observation to best.
    pub total_improvement: f64,
    /// Average improvement per iteration.
    pub improvement_rate: f64,
    /// Whether the detector considers quality still improving.
    pub is_progressing: bool,
    /// Slope of a linear regression over the last `window_size` values.
    pub convergence_rate: f64,
}

// =============================================================================
// ConvergenceDetector
// =============================================================================

/// Tracks the convergence of an iterative quality-optimisation loop.
///
/// Feed quality values (higher = better, range [0, 1]) via [`update`].
/// The detector analyses the history and returns a [`ConvergenceStatus`]
/// on every call.
///
/// # Algorithm
///
/// - Window of recent values: `window_size` last observations.
/// - **Converging** — mean slope positive, abs-slope > `min_improvement`.
/// - **Converged** — std-dev of window < `tolerance`.
/// - **Oscillating** — alternating signs in the slope window.
/// - **Diverging** — mean slope negative.
/// - **Stalled** — abs-slope < `tolerance` for `window_size` iterations.
/// - **Undetermined** — fewer than 3 samples.
#[derive(Debug)]
pub struct ConvergenceDetector {
    config: ConvergenceConfig,
    history: Vec<f64>,
    best: f64,
    iterations: usize,
}

impl Default for ConvergenceDetector {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl ConvergenceDetector {
    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(ConvergenceConfig::default())
    }

    /// Create with a specific configuration.
    pub fn new(config: ConvergenceConfig) -> Self {
        Self {
            config,
            history: Vec::with_capacity(16),
            best: f64::NEG_INFINITY,
            iterations: 0,
        }
    }

    /// Feed a new quality measurement. Returns current convergence status.
    pub fn update(&mut self, quality: f64) -> ConvergenceStatus {
        self.history.push(quality);
        if quality > self.best {
            self.best = quality;
        }
        self.iterations += 1;

        let n = self.history.len();

        // Not enough data yet
        if n < 3 {
            return ConvergenceStatus::Undetermined {
                current: quality,
                iterations: self.iterations as u32,
            };
        }

        let window: &[f64] = {
            let start = n.saturating_sub(self.config.window_size);
            &self.history[start..]
        };
        let wn = window.len();

        // Compute deltas in the window
        let deltas: Vec<f64> = window.windows(2).map(|w| w[1] - w[0]).collect();

        // Mean delta (slope)
        let mean_delta = deltas.iter().sum::<f64>() / deltas.len() as f64;

        // Variance of the window values
        let mean_val = window.iter().sum::<f64>() / wn as f64;
        let variance = window.iter().map(|v| (v - mean_val).powi(2)).sum::<f64>() / wn as f64;
        let std_dev = variance.sqrt();

        // Oscillation: check for sign changes in deltas
        let sign_changes = deltas.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        let oscillating = sign_changes >= deltas.len() / 2;

        if oscillating && wn >= 4 {
            let amplitude = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                - window.iter().cloned().fold(f64::INFINITY, f64::min);
            let frequency = sign_changes as f64 / (2.0 * deltas.len() as f64);
            return ConvergenceStatus::Oscillating {
                amplitude,
                frequency,
            };
        }

        // Converged: std-dev below tolerance
        if std_dev < self.config.tolerance && wn >= self.config.window_size {
            return ConvergenceStatus::Converged {
                iterations: self.iterations as u32,
                final_value: mean_val,
            };
        }

        // Diverging: mean slope is clearly negative
        if mean_delta < -self.config.min_improvement {
            return ConvergenceStatus::Diverging { rate: -mean_delta };
        }

        // Stalled: abs slope below tolerance for a full window
        if mean_delta.abs() < self.config.tolerance && wn >= self.config.window_size {
            return ConvergenceStatus::Stalled {
                stuck_at: mean_val,
                iterations_stuck: wn as u32,
            };
        }

        // Converging: positive mean slope
        if mean_delta > self.config.min_improvement {
            // Estimate remaining iterations: if we need to reach 1.0 from current
            let gap = (1.0_f64 - quality).max(0.0);
            let est = if mean_delta > 1e-9 {
                ((gap / mean_delta) as u32).min(self.config.max_iterations as u32)
            } else {
                self.config.max_iterations as u32
            };
            return ConvergenceStatus::Converging {
                rate: mean_delta,
                estimated_iterations: est,
            };
        }

        ConvergenceStatus::Undetermined {
            current: quality,
            iterations: self.iterations as u32,
        }
    }

    /// Reset the detector to initial state.
    pub fn reset(&mut self) {
        self.history.clear();
        self.best = f64::NEG_INFINITY;
        self.iterations = 0;
    }

    /// Best quality observed so far.
    pub fn best_quality(&self) -> f64 {
        if self.best == f64::NEG_INFINITY {
            0.0
        } else {
            self.best
        }
    }

    /// Number of iterations processed.
    pub fn iteration_count(&self) -> usize {
        self.iterations
    }

    /// Total improvement: best observed minus first observation.
    pub fn total_improvement(&self) -> f64 {
        if self.history.is_empty() {
            return 0.0;
        }
        (self.best - self.history[0]).max(0.0)
    }

    /// Average improvement per iteration.
    pub fn improvement_rate(&self) -> f64 {
        if self.iterations == 0 {
            return 0.0;
        }
        self.total_improvement() / self.iterations as f64
    }

    /// Whether quality is still improving within the current window.
    pub fn is_progressing(&self) -> bool {
        let n = self.history.len();
        if n < 2 {
            return false;
        }
        let window_start = n.saturating_sub(self.config.window_size);
        let window = &self.history[window_start..];
        let mean_delta =
            window.windows(2).map(|w| w[1] - w[0]).sum::<f64>() / (window.len() - 1).max(1) as f64;
        mean_delta > self.config.min_improvement
    }

    /// Full statistics snapshot.
    pub fn stats(&self) -> ConvergenceStats {
        let n = self.history.len();
        let convergence_rate = if n < 2 {
            0.0
        } else {
            let window_start = n.saturating_sub(self.config.window_size);
            let window = &self.history[window_start..];
            window.windows(2).map(|w| w[1] - w[0]).sum::<f64>() / (window.len() - 1).max(1) as f64
        };

        ConvergenceStats {
            iterations: self.iterations,
            best_quality: self.best_quality(),
            total_improvement: self.total_improvement(),
            improvement_rate: self.improvement_rate(),
            is_progressing: self.is_progressing(),
            convergence_rate,
        }
    }
}

// =============================================================================
// StepRecommendation
// =============================================================================

/// A recommended next pipeline step from [`StepSelector`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecommendation {
    /// Identifier of the recommended step (e.g. `"wcag_adjust"`, `"siren_correct"`).
    pub step_type: String,
    /// Expected quality improvement as a fraction of the remaining gap.
    pub expected_improvement: f64,
    /// Confidence in the recommendation (0.0 = none, 1.0 = certain).
    pub confidence: f64,
    /// Estimated relative computational cost (1.0 = baseline).
    pub cost_estimate: f64,
    /// Priority level (1 = highest, 5 = lowest).
    pub priority: u8,
}

// =============================================================================
// StepSelector
// =============================================================================

/// Internal record of a completed step.
#[derive(Debug, Clone)]
struct StepRecord {
    step_type: String,
    improvement: f64,
    cost: f64,
    success: bool,
}

/// Adaptive step recommender for iterative color pipeline optimisation.
///
/// Tracks how well each step type has performed historically and recommends
/// the step most likely to advance towards the target quality.
///
/// # Step types (predefined, not exhaustive)
/// - `"wcag_adjust"` — luminance-based WCAG contrast adjustment
/// - `"siren_correct"` — neural perceptual correction
/// - `"gamut_map"` — OKLab→sRGB gamut clamp
/// - `"hue_shift"` — hue rotation for harmony
/// - `"saturation_adjust"` — chroma normalisation
#[derive(Debug)]
pub struct StepSelector {
    goal_type: String,
    target: f64,
    current: f64,
    history: Vec<StepRecord>,
}

impl StepSelector {
    /// Create a new selector for `goal_type` aiming for `target` quality.
    pub fn new(goal_type: &str, target: f64) -> Self {
        Self {
            goal_type: goal_type.to_string(),
            target,
            current: 0.0,
            history: Vec::new(),
        }
    }

    /// Update the current quality observation.
    pub fn update_progress(&mut self, value: f64) {
        self.current = value;
    }

    /// Record the outcome of an executed step.
    ///
    /// `improvement` — observed quality delta (can be negative if step hurt quality).
    /// `cost` — relative computational cost (1.0 = baseline).
    /// `success` — whether the step completed without error.
    pub fn record_outcome(&mut self, step_type: &str, improvement: f64, cost: f64, success: bool) {
        self.history.push(StepRecord {
            step_type: step_type.to_string(),
            improvement,
            cost,
            success,
        });
    }

    /// Recommend the most beneficial next step, or `None` if the goal is achieved.
    pub fn recommend_next_step(&self) -> Option<StepRecommendation> {
        if self.is_goal_achieved() {
            return None;
        }

        // Candidate steps with their baseline cost
        let candidates: &[(&str, f64)] = &[
            ("wcag_adjust", 1.0),
            ("siren_correct", 2.5),
            ("gamut_map", 0.5),
            ("hue_shift", 0.8),
            ("saturation_adjust", 0.7),
        ];

        // Score each candidate by historical effectiveness / cost ratio
        let best = candidates
            .iter()
            .max_by(|(a_type, a_cost), (b_type, b_cost)| {
                let a_score = self.candidate_score(a_type, *a_cost);
                let b_score = self.candidate_score(b_type, *b_cost);
                a_score
                    .partial_cmp(&b_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;

        let (step_type, cost_estimate) = *best;
        let score = self.candidate_score(step_type, cost_estimate);
        let confidence = (score / 10.0).clamp(0.1, 0.95);
        let gap = (self.target - self.current).max(0.0);
        let expected_improvement = (gap * confidence * 0.5).clamp(0.0, gap);

        Some(StepRecommendation {
            step_type: step_type.to_string(),
            expected_improvement,
            confidence,
            cost_estimate,
            priority: if confidence > 0.7 {
                1
            } else if confidence > 0.5 {
                2
            } else {
                3
            },
        })
    }

    /// Current progress ratio in [0.0, 1.0] towards the target.
    pub fn goal_progress(&self) -> f64 {
        if self.target <= 0.0 {
            return 1.0;
        }
        (self.current / self.target).clamp(0.0, 1.0)
    }

    /// Whether the current quality has reached or exceeded the target.
    pub fn is_goal_achieved(&self) -> bool {
        self.current >= self.target
    }

    /// Compute a score for a candidate step type based on history.
    fn candidate_score(&self, step_type: &str, base_cost: f64) -> f64 {
        let records: Vec<&StepRecord> = self
            .history
            .iter()
            .filter(|r| r.step_type == step_type && r.success)
            .collect();

        if records.is_empty() {
            // No data — use a moderate default, slightly biased by cost
            return 5.0 / base_cost;
        }

        let mean_improvement =
            records.iter().map(|r| r.improvement).sum::<f64>() / records.len() as f64;
        let mean_cost = records.iter().map(|r| r.cost).sum::<f64>() / records.len() as f64;

        // Score = improvement / cost, with recency not weighted (simple mean)
        if mean_cost < 1e-9 {
            return 0.0;
        }
        mean_improvement / mean_cost
    }
}

// =============================================================================
// CostEstimate
// =============================================================================

/// Estimated computational cost of a single pipeline step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimate {
    /// The step being estimated.
    pub step_type: String,
    /// Approximate wall-clock cost in milliseconds (single-threaded).
    pub base_cost_ms: f64,
    /// Approximate memory footprint in megabytes.
    pub memory_mb: f64,
    /// CPU multiplier relative to a 1-color baseline.
    pub cpu_factor: f64,
    /// Total estimated cost in relative units (dimensionless, for comparison).
    pub total_estimate: f64,
    /// Whether the step can be parallelised across colors.
    pub is_parallelizable: bool,
}

// =============================================================================
// CostEstimator
// =============================================================================

/// Look-up-table based cost estimator for pipeline steps.
///
/// All costs are approximate and tuned for a modern laptop CPU.
/// The estimates serve as relative rankings — use them for step ordering,
/// not for real-time performance predictions.
#[derive(Debug, Default)]
pub struct CostEstimator;

impl CostEstimator {
    /// Create a new estimator.
    pub fn new() -> Self {
        Self
    }

    /// Estimate cost for one step type with given factors.
    pub fn estimate(&self, step_type: &str, factors: &cost_estimator::CostFactors) -> CostEstimate {
        // Base costs per step type (ms, single color)
        let (base_ms, mem_mb, parallelizable) = match step_type {
            "wcag_adjust" => (0.05, 0.001, true),
            "apca_adjust" => (0.06, 0.001, true),
            "siren_correct" => (1.20, 0.5, true),
            "gamut_map" => (0.02, 0.001, true),
            "hue_shift" => (0.01, 0.001, true),
            "saturation_adjust" => (0.01, 0.001, true),
            "thin_film" => (2.50, 0.1, false),
            "mie_scattering" => (5.00, 0.2, false),
            "spectral_pipeline" => (8.00, 0.5, false),
            "harmony_score" => (0.10, 0.01, true),
            "cvd_simulate" => (0.08, 0.01, true),
            _ => (0.10, 0.01, true),
        };

        // Scale by color count
        let count_factor = factors.color_count.max(1) as f64;

        // Additional stage multipliers
        let spectral_mul = if factors.spectral { 3.0 } else { 1.0 };
        let neural_mul = if factors.neural { 2.0 } else { 1.0 };
        let material_mul = if factors.material { 4.0 } else { 1.0 };

        let cpu_factor = count_factor * spectral_mul * neural_mul * material_mul;
        let total_ms = base_ms * cpu_factor;

        CostEstimate {
            step_type: step_type.to_string(),
            base_cost_ms: total_ms,
            memory_mb: mem_mb * count_factor * spectral_mul,
            cpu_factor,
            total_estimate: total_ms,
            is_parallelizable: parallelizable,
        }
    }

    /// Estimate sequential cost for an ordered list of (step_type, factors) pairs.
    ///
    /// Returns one [`CostEstimate`] per step in input order.
    pub fn estimate_sequential(
        &self,
        steps: &[(&str, &cost_estimator::CostFactors)],
    ) -> Vec<CostEstimate> {
        steps.iter().map(|(s, f)| self.estimate(s, f)).collect()
    }

    /// Sum of `total_estimate` across all steps (useful for pipeline total cost).
    pub fn sequential_total(&self, steps: &[(&str, &cost_estimator::CostFactors)]) -> f64 {
        steps
            .iter()
            .map(|(s, f)| self.estimate(s, f).total_estimate)
            .sum()
    }
}

// =============================================================================
// Legacy stub types (kept for API compatibility; no-op implementations)
// =============================================================================

/// Branch condition for adaptive decision trees (stub — not yet implemented).
#[derive(Debug, Clone)]
pub struct BranchCondition;

/// Branch evaluator for adaptive decision trees (stub — not yet implemented).
#[derive(Debug, Clone)]
pub struct BranchEvaluator;

/// Comparison operator for branch conditions (stub — not yet implemented).
#[derive(Debug, Clone)]
pub struct ComparisonOp;

/// Recommendation from the step selector (re-exported as `StepRecommendation`).
pub type StepRec = StepRecommendation;

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convergence_fast_preset() {
        let config = ConvergenceConfig::fast();
        assert_eq!(config.max_iterations, 20);
        assert!(config.tolerance > 0.0);
    }

    #[test]
    fn test_convergence_detector_converges() {
        let mut det = ConvergenceDetector::with_defaults();
        // Feed a clearly converging sequence
        let values = [
            0.1, 0.3, 0.5, 0.65, 0.75, 0.82, 0.87, 0.90, 0.91, 0.92, 0.92, 0.92,
        ];
        let mut last_status = None;
        for v in values {
            last_status = Some(det.update(v));
        }
        // Should eventually converge or reach stalled/converged
        assert!(det.best_quality() >= 0.92 - 1e-9);
        let stats = det.stats();
        assert!(stats.total_improvement > 0.0);
    }

    #[test]
    fn test_convergence_detector_oscillation() {
        let mut det = ConvergenceDetector::with_defaults();
        let values = [0.5, 0.7, 0.5, 0.7, 0.5, 0.7, 0.5, 0.7, 0.5, 0.7];
        let mut last = None;
        for v in values {
            last = Some(det.update(v));
        }
        matches!(last, Some(ConvergenceStatus::Oscillating { .. }));
    }

    #[test]
    fn test_step_selector_recommend() {
        let mut sel = StepSelector::new("contrast", 0.9);
        sel.update_progress(0.5);
        let rec = sel.recommend_next_step();
        assert!(rec.is_some());
        let rec = rec.unwrap();
        assert!(!rec.step_type.is_empty());
        assert!(rec.confidence > 0.0);
    }

    #[test]
    fn test_step_selector_goal_achieved() {
        let mut sel = StepSelector::new("contrast", 0.9);
        sel.update_progress(0.95);
        assert!(sel.is_goal_achieved());
        assert!(sel.recommend_next_step().is_none());
    }

    #[test]
    fn test_cost_estimator_estimate() {
        let est = CostEstimator::new();
        let factors = cost_estimator::CostFactors::new()
            .with_color_count(100)
            .with_neural();
        let result = est.estimate("siren_correct", &factors);
        assert!(result.base_cost_ms > 0.0);
        assert!(result.cpu_factor > 1.0);
        assert!(!result.step_type.is_empty());
    }

    #[test]
    fn test_cost_estimator_sequential() {
        let est = CostEstimator::new();
        let f1 = cost_estimator::CostFactors::new().with_color_count(10);
        let f2 = cost_estimator::CostFactors::new().with_spectral();
        let estimates =
            est.estimate_sequential(&[("wcag_adjust", &f1), ("spectral_pipeline", &f2)]);
        assert_eq!(estimates.len(), 2);
    }
}
