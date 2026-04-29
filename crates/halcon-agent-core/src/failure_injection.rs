//! FailureInjectionHarness — deterministic failure simulation for GDEM resilience testing.
//!
//! ## Purpose
//!
//! Wraps tool execution, confidence signals, embeddings, and critic outputs with
//! configurable probabilistic failure modes. All randomness is seeded for
//! deterministic replay.
//!
//! ## Failure modes
//!
//! | Mode                       | Effect                                                |
//! |----------------------------|-------------------------------------------------------|
//! | `ToolTimeout`              | Converts a successful result into a timeout           |
//! | `OutputCorruption`         | Injects garbage bytes at the midpoint of output       |
//! | `FalsePositiveToolSuccess` | Flips is_error from true → false (hallucinated OK)   |
//! | `EmbeddingNoise`           | Adds uniform noise to embedding float vectors         |
//! | `CriticBias`               | Adds a fixed delta to confidence before critic sees it|
//! | `MemoryDrift`              | Randomly drops episodes from retrieval results        |

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::critic::CriticSignal;

// ─── FailureMode ──────────────────────────────────────────────────────────────

/// A single failure mode with its activation probability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureMode {
    /// With probability `p`, inject a timeout (kills the result).
    ToolTimeout { probability: f64 },
    /// With probability `p`, corrupt the middle of the output string.
    OutputCorruption { probability: f64 },
    /// With probability `p`, flip `is_error = true` to `is_error = false`.
    FalsePositiveToolSuccess { probability: f64 },
    /// With probability `p`, add uniform noise ±`magnitude` to embedding values.
    EmbeddingNoise { probability: f64, magnitude: f32 },
    /// Always add `delta` to confidence score (positive = bias toward Continue).
    CriticBias { delta: f32 },
    /// With probability `p`, drop each retrieved memory episode.
    MemoryDrift { probability: f64 },
}

impl FailureMode {
    /// Human-readable label for the mode.
    pub fn label(&self) -> &'static str {
        match self {
            FailureMode::ToolTimeout { .. } => "ToolTimeout",
            FailureMode::OutputCorruption { .. } => "OutputCorruption",
            FailureMode::FalsePositiveToolSuccess { .. } => "FalsePositiveToolSuccess",
            FailureMode::EmbeddingNoise { .. } => "EmbeddingNoise",
            FailureMode::CriticBias { .. } => "CriticBias",
            FailureMode::MemoryDrift { .. } => "MemoryDrift",
        }
    }
}

// ─── InjectedToolResult ───────────────────────────────────────────────────────

/// The result of a tool call after failure injection has been applied.
#[derive(Debug, Clone)]
pub struct InjectedToolResult {
    /// Possibly corrupted / replaced output.
    pub output: String,
    /// Whether the tool is treated as having failed.
    pub is_error: bool,
    /// Whether the result simulates a timeout.
    pub timed_out: bool,
    /// True if any injection was applied.
    pub was_injected: bool,
    /// Which failure mode was applied (None if no injection).
    pub injection_kind: Option<String>,
}

// ─── InjectionStats ───────────────────────────────────────────────────────────

/// Aggregate counters for a harness session.
#[derive(Debug, Clone, Default)]
pub struct InjectionStats {
    pub total_calls: u64,
    pub total_injections: u64,
    pub timeouts_injected: u64,
    pub corruptions_injected: u64,
    pub false_positives_injected: u64,
    pub embedding_noise_injected: u64,
    pub critic_bias_applied: u64,
    pub memory_drift_applied: u64,
}

impl InjectionStats {
    pub fn injection_rate(&self) -> f64 {
        if self.total_calls == 0 {
            return 0.0;
        }
        self.total_injections as f64 / self.total_calls as f64
    }
}

// ─── FailureInjectionHarness ──────────────────────────────────────────────────

/// Seeded, deterministic failure injection harness.
///
/// Wrap any GDEM execution path with this harness to simulate adversarial conditions.
/// The harness is stateful (consumes RNG state) — use a fixed seed for reproducibility.
pub struct FailureInjectionHarness {
    modes: Vec<FailureMode>,
    rng: StdRng,
    stats: InjectionStats,
}

