//! User-accessible lifecycle hooks (Feature 2 — Halcon Frontier Roadmap 2026).
//!
//! # Overview
//!
//! Hooks let users run shell commands or Rhai scripts at well-defined lifecycle
//! events without modifying the Halcon binary.  They act as a **user-policy
//! layer** that sits between user intent and agent execution.
//!
//! # Security invariant
//!
//! Hooks are an **optional, user-policy layer**.  They run *before* FASE-2
//! CATASTROPHIC_PATTERNS checks in `bash.rs`.  A hook returning `Allow` does
//! **not** bypass FASE-2; both layers are independent.  A malicious
//! `.halcon/settings.toml` cannot disable the hard safety wall.
//!
//! ```text
//! ┌─ user-policy layer ─────────────────────────────────────────┐
//! │  PreToolUse hook → Deny? → block  / Allow? → continue       │
//! └─────────────────────────────────────────────────────────────┘
//!          ↓
//! ┌─ FASE-2 safety wall (immutable) ───────────────────────────┐
//! │  CATASTROPHIC_PATTERNS in bash.rs — never bypassed by hooks │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Feature flag
//!
//! Hooks are disabled by default.  To enable:
//! ```toml
//! # In ~/.halcon/settings.toml
//! [hooks]
//! enabled = true
//! [[hooks.hooks]]
//! event = "PreToolUse"
//! matcher = "bash"
//! type = "command"
//! command = "echo tool=$HALCON_TOOL_NAME"
//! ```
//!
//! The master kill switch `PolicyConfig::enable_hooks = false` (runtime) and
//! the per-session `HooksConfig::enabled = false` (config) must both be true
//! for any hook to fire.
//!
//! # Enterprise policy
//!
//! When `PolicyConfig::allow_managed_hooks_only = true`, only hooks defined in
//! `~/.halcon/settings.toml` (global/managed scope) are loaded.  Project-level
//! `.halcon/settings.toml` hooks are silently ignored.

pub mod command_hook;
pub mod config;
pub mod matcher;
pub mod rhai_hook;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::time::Duration;

pub use config::{HookDef, HookEventName, HookType, HooksConfig};

/// A lifecycle event fired during agent execution.
#[derive(Debug, Clone)]
pub struct HookEvent {
    /// Which event type occurred.
    pub name: HookEventName,
    /// Tool name (for tool-centric events; empty for UserPromptSubmit/Stop/SessionEnd).
    pub tool_name: String,
    /// JSON-encoded tool input (for tool-centric events; empty otherwise).
    pub tool_input_json: String,
    /// Current session ID (UUID string).
    pub session_id: String,
}

impl HookEvent {
    /// Build environment variables to inject into child processes / Rhai scripts.
    pub fn env_vars(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("HALCON_EVENT".to_string(), format!("{:?}", self.name));
        m.insert("HALCON_TOOL_NAME".to_string(), self.tool_name.clone());
        m.insert("HALCON_TOOL_INPUT".to_string(), self.tool_input_json.clone());
        m.insert("HALCON_SESSION_ID".to_string(), self.session_id.clone());
        m
    }
}

/// Outcome of firing a hook.
#[derive(Debug, Clone)]
pub enum HookOutcome {
    /// Operation is allowed.  Agent continues normally.
    Allow,
    /// Operation is blocked.  Contains the denial reason shown to the agent.
    ///
    /// Only `PreToolUse` and `UserPromptSubmit` hooks can produce a meaningful
    /// Deny — for other event types the denial is logged but not acted upon.
    Deny(String),
    /// Non-fatal warning.  Agent continues, warning is logged.
    Warn(String),
}

/// Fires registered hooks for lifecycle events.
///
/// Constructed once per session from [`HooksConfig`].  Cheap to clone
/// (the inner definitions are in an `Arc`).
#[derive(Clone)]
pub struct HookRunner {
    /// Whether the runner will fire any hooks.
    enabled: bool,
    /// Hook definitions, pre-filtered by event type for fast dispatch.
    definitions: std::sync::Arc<Vec<HookDef>>,
}

