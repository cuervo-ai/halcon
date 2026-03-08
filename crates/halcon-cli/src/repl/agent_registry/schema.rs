//! YAML frontmatter schema for agent definition files.
//!
//! Agent definitions live in `.halcon/agents/*.md` (project scope) or
//! `~/.halcon/agents/*.md` (user scope).  The file format is:
//!
//! ```markdown
//! ---
//! name: code-reviewer
//! description: "Expert code reviewer. Use after any code changes."
//! tools: [file_read, grep, glob]
//! model: haiku
//! max_turns: 15
//! ---
//!
//! You are an expert code reviewer.  Focus on security, performance, and maintainability.
//! ```
//!
//! The Markdown body below the frontmatter becomes the agent's **system prompt prefix** —
//! it is prepended to the parent agent's system prompt when the sub-agent executes.

use serde::{Deserialize, Serialize};

/// Parsed frontmatter from an agent definition file.
///
/// All fields are optional except `name` and `description`.
/// Unknown keys in the YAML are silently ignored (uses `#[serde(deny_unknown_fields)]` would
/// reject them, but we choose to warn instead — see `validator.rs`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentFrontmatter {
    /// Unique agent name in kebab-case (e.g. `code-reviewer`).
    ///
    /// Used by the parent agent when requesting delegation.
    /// Must match `^[a-z][a-z0-9-]*$`.
    pub name: String,

    /// Short human-readable description shown in `halcon agents list` and injected
    /// into the parent agent's routing manifest.
    ///
    /// Keep under 120 characters for readability in the routing manifest.
    pub description: String,

    /// Tool allowlist.  When non-empty, the sub-agent can only call these tools.
    /// An empty list means the sub-agent inherits all tools from the parent.
    #[serde(default)]
    pub tools: Vec<String>,

    /// Tool denylist.  Tools listed here are removed from the effective tool set.
    /// Applied after `tools` allowlist resolution.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,

    /// Model override.  Supported aliases (case-insensitive):
    /// - `"haiku"`  → `claude-haiku-4-5-20251001`
    /// - `"sonnet"` → `claude-sonnet-4-6`
    /// - `"opus"`   → `claude-opus-4-6`
    /// - `"inherit"` → inherit parent model (same as `None`)
    /// - Any other string is passed through as a fully-qualified model ID.
    pub model: Option<String>,

    /// Permission mode override (currently informational; enforcement is via `tools`).
    pub permission_mode: Option<String>,

    /// Maximum number of agent loop rounds (default: 20, range: 1–100).
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,

    /// Memory scope for cross-session agent memory.
    /// - `"project"` → `.halcon/memory/agents/<name>/MEMORY.md`
    /// - `"user"`    → `~/.halcon/memory/agents/<name>/MEMORY.md`
    /// - `"local"`   → session-only, no persistence
    pub memory: Option<String>,

    /// Isolation mode.  `"worktree"` runs the agent in an isolated git worktree.
    pub isolation: Option<String>,

    /// Per-agent hook configuration (same schema as Feature 2 hooks).
    /// Currently stored as raw YAML for forward compatibility.
    pub hooks: Option<serde_yaml::Value>,

    /// MCP server names the agent has access to.
    #[serde(default)]
    pub mcp_servers: Vec<String>,

    /// Skills to inject at agent startup.
    /// Skills are reusable system prompt snippets from `.halcon/skills/*.md`.
    #[serde(default)]
    pub skills: Vec<String>,

    /// Whether the agent runs in the background (non-blocking).  Default: false.
    #[serde(default)]
    pub background: bool,
}

