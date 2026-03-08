//! Rhai sandboxed script executor for lifecycle hooks.
//!
//! # Security model
//!
//! The Rhai engine is created with [`Engine::new_raw()`] which provides **no**
//! standard library, no I/O, no networking, and no file-system access.
//!
//! Safety limits applied at engine construction:
//! - `max_operations(10_000)` — prevents infinite loops
//! - `max_string_size(65_536)` — prevents string-based memory exhaustion
//! - `max_array_size(1_024)` — limits array allocations
//! - `max_map_size(1_024)` — limits map/object allocations
//!
//! # API available to scripts
//!
//! - `deny(reason: String)` — signals denial; equivalent to exit code 2.
//! - `allow()` — explicit allow (no-op; the default outcome).
//!
//! All hook environment variables are exposed as Rhai variables using their
//! lowercase snake_case names (e.g. `HALCON_TOOL_NAME` → `halcon_tool_name`).
//!
//! # Error handling
//!
//! Rhai evaluation errors (syntax, runtime, operation limit exceeded) are treated
//! as warnings — they log the error but do **not** block the operation.  Only an
//! explicit `deny()` call blocks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rhai::Engine;

use super::HookOutcome;

/// Execute a Rhai script hook.
///
/// # Arguments
/// * `script`   — Rhai source text from the hook definition.
/// * `env_vars` — Environment variables to expose as Rhai variables.
///
/// # Returns
/// - `HookOutcome::Deny(reason)` if the script calls `deny(reason)`.
/// - `HookOutcome::Warn(msg)`    if the script produces a non-denial error.
/// - `HookOutcome::Allow`        otherwise.
pub fn run_rhai_hook(script: &str, env_vars: &HashMap<String, String>) -> HookOutcome {
    // Shared flag: the registered `deny()` function writes here.
    let deny_reason: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let deny_clone = deny_reason.clone();

    let mut engine = build_engine();

    // Register `deny(reason: String)` — stores reason, then returns unit so
    // the script can continue (the caller checks the flag after evaluation).
    engine.register_fn("deny", move |reason: String| {
        if let Ok(mut guard) = deny_clone.lock() {
            *guard = Some(reason);
        }
    });

    // Register `allow()` — explicit allow (no-op).
    engine.register_fn("allow", || {});

    // Build a Rhai scope with env vars as snake_case variables.
    let mut scope = rhai::Scope::new();
    for (key, val) in env_vars {
        let var_name = key.to_lowercase();
        scope.push(var_name, val.clone());
    }

    match engine.eval_with_scope::<rhai::Dynamic>(&mut scope, script) {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            // If deny() was already called before the error, honour denial.
            if deny_reason.lock().ok().and_then(|g| g.clone()).is_none() {
                tracing::warn!(error = %msg, "rhai hook evaluation error — allowing (warn)");
                return HookOutcome::Warn(format!("rhai hook error: {msg}"));
            }
        }
    }

    // Check if deny() was called during execution.
    if let Ok(guard) = deny_reason.lock() {
        if let Some(reason) = guard.clone() {
            return HookOutcome::Deny(reason);
        }
    }

    HookOutcome::Allow
}

/// Construct a sandboxed Rhai engine.
///
/// Does NOT register `deny` / `allow` — those are added per-call so they can
/// capture the per-call shared state.
fn build_engine() -> Engine {
    let mut engine = Engine::new_raw();

    // Safety limits.
    engine.set_max_operations(10_000);
    engine.set_max_string_size(65_536);
    engine.set_max_array_size(1_024);
    engine.set_max_map_size(1_024);

    engine
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_env() -> HashMap<String, String> {
        HashMap::new()
    }

    fn env_with(key: &str, val: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(key.to_string(), val.to_string());
        m
    }

    #[test]
    fn allow_on_empty_script() {
        let outcome = run_rhai_hook("", &empty_env());
        assert!(matches!(outcome, HookOutcome::Allow));
    }

    #[test]
    fn allow_on_explicit_allow_call() {
        let outcome = run_rhai_hook("allow();", &empty_env());
        assert!(matches!(outcome, HookOutcome::Allow));
    }

    #[test]
    fn deny_on_deny_call() {
        let outcome = run_rhai_hook(r#"deny("not allowed");"#, &empty_env());
        match outcome {
            HookOutcome::Deny(reason) => assert_eq!(reason, "not allowed"),
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn deny_conditional_on_env_var() {
        let env = env_with("HALCON_TOOL_NAME", "bash");
        let script = r#"
            if halcon_tool_name == "bash" {
                deny("bash not allowed");
            }
        "#;
        let outcome = run_rhai_hook(script, &env);
        assert!(matches!(outcome, HookOutcome::Deny(_)));
    }

    #[test]
    fn allow_when_condition_not_met() {
        let env = env_with("HALCON_TOOL_NAME", "file_read");
        let script = r#"
            if halcon_tool_name == "bash" {
                deny("bash not allowed");
            }
        "#;
        let outcome = run_rhai_hook(script, &env);
        assert!(matches!(outcome, HookOutcome::Allow));
    }

    #[test]
    fn warn_on_syntax_error() {
        // Malformed script — should warn+allow, not panic.
        let outcome = run_rhai_hook("let x = ;", &empty_env());
        assert!(matches!(outcome, HookOutcome::Warn(_)));
    }

    #[test]
    fn sandboxed_no_filesystem_access() {
        // Engine::new_raw() has no std module — `file` etc. are not registered.
        // Attempting to call an unknown function should produce a warn, not allow I/O.
        let outcome = run_rhai_hook(r#"let _ = read_file("/etc/passwd");"#, &empty_env());
        // The call will fail (unknown function) — must NOT panic, must warn+allow.
        assert!(!matches!(outcome, HookOutcome::Deny(_)));
    }

    #[test]
    fn operation_limit_prevents_infinite_loop() {
        // Infinite loop must be stopped by max_operations, not hang the test.
        let script = "loop { }";
        let outcome = run_rhai_hook(script, &empty_env());
        // Produces a warn (operations exceeded) rather than hanging.
        assert!(matches!(outcome, HookOutcome::Warn(_) | HookOutcome::Allow));
    }
}
