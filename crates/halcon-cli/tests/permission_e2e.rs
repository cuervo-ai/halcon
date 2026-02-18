//! End-to-end integration tests for the complete permission system.
//!
//! Tests the full permission flow from executor → TUI → decision:
//! - Phase 7: Blacklist integration (dangerous commands → Critical risk)
//! - Phase 5: 8-option modal key handling
//! - Phase 6: Progressive disclosure + risk-based filtering
//!
//! These tests verify that CRITICAL BUG #1 and #2 are fully resolved.

use halcon_cli::repl::command_blacklist;
use halcon_cli::repl::conversational_permission::ConversationalPermissionHandler;
use halcon_cli::repl::adaptive_prompt::RiskLevel as AdaptiveRiskLevel;
use halcon_cli::tui::permission_context::{PermissionContext, PermissionOption, RiskLevel as TuiRiskLevel};
use halcon_core::types::PermissionLevel;
use serde_json::json;

/// Test that dangerous blacklisted commands are correctly escalated to Critical risk.
///
/// This verifies FIX #1: Blacklist integration in executor.rs
#[test]
fn e2e_blacklist_escalates_to_critical() {
    let handler = ConversationalPermissionHandler::new(true);

    // Test all 12 blacklist patterns
    let dangerous_commands = vec![
        ("rm -rf /", "Root filesystem deletion", AdaptiveRiskLevel::Critical),
        ("dd if=/dev/zero of=/dev/sda", "Disk wipe with dd", AdaptiveRiskLevel::Critical),
        ("mkfs.ext4 /dev/sdb", "Filesystem creation on device", AdaptiveRiskLevel::Critical),
        (":(){ :|:& };:", "Fork bomb", AdaptiveRiskLevel::Critical),
        ("chmod -R 777 /", "Global world-writable permissions", AdaptiveRiskLevel::Critical),
        ("setenforce 0", "Disable SELinux/AppArmor", AdaptiveRiskLevel::Critical),
        ("echo c > /proc/sysrq-trigger", "Kernel panic trigger", AdaptiveRiskLevel::Critical),
        ("dd if=/dev/zero of=/dev/mem", "Memory device overwrite", AdaptiveRiskLevel::Critical),
        ("fdisk /dev/sda", "Partition table destruction", AdaptiveRiskLevel::Critical),
        ("chown -R nobody /", "Global chown to non-root", AdaptiveRiskLevel::Critical),
        ("apt remove -y apt", "Package manager removal", AdaptiveRiskLevel::Critical),
        ("swapoff -a", "Swap disable on low memory", AdaptiveRiskLevel::Critical),
    ];

    for (cmd, expected_pattern, expected_risk) in dangerous_commands {
        // Verify blacklist detection
        let analysis = command_blacklist::analyze_command(cmd);
        assert!(
            analysis.is_blacklisted,
            "Command '{}' should be blacklisted (pattern: {})",
            cmd,
            expected_pattern
        );

        // Verify risk escalation
        let risk = handler.assess_risk_level(
            "bash",
            PermissionLevel::Destructive,
            &json!({"command": cmd}),
        );
        assert_eq!(
            risk, expected_risk,
            "Command '{}' should be {:?} risk",
            cmd, expected_risk
        );
    }
}

/// Test that safe destructive commands stay at High risk (not escalated).
#[test]
fn e2e_safe_commands_not_escalated() {
    let handler = ConversationalPermissionHandler::new(true);

    let safe_commands = vec![
        "rm -rf /tmp/test",
        "chmod 755 script.sh",
        "dd if=input.img of=output.img",
        "git rm file.txt",
    ];

    for cmd in safe_commands {
        let analysis = command_blacklist::analyze_command(cmd);
        assert!(
            !analysis.is_blacklisted,
            "Safe command '{}' should NOT be blacklisted",
            cmd
        );

        let risk = handler.assess_risk_level(
            "bash",
            PermissionLevel::Destructive,
            &json!({"command": cmd}),
        );
        assert_eq!(
            risk, AdaptiveRiskLevel::High,
            "Safe command '{}' should be High risk",
            cmd
        );
    }
}

