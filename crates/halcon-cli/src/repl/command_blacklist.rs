//! Command blacklist for detecting extremely dangerous operations.
//!
//! This module provides pattern-based detection of commands that should
//! NEVER be auto-approved and always require explicit user consent.
//!
//! Examples of blacklisted patterns:
//! - `rm -rf /` - Root filesystem deletion
//! - `dd if=/dev/zero of=/dev/sda` - Disk wipe
//! - `:(){ :|:& };:` - Fork bomb
//! - `chmod -R 777 /` - Dangerous permission changes

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
static BLACKLIST: LazyLock<Vec<DangerousPattern>> = LazyLock::new(|| {
    vec![
        DangerousPattern {
            name: "Root filesystem deletion",
            pattern: Regex::new(r"\brm\s+(-[a-zA-Z]*r[a-zA-Z]*f[a-zA-Z]*|--recursive.*--force)\s+(/$|/\s|/\*|/\.)")
                .unwrap(),
            reason: "Attempts to recursively delete root filesystem — unrecoverable data loss",
        },
        DangerousPattern {
            name: "Disk wipe with dd",
            pattern: Regex::new(r"\bdd\s+.*of=/dev/(sd[a-z]|nvme[0-9]|hd[a-z]|xvd[a-z])($|\s)")
                .unwrap(),
            reason: "Direct disk write — can destroy entire partitions or disks",
        },
        DangerousPattern {
            name: "Filesystem creation on device",
            pattern: Regex::new(r"\bmkfs\.[a-z0-9]+\s+/dev/(sd[a-z]|nvme[0-9]|hd[a-z]|xvd[a-z])")
                .unwrap(),
            reason: "Creates new filesystem — destroys all existing data on device",
        },
        DangerousPattern {
            name: "Fork bomb",
            pattern: Regex::new(r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:")
                .unwrap(),
            reason: "Fork bomb — exhausts system resources and crashes the system",
        },
        DangerousPattern {
            name: "Global world-writable permissions",
            pattern: Regex::new(r"\bchmod\s+(-R\s+)?777\s+(/$|/\s|/\*|/\.)")
                .unwrap(),
            reason: "Makes root filesystem world-writable — critical security vulnerability",
        },
        DangerousPattern {
            name: "Disable SELinux/AppArmor",
            pattern: Regex::new(r"\b(setenforce\s+0|systemctl\s+disable\s+apparmor)")
                .unwrap(),
            reason: "Disables security enforcement — removes critical security protections",
        },
        DangerousPattern {
            name: "Kernel panic trigger",
            pattern: Regex::new(r#"echo\s+['"]?c['"]?\s*>\s*/proc/sysrq-trigger"#)
                .unwrap(),
            reason: "Forces immediate kernel panic — crashes the system",
        },
        DangerousPattern {
            name: "Memory device overwrite",
            pattern: Regex::new(r"\bdd\s+.*of=/dev/(mem|kmem|null|zero|random)")
                .unwrap(),
            reason: "Writes to kernel memory devices — can corrupt system state",
        },
        DangerousPattern {
            name: "Partition table destruction",
            pattern: Regex::new(r"\b(fdisk|parted|gdisk)\s+/dev/(sd[a-z]|nvme[0-9]|hd[a-z])")
                .unwrap(),
            reason: "Modifies partition table — can make entire disk unreadable",
        },
        DangerousPattern {
            name: "Global chown to non-root",
            pattern: Regex::new(r"\bchown\s+(-R\s+)?[a-z][a-z0-9]*\s+(/$|/\s|/\*|/\.)")
                .unwrap(),
            reason: "Changes ownership of root filesystem — breaks system permissions",
        },
        DangerousPattern {
            name: "Package manager removal",
            pattern: Regex::new(r"\b(apt|yum|dnf)\s+(remove|purge|erase)\s+(-y\s+)?(apt|dpkg|rpm|yum)")
                .unwrap(),
            reason: "Removes package manager itself — breaks system update capability",
        },
        DangerousPattern {
            name: "Swap disable on low memory",
            pattern: Regex::new(r"\bswapoff\s+-a")
                .unwrap(),
            reason: "Disables all swap space — can cause out-of-memory crashes",
        },
    ]
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
        // Verify all 12 expected patterns are loaded
        assert_eq!(BLACKLIST.len(), 12);
    }

    #[test]
    fn all_patterns_have_name_and_reason() {
        for pattern in BLACKLIST.iter() {
            assert!(!pattern.name.is_empty());
            assert!(!pattern.reason.is_empty());
        }
    }
}