impl HookRunner {
    /// Create a runner from a loaded config.
    pub fn new(config: HooksConfig) -> Self {
        Self {
            enabled: config.enabled,
            definitions: std::sync::Arc::new(config.definitions),
        }
    }

    /// A no-op runner that never fires any hooks.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            definitions: std::sync::Arc::new(Vec::new()),
        }
    }

    /// Whether this runner has any hooks registered for the given event.
    pub fn has_hooks_for(&self, event: HookEventName) -> bool {
        self.enabled && self.definitions.iter().any(|d| d.event == event)
    }

    /// Fire all hooks matching the event.
    ///
    /// Returns the first `Deny` outcome encountered (short-circuit), or `Allow`
    /// if all hooks allow (or produce warnings).  Warnings are logged but do not
    /// stop subsequent hooks from running.
    ///
    /// # Async
    /// Command hooks run asynchronously with a per-hook timeout.
    /// Rhai hooks run synchronously (no I/O) but are wrapped in `spawn_blocking`
    /// to avoid blocking the async runtime.
    pub async fn fire(&self, event: &HookEvent) -> HookOutcome {
        if !self.enabled {
            return HookOutcome::Allow;
        }

        let env_vars = event.env_vars();

        for def in self.definitions.iter() {
            if def.event != event.name {
                continue;
            }
            // For tool events, check the matcher glob.
            if !event.tool_name.is_empty()
                && !matcher::tool_matches(&def.matcher, &event.tool_name)
            {
                continue;
            }

            let outcome = fire_one(def, &env_vars).await;
            match &outcome {
                HookOutcome::Deny(reason) => {
                    tracing::info!(
                        event = ?event.name,
                        tool = %event.tool_name,
                        reason = %reason,
                        "hook denied operation"
                    );
                    return outcome;
                }
                HookOutcome::Warn(msg) => {
                    tracing::warn!(
                        event = ?event.name,
                        tool = %event.tool_name,
                        msg = %msg,
                        "hook warning"
                    );
                    // Continue to next hook.
                }
                HookOutcome::Allow => {}
            }
        }

        HookOutcome::Allow
    }
}

/// Dispatch a single hook definition to the appropriate executor.
async fn fire_one(def: &HookDef, env_vars: &HashMap<String, String>) -> HookOutcome {
    let timeout_dur = Duration::from_secs(def.timeout_secs);

    match def.hook_type {
        HookType::Command => {
            let Some(ref command) = def.command else {
                tracing::warn!("hook definition has type=command but no command field — skipping");
                return HookOutcome::Warn("missing command field".to_string());
            };
            command_hook::run_command_hook(command, env_vars, timeout_dur).await
        }
        HookType::Rhai => {
            let Some(ref script) = def.script else {
                tracing::warn!("hook definition has type=rhai but no script field — skipping");
                return HookOutcome::Warn("missing script field".to_string());
            };
            // Run Rhai synchronously in a blocking thread so we don't block the async runtime.
            let script_owned = script.clone();
            let env_owned = env_vars.clone();
            tokio::task::spawn_blocking(move || rhai_hook::run_rhai_hook(&script_owned, &env_owned))
                .await
                .unwrap_or_else(|e| HookOutcome::Warn(format!("rhai thread error: {e}")))
        }
    }
}

/// Build a [`HookEvent`] for a tool-centric event.
pub fn tool_event(
    name: HookEventName,
    tool_name: &str,
    tool_input: &serde_json::Value,
    session_id: &str,
) -> HookEvent {
    HookEvent {
        name,
        tool_name: tool_name.to_string(),
        tool_input_json: tool_input.to_string(),
        session_id: session_id.to_string(),
    }
}

/// Build a [`HookEvent`] for a non-tool event (UserPromptSubmit, Stop, SessionEnd).
pub fn lifecycle_event(name: HookEventName, session_id: &str) -> HookEvent {
    HookEvent {
        name,
        tool_name: String::new(),
        tool_input_json: String::new(),
        session_id: session_id.to_string(),
    }
}
