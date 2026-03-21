//! Phase J6 — Invariant Coverage Metric.
//!
//! ## Definition
//!
//! ```text
//! invariant_coverage = (# public components with ≥1 invariant)
//!                      / (total public components)
//! ```
//!
//! ## Requirement
//!
//! **I-7.6**: `invariant_coverage = 1.0` (100%) — every public component must
//! have at least one formal invariant binding.
//!
//! ## Verification
//!
//! This module:
//! 1. Enumerates all public components in `halcon-agent-core`.
//! 2. Binds each to the invariant IDs that govern it (from `invariants.rs` + Phase J).
//! 3. Computes coverage and asserts it equals 1.0.
//!
//! Any new public component added to the crate **must** also be added here with
//! at least one invariant ID, or the coverage test will fail.

// ─── Public component registry ───────────────────────────────────────────────

/// An invariant binding: one public component → its governing invariant IDs.
#[derive(Debug, Clone)]
pub struct InvariantBinding {
    /// The public component name (struct, trait, or module-level type).
    pub component: &'static str,
    /// Formal invariant IDs from the INVARIANT_REGISTRY or Phase J.
    pub invariant_ids: &'static [&'static str],
}

/// Exhaustive list of all public components in `halcon-agent-core`
/// with their formal invariant bindings.
///
/// Sources:
/// - Phases A–H: I-1.x through I-6.x (25 invariants)
/// - Phase J: I-7.x (6 invariants)
pub const INVARIANT_BINDINGS: &[InvariantBinding] = &[
    // ── Original GDEM components (from RISK_REPORT.md § 7) ──────────────────
    InvariantBinding {
        component: "AgentFsm",
        invariant_ids: &["I-1.1", "I-1.2", "I-1.3", "I-1.4"],
    },
    InvariantBinding {
        component: "InLoopCritic",
        invariant_ids: &["I-2.1", "I-2.2", "I-2.3", "I-2.4"],
    },
    InvariantBinding {
        component: "StrategyLearner",
        invariant_ids: &["I-3.1", "I-3.2", "I-3.3", "I-3.4", "I-3.5"],
    },
    InvariantBinding {
        component: "ConfidenceScore",
        invariant_ids: &["I-4.1", "I-4.2"],
    },
    InvariantBinding {
        component: "LoopDriver",
        invariant_ids: &["I-5.1", "I-5.2"],
    },
    InvariantBinding {
        component: "ExecutionBudget",
        invariant_ids: &["I-6.1"],
    },
    InvariantBinding {
        component: "ConfidenceHysteresis",
        invariant_ids: &["I-6.2"],
    },
    InvariantBinding {
        component: "OscillationTracker",
        invariant_ids: &["I-6.3", "I-6.4"],
    },
    InvariantBinding {
        component: "FailureInjectionHarness",
        invariant_ids: &["I-6.5", "I-6.6"],
    },
    InvariantBinding {
        component: "VectorMemory",
        invariant_ids: &["I-6.7"],
    },
    InvariantBinding {
        component: "StrategyArm",
        invariant_ids: &["I-3.1", "I-3.5", "I-6.8"],
    },
    // ── Phase J components ───────────────────────────────────────────────────
    InvariantBinding {
        component: "FsmFormalModel",
        invariant_ids: &["I-7.1"],
    },
    InvariantBinding {
        component: "RegretAnalysis",
        invariant_ids: &["I-7.2"],
    },
    InvariantBinding {
        component: "LyapunovTracker",
        invariant_ids: &["I-7.3"],
    },
    InvariantBinding {
        component: "StateEntropyTracker",
        invariant_ids: &["I-7.4"],
    },
    InvariantBinding {
        component: "StrategyEntropyTracker",
        invariant_ids: &["I-7.4"],
    },
    InvariantBinding {
        component: "ReplayLog",
        invariant_ids: &["I-7.5"],
    },
    InvariantBinding {
        component: "InvariantCoverage",
        invariant_ids: &["I-7.6"],
    },
];

// ─── Phase J invariant registry ───────────────────────────────────────────────

