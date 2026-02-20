//! Plugin Registry — central hub for all V3 plugin state.
//!
//! The registry owns:
//! - Loaded plugin manifests + state FSM
//! - Per-plugin circuit breakers (isolated from global ToolFailureTracker)
//! - Per-plugin cost trackers
//! - The permission gate (session-wide ceiling + per-plugin supervisor restrictions)
//! - The capability resolver (BM25 index over all registered capabilities)
//!
//! All public methods are synchronous; the registry is wrapped in `Option<>` on
//! AgentContext so that **zero plugin code executes when plugins are not configured**.

use std::collections::HashMap;
use std::time::Duration;

use super::capability_index::CapabilityIndex;
use super::capability_resolver::CapabilityResolver;
use super::plugin_circuit_breaker::PluginCircuitBreaker;
use super::plugin_cost_tracker::{PluginCostSnapshot, PluginCostTracker};
use super::plugin_manifest::{PluginManifest, RiskTier};
use super::plugin_permission_gate::{PluginPermissionDecision, PluginPermissionGate};

// ─── UCB1 Arm ─────────────────────────────────────────────────────────────────

/// Per-plugin UCB1 bandit arm for cross-session reward tracking.
#[derive(Debug, Clone, Default)]
struct PluginUcbArm {
    n_uses: u32,
    sum_rewards: f64,
}

impl PluginUcbArm {
    fn avg_reward(&self) -> f64 {
        if self.n_uses == 0 { 0.5 } else { self.sum_rewards / self.n_uses as f64 }
    }

    fn ucb1_score(&self, total_uses: u32, c: f64) -> f64 {
        if self.n_uses == 0 {
            f64::MAX
        } else {
            let t = (total_uses as f64).max(1.0);
            self.avg_reward() + c * (t.ln() / self.n_uses as f64).sqrt()
        }
    }
}

// ─── Plugin State ─────────────────────────────────────────────────────────────

/// FSM state of a loaded plugin.
#[derive(Debug, Clone)]
pub enum PluginState {
    /// Normal operation.
    Active,
    /// Experiencing failures but not yet circuit-broken.
    Degraded { consecutive_failures: u32 },
    /// Suspended by supervisor action — all invocations are denied.
    Suspended { reason: String },
    /// Circuit breaker permanently tripped — requires manual reset.
    Failed { reason: String },
}

impl PluginState {
    pub fn is_active(&self) -> bool {
        matches!(self, PluginState::Active | PluginState::Degraded { .. })
    }
}

// ─── Loaded Plugin ────────────────────────────────────────────────────────────

/// A plugin that has been registered and validated.
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub state: PluginState,
}

// ─── Gate Result ─────────────────────────────────────────────────────────────

/// Outcome of the pre-invoke gate check.
#[derive(Debug, Clone)]
pub enum InvokeGateResult {
    /// Execution may proceed.
    Proceed,
    /// Execution denied; the message should become a synthetic `ToolResult` with `is_error: true`.
    Deny(String),
}

// ─── Registry ─────────────────────────────────────────────────────────────────

/// Central V3 plugin hub.
pub struct PluginRegistry {
    plugins: HashMap<String, LoadedPlugin>,
    circuit_breakers: HashMap<String, PluginCircuitBreaker>,
    cost_trackers: HashMap<String, PluginCostTracker>,
    permission_gate: PluginPermissionGate,
    capability_resolver: CapabilityResolver,
    /// Per-plugin UCB1 bandit arms for reward-based routing.
    plugin_bandits: HashMap<String, PluginUcbArm>,
}