/// Resolve model alias to a fully-qualified model ID.
///
/// - `"haiku"` / `"claude-haiku"` → haiku model
/// - `"sonnet"` / `"claude-sonnet"` → sonnet model
/// - `"opus"` / `"claude-opus"` → opus model
/// - `"inherit"` / `""` / `None` → `None` (inherit parent)
/// - anything else → `Some(as-is)`
pub fn resolve_model_alias(raw: Option<&str>) -> Option<String> {
    match raw {
        None => None,
        Some(s) => {
            let lower = s.trim().to_lowercase();
            match lower.as_str() {
                "inherit" | "" => None,
                "haiku" | "claude-haiku" => Some("claude-haiku-4-5-20251001".to_string()),
                "sonnet" | "claude-sonnet" => Some("claude-sonnet-4-6".to_string()),
                "opus" | "claude-opus" => Some("claude-opus-4-6".to_string()),
                _ => Some(s.trim().to_string()),
            }
        }
    }
}

/// Parsed and validated agent definition.
///
/// This is the live runtime representation after frontmatter parsing and
/// validation — all field errors have been collected and reported.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    /// Canonical agent name (kebab-case).
    pub name: String,
    /// Human-readable description for the routing manifest.
    pub description: String,
    /// Effective tool allowlist (post alias-resolution).  Empty = inherit all.
    pub tools: Vec<String>,
    /// Effective tool denylist (post alias-resolution).
    pub disallowed_tools: Vec<String>,
    /// Resolved model ID.  `None` = inherit parent model.
    pub resolved_model: Option<String>,
    /// Maximum loop rounds.
    pub max_turns: u32,
    /// Memory scope.
    pub memory: Option<String>,
    /// Skills to inject.
    pub skills: Vec<String>,
    /// Whether this agent runs in the background.
    pub background: bool,
    /// System prompt prefix (from the Markdown body of the definition file).
    pub system_prompt: String,
    /// Source file path (for diagnostics).
    pub source_path: std::path::PathBuf,
    /// Scope from which this definition was loaded.
    pub scope: AgentScope,
}

/// Scope from which an agent definition was loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentScope {
    /// User scope: `~/.halcon/agents/`
    User,
    /// Project scope: `.halcon/agents/`
    Project,
    /// Session scope: `--agents` CLI flag (highest priority)
    Session,
}

impl std::fmt::Display for AgentScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentScope::User => write!(f, "user"),
            AgentScope::Project => write!(f, "project"),
            AgentScope::Session => write!(f, "session"),
        }
    }
}

fn default_max_turns() -> u32 {
    20
}

/// Skill definition loaded from `.halcon/skills/<name>.md`.
#[derive(Debug, Clone)]
pub struct SkillDefinition {
    /// Skill name (from frontmatter `name` field, or derived from filename).
    pub name: String,
    /// Optional description.
    pub description: String,
    /// Skill body — appended to the agent's system prompt.
    pub body: String,
    /// Source file path.
    pub source_path: std::path::PathBuf,
}

/// Frontmatter for skill files (both fields optional).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_alias_haiku() {
        assert_eq!(
            resolve_model_alias(Some("haiku")),
            Some("claude-haiku-4-5-20251001".to_string())
        );
    }

    #[test]
    fn model_alias_sonnet() {
        assert_eq!(
            resolve_model_alias(Some("sonnet")),
            Some("claude-sonnet-4-6".to_string())
        );
    }

    #[test]
    fn model_alias_opus() {
        assert_eq!(
            resolve_model_alias(Some("opus")),
            Some("claude-opus-4-6".to_string())
        );
    }

    #[test]
    fn model_alias_inherit_is_none() {
        assert_eq!(resolve_model_alias(Some("inherit")), None);
        assert_eq!(resolve_model_alias(None), None);
        assert_eq!(resolve_model_alias(Some("")), None);
    }

    #[test]
    fn model_alias_passthrough() {
        assert_eq!(
            resolve_model_alias(Some("gpt-4o-2024-11-20")),
            Some("gpt-4o-2024-11-20".to_string())
        );
    }

    #[test]
    fn default_max_turns_is_20() {
        assert_eq!(default_max_turns(), 20);
    }
}