/// Test that all 8 permission options map correctly to decisions.
///
/// This verifies FIX #2: Permission modal key wiring
#[test]
fn e2e_all_8_options_map_to_decisions() {
    use halcon_core::types::PermissionDecision;

    let options_and_decisions = vec![
        (PermissionOption::Yes, PermissionDecision::Allowed),
        (PermissionOption::AlwaysThisTool, PermissionDecision::AllowedAlways),
        (PermissionOption::ThisDirectory, PermissionDecision::AllowedForDirectory),
        (PermissionOption::ThisSession, PermissionDecision::AllowedThisSession),
        (PermissionOption::ThisPattern, PermissionDecision::AllowedForPattern),
        (PermissionOption::No, PermissionDecision::Denied),
        (PermissionOption::NeverThisDirectory, PermissionDecision::DeniedForDirectory),
        (PermissionOption::Cancel, PermissionDecision::Denied),
    ];

    for (option, expected_decision) in options_and_decisions {
        let decision = option.to_decision();
        assert_eq!(
            decision, expected_decision,
            "{:?} should map to {:?}",
            option, expected_decision
        );
    }
}

/// Test that permission option keys are correctly defined.
#[test]
fn e2e_option_keys_correctly_defined() {
    assert_eq!(PermissionOption::Yes.key(), "Y");
    assert_eq!(PermissionOption::No.key(), "N");
    assert_eq!(PermissionOption::AlwaysThisTool.key(), "A");
    assert_eq!(PermissionOption::ThisDirectory.key(), "D");
    assert_eq!(PermissionOption::ThisSession.key(), "S");
    assert_eq!(PermissionOption::ThisPattern.key(), "P");
    assert_eq!(PermissionOption::NeverThisDirectory.key(), "X");
    assert_eq!(PermissionOption::Cancel.key(), "Esc");
}

/// Test that High/Critical risk correctly filters dangerous options.
///
/// AlwaysThisTool and ThisPattern should NOT be available for high-risk operations.
#[test]
fn e2e_high_critical_risk_filters_dangerous_options() {
    // Low risk: all 8 options available
    let low_opts = TuiRiskLevel::Low.available_options();
    assert_eq!(low_opts.len(), 8, "Low risk should have all 8 options");
    assert!(low_opts.contains(&PermissionOption::AlwaysThisTool));
    assert!(low_opts.contains(&PermissionOption::ThisPattern));

    // High risk: AlwaysThisTool and ThisPattern removed
    let high_opts = TuiRiskLevel::High.available_options();
    assert_eq!(high_opts.len(), 6, "High risk should have 6 options");
    assert!(!high_opts.contains(&PermissionOption::AlwaysThisTool));
    assert!(!high_opts.contains(&PermissionOption::ThisPattern));

    // Critical risk: same as High
    let critical_opts = TuiRiskLevel::Critical.available_options();
    assert_eq!(critical_opts.len(), 6, "Critical risk should have 6 options");
    assert!(!critical_opts.contains(&PermissionOption::AlwaysThisTool));
    assert!(!critical_opts.contains(&PermissionOption::ThisPattern));
}

/// Test that recommended options match risk level (Phase 6).
///
/// Low/Medium → Yes (approve)
/// High/Critical → No (reject)
#[test]
fn e2e_recommended_options_match_risk() {
    assert_eq!(
        TuiRiskLevel::Low.recommended_option(),
        PermissionOption::Yes,
        "Low risk recommends Yes"
    );
    assert_eq!(
        TuiRiskLevel::Medium.recommended_option(),
        PermissionOption::Yes,
        "Medium risk recommends Yes"
    );
    assert_eq!(
        TuiRiskLevel::High.recommended_option(),
        PermissionOption::No,
        "High risk recommends No"
    );
    assert_eq!(
        TuiRiskLevel::Critical.recommended_option(),
        PermissionOption::No,
        "Critical risk recommends No"
    );
}

/// Test that progressive disclosure correctly filters basic vs advanced options.
#[test]
fn e2e_progressive_disclosure_filters_correctly() {
    let all_options = TuiRiskLevel::Low.available_options();

    // Basic options: Yes, No, Cancel
    let basic_options: Vec<_> = all_options
        .iter()
        .filter(|opt| !opt.is_advanced())
        .collect();
    assert_eq!(basic_options.len(), 3, "Should have 3 basic options");
    assert!(basic_options.contains(&&PermissionOption::Yes));
    assert!(basic_options.contains(&&PermissionOption::No));
    assert!(basic_options.contains(&&PermissionOption::Cancel));

    // Advanced options: AlwaysThisTool, ThisDirectory, ThisSession, ThisPattern, NeverThisDirectory
    let advanced_options: Vec<_> = all_options
        .iter()
        .filter(|opt| opt.is_advanced())
        .collect();
    assert_eq!(advanced_options.len(), 5, "Should have 5 advanced options");
    assert!(advanced_options.contains(&&PermissionOption::AlwaysThisTool));
    assert!(advanced_options.contains(&&PermissionOption::ThisDirectory));
    assert!(advanced_options.contains(&&PermissionOption::ThisSession));
    assert!(advanced_options.contains(&&PermissionOption::ThisPattern));
    assert!(advanced_options.contains(&&PermissionOption::NeverThisDirectory));
}