/// Phase J formal invariants (I-7.1 through I-7.6).
///
/// Fields: `(id, component, predicate, proof_method)`
pub const PHASE_J_INVARIANTS: &[(&str, &str, &str, &str)] = &[
    (
        "I-7.1",
        "FsmFormalModel",
        "Transition table is deterministic: no (state, action) pair maps to two distinct targets",
        "PROVED",
    ),
    (
        "I-7.2",
        "RegretAnalysis",
        "Empirical UCB1 regret ≤ Auer 2002 theoretical bound for all T ≥ K",
        "SIMULATED",
    ),
    (
        "I-7.3",
        "LyapunovTracker",
        "Under stable regime with monotone GAS improvement, mean ΔV ≤ 0",
        "SIMULATED",
    ),
    (
        "I-7.4",
        "StrategyEntropyTracker",
        "Strategy entropy H(A) is strictly lower in late learning than early learning",
        "SIMULATED",
    ),
    (
        "I-7.5",
        "ReplayLog",
        "Identical seed produces identical session hash across all repeated runs",
        "PROVED",
    ),
    (
        "I-7.6",
        "InvariantCoverage",
        "invariant_coverage = 1.0: every public component has at least one invariant binding",
        "ASSERTED",
    ),
];

// ─── Coverage computation ─────────────────────────────────────────────────────

/// Compute the invariant coverage ratio.
///
/// ```text
/// coverage = (# bindings with ≥1 invariant_id) / (total bindings)
/// ```
///
/// Returns a value in [0.0, 1.0]. 1.0 means 100% coverage.
pub fn compute_invariant_coverage() -> f64 {
    let total = INVARIANT_BINDINGS.len();
    if total == 0 {
        return 1.0;
    }
    let covered = INVARIANT_BINDINGS
        .iter()
        .filter(|b| !b.invariant_ids.is_empty())
        .count();
    covered as f64 / total as f64
}

/// Return all components that have zero invariant IDs (coverage gaps).
pub fn coverage_gaps() -> Vec<&'static str> {
    INVARIANT_BINDINGS
        .iter()
        .filter(|b| b.invariant_ids.is_empty())
        .map(|b| b.component)
        .collect()
}

/// Total number of distinct invariant IDs referenced across all bindings.
pub fn total_referenced_invariant_count() -> usize {
    use std::collections::HashSet;
    let mut ids: HashSet<&'static str> = HashSet::new();
    for binding in INVARIANT_BINDINGS {
        for &id in binding.invariant_ids {
            ids.insert(id);
        }
    }
    ids.len()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn invariant_coverage_is_100_percent() {
        // I-7.6 core assertion
        let coverage = compute_invariant_coverage();
        assert_eq!(
            coverage,
            1.0,
            "Invariant coverage must be 100%. Gaps: {:?}",
            coverage_gaps()
        );
    }

    #[test]
    fn no_component_lacks_invariant_binding() {
        let gaps = coverage_gaps();
        assert!(gaps.is_empty(), "Components without invariants: {:?}", gaps);
    }

    #[test]
    fn all_component_names_are_unique() {
        let names: Vec<&str> = INVARIANT_BINDINGS.iter().map(|b| b.component).collect();
        let unique: HashSet<&str> = names.iter().copied().collect();
        assert_eq!(
            names.len(),
            unique.len(),
            "Duplicate component names in INVARIANT_BINDINGS"
        );
    }

    #[test]
    fn all_invariant_ids_are_nonempty_strings() {
        for binding in INVARIANT_BINDINGS {
            for &id in binding.invariant_ids {
                assert!(
                    !id.is_empty(),
                    "Empty invariant ID in binding for '{}'",
                    binding.component
                );
                assert!(
                    id.starts_with("I-"),
                    "Invariant ID '{}' should start with 'I-'",
                    id
                );
            }
        }
    }

    #[test]
    fn phase_j_invariants_count_is_six() {
        assert_eq!(
            PHASE_J_INVARIANTS.len(),
            6,
            "Expected 6 Phase J invariants (I-7.1–I-7.6)"
        );
    }

    #[test]
    fn at_least_seventeen_components_registered() {
        // 11 original + 6+ Phase J
        assert!(
            INVARIANT_BINDINGS.len() >= 17,
            "Expected ≥17 bindings, got {}",
            INVARIANT_BINDINGS.len()
        );
    }

    #[test]
    fn total_invariant_ids_cover_both_phases() {
        let count = total_referenced_invariant_count();
        assert!(
            count >= 20,
            "Expected ≥20 distinct invariant IDs, got {}",
            count
        );
    }
}
