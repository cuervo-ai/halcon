//! BudgetEnvelope — centralized session-level resource tracking.
//!
//! Tracks time, tokens, cost, and rounds as a single consumable envelope.
//! All fields are decremented atomically as the agent loop progresses.
//!
//! ## Invariants
//!
//! - All fields are non-negative (clamped to 0 on underflow).
//! - `is_exhausted()` returns true when ANY resource hits 0.
//! - Thread-safe: uses atomic operations for token/round counters.
//!
//! ## Resolves
//!
//! - RE-1: `max_duration_secs = 0` allows unbounded execution.
//! - P3/P7: Budget governance principle.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

/// Session-level budget envelope.
///
/// Created once at session start from `AgentLimits`. Consumed during execution.
/// When any resource is exhausted, the agent loop should synthesize and terminate.
#[derive(Debug)]
pub struct BudgetEnvelope {
    /// Session start time (for duration tracking).
    start: Instant,
    /// Maximum session duration in seconds. 0 = unlimited.
    max_duration_secs: u64,
    /// Maximum total tokens. 0 = unlimited.
    max_tokens: u64,
    /// Tokens consumed so far (input + output).
    tokens_used: AtomicU64,
    /// Maximum cost in microdollars (USD × 1_000_000). 0 = unlimited.
    /// Using integer microdollars avoids AtomicF64 (which doesn't exist in std).
    max_cost_micros: u64,
    /// Cost consumed so far in microdollars.
    cost_used_micros: AtomicU64,
    /// Maximum rounds. 0 = unlimited.
    max_rounds: u32,
    /// Rounds consumed so far.
    rounds_used: AtomicU32,
}

/// Snapshot of remaining budget (immutable, for passing to components).
#[derive(Debug, Clone)]
pub struct BudgetSnapshot {
    pub time_remaining_secs: u64,
    pub tokens_remaining: u64,
    pub cost_remaining_usd: f64,
    pub rounds_remaining: u32,
    pub time_fraction_used: f64,
    pub cost_fraction_used: f64,
}

/// What was consumed by a single round.
#[derive(Debug, Clone, Default)]
pub struct RoundCost {
    pub tokens: u64,
    pub cost_usd: f64,
}

impl BudgetEnvelope {
    /// Create a new envelope from agent limits.
    pub fn from_limits(limits: &super::config::AgentLimits) -> Self {
        Self {
            start: Instant::now(),
            max_duration_secs: limits.max_duration_secs,
            max_tokens: limits.max_total_tokens as u64,
            tokens_used: AtomicU64::new(0),
            max_cost_micros: (limits.max_cost_usd * 1_000_000.0) as u64,
            cost_used_micros: AtomicU64::new(0),
            max_rounds: limits.max_rounds as u32,
            rounds_used: AtomicU32::new(0),
        }
    }

    /// Record consumption of one round.
    pub fn deduct_round(&self, cost: &RoundCost) {
        self.tokens_used.fetch_add(cost.tokens, Ordering::Relaxed);
        let cost_micros = (cost.cost_usd * 1_000_000.0) as u64;
        self.cost_used_micros
            .fetch_add(cost_micros, Ordering::Relaxed);
        self.rounds_used.fetch_add(1, Ordering::Relaxed);
    }

    /// Check if there's budget for at least one more round.
    pub fn has_budget(&self) -> bool {
        // Time check
        if self.max_duration_secs > 0 {
            let elapsed = self.start.elapsed().as_secs();
            if elapsed >= self.max_duration_secs {
                return false;
            }
        }
        // Token check
        if self.max_tokens > 0 {
            let used = self.tokens_used.load(Ordering::Relaxed);
            if used >= self.max_tokens {
                return false;
            }
        }
        // Cost check
        if self.max_cost_micros > 0 {
            let used = self.cost_used_micros.load(Ordering::Relaxed);
            if used >= self.max_cost_micros {
                return false;
            }
        }
        // Round check
        if self.max_rounds > 0 {
            let used = self.rounds_used.load(Ordering::Relaxed);
            if used >= self.max_rounds {
                return false;
            }
        }
        true
    }

    /// Whether >80% of any budget dimension is consumed (for warnings).
    pub fn is_warning_threshold(&self) -> bool {
        if self.max_duration_secs > 0 {
            let elapsed = self.start.elapsed().as_secs();
            if elapsed as f64 >= self.max_duration_secs as f64 * 0.8 {
                return true;
            }
        }
        if self.max_cost_micros > 0 {
            let used = self.cost_used_micros.load(Ordering::Relaxed);
            if used as f64 >= self.max_cost_micros as f64 * 0.8 {
                return true;
            }
        }
        if self.max_tokens > 0 {
            let used = self.tokens_used.load(Ordering::Relaxed);
            if used as f64 >= self.max_tokens as f64 * 0.8 {
                return true;
            }
        }
        false
    }

