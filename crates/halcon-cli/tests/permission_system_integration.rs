//! End-to-end integration tests for the complete permission system.
//!
//! Tests the full permission flow including:
//! - Phase 5: 8-option TUI modal
//! - Phase 6: Smart recommendations & progressive disclosure
//! - Phase 7: Command blacklist & safety hardening
//!
//! These tests verify that all phases work together correctly.

use halcon_cli::repl::command_blacklist;
use halcon_cli::repl::conversational_permission::ConversationalPermissionHandler;
use halcon_cli::repl::adaptive_prompt::RiskLevel;
use halcon_cli::tui::permission_context::{PermissionContext, PermissionOption};
use halcon_core::types::PermissionLevel;
use serde_json::json;

/// Test that blacklisted commands are escalated to Critical risk.
#[test]
fn e2e_blacklist_escalates_to_critical_risk() {
    let handler = ConversationalPermissionHandler::new(true);

    // Test multiple blacklisted commands
    let dangerous_commands = vec![
        ("rm -rf /", "Root filesystem deletion"),
        ("dd if=/dev/zero of=/dev/sda", "Disk wipe with dd"),
        ("mkfs.ext4 /dev/sdb", "Filesystem creation on device"),
        (":(){ :|:& };:", "Fork bomb"),
        ("chmod -R 777 /", "Global world-writable permissions"),
    ];

    for (cmd, expected_pattern) in dangerous_commands {
        let risk = handler.assess_risk_level(
            "bash",
            PermissionLevel::Destructive,
            &json!({"command": cmd}),
        );

        assert_eq!(risk, RiskLevel::Critical, "Command '{}' should be Critical risk", cmd);

        // Verify the blacklist analysis
        let analysis = command_blacklist::analyze_command(cmd);
        assert!(analysis.is_blacklisted, "Command '{}' should be blacklisted", cmd);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            expected_pattern,
            "Command '{}' should match pattern '{}'",
            cmd,
            expected_pattern
        );
    }
}

/// Test that safe commands are not escalated.
#[test]
fn e2e_safe_commands_not_escalated() {
    let handler = ConversationalPermissionHandler::new(true);

    let safe_commands = vec![
        ("ls -la", RiskLevel::High), // bash is Destructive level = High
        ("echo hello", RiskLevel::High),
        ("rm -rf /tmp/test", RiskLevel::High), // Safe rm is High, not Critical
        ("chmod 755 script.sh", RiskLevel::High),
    ];

    for (cmd, expected_risk) in safe_commands {
        let risk = handler.assess_risk_level(
            "bash",
            PermissionLevel::Destructive,
            &json!({"command": cmd}),
        );

        assert_eq!(risk, expected_risk, "Command '{}' should be {:?}", cmd, expected_risk);

        // Verify NOT blacklisted
        let analysis = command_blacklist::analyze_command(cmd);
        assert!(!analysis.is_blacklisted, "Command '{}' should NOT be blacklisted", cmd);
    }
}

/// Test that recommended options are correct for each risk level.
#[test]
fn e2e_recommendations_match_risk_level() {
    // Low/Medium risk → Yes (approve)
    let low_ctx = PermissionContext::new(
        "file_read".to_string(),
        json!({"path": "/tmp/test.txt"}),
        halcon_cli::tui::permission_context::RiskLevel::Low,
    );
    assert_eq!(
        low_ctx.risk_level.recommended_option(),
        PermissionOption::Yes,
        "Low risk should recommend Yes"
    );

    let medium_ctx = PermissionContext::new(
        "file_write".to_string(),
        json!({"path": "/tmp/test.txt"}),
        halcon_cli::tui::permission_context::RiskLevel::Medium,
    );
    assert_eq!(
        medium_ctx.risk_level.recommended_option(),
        PermissionOption::Yes,
        "Medium risk should recommend Yes"
    );

    // High/Critical risk → No (reject)
    let high_ctx = PermissionContext::new(
        "file_delete".to_string(),
        json!({"path": "/tmp/test.txt"}),
        halcon_cli::tui::permission_context::RiskLevel::High,
    );
    assert_eq!(
        high_ctx.risk_level.recommended_option(),
        PermissionOption::No,
        "High risk should recommend No"
    );

    let critical_ctx = PermissionContext::new(
        "bash".to_string(),
        json!({"command": "rm -rf /"}),
        halcon_cli::tui::permission_context::RiskLevel::Critical,
    );
    assert_eq!(
        critical_ctx.risk_level.recommended_option(),
        PermissionOption::No,
        "Critical risk should recommend No"
    );
}