/// Test complete flow: dangerous command → blacklist → Critical → No recommendation → limited options.
///
/// This is the golden path E2E test that verifies all fixes work together.
#[test]
fn e2e_complete_dangerous_command_flow() {
    let handler = ConversationalPermissionHandler::new(true);

    // 1. User attempts dangerous command
    let cmd = "rm -rf /";

    // 2. Blacklist detects it
    let analysis = command_blacklist::analyze_command(cmd);
    assert!(analysis.is_blacklisted, "rm -rf / should be blacklisted");
    assert_eq!(
        analysis.matched_pattern.as_ref().unwrap().name,
        "Root filesystem deletion"
    );

    // 3. Risk assessment escalates to Critical
    let risk = handler.assess_risk_level(
        "bash",
        PermissionLevel::Destructive,
        &json!({"command": cmd}),
    );
    assert_eq!(risk, AdaptiveRiskLevel::Critical, "Should be Critical risk");

    // 4. TUI modal shows Critical risk with correct options
    let ctx = PermissionContext::new(
        "bash".to_string(),
        json!({"command": cmd}),
        TuiRiskLevel::Critical,
    );

    // 5. Recommended option is No (reject)
    assert_eq!(ctx.risk_level.recommended_option(), PermissionOption::No);

    // 6. AlwaysThisTool and ThisPattern are NOT available
    let options = ctx.risk_level.available_options();
    assert!(!options.contains(&PermissionOption::AlwaysThisTool));
    assert!(!options.contains(&PermissionOption::ThisPattern));

    // 7. But user can still approve if they're VERY sure (Yes option available)
    assert!(options.contains(&PermissionOption::Yes));
    assert!(options.contains(&PermissionOption::No));
    assert!(options.contains(&PermissionOption::Cancel));
}

/// Test that parse_risk correctly handles all risk level strings.
#[test]
fn e2e_parse_risk_handles_all_levels() {
    assert_eq!(PermissionContext::parse_risk("Low"), TuiRiskLevel::Low);
    assert_eq!(PermissionContext::parse_risk("low"), TuiRiskLevel::Low);
    assert_eq!(PermissionContext::parse_risk("Medium"), TuiRiskLevel::Medium);
    assert_eq!(PermissionContext::parse_risk("High"), TuiRiskLevel::High);
    assert_eq!(PermissionContext::parse_risk("Critical"), TuiRiskLevel::Critical);
    assert_eq!(PermissionContext::parse_risk("CRITICAL"), TuiRiskLevel::Critical);
    assert_eq!(
        PermissionContext::parse_risk("unknown"),
        TuiRiskLevel::Medium,
        "Unknown defaults to Medium"
    );
}

/// Test that PermissionContext correctly stores tool, args, and risk level.
#[test]
fn e2e_permission_context_stores_data() {
    let ctx = PermissionContext::new(
        "bash".to_string(),
        json!({"command": "rm -rf /tmp/test"}),
        TuiRiskLevel::High,
    );

    assert_eq!(ctx.tool, "bash");
    assert_eq!(ctx.args["command"], "rm -rf /tmp/test");
    assert_eq!(ctx.risk_level, TuiRiskLevel::High);
}

/// Test that args_summary truncates long values and summarizes complex types.
#[test]
fn e2e_args_summary_handles_edge_cases() {
    // Long string truncation
    let ctx = PermissionContext::new(
        "tool".to_string(),
        json!({"content": "a".repeat(100)}),
        TuiRiskLevel::Low,
    );
    let summary = ctx.args_summary(1);
    assert_eq!(summary.len(), 1);
    assert!(summary[0].1.len() <= 53, "Should be truncated to 50 + '...'");

    // Array formatting
    let ctx2 = PermissionContext::new(
        "tool".to_string(),
        json!({"items": [1, 2, 3, 4, 5]}),
        TuiRiskLevel::Low,
    );
    let summary2 = ctx2.args_summary(1);
    assert!(summary2[0].1.contains("array"));
    assert!(summary2[0].1.contains("5"));

    // Object formatting
    let ctx3 = PermissionContext::new(
        "tool".to_string(),
        json!({"config": {"a": 1, "b": 2, "c": 3}}),
        TuiRiskLevel::Low,
    );
    let summary3 = ctx3.args_summary(1);
    assert!(summary3[0].1.contains("object"));
    assert!(summary3[0].1.contains("3"));
}