impl FailureInjectionHarness {
    /// Construct a harness with the given seed and failure modes.
    ///
    /// The same seed + modes produce the same injection sequence every run.
    pub fn new(seed: u64, modes: Vec<FailureMode>) -> Self {
        Self {
            modes,
            rng: StdRng::seed_from_u64(seed),
            stats: InjectionStats::default(),
        }
    }

    /// Construct a harness with a single failure mode.
    pub fn single(seed: u64, mode: FailureMode) -> Self {
        Self::new(seed, vec![mode])
    }

    /// Construct a harness with no failures (pass-through, useful for baseline).
    pub fn passthrough() -> Self {
        Self::new(0, vec![])
    }

    // ─── Injection methods ──────────────────────────────────────────────────

    /// Potentially inject a failure into a tool result.
    ///
    /// Modes are checked in order; the first matching injection wins.
    // Each match arm gates its body on a probability roll; collapsing
    // those into match-arm guards would tangle the rng call with the
    // pattern. Kept readable as nested if-inside-match.
    #[allow(clippy::collapsible_match, clippy::collapsible_if)]
    pub fn inject_tool_result(&mut self, output: &str, is_error: bool) -> InjectedToolResult {
        self.stats.total_calls += 1;

        // Snapshot modes to avoid borrow checker issues with self.rng
        let modes = self.modes.clone();
        for mode in &modes {
            match mode {
                FailureMode::ToolTimeout { probability } => {
                    if self.rng.random::<f64>() < *probability {
                        self.stats.total_injections += 1;
                        self.stats.timeouts_injected += 1;
                        return InjectedToolResult {
                            output: "[INJECTED TIMEOUT] Command exceeded time limit".to_string(),
                            is_error: true,
                            timed_out: true,
                            was_injected: true,
                            injection_kind: Some("ToolTimeout".to_string()),
                        };
                    }
                }
                FailureMode::OutputCorruption { probability } => {
                    if self.rng.random::<f64>() < *probability {
                        self.stats.total_injections += 1;
                        self.stats.corruptions_injected += 1;
                        let corrupt_id: u32 = self.rng.random();
                        let half = output.len() / 2;
                        let head = &output[..half.min(output.len())];
                        let tail = if half < output.len() {
                            &output[half..]
                        } else {
                            ""
                        };
                        let corrupted = format!("{}\0[CORRUPT:{}]\0{}", head, corrupt_id, tail);
                        return InjectedToolResult {
                            output: corrupted,
                            is_error: false,
                            timed_out: false,
                            was_injected: true,
                            injection_kind: Some("OutputCorruption".to_string()),
                        };
                    }
                }
                FailureMode::FalsePositiveToolSuccess { probability } => {
                    if is_error && self.rng.random::<f64>() < *probability {
                        self.stats.total_injections += 1;
                        self.stats.false_positives_injected += 1;
                        return InjectedToolResult {
                            output: output.to_string(),
                            is_error: false,
                            timed_out: false,
                            was_injected: true,
                            injection_kind: Some("FalsePositiveToolSuccess".to_string()),
                        };
                    }
                }
                _ => {} // Other modes handled in dedicated methods
            }
        }

        InjectedToolResult {
            output: output.to_string(),
            is_error,
            timed_out: false,
            was_injected: false,
            injection_kind: None,
        }
    }

    /// Potentially inject noise into a confidence score [0,1].
    ///
    /// Used by `CriticBias` mode — result is clamped to [0,1].
    pub fn inject_confidence(&mut self, confidence: f32) -> f32 {
        for mode in &self.modes.clone() {
            if let FailureMode::CriticBias { delta } = mode {
                self.stats.critic_bias_applied += 1;
                return (confidence + delta).clamp(0.0, 1.0);
            }
        }
        confidence
    }

