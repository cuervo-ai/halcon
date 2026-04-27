//! Role-based limit policy for multi-agent orchestration.
//!
//! Different agent roles (Lead, Teammate, Specialist, Observer) receive
//! different execution budgets. This module extracts that logic from
//! `halcon-cli/src/repl/orchestrator.rs` (Stack #3) lines 488-507 into
//! a trait so the coordinator can consume it via dependency injection.

use halcon_core::types::{AgentLimits, AgentRole};

/// Policy that adjusts execution limits based on the agent's role.
pub trait RolePolicy: Send + Sync {
    /// Apply role-based adjustments to base limits.
    fn apply(&self, role: &AgentRole, base: AgentLimits) -> AgentLimits;
}

/// Default implementation matching `repl/orchestrator.rs` lines 488-507.
///
/// Rules:
/// - Duration scaled by `role.timeout_multiplier()`.
/// - Rounds scaled by `role.max_rounds_multiplier()` (ceil).
/// - Observer: hard-zero rounds (never enters tool loop).
pub struct DefaultRolePolicy;

impl RolePolicy for DefaultRolePolicy {
    fn apply(&self, role: &AgentRole, mut limits: AgentLimits) -> AgentLimits {
        // Duration multiplier.
        if limits.max_duration_secs > 0 {
            limits.max_duration_secs =
                (limits.max_duration_secs as f64 * role.timeout_multiplier()) as u64;
        }

        // Rounds multiplier for non-Lead roles.
        let rounds_mult = role.max_rounds_multiplier();
        if rounds_mult < 1.0 {
            limits.max_rounds = ((limits.max_rounds as f64) * rounds_mult).ceil() as usize;
        }

        // Observer: never enters tool loop.
        if !role.can_execute_tools() {
            limits.max_rounds = 0;
        }

        limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_limits() -> AgentLimits {
        AgentLimits {
            max_rounds: 10,
            max_duration_secs: 120,
            max_total_tokens: 50_000,
            ..Default::default()
        }
    }

    #[test]
    fn lead_role_unchanged() {
        let policy = DefaultRolePolicy;
        let limits = policy.apply(&AgentRole::Lead, base_limits());
        assert_eq!(limits.max_rounds, 10);
        assert_eq!(limits.max_duration_secs, 120);
    }

    #[test]
    fn observer_gets_zero_rounds() {
        let policy = DefaultRolePolicy;
        let limits = policy.apply(&AgentRole::Observer, base_limits());
        assert_eq!(limits.max_rounds, 0);
    }

    #[test]
    fn teammate_scales_duration() {
        let policy = DefaultRolePolicy;
        let limits = policy.apply(&AgentRole::Teammate, base_limits());
        // Teammate timeout_multiplier is typically 0.8 — check it's scaled
        let expected_secs = (120.0 * AgentRole::Teammate.timeout_multiplier()) as u64;
        assert_eq!(limits.max_duration_secs, expected_secs);
    }

    #[test]
    fn specialist_scales_rounds() {
        let policy = DefaultRolePolicy;
        let limits = policy.apply(&AgentRole::Specialist, base_limits());
        let mult = AgentRole::Specialist.max_rounds_multiplier();
        if mult < 1.0 {
            let expected = ((10.0_f64) * mult).ceil() as usize;
            assert_eq!(limits.max_rounds, expected);
        } else {
            assert_eq!(limits.max_rounds, 10);
        }
    }

    #[test]
    fn zero_duration_stays_zero() {
        let policy = DefaultRolePolicy;
        let mut limits = base_limits();
        limits.max_duration_secs = 0;
        let result = policy.apply(&AgentRole::Teammate, limits);
        assert_eq!(result.max_duration_secs, 0);
    }
}
