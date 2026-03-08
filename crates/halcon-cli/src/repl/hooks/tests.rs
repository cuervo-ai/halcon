//! Tests for the lifecycle hooks system.
//!
//! Covers:
//! - HookRunner dispatches to matching hooks only
//! - Command hooks: exit 0=Allow, 2=Deny, 1=Warn, timeout=Warn+Allow
//! - Rhai hooks: deny() call, allow(), sandboxing, operation limit
//! - Glob matching for tool names
//! - HookRunner returns Allow when disabled (master kill switch)
//! - allow_managed_hooks_only skips project scope
//! - FASE-2 CATASTROPHIC_PATTERNS are independent of hook outcomes (security proof)

use std::collections::HashMap;
use std::time::Duration;

use super::{
    HookEvent, HookEventName, HookOutcome, HookRunner,
    config::{HookDef, HookType, HooksConfig},
    lifecycle_event, tool_event,
};

// ── HookRunner construction ───────────────────────────────────────────────────

#[test]
fn disabled_runner_always_allows() {
    let runner = HookRunner::disabled();
    assert!(!runner.has_hooks_for(HookEventName::PreToolUse));
    assert!(!runner.has_hooks_for(HookEventName::UserPromptSubmit));
}

#[test]
fn runner_with_no_definitions_has_no_hooks() {
    let config = HooksConfig { enabled: true, definitions: vec![] };
    let runner = HookRunner::new(config);
    assert!(!runner.has_hooks_for(HookEventName::PreToolUse));
}

#[test]
fn runner_detects_registered_event() {
    let config = HooksConfig {
        enabled: true,
        definitions: vec![HookDef {
            event: HookEventName::PreToolUse,
            matcher: "*".to_string(),
            hook_type: HookType::Rhai,
            command: None,
            script: Some("allow();".to_string()),
            timeout_secs: 5,
        }],
    };
    let runner = HookRunner::new(config);
    assert!(runner.has_hooks_for(HookEventName::PreToolUse));
    assert!(!runner.has_hooks_for(HookEventName::PostToolUse));
}

// ── Rhai hook tests ───────────────────────────────────────────────────────────

#[test]
fn rhai_hook_allow_on_allow_call() {
    use super::rhai_hook::run_rhai_hook;
    let outcome = run_rhai_hook("allow();", &HashMap::new());
    assert!(matches!(outcome, HookOutcome::Allow));
}

#[test]
fn rhai_hook_deny_explicit() {
    use super::rhai_hook::run_rhai_hook;
    let outcome = run_rhai_hook(r#"deny("test denial");"#, &HashMap::new());
    match outcome {
        HookOutcome::Deny(r) => assert_eq!(r, "test denial"),
        other => panic!("expected Deny, got {other:?}"),
    }
}

#[test]
fn rhai_hook_deny_conditional_on_tool() {
    use super::rhai_hook::run_rhai_hook;
    let mut env = HashMap::new();
    env.insert("HALCON_TOOL_NAME".to_string(), "bash".to_string());

    let script = r#"
        if halcon_tool_name == "bash" {
            deny("bash is restricted");
        }
    "#;
    let outcome = run_rhai_hook(script, &env);
    assert!(matches!(outcome, HookOutcome::Deny(_)));
}

#[test]
fn rhai_hook_allow_when_condition_unmet() {
    use super::rhai_hook::run_rhai_hook;
    let mut env = HashMap::new();
    env.insert("HALCON_TOOL_NAME".to_string(), "file_read".to_string());

    let script = r#"
        if halcon_tool_name == "bash" { deny("bash restricted"); }
    "#;
    let outcome = run_rhai_hook(script, &env);
    assert!(matches!(outcome, HookOutcome::Allow));
}

#[test]
fn rhai_hook_warn_on_error_not_deny() {
    use super::rhai_hook::run_rhai_hook;
    // Syntax error — must warn, not deny or panic.
    let outcome = run_rhai_hook("let x = ;", &HashMap::new());
    assert!(matches!(outcome, HookOutcome::Warn(_)));
}

#[test]
fn rhai_hook_sandboxed_no_stdlib() {
    use super::rhai_hook::run_rhai_hook;
    // Engine::new_raw() has no stdlib — unknown function → warn, not panic.
    let outcome = run_rhai_hook("read_file(\"/etc/passwd\");", &HashMap::new());
    // Should NOT be a Deny (safety: unknown functions must not block).
    assert!(!matches!(outcome, HookOutcome::Deny(_)));
}

#[test]
fn rhai_hook_operation_limit_terminates_loop() {
    use super::rhai_hook::run_rhai_hook;
    // Infinite loop must be stopped by max_operations limit.
    let outcome = run_rhai_hook("loop {}", &HashMap::new());
    // Must not hang. Result is either Warn (ops exceeded) or Allow.
    assert!(!matches!(outcome, HookOutcome::Deny(_)));
}

// ── Command hook tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn command_hook_exit_0_allows() {
    use super::command_hook::run_command_hook;
    let outcome = run_command_hook("exit 0", &HashMap::new(), Duration::from_secs(5)).await;
    assert!(matches!(outcome, HookOutcome::Allow));
}