    /// Potentially inject noise into an embedding vector.
    ///
    /// Returns a cloned, potentially perturbed vector; original is unchanged.
    pub fn inject_embedding(&mut self, embedding: &[f32]) -> Vec<f32> {
        for mode in &self.modes.clone() {
            if let FailureMode::EmbeddingNoise {
                probability,
                magnitude,
            } = mode
            {
                if self.rng.random::<f64>() < *probability {
                    self.stats.total_injections += 1;
                    self.stats.embedding_noise_injected += 1;
                    let mag = *magnitude;
                    return embedding
                        .iter()
                        .map(|&v| {
                            let noise: f32 = self.rng.random_range(-mag..=mag);
                            (v + noise).clamp(-1.0, 1.0)
                        })
                        .collect();
                }
            }
        }
        embedding.to_vec()
    }

    /// Potentially bias a critic signal.
    ///
    /// - `CriticBias { delta > 0.05 }` suppresses Replan → Continue.
    /// - `CriticBias { delta < -0.05 }` demotes Continue → Replan.
    pub fn inject_critic_signal(&mut self, signal: CriticSignal, confidence: f32) -> CriticSignal {
        // Terminal signals are never suppressed — safety invariant.
        if signal.is_terminal() {
            return signal;
        }

        for mode in &self.modes.clone() {
            if let FailureMode::CriticBias { delta } = mode {
                let delta = *delta;
                if delta > 0.05 && signal.requires_replan() {
                    self.stats.total_injections += 1;
                    self.stats.critic_bias_applied += 1;
                    return CriticSignal::Continue;
                }
                if delta < -0.05 && signal == CriticSignal::Continue {
                    self.stats.total_injections += 1;
                    self.stats.critic_bias_applied += 1;
                    return CriticSignal::Replan {
                        reason: "[CriticBias] forced replan injection".to_string(),
                        alignment_score: confidence,
                    };
                }
            }
        }
        signal
    }

    /// Potentially drop episodes from a memory retrieval result.
    ///
    /// Each episode is independently dropped with `probability`.
    pub fn inject_memory_retrieval<T>(&mut self, episodes: Vec<T>) -> Vec<T> {
        for mode in &self.modes.clone() {
            if let FailureMode::MemoryDrift { probability } = mode {
                let p = *probability;
                let kept: Vec<T> = episodes
                    .into_iter()
                    .filter(|_| {
                        let drop = self.rng.random::<f64>() < p;
                        if drop {
                            self.stats.memory_drift_applied += 1;
                        }
                        !drop
                    })
                    .collect();
                self.stats.total_injections += 1;
                return kept;
            }
        }
        episodes
    }

    // ─── Accessors ──────────────────────────────────────────────────────────

    pub fn stats(&self) -> &InjectionStats {
        &self.stats
    }

    pub fn total_injections(&self) -> u64 {
        self.stats.total_injections
    }

    pub fn total_calls(&self) -> u64 {
        self.stats.total_calls
    }