/// Test that progressive disclosure filters advanced options correctly.
#[test]
fn e2e_progressive_disclosure_filters_advanced() {
    use halcon_cli::tui::permission_context::RiskLevel as TuiRiskLevel;

    let ctx = PermissionContext::new(
        "bash".to_string(),
        json!({"command": "ls"}),
        TuiRiskLevel::Low,
    );

    let all_options = ctx.risk_level.available_options();
    let basic_options: Vec<_> = all_options
        .iter()
        .filter(|opt| !opt.is_advanced())
        .collect();

    // Basic options: Yes, No, Cancel = 3
    assert_eq!(basic_options.len(), 3, "Should have 3 basic options");

    // Advanced options: AlwaysThisTool, ThisDirectory, ThisSession, ThisPattern, NeverThisDirectory = 5
    let advanced_options: Vec<_> = all_options
        .iter()
        .filter(|opt| opt.is_advanced())
        .collect();
    assert_eq!(advanced_options.len(), 5, "Should have 5 advanced options");
}

/// Test that High/Critical risk removes dangerous advanced options.
#[test]
fn e2e_high_critical_removes_dangerous_options() {
    use halcon_cli::tui::permission_context::RiskLevel as TuiRiskLevel;

    // High risk should remove AlwaysThisTool and ThisPattern
    let high_ctx = PermissionContext::new(
        "bash".to_string(),
        json!({"command": "rm -rf /tmp"}),
        TuiRiskLevel::High,
    );

    let high_options = high_ctx.risk_level.available_options();
    assert!(
        !high_options.contains(&PermissionOption::AlwaysThisTool),
        "High risk should not have AlwaysThisTool"
    );
    assert!(
        !high_options.contains(&PermissionOption::ThisPattern),
        "High risk should not have ThisPattern"
    );

    // Critical risk should also remove them
    let critical_ctx = PermissionContext::new(
        "bash".to_string(),
        json!({"command": "rm -rf /"}),
        TuiRiskLevel::Critical,
    );

    let critical_options = critical_ctx.risk_level.available_options();
    assert!(
        !critical_options.contains(&PermissionOption::AlwaysThisTool),
        "Critical risk should not have AlwaysThisTool"
    );
    assert!(
        !critical_options.contains(&PermissionOption::ThisPattern),
        "Critical risk should not have ThisPattern"
    );
}

/// Test complete permission flow: bash command → risk assessment → options
#[test]
fn e2e_complete_bash_permission_flow() {
    let handler = ConversationalPermissionHandler::new(true);

    // 1. Assess risk for a dangerous command
    let cmd = "rm -rf /";
    let risk = handler.assess_risk_level(
        "bash",
        PermissionLevel::Destructive,
        &json!({"command": cmd}),
    );

    // 2. Should be Critical due to blacklist
    assert_eq!(risk, RiskLevel::Critical);

    // 3. Create TUI context (map RiskLevel types)
    use halcon_cli::tui::permission_context::RiskLevel as TuiRiskLevel;
    let tui_risk = TuiRiskLevel::Critical;
    let ctx = PermissionContext::new(
        "bash".to_string(),
        json!({"command": cmd}),
        tui_risk,
    );

    // 4. Recommended option should be No (reject)
    assert_eq!(ctx.risk_level.recommended_option(), PermissionOption::No);

    // 5. Available options should exclude AlwaysThisTool and ThisPattern
    let options = ctx.risk_level.available_options();
    assert!(!options.contains(&PermissionOption::AlwaysThisTool));
    assert!(!options.contains(&PermissionOption::ThisPattern));

    // 6. Should still have basic options
    assert!(options.contains(&PermissionOption::Yes));
    assert!(options.contains(&PermissionOption::No));
    assert!(options.contains(&PermissionOption::Cancel));
}

