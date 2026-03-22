//! Capability resolver — maps plan step tool names to the best execution source.
//!
//! Resolution priority:
//! 1. Built-in tool (exact name match in the session ToolRegistry)
//! 2. Plugin tool (BM25 search in CapabilityIndex — best match above score threshold)
//! 3. MCP tool (tool_name contains "::" separator, treated as "server::tool")
//! 4. Synthesis (no executable tool found — direct text response required)

use super::capability_index::{CapabilityCandidate, CapabilityIndex};
use super::manifest::RiskTier;

/// Minimum BM25 score required to accept a plugin capability match.
const MIN_PLUGIN_SCORE: f64 = 0.5;

// ─── Capability Source ────────────────────────────────────────────────────────

/// The resolved execution pathway for a plan step.
#[derive(Debug, Clone, PartialEq)]
pub enum CapabilitySource {
    /// Tool found in the session's ToolRegistry — use existing pipeline.
    BuiltinTool { name: String },
    /// Tool routed to a registered plugin (registry pre/post invoke hooks apply).
    PluginTool {
        plugin_id: String,
        tool_name: String,
    },
    /// Tool routed to an MCP server (`tool_name` had the form "server::tool").
    McpTool { server: String, tool_name: String },
    /// No executable tool — the agent should synthesize a direct text answer.
    Synthesis,
}

// ─── Resolved Capability ─────────────────────────────────────────────────────

/// Full resolution result for a plan step.
#[derive(Debug, Clone)]
pub struct ResolvedCapability {
    /// Where the tool invocation should be routed.
    pub source: CapabilitySource,
    /// Effective risk tier for the resolved tool.
    pub risk_tier: RiskTier,
    /// Estimated token cost of one invocation (0 = unknown).
    pub budget_tokens_estimate: u32,
    /// Whether human confirmation is required before execution.
    pub requires_confirmation: bool,
}

// ─── Resolver ────────────────────────────────────────────────────────────────

/// Resolves plan step tool names to the best available execution source.
pub struct CapabilityResolver {
    index: CapabilityIndex,
}

impl CapabilityResolver {
    /// Create a resolver backed by the given capability index.
    pub fn new(index: CapabilityIndex) -> Self {
        Self { index }
    }

    /// Resolve `tool_name` using the four-level fallthrough.
    ///
    /// # Parameters
    /// - `tool_name`: raw tool name from the plan step.
    /// - `builtin_names`: all tool names currently registered in the ToolRegistry.
    pub fn resolve(&self, tool_name: &str, builtin_names: &[String]) -> ResolvedCapability {
        // ── Level 1: Built-in tool (exact match) ──────────────────────────────
        if builtin_names.iter().any(|n| n == tool_name) {
            return ResolvedCapability {
                source: CapabilitySource::BuiltinTool {
                    name: tool_name.to_string(),
                },
                risk_tier: RiskTier::Low,
                budget_tokens_estimate: 0,
                requires_confirmation: false,
            };
        }

        // ── Level 2: Plugin tool (exact name, then BM25 search) ──────────────
        if self.index.has_entries() {
            // Exact tool_name match takes priority over BM25 (avoids low-IDF false negatives
            // in small indexes where all terms appear in every document).
            if let Some(exact) = self.index.exact_match(tool_name) {
                let requires_confirmation = exact.risk_tier >= RiskTier::High;
                return ResolvedCapability {
                    source: CapabilitySource::PluginTool {
                        plugin_id: exact.plugin_id,
                        tool_name: exact.tool_name,
                    },
                    risk_tier: exact.risk_tier,
                    budget_tokens_estimate: 0,
                    requires_confirmation,
                };
            }
            // Fall back to BM25 for natural language queries.
            let candidates: Vec<CapabilityCandidate> = self.index.search(tool_name, 3);
            if let Some(best) = candidates.into_iter().find(|c| c.score >= MIN_PLUGIN_SCORE) {
                let requires_confirmation = best.risk_tier >= RiskTier::High;
                return ResolvedCapability {
                    source: CapabilitySource::PluginTool {
                        plugin_id: best.plugin_id,
                        tool_name: best.tool_name,
                    },
                    risk_tier: best.risk_tier,
                    budget_tokens_estimate: 0,
                    requires_confirmation,
                };
            }
        }

        // ── Level 3: MCP tool (contains "::" separator) ───────────────────────
        if let Some((server, mcp_tool)) = tool_name.split_once("::") {
            return ResolvedCapability {
                source: CapabilitySource::McpTool {
                    server: server.to_string(),
                    tool_name: mcp_tool.to_string(),
                },
                risk_tier: RiskTier::Medium,
                budget_tokens_estimate: 0,
                requires_confirmation: false,
            };
        }

        // ── Level 4: Synthesis fallback ───────────────────────────────────────
        ResolvedCapability {
            source: CapabilitySource::Synthesis,
            risk_tier: RiskTier::Low,
            budget_tokens_estimate: 0,
            requires_confirmation: false,
        }
    }