impl PluginRegistry {
    /// Create an empty registry with a default permissive permission gate.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            circuit_breakers: HashMap::new(),
            cost_trackers: HashMap::new(),
            permission_gate: PluginPermissionGate::default_permissive(),
            capability_resolver: CapabilityResolver::new(CapabilityIndex::build(&[])),
            plugin_bandits: HashMap::new(),
        }
    }

    /// Register a plugin from a manifest.
    ///
    /// Rebuilds the capability index after registration.
    /// No I/O — safe to call in tests.
    pub fn register(&mut self, manifest: PluginManifest) {
        let plugin_id = manifest.meta.id.clone();
        let threshold = manifest.supervisor_policy.halt_on_failures;

        self.circuit_breakers.insert(
            plugin_id.clone(),
            PluginCircuitBreaker::new(threshold, Duration::from_secs(60)),
        );
        self.cost_trackers.insert(
            plugin_id.clone(),
            PluginCostTracker::unlimited(plugin_id.clone()),
        );
        self.plugins.insert(
            plugin_id.clone(),
            LoadedPlugin { manifest, state: PluginState::Active },
        );

        // Rebuild capability index after every registration
        self.rebuild_capability_index();
    }

    /// Pre-invoke gate: check circuit breaker + cost budget + permissions.
    ///
    /// Returns `Deny(reason)` when the call should be blocked; `Proceed` otherwise.
    pub fn pre_invoke_gate(&self, plugin_id: &str, tool_name: &str, budget_low: bool) -> InvokeGateResult {
        // Plugin must exist and be in an invocable state
        let plugin = match self.plugins.get(plugin_id) {
            Some(p) => p,
            None => return InvokeGateResult::Deny(format!("plugin '{plugin_id}' not found")),
        };

        // Suspended or Failed → deny immediately
        if !plugin.state.is_active() {
            let reason = match &plugin.state {
                PluginState::Suspended { reason } => {
                    format!("plugin '{plugin_id}' is suspended: {reason}")
                }
                PluginState::Failed { reason } => {
                    format!("plugin '{plugin_id}' has failed: {reason}")
                }
                _ => format!("plugin '{plugin_id}' is not active"),
            };
            return InvokeGateResult::Deny(reason);
        }

        // Circuit breaker open → deny
        if let Some(cb) = self.circuit_breakers.get(plugin_id) {
            if cb.is_open() {
                return InvokeGateResult::Deny(format!(
                    "plugin '{plugin_id}' circuit breaker is open — backing off"
                ));
            }
        }

        // Cost budget → deny if exceeded
        if let Some(tracker) = self.cost_trackers.get(plugin_id) {
            if let Some(budget_err) = tracker.check_budget() {
                return InvokeGateResult::Deny(format!("plugin budget: {budget_err}"));
            }
        }

        // Permission gate: find the capability descriptor
        let cap = plugin.manifest.capabilities.iter().find(|c| c.name == tool_name);
        if let Some(cap) = cap {
            let decision = self.permission_gate.evaluate(plugin_id, cap, budget_low);
            match decision {
                PluginPermissionDecision::Allowed => {}
                PluginPermissionDecision::NeedsConfirmation => {
                    // In non-interactive mode, treat NeedsConfirmation as Allowed.
                    // In interactive mode, the executor should prompt the user.
                    // For now, we allow it — Phase 8 will wire the interactive prompt.
                }
                PluginPermissionDecision::Denied { reason } => {
                    return InvokeGateResult::Deny(reason);
                }
            }
        }

        InvokeGateResult::Proceed
    }

    /// Post-invoke: record call outcome in circuit breaker and cost tracker.
    pub fn post_invoke(
        &mut self,
        plugin_id: &str,
        _tool_name: &str,
        tokens_used: u64,
        usd_cost: f64,
        success: bool,
        error: Option<&str>,
    ) {
        // Update circuit breaker
        if let Some(cb) = self.circuit_breakers.get_mut(plugin_id) {
            if success {
                cb.record_success();
            } else {
                let tripped = cb.record_failure();
                if tripped {
                    let reason = error.unwrap_or("circuit breaker tripped").to_string();
                    if let Some(p) = self.plugins.get_mut(plugin_id) {
                        p.state = PluginState::Failed { reason };
                    }
                    return;
                }
                // Update to Degraded state
                if let Some(p) = self.plugins.get_mut(plugin_id) {
                    let consec = cb.consecutive_failures();
                    p.state = PluginState::Degraded { consecutive_failures: consec };
                }
            }
        }

        // Update cost tracker
        if let Some(tracker) = self.cost_trackers.get_mut(plugin_id) {
            tracker.record_call(tokens_used, usd_cost, success);
        }

        // On success, restore to Active if it was Degraded
        if success {
            if let Some(p) = self.plugins.get_mut(plugin_id) {
                if matches!(p.state, PluginState::Degraded { .. }) {
                    p.state = PluginState::Active;
                }
            }
        }
    }

    /// Suspend a plugin (called by Supervisor `SuspendPlugin` verdict).
    pub fn suspend_plugin(&mut self, plugin_id: &str, reason: String) {
        if let Some(p) = self.plugins.get_mut(plugin_id) {
            p.state = PluginState::Suspended { reason };
        }
    }

    /// Record a per-plugin UCB1 reward signal (called post-loop by LoopCritic).
    ///
    /// Reward is clamped to [0.0, 1.0]. Accumulated per arm; UCB1 uses the total
    /// count across ALL arms as `t` (promotes exploration of under-used plugins).
    pub fn record_reward(&mut self, plugin_id: &str, reward: f64) {
        let arm = self.plugin_bandits
            .entry(plugin_id.to_string())
            .or_insert_with(PluginUcbArm::default);
        arm.n_uses += 1;
        arm.sum_rewards += reward.clamp(0.0, 1.0);
    }

    /// Select the best active plugin for a given capability tag using UCB1.
    ///
    /// The `capability_tag` is matched as a substring against each plugin's
    /// capability names. Returns `None` when no active plugin matches.
    pub fn select_best_for_capability(&self, capability_tag: &str) -> Option<&str> {
        let total: u32 = self.plugin_bandits.values().map(|a| a.n_uses).sum();
        self.plugins
            .iter()
            .filter(|(_, p)| p.state.is_active())
            .filter(|(_, p)| {
                p.manifest.capabilities.iter().any(|c| c.name.contains(capability_tag))
            })
            .max_by(|(id_a, _), (id_b, _)| {
                let score_a = self.plugin_bandits.get(*id_a)
                    .map(|a| a.ucb1_score(total, 1.4))
                    .unwrap_or(f64::MAX);
                let score_b = self.plugin_bandits.get(*id_b)
                    .map(|b| b.ucb1_score(total, 1.4))
                    .unwrap_or(f64::MAX);
                score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(id, _)| id.as_str())
    }

    /// Average reward for a plugin (0.5 if no data yet).
    pub fn plugin_avg_reward(&self, plugin_id: &str) -> f64 {
        self.plugin_bandits.get(plugin_id).map_or(0.5, |a| a.avg_reward())
    }

    /// Snapshot UCB1 arm data as (plugin_id, n_uses, sum_rewards) for persistence.
    pub fn ucb1_snapshot(&self) -> Vec<(String, u32, f64)> {
        self.plugin_bandits
            .iter()
            .map(|(id, arm)| (id.clone(), arm.n_uses, arm.sum_rewards))
            .collect()
    }

    /// Seed UCB1 arms from persisted data (loaded at session start).
    pub fn seed_ucb1_from_metrics(&mut self, seeds: &[(String, i64, f64)]) {
        for (plugin_id, n_uses, sum_rewards) in seeds {
            let arm = self.plugin_bandits
                .entry(plugin_id.clone())
                .or_insert_with(PluginUcbArm::default);
            arm.n_uses = (*n_uses as u32).max(arm.n_uses);
            arm.sum_rewards = if arm.n_uses > 0 { *sum_rewards } else { arm.sum_rewards };
        }
    }

    /// Collect cost snapshots for all registered plugins (for AgentLoopResult).
    pub fn cost_snapshot(&self) -> Vec<PluginCostSnapshot> {
        self.cost_trackers
            .values()
            .map(|t| t.snapshot())
            .collect()
    }

    /// Resolve a plugin ID for a tool name that uses the "plugin_<id>_<tool>" prefix pattern.
    ///
    /// Returns `None` for non-plugin tools.
    pub fn plugin_id_for_tool(&self, tool_name: &str) -> Option<&str> {
        // Pattern: tool_name starts with plugin_<id>_
        if tool_name.starts_with("plugin_") {
            // Find a plugin whose ID is embedded in the tool name
            for plugin_id in self.plugins.keys() {
                let prefix = format!("plugin_{}_", plugin_id.replace('-', "_"));
                if tool_name.starts_with(&prefix) {
                    return Some(plugin_id.as_str());
                }
            }
        }
        None
    }

    /// Whether any active plugins are registered.
    pub fn has_active_plugins(&self) -> bool {
        self.plugins.values().any(|p| p.state.is_active())
    }

    /// Count of active plugins.
    pub fn active_plugin_count(&self) -> usize {
        self.plugins.values().filter(|p| p.state.is_active()).count()
    }

    /// Access the capability resolver (for plan step routing).
    pub fn get_capability_resolver(&self) -> &CapabilityResolver {
        &self.capability_resolver
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn rebuild_capability_index(&mut self) {
        let pairs: Vec<(String, &PluginManifest)> = self
            .plugins
            .iter()
            .map(|(id, p)| (id.clone(), &p.manifest))
            .collect();
        let index = CapabilityIndex::build(&pairs);
        self.capability_resolver = CapabilityResolver::new(index);
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::plugin_manifest::{PluginManifest, ToolCapabilityDescriptor};

    fn make_manifest(id: &str) -> PluginManifest {
        PluginManifest::new_local(id, id, "1.0.0", vec![
            ToolCapabilityDescriptor {
                name: format!("plugin_{}_run", id.replace('-', "_")),
                description: format!("Run a task in {id}"),
                risk_tier: RiskTier::Low,
                idempotent: false,
                permission_level: halcon_core::types::PermissionLevel::ReadOnly,
                budget_tokens_per_call: 100,
            },
        ])
    }

    #[test]
    fn register_and_active_count() {
        let mut reg = PluginRegistry::new();
        assert_eq!(reg.active_plugin_count(), 0);
        reg.register(make_manifest("plugin-a"));
        assert_eq!(reg.active_plugin_count(), 1);
        reg.register(make_manifest("plugin-b"));
        assert_eq!(reg.active_plugin_count(), 2);
    }

    #[test]
    fn pre_invoke_circuit_open_denies() {
        let mut reg = PluginRegistry::new();
        // Use threshold=1 so first failure trips the circuit
        let mut manifest = make_manifest("fast-trip");
        manifest.supervisor_policy.halt_on_failures = 1;
        reg.register(manifest);

        let tool = "plugin_fast_trip_run";
        reg.post_invoke("fast-trip", tool, 0, 0.0, false, Some("err"));

        let result = reg.pre_invoke_gate("fast-trip", tool, false);
        assert!(matches!(result, InvokeGateResult::Deny(_)));
    }

    #[test]
    fn budget_denial_blocks_invoke() {
        let mut reg = PluginRegistry::new();
        reg.register(make_manifest("budget-plugin"));

        // Manually set call limit by replacing the tracker
        reg.cost_trackers.insert(
            "budget-plugin".into(),
            PluginCostTracker::new("budget-plugin".into(), None, None, Some(0)),
        );

        let result = reg.pre_invoke_gate("budget-plugin", "some_tool", false);
        assert!(matches!(result, InvokeGateResult::Deny(_)));
    }

    #[test]
    fn post_invoke_updates_cost_tracker() {
        let mut reg = PluginRegistry::new();
        reg.register(make_manifest("tracker-plugin"));
        reg.post_invoke("tracker-plugin", "tool", 150, 0.01, true, None);

        let snap = reg.cost_snapshot();
        let tracker_snap = snap.iter().find(|s| s.plugin_id == "tracker-plugin").unwrap();
        assert_eq!(tracker_snap.tokens_used, 150);
        assert_eq!(tracker_snap.calls_made, 1);
    }

    #[test]
    fn suspend_denies_all_invocations() {
        let mut reg = PluginRegistry::new();
        reg.register(make_manifest("suspend-me"));
        reg.suspend_plugin("suspend-me", "test suspension".into());

        let result = reg.pre_invoke_gate("suspend-me", "any_tool", false);
        assert!(matches!(result, InvokeGateResult::Deny(reason) if reason.contains("suspended")));
    }

    #[test]
    fn record_reward_accumulates_ucb1() {
        let mut reg = PluginRegistry::new();
        reg.register(make_manifest("reward-plugin"));
        reg.record_reward("reward-plugin", 0.8);
        reg.record_reward("reward-plugin", 1.0);
        assert!((reg.plugin_avg_reward("reward-plugin") - 0.9).abs() < 1e-9);
    }

    #[test]
    fn plugin_avg_reward_default_is_half() {
        let reg = PluginRegistry::new();
        assert!((reg.plugin_avg_reward("unknown") - 0.5).abs() < 1e-9);
    }

    #[test]
    fn select_best_for_capability_prefers_higher_reward() {
        let mut reg = PluginRegistry::new();
        reg.register(make_manifest("low-quality"));
        reg.register(make_manifest("high-quality"));

        // Give both some experience so UCB1 doesn't pick f64::MAX (unexplored)
        for _ in 0..5 {
            reg.record_reward("low-quality", 0.2);
            reg.record_reward("high-quality", 0.9);
        }

        // Both have capability "run" (from make_manifest)
        let winner = reg.select_best_for_capability("run");
        assert_eq!(winner, Some("high-quality"));
    }

    #[test]
    fn ucb1_snapshot_and_seed_roundtrip() {
        let mut reg = PluginRegistry::new();
        reg.register(make_manifest("snap-plugin"));
        reg.record_reward("snap-plugin", 0.7);
        reg.record_reward("snap-plugin", 0.9);

        let snap = reg.ucb1_snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].0, "snap-plugin");
        assert_eq!(snap[0].1, 2);
        assert!((snap[0].2 - 1.6).abs() < 1e-9);

        let mut reg2 = PluginRegistry::new();
        reg2.register(make_manifest("snap-plugin"));
        let seeds: Vec<(String, i64, f64)> = snap
            .iter()
            .map(|(id, n, r)| (id.clone(), *n as i64, *r))
            .collect();
        reg2.seed_ucb1_from_metrics(&seeds);
        assert!((reg2.plugin_avg_reward("snap-plugin") - 0.8).abs() < 1e-9);
    }

    #[test]
    fn cost_snapshot_returns_all_plugins() {
        let mut reg = PluginRegistry::new();
        reg.register(make_manifest("p1"));
        reg.register(make_manifest("p2"));
        let snaps = reg.cost_snapshot();
        assert_eq!(snaps.len(), 2);
    }

    #[test]
    fn plugin_id_for_tool_extracts_id() {
        let mut reg = PluginRegistry::new();
        reg.register(make_manifest("my-plugin"));
        let id = reg.plugin_id_for_tool("plugin_my_plugin_run");
        assert_eq!(id, Some("my-plugin"));
    }

    #[test]
    fn empty_registry_has_no_active_plugins() {
        let reg = PluginRegistry::new();
        assert!(!reg.has_active_plugins());
        assert_eq!(reg.active_plugin_count(), 0);
        let snaps = reg.cost_snapshot();
        assert!(snaps.is_empty());
    }
}
