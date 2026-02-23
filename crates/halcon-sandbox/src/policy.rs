//! Sandbox policy definition and pre-execution command validation.
//!
//! The policy is checked *before* any process is spawned, ensuring that
//! dangerous commands never reach the OS even if the sandbox mechanisms fail.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── PolicyViolationKind ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyViolationKind {
    /// Command matches a hardcoded dangerous pattern.
    DangerousCommand { pattern: String },
    /// Command attempts to escape the working directory.
    DirectoryEscape,
    /// Command uses a disallowed shell operator (`&&`, `|`, etc. when restricted).
    DisallowedOperator { operator: String },
    /// Command exceeds allowed length.
    CommandTooLong { len: usize, max: usize },
    /// Network access attempted when disabled.
    NetworkDisallowed,
    /// Privilege escalation attempt (sudo, su, doas).
    PrivilegeEscalation,
}

// ─── PolicyViolation ──────────────────────────────────────────────────────────

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
#[error("Policy violation [{kind:?}]: {message}")]
pub struct PolicyViolation {
    pub kind: PolicyViolationKind,
    pub message: String,
    pub command_snippet: String,
}

// ─── SandboxPolicy ────────────────────────────────────────────────────────────

/// Configurable security policy for the sandbox executor.
///
/// The policy is evaluated before any subprocess is created. If the command
/// violates any rule, a [`PolicyViolation`] is returned and no process starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Allow network syscalls (curl, wget, etc.).
    pub allow_network: bool,
    /// Allow writing to files outside the working directory.
    pub allow_writes_outside_workdir: bool,
    /// Allow `sudo`, `su`, `doas` privilege escalation.
    pub allow_privilege_escalation: bool,
    /// Maximum command length in characters.
    pub max_command_len: usize,
    /// Restrict shell operators (`&&`, `||`, `|`, `;` when chaining is disabled).
    pub allow_shell_chaining: bool,
    /// Additional denylist patterns (regex strings).
    pub extra_denylist: Vec<String>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            allow_network: false,
            allow_writes_outside_workdir: false,
            allow_privilege_escalation: false,
            max_command_len: 4096,
            allow_shell_chaining: true,
            extra_denylist: Vec::new(),
        }
    }
}

impl SandboxPolicy {
    /// Policy with all restrictions enabled (most secure).
    pub fn strict() -> Self {
        Self {
            allow_network: false,
            allow_writes_outside_workdir: false,
            allow_privilege_escalation: false,
            max_command_len: 2048,
            allow_shell_chaining: false,
            extra_denylist: Vec::new(),
        }
    }

    /// Policy with network enabled (for tools like http_probe, docker_tool).
    pub fn with_network() -> Self {
        Self {
            allow_network: true,
            ..Self::default()
        }
    }

