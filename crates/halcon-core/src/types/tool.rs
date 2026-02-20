use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Permission level required to execute a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    /// Read-only operations: file_read, glob, grep, git_status.
    ReadOnly,
    /// Read-write operations: file_write, file_edit.
    ReadWrite,
    /// Destructive operations: bash, git_push, file_delete.
    Destructive,
}

impl std::fmt::Display for PermissionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PermissionLevel::ReadOnly => write!(f, "read-only"),
            PermissionLevel::ReadWrite => write!(f, "read-write"),
            PermissionLevel::Destructive => write!(f, "destructive"),
        }
    }
}

/// Input passed to a tool for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInput {
    pub tool_use_id: String,
    pub arguments: serde_json::Value,
    pub working_directory: String,
}

/// Result of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Permission decision for a tool execution request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    /// User granted permission for this execution.
    Allowed,
    /// User granted permission for all executions of this tool in this session.
    AllowedAlways,
    /// User granted permission for this tool in the current directory.
    AllowedForDirectory,
    /// User granted permission for this tool in the current repository.
    AllowedForRepository,
    /// User granted permission for this specific tool+args pattern.
    AllowedForPattern,
    /// User granted permission for this session only (overrides persisted rules).
    AllowedThisSession,
    /// User denied permission.
    Denied,
    /// User denied permission for this directory.
    DeniedForDirectory,
    /// User denied permission for this pattern.
    DeniedForPattern,
}

/// Scope of a permission rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    /// Rule applies only to current session (in-memory, expires on exit).
    Session,
    /// Rule applies to specific directory (canonicalized path prefix).
    Directory,
    /// Rule applies to git repository (root directory).
    Repository,
    /// Rule applies globally across all sessions and directories.
    Global,
}

/// Pattern matching type for tool names and arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    /// Exact string match (O(1) hash lookup).
    #[default]
    Exact,
    /// Glob pattern (e.g., "bash ls*").
    Glob,
    /// Regex pattern (most flexible, highest overhead).
    Regex,
}


/// Persistent permission rule with scoping and pattern matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    /// Unique rule ID (UUID).
    pub rule_id: String,
    /// Scope of this rule (Session/Directory/Repository/Global).
    pub scope: RuleScope,
    /// Scope value (directory path, repo root, or empty for Session/Global).
    pub scope_value: String,
    /// Tool name pattern (exact, glob, or regex).
    pub tool_pattern: String,
    /// Pattern type for tool name matching.
    pub tool_pattern_type: PatternType,
    /// Optional JSON pattern for arguments (glob or null for any).
    pub param_pattern: Option<String>,
    /// The permission decision this rule grants/denies.
    pub decision: PermissionDecision,
    /// User-provided explanation for this rule.
    pub reason: Option<String>,
    /// Metadata (e.g., {"created_via": "tui", "command_preview": "rm -rf..."}).
    pub metadata: HashMap<String, String>,
    /// ISO 8601 timestamp when rule was created.
    pub created_at: String,
    /// Optional expiration timestamp (ISO 8601).
    pub expires_at: Option<String>,
    /// Whether this rule is currently active (soft delete support).
    pub active: bool,
}

impl PermissionRule {
    /// Create a new permission rule with defaults.
    pub fn new(
        scope: RuleScope,
        scope_value: String,
        tool_pattern: String,
        decision: PermissionDecision,
    ) -> Self {
        Self {
            rule_id: uuid::Uuid::new_v4().to_string(),
            scope,
            scope_value,
            tool_pattern,
            tool_pattern_type: PatternType::Exact,
            param_pattern: None,
            decision,
            reason: None,
            metadata: HashMap::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: None,
            active: true,
        }
    }

    /// Check if this rule has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(ref expires_at) = self.expires_at {
            if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at) {
                return chrono::Utc::now() > expiry;
            }
        }
        false
    }

    /// Get display string for this rule (for TUI).
    pub fn display_summary(&self) -> String {
        let scope_label = match self.scope {
            RuleScope::Session => "Session",
            RuleScope::Directory => "Directory",
            RuleScope::Repository => "Repository",
            RuleScope::Global => "Global",
        };
        let decision_label = match self.decision {
            PermissionDecision::Allowed
            | PermissionDecision::AllowedAlways
            | PermissionDecision::AllowedForDirectory
            | PermissionDecision::AllowedForRepository
            | PermissionDecision::AllowedForPattern
            | PermissionDecision::AllowedThisSession => "ALLOW",
            PermissionDecision::Denied
            | PermissionDecision::DeniedForDirectory
            | PermissionDecision::DeniedForPattern => "DENY",
        };
        format!(
            "[{}] {} {} ({})",
            decision_label, scope_label, self.tool_pattern, self.scope_value
        )
    }
}