    pub fn injection_rate(&self) -> f64 {
        self.stats.injection_rate()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_makes_no_injections() {
        let mut h = FailureInjectionHarness::passthrough();
        let result = h.inject_tool_result("output", false);
        assert!(!result.was_injected);
        assert!(!result.is_error);
        assert_eq!(h.total_injections(), 0);
    }

    #[test]
    fn timeout_probability_1_always_fires() {
        let mut h =
            FailureInjectionHarness::single(0, FailureMode::ToolTimeout { probability: 1.0 });
        let result = h.inject_tool_result("output", false);
        assert!(result.was_injected);
        assert!(result.timed_out);
        assert!(result.is_error);
    }

    #[test]
    fn timeout_probability_0_never_fires() {
        let mut h =
            FailureInjectionHarness::single(0, FailureMode::ToolTimeout { probability: 0.0 });
        for _ in 0..100 {
            let result = h.inject_tool_result("output", false);
            assert!(!result.was_injected);
        }
    }

    #[test]
    fn false_positive_only_flips_errors() {
        let mut h = FailureInjectionHarness::single(
            42,
            FailureMode::FalsePositiveToolSuccess { probability: 1.0 },
        );
        // When is_error=false, nothing should change
        let result = h.inject_tool_result("ok", false);
        assert!(!result.was_injected);

        // When is_error=true, it should be flipped
        let result = h.inject_tool_result("failed", true);
        assert!(result.was_injected);
        assert!(!result.is_error);
    }

    #[test]
    fn output_corruption_changes_content() {
        let mut h =
            FailureInjectionHarness::single(99, FailureMode::OutputCorruption { probability: 1.0 });
        let result = h.inject_tool_result("hello world", false);
        assert!(result.was_injected);
        assert!(result.output.contains("CORRUPT"));
    }

    #[test]
    fn embedding_noise_changes_values() {
        let mut h = FailureInjectionHarness::single(
            7,
            FailureMode::EmbeddingNoise {
                probability: 1.0,
                magnitude: 0.1,
            },
        );
        let orig = vec![0.5f32, 0.5, 0.5];
        let noisy = h.inject_embedding(&orig);
        assert_eq!(noisy.len(), orig.len());
        // All values should still be in [-1, 1]
        for v in &noisy {
            assert!(*v >= -1.0 && *v <= 1.0);
        }
        // At least one value should differ (p=1.0)
        assert!(noisy != orig || orig.iter().all(|&v| v.abs() == 1.0));
    }

    #[test]
    fn critic_bias_positive_suppresses_replan() {
        let mut h = FailureInjectionHarness::single(0, FailureMode::CriticBias { delta: 0.2 });
        let replan = CriticSignal::Replan {
            reason: "stalled".into(),
            alignment_score: 0.5,
        };
        let out = h.inject_critic_signal(replan, 0.5);
        assert_eq!(out, CriticSignal::Continue);
    }

    #[test]
    fn critic_bias_never_suppresses_terminal() {
        let mut h = FailureInjectionHarness::single(0, FailureMode::CriticBias { delta: 1.0 });
        let terminate = CriticSignal::Terminate {
            reason: "budget exhausted".into(),
        };
        let out = h.inject_critic_signal(terminate.clone(), 0.0);
        // Terminal is always passed through unchanged
        assert!(out.is_terminal());
    }

    #[test]
    fn memory_drift_reduces_episode_count() {
        let mut h =
            FailureInjectionHarness::single(1, FailureMode::MemoryDrift { probability: 0.5 });
        let episodes: Vec<i32> = (0..100).collect();
        let kept = h.inject_memory_retrieval(episodes);
        // With p=0.5 and 100 items, expect roughly 50 kept (±20 for randomness)
        assert!(kept.len() < 100, "drift should remove some episodes");
        assert!(
            kept.len() > 20,
            "drift should not remove nearly all episodes at p=0.5"
        );
    }

    #[test]
    fn memory_drift_p0_keeps_all() {
        let mut h =
            FailureInjectionHarness::single(1, FailureMode::MemoryDrift { probability: 0.0 });
        let episodes: Vec<i32> = (0..50).collect();
        let kept = h.inject_memory_retrieval(episodes.clone());
        assert_eq!(kept.len(), episodes.len());
    }

    #[test]
    fn injection_rate_accurate() {
        let p = 0.5;
        let n = 10_000;
        let mut h =
            FailureInjectionHarness::single(42, FailureMode::ToolTimeout { probability: p });
        for _ in 0..n {
            h.inject_tool_result("cmd", false);
        }
        let rate = h.injection_rate();
        assert!(
            (rate - p).abs() < 0.03,
            "rate={:.3} expected≈{:.3}",
            rate,
            p
        );
    }

    #[test]
    fn stats_total_calls_tracked() {
        let mut h = FailureInjectionHarness::passthrough();
        for _ in 0..37 {
            h.inject_tool_result("x", false);
        }
        assert_eq!(h.total_calls(), 37);
    }

    #[test]
    fn mode_label_exhaustive() {
        let modes: Vec<FailureMode> = vec![
            FailureMode::ToolTimeout { probability: 0.1 },
            FailureMode::OutputCorruption { probability: 0.1 },
            FailureMode::FalsePositiveToolSuccess { probability: 0.1 },
            FailureMode::EmbeddingNoise {
                probability: 0.1,
                magnitude: 0.05,
            },
            FailureMode::CriticBias { delta: 0.1 },
            FailureMode::MemoryDrift { probability: 0.1 },
        ];
        let labels: Vec<&str> = modes.iter().map(|m| m.label()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()));
        // All labels are distinct
        let mut uniq = labels.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), modes.len());
    }
}