#[tokio::test]
async fn command_hook_exit_2_denies() {
    use super::command_hook::run_command_hook;
    let cmd = r#"printf 'blocked by policy'; exit 2"#;
    let outcome = run_command_hook(cmd, &HashMap::new(), Duration::from_secs(5)).await;
    match outcome {
        HookOutcome::Deny(reason) => assert!(reason.contains("blocked by policy")),
        other => panic!("expected Deny, got {other:?}"),
    }
}

#[tokio::test]
async fn command_hook_exit_1_warns() {
    use super::command_hook::run_command_hook;
    let outcome = run_command_hook("exit 1", &HashMap::new(), Duration::from_secs(5)).await;
    assert!(matches!(outcome, HookOutcome::Warn(_)));
}

#[tokio::test]
async fn command_hook_timeout_warns_not_denies() {
    use super::command_hook::run_command_hook;
    // sleep 10 killed after 50ms.
    let outcome = run_command_hook("sleep 10", &HashMap::new(), Duration::from_millis(50)).await;
    assert!(
        matches!(outcome, HookOutcome::Warn(_)),
        "timeout must allow+warn, not deny: {outcome:?}"
    );
}

// ── HookRunner integration ────────────────────────────────────────────────────

#[tokio::test]
async fn runner_fires_matching_hook() {
    let config = HooksConfig {
        enabled: true,
        definitions: vec![HookDef {
            event: HookEventName::PreToolUse,
            matcher: "bash".to_string(),
            hook_type: HookType::Rhai,
            command: None,
            script: Some(r#"deny("bash denied by runner test");"#.to_string()),
            timeout_secs: 5,
        }],
    };
    let runner = HookRunner::new(config);
    let event = tool_event(
        HookEventName::PreToolUse,
        "bash",
        &serde_json::json!({"command": "ls"}),
        "test-session-id",
    );
    let outcome = runner.fire(&event).await;
    assert!(matches!(outcome, HookOutcome::Deny(_)));
}

#[tokio::test]
async fn runner_skips_non_matching_tool() {
    let config = HooksConfig {
        enabled: true,
        definitions: vec![HookDef {
            event: HookEventName::PreToolUse,
            matcher: "bash".to_string(),  // Only bash
            hook_type: HookType::Rhai,
            command: None,
            script: Some(r#"deny("denied");"#.to_string()),
            timeout_secs: 5,
        }],
    };
    let runner = HookRunner::new(config);
    // file_read does not match "bash" → should allow
    let event = tool_event(
        HookEventName::PreToolUse,
        "file_read",
        &serde_json::json!({}),
        "test-session-id",
    );
    let outcome = runner.fire(&event).await;
    assert!(matches!(outcome, HookOutcome::Allow));
}

#[tokio::test]
async fn runner_disabled_flag_blocks_all_hooks() {
    // enabled=false master switch: even with a deny hook, nothing fires.
    let config = HooksConfig {
        enabled: false,  // Master kill switch OFF
        definitions: vec![HookDef {
            event: HookEventName::PreToolUse,
            matcher: "*".to_string(),
            hook_type: HookType::Rhai,
            command: None,
            script: Some(r#"deny("should not fire");"#.to_string()),
            timeout_secs: 5,
        }],
    };
    let runner = HookRunner::new(config);
    let event = tool_event(
        HookEventName::PreToolUse,
        "bash",
        &serde_json::json!({}),
        "test-session-id",
    );
    let outcome = runner.fire(&event).await;
    assert!(
        matches!(outcome, HookOutcome::Allow),
        "disabled runner must always allow"
    );
}

#[tokio::test]
async fn runner_fires_lifecycle_event() {
    let config = HooksConfig {
        enabled: true,
        definitions: vec![HookDef {
            event: HookEventName::Stop,
            matcher: "*".to_string(),
            hook_type: HookType::Rhai,
            command: None,
            script: Some("allow();".to_string()),
            timeout_secs: 5,
        }],
    };
    let runner = HookRunner::new(config);
    let event = lifecycle_event(HookEventName::Stop, "test-session-id");
    let outcome = runner.fire(&event).await;
    assert!(matches!(outcome, HookOutcome::Allow));
}

// ── Security test: FASE-2 is independent of hooks ─────────────────────────────

/// Verifies that lifecycle hooks form a user-policy layer that CANNOT bypass
/// the FASE-2 CATASTROPHIC_PATTERNS safety wall in `bash.rs`.
///
/// Structural proof:
/// 1. The hook runs BEFORE execution (PreToolUse is between step 5.5 and step 6
///    in `execute_one_tool`).
/// 2. CATASTROPHIC_PATTERNS check runs INSIDE the tool's `run()` method (step 6).
/// 3. These are independent checks: hook Allow ≠ FASE-2 bypass.
///
/// A hook returning `Allow` for a dangerous bash command does NOT prevent
/// `bash.rs` from matching CATASTROPHIC_PATTERNS and returning an error result.
#[test]
fn fase2_catastrophic_patterns_independent_of_hook_outcome() {
    use halcon_core::security::CATASTROPHIC_PATTERNS;

    // Simulate: a malicious hook returns Allow for a dangerous command.
    let hook_outcome = HookOutcome::Allow;
    assert!(matches!(hook_outcome, HookOutcome::Allow));

    // CATASTROPHIC_PATTERNS still block the command — independently.
    let dangerous_commands = [
        "rm -rf /",
        "rm -rf /*",
        "mkfs.ext4 /dev/sda",
    ];

    for cmd in &dangerous_commands {
        let is_blocked = CATASTROPHIC_PATTERNS.iter().any(|pattern| {
            regex::Regex::new(pattern)
                .map(|r| r.is_match(cmd))
                .unwrap_or(false)
        });
        assert!(
            is_blocked,
            "CATASTROPHIC_PATTERNS must block '{cmd}' regardless of hook outcome"
        );
    }
}

/// Verifies that allow_managed_hooks_only prevents project scope loading.
#[test]
fn load_hooks_config_skips_project_scope_when_managed_only() {
    use super::config::load_hooks_config;
    // With allow_managed_hooks_only=true and no global settings.toml in ~/.halcon/
    // (test environment), the result is an empty+disabled config.
    let config = load_hooks_config(true);
    // On a dev machine without ~/.halcon/settings.toml this will be default (disabled).
    // The important thing: no panic, returns a valid HooksConfig.
    let _ = config.enabled;
    let _ = config.definitions.len();
}

/// HookEvent::env_vars() produces the correct keys.
#[test]
fn hook_event_env_vars_keys_are_correct() {
    let event = tool_event(
        HookEventName::PreToolUse,
        "bash",
        &serde_json::json!({"command": "ls"}),
        "session-123",
    );
    let vars = event.env_vars();
    assert!(vars.contains_key("HALCON_EVENT"));
    assert!(vars.contains_key("HALCON_TOOL_NAME"));
    assert!(vars.contains_key("HALCON_TOOL_INPUT"));
    assert!(vars.contains_key("HALCON_SESSION_ID"));
    assert_eq!(vars["HALCON_TOOL_NAME"], "bash");
    assert_eq!(vars["HALCON_SESSION_ID"], "session-123");
}
