//! Command blacklist for detecting extremely dangerous operations (G7 HARD VETO gate).
//!
//! This module provides pattern-based detection of commands that should
//! NEVER be auto-approved and always require explicit user consent.
//!
//! Examples of blacklisted patterns:
//! - `rm -rf /` - Root filesystem deletion
//! - `dd if=/dev/zero of=/dev/sda` - Disk wipe
//! - `:(){ :|:& };:` - Fork bomb
//! - `chmod -R 777 /` - Dangerous permission changes
//!
//! ## Single Source of Truth
//!
//! Patterns are defined in `halcon_core::security::DANGEROUS_COMMAND_PATTERNS`
//! and compiled here. This eliminates duplication with the runtime blacklist
//! in `halcon-tools/bash.rs`, which uses `halcon_core::security::CATASTROPHIC_PATTERNS`.

use regex::Regex;
use std::sync::LazyLock;

/// Dangerous command pattern with explanation.
#[derive(Debug, Clone)]
pub struct DangerousPattern {
    /// Human-readable name for this pattern.
    pub name: &'static str,
    /// Regex pattern to match.
    pub pattern: Regex,
    /// Explanation of why this is dangerous.
    pub reason: &'static str,
}

/// Compiled blacklist patterns (initialized once at startup).
///
/// Sourced from `halcon_core::security::DANGEROUS_COMMAND_PATTERNS` — single source of truth.
static BLACKLIST: LazyLock<Vec<DangerousPattern>> = LazyLock::new(|| {
    halcon_core::security::DANGEROUS_COMMAND_PATTERNS
        .iter()
        .map(|(name, pattern, reason)| DangerousPattern {
            name,
            pattern: Regex::new(pattern).unwrap_or_else(|e| {
                panic!("Invalid G7 blacklist pattern '{}': {}", pattern, e)
            }),
            reason,
        })
        .collect()
});

/// Result of command safety analysis.
#[derive(Debug, Clone)]
pub struct SafetyAnalysis {
    /// Whether the command matches any blacklist pattern.
    pub is_blacklisted: bool,
    /// First matched dangerous pattern (if any).
    pub matched_pattern: Option<DangerousPattern>,
}

/// Analyze a command for dangerous patterns.
///
/// Returns `SafetyAnalysis` indicating whether the command is blacklisted
/// and which pattern it matched (if any).
///
/// # Examples
/// ```
/// use halcon_cli::repl::command_blacklist::analyze_command;
///
/// let analysis = analyze_command("rm -rf /");
/// assert!(analysis.is_blacklisted);
/// assert_eq!(analysis.matched_pattern.unwrap().name, "Root filesystem deletion");
/// ```
pub fn analyze_command(command: &str) -> SafetyAnalysis {
    for pattern in BLACKLIST.iter() {
        if pattern.pattern.is_match(command) {
            return SafetyAnalysis {
                is_blacklisted: true,
                matched_pattern: Some(pattern.clone()),
            };
        }
    }

    SafetyAnalysis {
        is_blacklisted: false,
        matched_pattern: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rm_rf_root_detected() {
        let analysis = analyze_command("rm -rf /");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Root filesystem deletion"
        );
    }

    #[test]
    fn rm_rf_root_with_star_detected() {
        let analysis = analyze_command("rm -rf /*");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Root filesystem deletion"
        );
    }

    #[test]
    fn dd_disk_wipe_detected() {
        let analysis = analyze_command("dd if=/dev/zero of=/dev/sda bs=1M");
        assert!(analysis.is_blacklisted);
        assert_eq!(analysis.matched_pattern.unwrap().name, "Disk wipe with dd");
    }

    #[test]
    fn dd_nvme_wipe_detected() {
        let analysis = analyze_command("dd if=/dev/urandom of=/dev/nvme0");
        assert!(analysis.is_blacklisted);
        assert_eq!(analysis.matched_pattern.unwrap().name, "Disk wipe with dd");
    }

    #[test]
    fn mkfs_on_device_detected() {
        let analysis = analyze_command("mkfs.ext4 /dev/sdb1");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Filesystem creation on device"
        );
    }

    #[test]
    fn fork_bomb_detected() {
        let analysis = analyze_command(":(){ :|:& };:");
        assert!(analysis.is_blacklisted);
        assert_eq!(analysis.matched_pattern.unwrap().name, "Fork bomb");
    }

    #[test]
    fn chmod_777_root_detected() {
        let analysis = analyze_command("chmod -R 777 /");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Global world-writable permissions"
        );
    }

    #[test]
    fn selinux_disable_detected() {
        let analysis = analyze_command("setenforce 0");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Disable SELinux/AppArmor"
        );
    }

    #[test]
    fn kernel_panic_detected() {
        let analysis = analyze_command("echo c > /proc/sysrq-trigger");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Kernel panic trigger"
        );
    }

    #[test]
    fn dd_mem_detected() {
        let analysis = analyze_command("dd if=/dev/zero of=/dev/mem");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Memory device overwrite"
        );
    }

    #[test]
    fn fdisk_detected() {
        let analysis = analyze_command("fdisk /dev/sda");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Partition table destruction"
        );
    }

    #[test]
    fn chown_root_detected() {
        let analysis = analyze_command("chown -R nobody /");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Global chown to non-root"
        );
    }

    #[test]
    fn package_manager_removal_detected() {
        let analysis = analyze_command("apt remove -y apt");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Package manager removal"
        );
    }

    #[test]
    fn swapoff_detected() {
        let analysis = analyze_command("swapoff -a");
        assert!(analysis.is_blacklisted);
        assert_eq!(
            analysis.matched_pattern.unwrap().name,
            "Swap disable on low memory"
        );
    }

    #[test]
    fn safe_rm_not_detected() {
        let analysis = analyze_command("rm -rf /tmp/test");
        assert!(!analysis.is_blacklisted);
    }

    #[test]
    fn safe_dd_not_detected() {
        let analysis = analyze_command("dd if=input.img of=output.img");
        assert!(!analysis.is_blacklisted);
    }

    #[test]
    fn safe_chmod_not_detected() {
        let analysis = analyze_command("chmod 755 /usr/local/bin/script.sh");
        assert!(!analysis.is_blacklisted);
    }

    #[test]
    fn safe_chown_not_detected() {
        let analysis = analyze_command("chown user:group /home/user/file.txt");
        assert!(!analysis.is_blacklisted);
    }

    #[test]
    fn blacklist_has_all_patterns() {
        // Verify all 12 patterns from halcon_core::security::DANGEROUS_COMMAND_PATTERNS are loaded
        assert_eq!(BLACKLIST.len(), 12);
        assert_eq!(BLACKLIST.len(), halcon_core::security::DANGEROUS_COMMAND_PATTERNS.len());
    }

    #[test]
    fn all_patterns_have_name_and_reason() {
        for pattern in BLACKLIST.iter() {
            assert!(!pattern.name.is_empty());
            assert!(!pattern.reason.is_empty());
        }
    }

    #[test]
    fn centralized_source_matches_compiled_blacklist() {
        // PASO 4: verify blacklist names match the centralized source exactly.
        for (i, (name, _, _)) in halcon_core::security::DANGEROUS_COMMAND_PATTERNS.iter().enumerate() {
            assert_eq!(
                BLACKLIST[i].name, *name,
                "Pattern {i} name mismatch between centralized source and compiled blacklist"
            );
        }
    }
}
