//! Tenant identity + per-tenant limits threaded through the execution stack.
//!
//! `TenantScope` is the single struct that carries WHO is executing and
//! WHAT they are allowed to spend.  It is consumed by the `ExecutionCoordinator`
//! (budget + rate limit), by the agent loop (audit event tagging), and by the
//! API layer (admission control at submit time).
//!
//! ## Why a value-type, not a trait
//!
//! Policy shapes vary between deployments (free tier vs enterprise vs internal),
//! but the *enforcement surface* is small (budget + rps + concurrency).  A value
//! struct threads cheaply across async boundaries and is trivially `Clone`.
//!
//! ## Provider / modality invariance
//!
//! TenantScope carries no provider-specific or modality-specific state.  It is
//! the same for text, image, audio, or mixed-modality sessions.
//!
//! ## Limits semantics
//!
//! - `token_limit` and `cost_limit_usd` are **per session** caps (enforced by
//!   `RuntimeBudget`).
//! - `rps_limit` is the steady-state submit rate per tenant (enforced at the
//!   API layer, not here — this struct only transports the value).
//! - `concurrent_limit` is the max simultaneous sessions per tenant (enforced
//!   at the API layer).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::budget::RuntimeBudget;

/// Deployment tier for the tenant.  Drives default limits and fair-share weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantTier {
    /// Paid enterprise plan: highest limits, priority weight 4.
    Enterprise,
    /// Standard paid tier: default limits, priority weight 2.
    Standard,
    /// Free or preview tier: tight limits, priority weight 1.
    Trial,
    /// Internal / system agent (no enforcement).  Use sparingly.
    System,
}

impl TenantTier {
    /// Weight for weighted-fair-queue scheduling.
    pub fn weight(&self) -> u32 {
        match self {
            Self::Enterprise => 4,
            Self::Standard => 2,
            Self::Trial => 1,
            Self::System => 8, // system drains its own queue first
        }
    }
}

/// Per-session limits derived from the tenant's plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TenantLimits {
    pub token_limit: u64,
    pub cost_limit_usd: f64,
    pub duration_limit: Duration,
    pub rps_limit: u32,
    pub concurrent_limit: usize,
}

impl TenantLimits {
    /// Default limits for a given tier — safe ceilings for a shared deployment.
    pub fn for_tier(tier: TenantTier) -> Self {
        match tier {
            TenantTier::Enterprise => Self {
                token_limit: 2_000_000,
                cost_limit_usd: 25.0,
                duration_limit: Duration::from_secs(3_600),
                rps_limit: 60,
                concurrent_limit: 50,
            },
            TenantTier::Standard => Self {
                token_limit: 500_000,
                cost_limit_usd: 5.0,
                duration_limit: Duration::from_secs(1_800),
                rps_limit: 20,
                concurrent_limit: 10,
            },
            TenantTier::Trial => Self {
                token_limit: 50_000,
                cost_limit_usd: 0.50,
                duration_limit: Duration::from_secs(300),
                rps_limit: 5,
                concurrent_limit: 2,
            },
            TenantTier::System => Self {
                token_limit: 0, // unlimited
                cost_limit_usd: 0.0,
                duration_limit: Duration::ZERO,
                rps_limit: 0,
                concurrent_limit: 0,
            },
        }
    }
}

/// Tenant identity + limits.  Thread this through every execution entry point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TenantScope {
    pub tenant_id: String,
    pub tier: TenantTier,
    pub limits: TenantLimits,
}

impl TenantScope {
    /// Build a scope from an explicit tenant id + tier using tier-default limits.
    pub fn new(tenant_id: impl Into<String>, tier: TenantTier) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            tier,
            limits: TenantLimits::for_tier(tier),
        }
    }

    /// Anonymous / system scope — limits are unrestricted.  Use for local CLI
    /// sessions or internal agents where no tenant auth is present.
    pub fn anonymous() -> Self {
        Self {
            tenant_id: "anonymous".into(),
            tier: TenantTier::System,
            limits: TenantLimits::for_tier(TenantTier::System),
        }
    }

    /// Override limits while keeping identity (e.g. config-loaded custom plan).
    pub fn with_limits(mut self, limits: TenantLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Materialize a `RuntimeBudget` reflecting this scope's caps.
    pub fn to_runtime_budget(&self) -> RuntimeBudget {
        RuntimeBudget::new(
            self.limits.token_limit,
            self.limits.cost_limit_usd,
            self.limits.duration_limit,
        )
    }
}

impl Default for TenantScope {
    fn default() -> Self {
        Self::anonymous()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_weights_respect_priority_order() {
        assert!(TenantTier::System.weight() > TenantTier::Enterprise.weight());
        assert!(TenantTier::Enterprise.weight() > TenantTier::Standard.weight());
        assert!(TenantTier::Standard.weight() > TenantTier::Trial.weight());
    }

    #[test]
    fn tier_limits_monotonic() {
        let ent = TenantLimits::for_tier(TenantTier::Enterprise);
        let std = TenantLimits::for_tier(TenantTier::Standard);
        let trial = TenantLimits::for_tier(TenantTier::Trial);
        assert!(ent.token_limit > std.token_limit);
        assert!(std.token_limit > trial.token_limit);
        assert!(ent.cost_limit_usd > std.cost_limit_usd);
        assert!(std.cost_limit_usd > trial.cost_limit_usd);
    }

    #[test]
    fn anonymous_is_system_tier_unrestricted() {
        let scope = TenantScope::anonymous();
        assert_eq!(scope.tenant_id, "anonymous");
        assert_eq!(scope.tier, TenantTier::System);
        assert_eq!(scope.limits.token_limit, 0); // 0 = unlimited in RuntimeBudget
    }

    #[test]
    fn scope_to_budget_respects_limits() {
        let scope = TenantScope::new("tenant-42", TenantTier::Trial);
        let budget = scope.to_runtime_budget();
        assert_eq!(budget.token_limit(), 50_000);
        assert!((budget.cost_limit_usd() - 0.50).abs() < 1e-6);
    }

    #[test]
    fn with_limits_override_works() {
        let scope = TenantScope::new("t", TenantTier::Standard).with_limits(TenantLimits {
            token_limit: 42,
            cost_limit_usd: 0.01,
            duration_limit: Duration::from_secs(10),
            rps_limit: 1,
            concurrent_limit: 1,
        });
        assert_eq!(scope.limits.token_limit, 42);
    }

    #[test]
    fn serde_roundtrip() {
        let scope = TenantScope::new("acme-corp", TenantTier::Enterprise);
        let json = serde_json::to_string(&scope).unwrap();
        let back: TenantScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, back);
    }
}