    /// Immutable snapshot of remaining budget.
    pub fn snapshot(&self) -> BudgetSnapshot {
        let elapsed_secs = self.start.elapsed().as_secs();
        let tokens_used = self.tokens_used.load(Ordering::Relaxed);
        let cost_used_micros = self.cost_used_micros.load(Ordering::Relaxed);
        let rounds_used = self.rounds_used.load(Ordering::Relaxed);

        BudgetSnapshot {
            time_remaining_secs: if self.max_duration_secs > 0 {
                self.max_duration_secs.saturating_sub(elapsed_secs)
            } else {
                u64::MAX
            },
            tokens_remaining: if self.max_tokens > 0 {
                self.max_tokens.saturating_sub(tokens_used)
            } else {
                u64::MAX
            },
            cost_remaining_usd: if self.max_cost_micros > 0 {
                (self.max_cost_micros.saturating_sub(cost_used_micros)) as f64 / 1_000_000.0
            } else {
                f64::MAX
            },
            rounds_remaining: if self.max_rounds > 0 {
                self.max_rounds.saturating_sub(rounds_used)
            } else {
                u32::MAX
            },
            time_fraction_used: if self.max_duration_secs > 0 {
                elapsed_secs as f64 / self.max_duration_secs as f64
            } else {
                0.0
            },
            cost_fraction_used: if self.max_cost_micros > 0 {
                cost_used_micros as f64 / self.max_cost_micros as f64
            } else {
                0.0
            },
        }
    }

    /// Total elapsed time since session start.
    pub fn elapsed_secs(&self) -> u64 {
        self.start.elapsed().as_secs()
    }

    /// Total tokens consumed.
    pub fn tokens_used(&self) -> u64 {
        self.tokens_used.load(Ordering::Relaxed)
    }

    /// Total cost in USD.
    pub fn cost_used_usd(&self) -> f64 {
        self.cost_used_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    /// Rounds consumed.
    pub fn rounds_used(&self) -> u32 {
        self.rounds_used.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::AgentLimits;

    #[test]
    fn budget_from_defaults_has_budget() {
        let limits = AgentLimits::default();
        let env = BudgetEnvelope::from_limits(&limits);
        assert!(env.has_budget());
    }

    #[test]
    fn deduct_round_decrements_counters() {
        let mut limits = AgentLimits::default();
        limits.max_rounds = 5;
        limits.max_total_tokens = 10000;
        limits.max_cost_usd = 1.0;
        let env = BudgetEnvelope::from_limits(&limits);

        env.deduct_round(&RoundCost {
            tokens: 2000,
            cost_usd: 0.15,
        });
        assert_eq!(env.tokens_used(), 2000);
        assert_eq!(env.rounds_used(), 1);
        assert!((env.cost_used_usd() - 0.15).abs() < 0.001);
        assert!(env.has_budget());
    }

    #[test]
    fn budget_exhausted_by_rounds() {
        let mut limits = AgentLimits::default();
        limits.max_rounds = 2;
        let env = BudgetEnvelope::from_limits(&limits);

        env.deduct_round(&RoundCost::default());
        assert!(env.has_budget());
        env.deduct_round(&RoundCost::default());
        assert!(!env.has_budget());
    }

    #[test]
    fn budget_exhausted_by_tokens() {
        let mut limits = AgentLimits::default();
        limits.max_total_tokens = 1000;
        let env = BudgetEnvelope::from_limits(&limits);

        env.deduct_round(&RoundCost {
            tokens: 1001,
            cost_usd: 0.0,
        });
        assert!(!env.has_budget());
    }

    #[test]
    fn budget_exhausted_by_cost() {
        let mut limits = AgentLimits::default();
        limits.max_cost_usd = 0.50;
        let env = BudgetEnvelope::from_limits(&limits);

        env.deduct_round(&RoundCost {
            tokens: 0,
            cost_usd: 0.51,
        });
        assert!(!env.has_budget());
    }

    #[test]
    fn snapshot_shows_remaining() {
        let mut limits = AgentLimits::default();
        limits.max_rounds = 10;
        limits.max_total_tokens = 5000;
        limits.max_cost_usd = 2.0;
        let env = BudgetEnvelope::from_limits(&limits);

        env.deduct_round(&RoundCost {
            tokens: 1000,
            cost_usd: 0.30,
        });
        let snap = env.snapshot();
        assert_eq!(snap.tokens_remaining, 4000);
        assert_eq!(snap.rounds_remaining, 9);
        assert!((snap.cost_remaining_usd - 1.70).abs() < 0.01);
    }

    #[test]
    fn unlimited_budget_returns_max() {
        let mut limits = AgentLimits::default();
        limits.max_total_tokens = 0;
        limits.max_cost_usd = 0.0;
        limits.max_rounds = 0;
        limits.max_duration_secs = 0;
        let env = BudgetEnvelope::from_limits(&limits);

        assert!(env.has_budget());
        let snap = env.snapshot();
        assert_eq!(snap.tokens_remaining, u64::MAX);
        assert_eq!(snap.rounds_remaining, u32::MAX);
    }

    #[test]
    fn warning_threshold_at_80_percent() {
        let mut limits = AgentLimits::default();
        limits.max_total_tokens = 100;
        limits.max_cost_usd = 1.0;
        let env = BudgetEnvelope::from_limits(&limits);

        // Below 80%
        env.deduct_round(&RoundCost {
            tokens: 79,
            cost_usd: 0.0,
        });
        assert!(!env.is_warning_threshold());

        // At 80%+
        env.deduct_round(&RoundCost {
            tokens: 2,
            cost_usd: 0.0,
        });
        assert!(env.is_warning_threshold());
    }
}