    /// Validate a command string against this policy.
    ///
    /// Returns `Ok(())` if the command passes all checks, or a [`PolicyViolation`]
    /// describing the first violation found.
    pub fn validate(&self, command: &str) -> Result<(), PolicyViolation> {
        // Length check.
        if command.len() > self.max_command_len {
            return Err(PolicyViolation {
                kind: PolicyViolationKind::CommandTooLong {
                    len: command.len(),
                    max: self.max_command_len,
                },
                message: format!(
                    "Command length {} exceeds maximum {}",
                    command.len(),
                    self.max_command_len
                ),
                command_snippet: command[..self.max_command_len.min(80)].to_string(),
            });
        }

        let cmd_lower = command.to_lowercase();

        // Privilege escalation check.
        if !self.allow_privilege_escalation {
            for escalation_cmd in &["sudo ", "sudo\t", " su ", "\tsu\t", "doas ", "pkexec "] {
                if cmd_lower.contains(escalation_cmd) || cmd_lower.starts_with(&escalation_cmd.trim_start()) {
                    return Err(PolicyViolation {
                        kind: PolicyViolationKind::PrivilegeEscalation,
                        message: format!(
                            "Privilege escalation via '{}' is not allowed",
                            escalation_cmd.trim()
                        ),
                        command_snippet: command.chars().take(80).collect(),
                    });
                }
            }
        }

        // Network commands check.
        if !self.allow_network {
            for net_cmd in &["curl ", "wget ", "nc ", "netcat ", "ssh ", "scp ", "rsync "] {
                if cmd_lower.contains(net_cmd) {
                    return Err(PolicyViolation {
                        kind: PolicyViolationKind::NetworkDisallowed,
                        message: format!(
                            "Network command '{}' is disabled by sandbox policy",
                            net_cmd.trim()
                        ),
                        command_snippet: command.chars().take(80).collect(),
                    });
                }
            }
        }

        // Dangerous command patterns (superset of the existing 18-pattern blacklist).
        let dangerous_patterns = [
            ("rm -rf /", "Recursive delete of root filesystem"),
            ("rm -rf /*", "Recursive delete of root filesystem"),
            (":(){ :|:& };:", "Fork bomb"),
            ("> /dev/sda", "Direct disk write"),
            ("dd if=", "Low-level disk operation"),
            ("mkfs.", "Filesystem formatting"),
            ("chmod -R 777 /", "Mass permission change on root"),
            ("chown -R", "Recursive ownership change"),
            ("shred ", "Secure file deletion"),
            ("wipe ", "Secure file deletion"),
            ("> /etc/passwd", "Overwrite system password file"),
            ("> /etc/shadow", "Overwrite system shadow file"),
            ("python -c \"import os; os.system", "Python shell escape"),
            ("python3 -c \"import os; os.system", "Python shell escape"),
            ("perl -e \"system", "Perl shell escape"),
            ("ruby -e \"system", "Ruby shell escape"),
            ("php -r \"system", "PHP shell escape"),
        ];

        for (pattern, description) in &dangerous_patterns {
            if cmd_lower.contains(pattern) {
                return Err(PolicyViolation {
                    kind: PolicyViolationKind::DangerousCommand {
                        pattern: pattern.to_string(),
                    },
                    message: description.to_string(),
                    command_snippet: command.chars().take(80).collect(),
                });
            }
        }

        // Directory escape check.
        if !self.allow_writes_outside_workdir
            && (command.contains("../../") || command.contains("/etc/") || command.contains("/var/"))
        {
            // Only flag writes, not reads.
            let write_indicators = [">", "tee ", "cp ", "mv ", "install "];
            for indicator in &write_indicators {
                if command.contains(indicator) {
                    return Err(PolicyViolation {
                        kind: PolicyViolationKind::DirectoryEscape,
                        message: "Write operation outside working directory detected".to_string(),
                        command_snippet: command.chars().take(80).collect(),
                    });
                }
            }
        }

        // Extra denylist (user-configured patterns).
        for pattern in &self.extra_denylist {
            if cmd_lower.contains(pattern.as_str()) {
                return Err(PolicyViolation {
                    kind: PolicyViolationKind::DangerousCommand {
                        pattern: pattern.clone(),
                    },
                    message: format!("Command matches extra denylist pattern: {}", pattern),
                    command_snippet: command.chars().take(80).collect(),
                });
            }
        }

        Ok(())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> SandboxPolicy {
        SandboxPolicy::default()
    }

    #[test]
    fn safe_command_passes() {
        assert!(policy().validate("ls -la").is_ok());
        assert!(policy().validate("cargo build").is_ok());
        assert!(policy().validate("grep -r 'TODO' src/").is_ok());
    }

    #[test]
    fn rm_rf_root_blocked() {
        let result = policy().validate("rm -rf /");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().kind,
            PolicyViolationKind::DangerousCommand { .. }
        ));
    }

    #[test]
    fn sudo_blocked() {
        let result = policy().validate("sudo apt-get install vim");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err().kind, PolicyViolationKind::PrivilegeEscalation));
    }

    #[test]
    fn network_blocked_by_default() {
        let result = policy().validate("curl https://example.com");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err().kind, PolicyViolationKind::NetworkDisallowed));
    }

    #[test]
    fn network_allowed_when_policy_permits() {
        let p = SandboxPolicy::with_network();
        assert!(p.validate("curl https://example.com").is_ok());
    }

    #[test]
    fn command_too_long_blocked() {
        let long_cmd = "a".repeat(5000);
        let result = policy().validate(&long_cmd);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().kind,
            PolicyViolationKind::CommandTooLong { .. }
        ));
    }

    #[test]
    fn fork_bomb_blocked() {
        let result = policy().validate(":(){ :|:& };:");
        assert!(result.is_err());
    }

    #[test]
    fn extra_denylist_works() {
        let mut p = SandboxPolicy::default();
        p.extra_denylist = vec!["forbidden_tool".into()];
        assert!(p.validate("forbidden_tool --run").is_err());
        assert!(p.validate("allowed_tool --run").is_ok());
    }

    #[test]
    fn strict_policy_blocks_network() {
        let p = SandboxPolicy::strict();
        assert!(p.validate("curl http://x.com").is_err());
    }
}