    /// Whether any plugins are indexed (affects Level 2 routing).
    pub fn has_plugins(&self) -> bool {
        self.index.has_entries()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::capability_index::CapabilityIndex;
    use super::*;
    use crate::repl::plugins::manifest::{PluginManifest, ToolCapabilityDescriptor};

    fn empty_resolver() -> CapabilityResolver {
        CapabilityResolver::new(CapabilityIndex::build(&[]))
    }

    fn resolver_with_plugin() -> CapabilityResolver {
        let manifest = PluginManifest::new_local(
            "git-plugin",
            "Git Plugin",
            "1.0.0",
            vec![ToolCapabilityDescriptor {
                name: "git_search".into(),
                description: "Search git history for commits and changes".into(),
                risk_tier: RiskTier::Low,
                idempotent: true,
                permission_level: halcon_core::types::PermissionLevel::ReadOnly,
                budget_tokens_per_call: 200,
            }],
        );
        let plugins = vec![("git-plugin".into(), &manifest)];
        let index = CapabilityIndex::build(&plugins);
        CapabilityResolver::new(index)
    }

    #[test]
    fn builtin_passthrough() {
        let resolver = empty_resolver();
        let builtins = vec!["file_read".to_string(), "bash".to_string()];
        let result = resolver.resolve("file_read", &builtins);
        assert!(
            matches!(result.source, CapabilitySource::BuiltinTool { name } if name == "file_read")
        );
    }

    #[test]
    fn plugin_resolution_via_bm25() {
        let resolver = resolver_with_plugin();
        let result = resolver.resolve("git_search", &[]);
        // With an exact name match in BM25, should route to plugin
        assert!(matches!(result.source, CapabilitySource::PluginTool { .. }));
    }

    #[test]
    fn mcp_tool_double_colon_separator() {
        let resolver = empty_resolver();
        let result = resolver.resolve("github::list_prs", &[]);
        assert!(matches!(
            result.source,
            CapabilitySource::McpTool { ref server, ref tool_name }
            if server == "github" && tool_name == "list_prs"
        ));
    }

    #[test]
    fn synthesis_fallback_when_nothing_matches() {
        let resolver = empty_resolver();
        let result = resolver.resolve("completely_unknown_tool_xyz", &[]);
        assert_eq!(result.source, CapabilitySource::Synthesis);
    }

    #[test]
    fn builtin_takes_priority_over_plugins() {
        let resolver = resolver_with_plugin();
        // "git_search" exists in the plugin, but also appears as a builtin
        let builtins = vec!["git_search".to_string()];
        let result = resolver.resolve("git_search", &builtins);
        // Builtin should win (Level 1 before Level 2)
        assert!(matches!(
            result.source,
            CapabilitySource::BuiltinTool { .. }
        ));
    }

    #[test]
    fn has_plugins_false_for_empty_index() {
        let resolver = empty_resolver();
        assert!(!resolver.has_plugins());
    }
}