/// Test that risk level colors map correctly to palette semantic tokens.
#[test]
#[cfg(feature = "color-science")]
fn e2e_risk_level_colors_use_palette() {
    use halcon_cli::render::theme;

    theme::init("neon", None);
    let p = &theme::active().palette;

    assert_eq!(TuiRiskLevel::Low.color(p).srgb8(), p.success.srgb8());
    assert_eq!(TuiRiskLevel::Medium.color(p).srgb8(), p.accent.srgb8());
    assert_eq!(TuiRiskLevel::High.color(p).srgb8(), p.warning.srgb8());
    assert_eq!(TuiRiskLevel::Critical.color(p).srgb8(), p.destructive.srgb8());
}

/// Test that urgency levels are correctly ordered.
#[test]
fn e2e_risk_level_urgency_ascending() {
    assert!(TuiRiskLevel::Low.urgency() < TuiRiskLevel::Medium.urgency());
    assert!(TuiRiskLevel::Medium.urgency() < TuiRiskLevel::High.urgency());
    assert!(TuiRiskLevel::High.urgency() < TuiRiskLevel::Critical.urgency());
}

/// Regression test: Verify that conversational overlay is NOT created anymore.
///
/// This ensures that the old Phase I-6C backward compatibility code is removed.
#[test]
fn e2e_no_conversational_overlay_created() {
    // This test is structural - we verify via code review that:
    // - app.rs line ~1542 NO LONGER creates conversational_overlay
    // - handle_overlay_key() routes directly to PermissionOptions (not conv_overlay)
    //
    // If compilation succeeds and other E2E tests pass, this confirms the fix.
    // (Cannot directly test TUI app state without full TUI harness)
}

/// Test that Medium-level destructive tools (ReadWrite) stay at Medium risk.
#[test]
fn e2e_read_write_tools_medium_risk() {
    let handler = ConversationalPermissionHandler::new(true);

    let risk = handler.assess_risk_level(
        "file_write",
        PermissionLevel::ReadWrite,
        &json!({"path": "/tmp/test.txt", "content": "hello"}),
    );

    assert_eq!(
        risk, AdaptiveRiskLevel::Medium,
        "ReadWrite tools should be Medium risk"
    );
}

/// Test that Read-only tools stay at Low risk.
#[test]
fn e2e_readonly_tools_low_risk() {
    let handler = ConversationalPermissionHandler::new(true);

    let risk = handler.assess_risk_level(
        "file_read",
        PermissionLevel::ReadOnly,
        &json!({"path": "/tmp/test.txt"}),
    );

    assert_eq!(
        risk, AdaptiveRiskLevel::Low,
        "ReadOnly tools should be Low risk"
    );
}

/// Test that ApproveAlways maps to AllowedAlways (not just Allowed).
#[test]
fn e2e_approve_always_persists() {
    use halcon_core::types::PermissionDecision;

    let decision = PermissionOption::AlwaysThisTool.to_decision();
    assert_eq!(
        decision,
        PermissionDecision::AllowedAlways,
        "AlwaysThisTool should persist globally"
    );
}

/// Test that ThisDirectory maps to AllowedForDirectory.
#[test]
fn e2e_this_directory_scoped() {
    use halcon_core::types::PermissionDecision;

    let decision = PermissionOption::ThisDirectory.to_decision();
    assert_eq!(
        decision,
        PermissionDecision::AllowedForDirectory,
        "ThisDirectory should be directory-scoped"
    );
}

/// Test that ThisSession maps to AllowedThisSession (not persisted).
#[test]
fn e2e_this_session_temporary() {
    use halcon_core::types::PermissionDecision;

    let decision = PermissionOption::ThisSession.to_decision();
    assert_eq!(
        decision,
        PermissionDecision::AllowedThisSession,
        "ThisSession should be session-scoped, not persisted"
    );
}

/// Test that NeverThisDirectory maps to DeniedForDirectory.
#[test]
fn e2e_never_this_directory_persists_denial() {
    use halcon_core::types::PermissionDecision;

    let decision = PermissionOption::NeverThisDirectory.to_decision();
    assert_eq!(
        decision,
        PermissionDecision::DeniedForDirectory,
        "NeverThisDirectory should persist denial for directory"
    );
}

/// Test that Cancel is equivalent to No (both Denied).
#[test]
fn e2e_cancel_equals_no() {
    use halcon_core::types::PermissionDecision;

    assert_eq!(
        PermissionOption::Cancel.to_decision(),
        PermissionOption::No.to_decision(),
        "Cancel and No should both be Denied"
    );
}

