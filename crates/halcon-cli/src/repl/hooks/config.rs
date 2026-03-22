//! TOML schema for lifecycle hooks.
//!
//! Loaded from two scopes (merged, last-wins):
//!   1. Global:  `~/.halcon/settings.toml`  — `[hooks]` table
//!   2. Project: `.halcon/settings.toml`     — `[hooks]` table
//!
//! The project scope is merged on top of the global scope so that project-level
//! hooks can extend (but not suppress) global hooks.
//!
//! When `PolicyConfig::allow_managed_hooks_only` is `true`, the project scope
//! is ignored — only the global/managed scope is applied (enterprise policy).

use serde::{Deserialize, Serialize};

/// Top-level hooks configuration block (TOML `[hooks]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Master switch.  When `false` (default) no hooks run regardless of definitions.
    #[serde(default)]
    pub enabled: bool,

    /// Hook definitions.
    ///
    /// Declared as `[[hooks.hooks]]` in TOML.
    #[serde(default, rename = "hooks")]
    pub definitions: Vec<HookDef>,
}

/// A single hook definition.
///
/// # TOML examples
///
/// Shell command hook that logs every bash call:
/// ```toml
/// [[hooks.hooks]]
/// event = "PreToolUse"
/// matcher = "bash"
/// type = "command"
/// command = "echo \"tool=$HALCON_TOOL_NAME\" >> /tmp/halcon-hooks.log"
/// timeout_secs = 5
/// ```
///
/// Rhai script hook that blocks bash tool usage:
/// ```toml
/// [[hooks.hooks]]
/// event = "PreToolUse"
/// matcher = "bash"
/// type = "rhai"
/// script = """
///   if halcon_tool_name == "bash" { deny("bash is not allowed in this project") }
/// """
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDef {
    /// Lifecycle event that triggers this hook.
    pub event: HookEventName,

    /// Glob pattern matched against the tool name for tool-centric events
    /// (`PreToolUse`, `PostToolUse`, `PostToolUseFailure`).
    ///
    /// Use `"*"` to match all tools.  Ignored for `UserPromptSubmit`, `Stop`,
    /// and `SessionEnd` events (which are not tool-specific).
    #[serde(default = "default_matcher")]
    pub matcher: String,

    /// Hook handler type.
    #[serde(rename = "type")]
    pub hook_type: HookType,

    /// Shell command to execute (required when `type = "command"`).
    ///
    /// Executed via `sh -c "<command>"` on Unix / `cmd /C "<command>"` on Windows.
    /// The following environment variables are set before execution:
    /// - `HALCON_TOOL_NAME`  — tool name (tool events only)
    /// - `HALCON_TOOL_INPUT` — JSON-encoded tool input (tool events only)
    /// - `HALCON_EVENT`      — event name string
    /// - `HALCON_SESSION_ID` — session UUID
    #[serde(default)]
    pub command: Option<String>,

    /// Rhai script source (required when `type = "rhai"`).
    ///
    /// The script runs inside a sandboxed engine with no stdlib / no I/O.
    /// Available functions:
    /// - `deny(reason: String)` — blocks the operation (exit code 2 equivalent)
    /// - `allow()` — explicit allow (no-op; default)
    ///
    /// Rhai variables exposed (snake_case of the env var names):
    /// - `halcon_tool_name`, `halcon_tool_input`, `halcon_event`, `halcon_session_id`
    #[serde(default)]
    pub script: Option<String>,

    /// Maximum time in seconds the hook is allowed to run (default: 30).
    ///
    /// On timeout: hook is allowed, a warning is logged.  The agent loop is never
    /// blocked beyond this duration.
    #[serde(default = "default_hook_timeout_secs")]
    pub timeout_secs: u64,
}

/// Lifecycle event that can trigger a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEventName {
    /// Fires once at the start of the agent loop, before any tool execution.
    UserPromptSubmit,
    /// Fires before every tool call (after argument validation, before execution).
    PreToolUse,
    /// Fires after a successful tool call.
    PostToolUse,
    /// Fires after a tool call that produced an error result.
    PostToolUseFailure,
    /// Fires when the agent loop terminates (EndTurn, convergence, max-rounds, etc.).
    Stop,
    /// Fires when the user session ends (CTRL+D, `/exit`).
    SessionEnd,
}

/// Hook handler type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookType {
    /// Shell command hook — runs via `sh -c`.
    Command,
    /// Rhai sandboxed script hook.
    Rhai,
}

/// Load hooks config from the two scopes (global then project, last-wins merge).
///
/// If `allow_managed_hooks_only` is true, the project scope is skipped.
pub fn load_hooks_config(allow_managed_hooks_only: bool) -> HooksConfig {
    let global_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".halcon")
        .join("settings.toml");

    let mut config = HooksConfig::default();

    // Scope 1: Global (~/.halcon/settings.toml)
    if let Ok(text) = std::fs::read_to_string(&global_path) {
        if let Ok(parsed) = toml::from_str::<HooksConfigFile>(&text) {
            if let Some(h) = parsed.hooks {
                merge_hooks(&mut config, h);
            }
        }
    }

    // Scope 2: Project (.halcon/settings.toml) — skipped under managed-only policy
    if !allow_managed_hooks_only {
        let project_path = std::path::Path::new(".halcon").join("settings.toml");
        if let Ok(text) = std::fs::read_to_string(&project_path) {
            if let Ok(parsed) = toml::from_str::<HooksConfigFile>(&text) {
                if let Some(h) = parsed.hooks {
                    merge_hooks(&mut config, h);
                }
            }
        }
    }

    config
}

/// Intermediate deserialization wrapper to allow `[hooks]` as a TOML section.
#[derive(Debug, Deserialize)]
struct HooksConfigFile {
    hooks: Option<HooksConfig>,
}

/// Merge `src` on top of `dst` (last-wins: src.enabled overrides, definitions extend).
fn merge_hooks(dst: &mut HooksConfig, src: HooksConfig) {
    if src.enabled {
        dst.enabled = true;
    }
    dst.definitions.extend(src.definitions);
}

fn default_matcher() -> String {
    "*".to_string()
}

fn default_hook_timeout_secs() -> u64 {
    30
}