/// Test that all 12 blacklist patterns are active.
#[test]
fn e2e_all_blacklist_patterns_active() {
    let dangerous_commands = vec![
        "rm -rf /",                          // Root filesystem deletion
        "dd if=/dev/zero of=/dev/sda",       // Disk wipe
        "mkfs.ext4 /dev/sdb",                // Filesystem creation
        ":(){ :|:& };:",                     // Fork bomb
        "chmod -R 777 /",                    // World-writable permissions
        "setenforce 0",                      // SELinux disable
        "echo c > /proc/sysrq-trigger",      // Kernel panic
        "dd if=/dev/zero of=/dev/mem",       // Memory device write
        "fdisk /dev/sda",                    // Partition table destruction
        "chown -R nobody /",                 // Global chown
        "apt remove -y apt",                 // Package manager removal
        "swapoff -a",                        // Swap disable
    ];

    for cmd in dangerous_commands {
        let analysis = command_blacklist::analyze_command(cmd);
        assert!(
            analysis.is_blacklisted,
            "Command '{}' should be blacklisted",
            cmd
        );
        assert!(
            analysis.matched_pattern.is_some(),
            "Command '{}' should have a matched pattern",
            cmd
        );
    }
}

/// Test that permission decisions convert correctly.
#[test]
fn e2e_permission_option_to_decision() {
    use halcon_core::types::PermissionDecision;

    assert_eq!(
        PermissionOption::Yes.to_decision(),
        PermissionDecision::Allowed
    );
    assert_eq!(
        PermissionOption::AlwaysThisTool.to_decision(),
        PermissionDecision::AllowedAlways
    );
    assert_eq!(
        PermissionOption::ThisDirectory.to_decision(),
        PermissionDecision::AllowedForDirectory
    );
    assert_eq!(
        PermissionOption::ThisSession.to_decision(),
        PermissionDecision::AllowedThisSession
    );
    assert_eq!(
        PermissionOption::ThisPattern.to_decision(),
        PermissionDecision::AllowedForPattern
    );
    assert_eq!(
        PermissionOption::No.to_decision(),
        PermissionDecision::Denied
    );
    assert_eq!(
        PermissionOption::NeverThisDirectory.to_decision(),
        PermissionDecision::DeniedForDirectory
    );
    assert_eq!(
        PermissionOption::Cancel.to_decision(),
        PermissionDecision::Denied
    );
}

/// Test cross-phase interaction: blacklist → Critical → No recommendation → limited options
#[test]
fn e2e_cross_phase_blacklist_critical_flow() {
    use halcon_cli::tui::permission_context::RiskLevel as TuiRiskLevel;

    let handler = ConversationalPermissionHandler::new(true);

    // Phase 7: Blacklist detection
    let cmd = "dd if=/dev/zero of=/dev/sda";
    let analysis = command_blacklist::analyze_command(cmd);
    assert!(analysis.is_blacklisted);

    // Risk assessment escalates to Critical
    let risk = handler.assess_risk_level(
        "bash",
        PermissionLevel::Destructive,
        &json!({"command": cmd}),
    );
    assert_eq!(risk, RiskLevel::Critical);

    // Phase 6: Recommendation for Critical = No
    let ctx = PermissionContext::new(
        "bash".to_string(),
        json!({"command": cmd}),
        TuiRiskLevel::Critical,
    );
    assert_eq!(ctx.risk_level.recommended_option(), PermissionOption::No);

    // Phase 5: Limited options for Critical risk
    let options = ctx.risk_level.available_options();
    assert_eq!(options.len(), 6); // Yes, ThisDirectory, ThisSession, No, NeverThisDirectory, Cancel

    // Phase 6: Progressive disclosure filters to 3 basic options
    let basic_options: Vec<_> = options.iter().filter(|o| !o.is_advanced()).collect();
    assert_eq!(basic_options.len(), 3); // Yes, No, Cancel
}