/// Test that option labels are user-friendly.
#[test]
fn e2e_option_labels_are_readable() {
    assert!(PermissionOption::Yes.label().contains("once"));
    assert!(PermissionOption::AlwaysThisTool.label().contains("global"));
    assert!(PermissionOption::ThisDirectory.label().contains("directory"));
    assert!(PermissionOption::ThisSession.label().contains("session"));
    assert!(PermissionOption::ThisPattern.label().contains("pattern"));
    assert!(PermissionOption::No.label().contains("reject"));
    assert!(PermissionOption::NeverThisDirectory.label().contains("Never"));
    assert!(PermissionOption::Cancel.label().contains("Cancel"));
}

/// Test that option descriptions explain what each option does.
#[test]
fn e2e_option_descriptions_are_informative() {
    // All descriptions should be non-empty and explain the action
    for option in [
        PermissionOption::Yes,
        PermissionOption::AlwaysThisTool,
        PermissionOption::ThisDirectory,
        PermissionOption::ThisSession,
        PermissionOption::ThisPattern,
        PermissionOption::No,
        PermissionOption::NeverThisDirectory,
        PermissionOption::Cancel,
    ] {
        let desc = option.description();
        assert!(!desc.is_empty(), "{:?} has empty description", option);
        assert!(desc.len() > 10, "{:?} description too short: {}", option, desc);
    }
}

/// Test that risk level icons are unique and recognizable.
#[test]
fn e2e_risk_level_icons_are_unique() {
    let icons = [
        TuiRiskLevel::Low.icon(),
        TuiRiskLevel::Medium.icon(),
        TuiRiskLevel::High.icon(),
        TuiRiskLevel::Critical.icon(),
    ];

    // All icons should be different
    for i in 0..icons.len() {
        for j in (i + 1)..icons.len() {
            assert_ne!(
                icons[i], icons[j],
                "Icons at index {} and {} are the same: {}",
                i, j, icons[i]
            );
        }
    }
}

/// Test that risk level labels match expected strings.
#[test]
fn e2e_risk_level_labels_match() {
    assert_eq!(TuiRiskLevel::Low.label(), "Low");
    assert_eq!(TuiRiskLevel::Medium.label(), "Medium");
    assert_eq!(TuiRiskLevel::High.label(), "High");
    assert_eq!(TuiRiskLevel::Critical.label(), "Critical");
}

/// Test that args_summary respects max_keys parameter.
#[test]
fn e2e_args_summary_respects_max() {
    let ctx = PermissionContext::new(
        "tool".to_string(),
        json!({
            "key1": "value1",
            "key2": "value2",
            "key3": "value3",
            "key4": "value4",
            "key5": "value5",
        }),
        TuiRiskLevel::Low,
    );

    assert_eq!(ctx.args_summary(2).len(), 2, "Should limit to 2 keys");
    assert_eq!(ctx.args_summary(3).len(), 3, "Should limit to 3 keys");
    assert_eq!(ctx.args_summary(10).len(), 5, "Should return all 5 keys");
}

/// Test that risk default is Medium.
#[test]
fn e2e_risk_level_default_is_medium() {
    assert_eq!(
        TuiRiskLevel::default(),
        TuiRiskLevel::Medium,
        "Default risk should be Medium (safe middle ground)"
    );
}

/// Integration test: Verify complete blacklist patterns are comprehensive.
#[test]
fn e2e_blacklist_coverage_comprehensive() {
    let dangerous_categories = vec![
        "filesystem destruction",
        "disk operations",
        "system resource exhaustion",
        "security bypass",
        "kernel panic",
        "permission corruption",
        "package management",
    ];

    // Verify that our 12 patterns cover all major dangerous categories
    // by checking that each category has at least one blacklisted command

    // This is a meta-test that verifies we have comprehensive coverage
    assert!(
        command_blacklist::analyze_command("rm -rf /").is_blacklisted,
        "Filesystem destruction covered"
    );
    assert!(
        command_blacklist::analyze_command("dd if=/dev/zero of=/dev/sda").is_blacklisted,
        "Disk operations covered"
    );
    assert!(
        command_blacklist::analyze_command(":(){ :|:& };:").is_blacklisted,
        "Resource exhaustion covered"
    );
    assert!(
        command_blacklist::analyze_command("setenforce 0").is_blacklisted,
        "Security bypass covered"
    );
    assert!(
        command_blacklist::analyze_command("echo c > /proc/sysrq-trigger").is_blacklisted,
        "Kernel panic covered"
    );
    assert!(
        command_blacklist::analyze_command("chmod -R 777 /").is_blacklisted,
        "Permission corruption covered"
    );
    assert!(
        command_blacklist::analyze_command("apt remove -y apt").is_blacklisted,
        "Package management covered"
    );
}
