//! Tool implementations for the SOTA 2026 Project Intelligence Engine.
//!
//! Each tool is a pure async function that takes a `&Path` (project root) and
//! returns a `ToolOutput` — a sparse struct with `Option<T>` fields.
//! The orchestrator (mod.rs) merges all outputs into a single `ProjectContext`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Core types ───────────────────────────────────────────────────────────────

/// Accumulated project intelligence from all analysis waves.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectContext {
    // ── Identity ──────────────────────────────────────────────────────────────
    pub root: String,
    pub project_type: String,
    pub package_name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub edition: Option<String>,
    pub license: Option<String>,
    pub members: Vec<WorkspaceMember>,
    pub stack: Vec<String>,
    pub top_dirs: Vec<String>,
    pub has_readme: bool,
    pub files_scanned: u32,
    // ── Git intelligence ──────────────────────────────────────────────────────
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub last_commit: Option<String>,
    pub status_summary: Option<String>,
    pub total_commits: Option<u32>,
    pub contributors: Vec<String>,
    pub commit_velocity_per_week: Option<f32>,
    pub bus_factor: Option<u32>,
    // ── Infrastructure ────────────────────────────────────────────────────────
    pub has_ci: bool,
    pub ci_system: Option<String>,
    pub has_docker: bool,
    // ── Security & Quality ────────────────────────────────────────────────────
    pub has_tests: bool,
    pub test_coverage_est: Option<u8>,
    pub has_security_policy: bool,
    pub dep_count: Option<u32>,
    pub has_audit_config: bool,
    // ── Architecture ──────────────────────────────────────────────────────────
    pub architecture_style: Option<String>,
    pub has_circular_deps: bool,
    pub complexity_score: Option<u8>,
    // ── System Profile (Phase 103-A) ──────────────────────────────────────────
    pub sys_os: String,
    pub sys_cpu_cores: u32,
    pub sys_ram_mb: u64,
    pub sys_disk_free_gb: Option<f64>,
    pub sys_gpu_available: bool,
    pub sys_is_wsl: bool,
    pub sys_is_container: bool,
    // ── Tool Versions (Phase 103-B) ───────────────────────────────────────────
    pub tool_git_version: Option<String>,
    pub tool_docker_version: Option<String>,
    pub tool_node_version: Option<String>,
    pub tool_python_version: Option<String>,
    pub tool_rust_version: Option<String>,
    pub tool_go_version: Option<String>,
    pub tool_cargo_version: Option<String>,
    pub tool_make_version: Option<String>,
    pub tool_has_kubectl: bool,
    pub tool_has_helm: bool,
    pub tool_has_terraform: bool,
    pub tool_has_ansible: bool,
    // ── IDE Context (Phase 103-C) ─────────────────────────────────────────────
    pub ide_detected: Option<String>,
    pub ide_workspace: Option<String>,
    pub ide_active_file: Option<String>,
    pub ide_lsp_connected: bool,
    pub ide_lsp_port: Option<u16>,
    // ── Agent Capabilities (Phase 103-D) ──────────────────────────────────────
    pub agent_tools_available: Vec<String>,
    pub agent_mcp_servers: Vec<String>,
    pub agent_plugins_loaded: u32,
    pub agent_model_name: Option<String>,
    pub agent_model_tier: Option<String>,
    pub agent_reasoning_enabled: bool,
    pub agent_orchestration_on: bool,
    pub agent_plugin_system_on: bool,
    pub agent_multimodal_on: bool,
    pub agent_hicon_active: bool,
    // ── Runtime Profile (Phase 103-E) ─────────────────────────────────────────
    pub runtime_model_router_active: bool,
    pub runtime_convergence_controller_on: bool,
    pub runtime_intent_scorer_on: bool,
    pub runtime_token_budget: Option<u32>,
    pub runtime_ci_environment: bool,
    // ── Composite Scores (Phase 107) ──────────────────────────────────────────
    pub health_score: u8,
    pub health_issues: Vec<String>,
    pub health_recommendations: Vec<String>,
    pub agent_readiness_score: u8,
    pub environment_compatibility_score: u8,
    // ── Language Intelligence (Phase 110) ─────────────────────────────────────
    pub primary_language: String,
    pub secondary_languages: Vec<String>,
    pub is_polyglot: bool,
    pub language_breakdown: Vec<(String, u32)>,
    pub frontend_framework: Option<String>,
    pub mobile_framework: Option<String>,
    pub data_framework: Option<String>,
    pub infra_tool: Option<String>,
    // ── Monorepo Intelligence (Phase 111) ─────────────────────────────────────
    pub is_monorepo: bool,
    pub monorepo_tool: Option<String>,
    pub sub_project_count: u32,
    pub sub_projects: Vec<String>,
    // ── Project Scale (Phase 112) ─────────────────────────────────────────────
    pub project_scale: String,
    pub total_file_count: u32,
    pub estimated_loc: u64,
    // ── Distributed Architecture (Phase 113) ──────────────────────────────────
    pub architecture_patterns: Vec<String>,
    pub has_message_broker: bool,
    pub message_broker_type: Option<String>,
    pub has_service_mesh: bool,
    pub has_observability_stack: bool,
    pub has_api_gateway: bool,
    pub distributed_services_count: u32,
    // ── Advanced Scores (Phase 117) ───────────────────────────────────────────
    pub architecture_quality_score: u8,
    pub scalability_score: u8,
    pub maintainability_score: u8,
    pub technical_debt_score: u8,
    pub dev_ex_score: u8,
    pub ai_readiness_score: u8,
    pub distributed_maturity_score: u8,
    // ── Auto-Mode Suggestions (Phase 119) ────────────────────────────────────
    pub suggested_model_tier: Option<String>,
    pub suggested_agent_flags: Vec<String>,
    pub suggested_planning_strategy: Option<String>,
    pub activate_reasoning_deep: bool,
    pub activate_multimodal_for_init: bool,
    pub use_fast_mode: bool,
    pub agent_mode_rationale: Option<String>,
    // ── AI Context Files (Phase 122 — SOTA 2026) ─────────────────────────────
    /// Peer AI agent instruction files discovered in the project root.
    /// Each entry is `(filename, tool_name)` e.g. `("AGENTS.md", "OpenAI Codex/Amp")`.
    pub ai_context_files: Vec<(String, String)>,
    // ── Telemetry (Phase 109) ─────────────────────────────────────────────────
    pub analysis_duration_ms: u64,
    pub cache_hit: bool,
    pub tools_run: u32,
    pub resource_detection_time_ms: u64,
    pub environment_scan_time_ms: u64,
    pub ide_context_integration_time_ms: u64,
    pub hicon_query_time_ms: u64,
}

/// Sparse accumulator returned by each individual tool.
///
/// Only fields that the tool actually computed are `Some`. All others remain
/// `None` and are ignored during `merge_into()`.
#[derive(Debug, Default)]
pub struct ToolOutput {
    // ── Project identity ──────────────────────────────────────────────────────
    pub project_type: Option<String>,
    pub package_name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub edition: Option<String>,
    pub license: Option<String>,
    pub members: Option<Vec<WorkspaceMember>>,
    pub stack: Option<Vec<String>>,
    pub top_dirs: Option<Vec<String>>,
    pub has_readme: Option<bool>,
    pub files_scanned: Option<u32>,
    // ── Git ───────────────────────────────────────────────────────────────────
    pub branch: Option<String>,
    pub remote: Option<String>,
    pub last_commit: Option<String>,
    pub status_summary: Option<String>,
    pub total_commits: Option<u32>,
    pub contributors: Option<Vec<String>>,
    pub commit_velocity_per_week: Option<f32>,
    pub bus_factor: Option<u32>,
    // ── Infrastructure ────────────────────────────────────────────────────────
    pub has_ci: Option<bool>,
    pub ci_system: Option<String>,
    pub has_docker: Option<bool>,
    // ── Security & Quality ────────────────────────────────────────────────────
    pub has_tests: Option<bool>,
    pub test_coverage_est: Option<u8>,
    pub has_security_policy: Option<bool>,
    pub dep_count: Option<u32>,
    pub has_audit_config: Option<bool>,
    // ── Architecture ──────────────────────────────────────────────────────────
    pub architecture_style: Option<String>,
    pub has_circular_deps: Option<bool>,
    pub complexity_score: Option<u8>,
    // ── System Profile (Phase 103-A) ──────────────────────────────────────────
    pub sys_os: Option<String>,
    pub sys_cpu_cores: Option<u32>,
    pub sys_ram_mb: Option<u64>,
    pub sys_disk_free_gb: Option<f64>,
    pub sys_gpu_available: Option<bool>,
    pub sys_is_wsl: Option<bool>,
    pub sys_is_container: Option<bool>,
    // ── Tool Versions (Phase 103-B) ───────────────────────────────────────────
    pub tool_git_version: Option<String>,
    pub tool_docker_version: Option<String>,
    pub tool_node_version: Option<String>,
    pub tool_python_version: Option<String>,
    pub tool_rust_version: Option<String>,
    pub tool_go_version: Option<String>,
    pub tool_cargo_version: Option<String>,
    pub tool_make_version: Option<String>,
    pub tool_has_kubectl: Option<bool>,
    pub tool_has_helm: Option<bool>,
    pub tool_has_terraform: Option<bool>,
    pub tool_has_ansible: Option<bool>,
    // ── IDE Context (Phase 103-C) ─────────────────────────────────────────────
    pub ide_detected: Option<String>,
    pub ide_workspace: Option<String>,
    pub ide_active_file: Option<String>,
    pub ide_lsp_connected: Option<bool>,
    pub ide_lsp_port: Option<u16>,
    // ── Agent Capabilities (Phase 103-D) ──────────────────────────────────────
    pub agent_tools_available: Option<Vec<String>>,
    pub agent_mcp_servers: Option<Vec<String>>,
    pub agent_plugins_loaded: Option<u32>,
    pub agent_model_name: Option<String>,
    pub agent_model_tier: Option<String>,
    pub agent_reasoning_enabled: Option<bool>,
    pub agent_orchestration_on: Option<bool>,
    pub agent_plugin_system_on: Option<bool>,
    pub agent_multimodal_on: Option<bool>,
    pub agent_hicon_active: Option<bool>,
    // ── Runtime Profile (Phase 103-E) ─────────────────────────────────────────
    pub runtime_model_router_active: Option<bool>,
    pub runtime_convergence_controller_on: Option<bool>,
    pub runtime_intent_scorer_on: Option<bool>,
    pub runtime_token_budget: Option<u32>,
    pub runtime_ci_environment: Option<bool>,
    // ── Language Intelligence (Phase 110) ─────────────────────────────────────
    pub primary_language: Option<String>,
    pub secondary_languages: Option<Vec<String>>,
    pub is_polyglot: Option<bool>,
    pub language_breakdown: Option<Vec<(String, u32)>>,
    pub frontend_framework: Option<String>,
    pub mobile_framework: Option<String>,
    pub data_framework: Option<String>,
    pub infra_tool: Option<String>,
    // ── Monorepo Intelligence (Phase 111) ─────────────────────────────────────
    pub is_monorepo: Option<bool>,
    pub monorepo_tool: Option<String>,
    pub sub_project_count: Option<u32>,
    pub sub_projects: Option<Vec<String>>,
    // ── Project Scale (Phase 112) ─────────────────────────────────────────────
    pub project_scale: Option<String>,
    pub total_file_count: Option<u32>,
    pub estimated_loc: Option<u64>,
    // ── Distributed Architecture (Phase 113) ──────────────────────────────────
    pub architecture_patterns: Option<Vec<String>>,
    pub has_message_broker: Option<bool>,
    pub message_broker_type: Option<String>,
    pub has_service_mesh: Option<bool>,
    pub has_observability_stack: Option<bool>,
    pub has_api_gateway: Option<bool>,
    pub distributed_services_count: Option<u32>,
    // ── Advanced Scores (Phase 117) ───────────────────────────────────────────
    pub architecture_quality_score: Option<u8>,
    pub scalability_score: Option<u8>,
    pub maintainability_score: Option<u8>,
    pub technical_debt_score: Option<u8>,
    pub dev_ex_score: Option<u8>,
    pub ai_readiness_score: Option<u8>,
    pub distributed_maturity_score: Option<u8>,
    // ── Auto-Mode Suggestions (Phase 119) ────────────────────────────────────
    pub suggested_model_tier: Option<String>,
    pub suggested_agent_flags: Option<Vec<String>>,
    pub suggested_planning_strategy: Option<String>,
    pub activate_reasoning_deep: Option<bool>,
    pub activate_multimodal_for_init: Option<bool>,
    pub use_fast_mode: Option<bool>,
    pub agent_mode_rationale: Option<String>,
    // ── AI Context Files (Phase 122) ──────────────────────────────────────────
    pub ai_context_files: Option<Vec<(String, String)>>,
    // ── Telemetry (Phase 109) ─────────────────────────────────────────────────
    pub resource_detection_time_ms: Option<u64>,
    pub environment_scan_time_ms: Option<u64>,
    pub ide_context_integration_time_ms: Option<u64>,
    pub hicon_query_time_ms: Option<u64>,
}

impl ToolOutput {
    /// Merge all `Some` values from this output into the accumulated context.
    pub fn merge_into(self, ctx: &mut ProjectContext) {
        // Project identity
        if let Some(v) = self.project_type {
            ctx.project_type = v;
        }
        if let Some(v) = self.package_name {
            ctx.package_name = Some(v);
        }
        if let Some(v) = self.version {
            ctx.version = Some(v);
        }
        if let Some(v) = self.description {
            ctx.description = Some(v);
        }
        if let Some(v) = self.edition {
            ctx.edition = Some(v);
        }
        if let Some(v) = self.license {
            ctx.license = Some(v);
        }
        if let Some(v) = self.members {
            ctx.members = v;
        }
        if let Some(v) = self.stack {
            ctx.stack = v;
        }
        if let Some(v) = self.top_dirs {
            ctx.top_dirs = v;
        }
        if let Some(v) = self.has_readme {
            ctx.has_readme = v;
        }
        if let Some(v) = self.files_scanned {
            ctx.files_scanned = v;
        }
        // Git
        if let Some(v) = self.branch {
            ctx.branch = Some(v);
        }
        if let Some(v) = self.remote {
            ctx.remote = Some(v);
        }
        if let Some(v) = self.last_commit {
            ctx.last_commit = Some(v);
        }
        if let Some(v) = self.status_summary {
            ctx.status_summary = Some(v);
        }
        if let Some(v) = self.total_commits {
            ctx.total_commits = Some(v);
        }
        if let Some(v) = self.contributors {
            ctx.contributors = v;
        }
        if let Some(v) = self.commit_velocity_per_week {
            ctx.commit_velocity_per_week = Some(v);
        }
        if let Some(v) = self.bus_factor {
            ctx.bus_factor = Some(v);
        }
        // Infrastructure
        if let Some(v) = self.has_ci {
            ctx.has_ci = v;
        }
        if let Some(v) = self.ci_system {
            ctx.ci_system = Some(v);
        }
        if let Some(v) = self.has_docker {
            ctx.has_docker = v;
        }
        // Security & Quality
        if let Some(v) = self.has_tests {
            ctx.has_tests = v;
        }
        if let Some(v) = self.test_coverage_est {
            ctx.test_coverage_est = Some(v);
        }
        if let Some(v) = self.has_security_policy {
            ctx.has_security_policy = v;
        }
        if let Some(v) = self.dep_count {
            ctx.dep_count = Some(v);
        }
        if let Some(v) = self.has_audit_config {
            ctx.has_audit_config = v;
        }
        // Architecture
        if let Some(v) = self.architecture_style {
            ctx.architecture_style = Some(v);
        }
        if let Some(v) = self.has_circular_deps {
            ctx.has_circular_deps = v;
        }
        if let Some(v) = self.complexity_score {
            ctx.complexity_score = Some(v);
        }
        // System Profile
        if let Some(v) = self.sys_os {
            ctx.sys_os = v;
        }
        if let Some(v) = self.sys_cpu_cores {
            ctx.sys_cpu_cores = v;
        }
        if let Some(v) = self.sys_ram_mb {
            ctx.sys_ram_mb = v;
        }
        if let Some(v) = self.sys_disk_free_gb {
            ctx.sys_disk_free_gb = Some(v);
        }
        if let Some(v) = self.sys_gpu_available {
            ctx.sys_gpu_available = v;
        }
        if let Some(v) = self.sys_is_wsl {
            ctx.sys_is_wsl = v;
        }
        if let Some(v) = self.sys_is_container {
            ctx.sys_is_container = v;
        }
        // Tool Versions
        if let Some(v) = self.tool_git_version {
            ctx.tool_git_version = Some(v);
        }
        if let Some(v) = self.tool_docker_version {
            ctx.tool_docker_version = Some(v);
        }
        if let Some(v) = self.tool_node_version {
            ctx.tool_node_version = Some(v);
        }
        if let Some(v) = self.tool_python_version {
            ctx.tool_python_version = Some(v);
        }
        if let Some(v) = self.tool_rust_version {
            ctx.tool_rust_version = Some(v);
        }
        if let Some(v) = self.tool_go_version {
            ctx.tool_go_version = Some(v);
        }
        if let Some(v) = self.tool_cargo_version {
            ctx.tool_cargo_version = Some(v);
        }
        if let Some(v) = self.tool_make_version {
            ctx.tool_make_version = Some(v);
        }
        if let Some(v) = self.tool_has_kubectl {
            ctx.tool_has_kubectl = v;
        }
        if let Some(v) = self.tool_has_helm {
            ctx.tool_has_helm = v;
        }
        if let Some(v) = self.tool_has_terraform {
            ctx.tool_has_terraform = v;
        }
        if let Some(v) = self.tool_has_ansible {
            ctx.tool_has_ansible = v;
        }
        // IDE Context
        if let Some(v) = self.ide_detected {
            ctx.ide_detected = Some(v);
        }
        if let Some(v) = self.ide_workspace {
            ctx.ide_workspace = Some(v);
        }
        if let Some(v) = self.ide_active_file {
            ctx.ide_active_file = Some(v);
        }
        if let Some(v) = self.ide_lsp_connected {
            ctx.ide_lsp_connected = v;
        }
        if let Some(v) = self.ide_lsp_port {
            ctx.ide_lsp_port = Some(v);
        }
        // Agent Capabilities
        if let Some(v) = self.agent_tools_available {
            ctx.agent_tools_available = v;
        }
        if let Some(v) = self.agent_mcp_servers {
            ctx.agent_mcp_servers = v;
        }
        if let Some(v) = self.agent_plugins_loaded {
            ctx.agent_plugins_loaded = v;
        }
        if let Some(v) = self.agent_model_name {
            ctx.agent_model_name = Some(v);
        }
        if let Some(v) = self.agent_model_tier {
            ctx.agent_model_tier = Some(v);
        }
        if let Some(v) = self.agent_reasoning_enabled {
            ctx.agent_reasoning_enabled = v;
        }
        if let Some(v) = self.agent_orchestration_on {
            ctx.agent_orchestration_on = v;
        }
        if let Some(v) = self.agent_plugin_system_on {
            ctx.agent_plugin_system_on = v;
        }
        if let Some(v) = self.agent_multimodal_on {
            ctx.agent_multimodal_on = v;
        }
        if let Some(v) = self.agent_hicon_active {
            ctx.agent_hicon_active = v;
        }
        // Runtime Profile
        if let Some(v) = self.runtime_model_router_active {
            ctx.runtime_model_router_active = v;
        }
        if let Some(v) = self.runtime_convergence_controller_on {
            ctx.runtime_convergence_controller_on = v;
        }
        if let Some(v) = self.runtime_intent_scorer_on {
            ctx.runtime_intent_scorer_on = v;
        }
        if let Some(v) = self.runtime_token_budget {
            ctx.runtime_token_budget = Some(v);
        }
        if let Some(v) = self.runtime_ci_environment {
            ctx.runtime_ci_environment = v;
        }
        // Language Intelligence (Phase 110)
        if let Some(v) = self.primary_language {
            ctx.primary_language = v;
        }
        if let Some(v) = self.secondary_languages {
            ctx.secondary_languages = v;
        }
        if let Some(v) = self.is_polyglot {
            ctx.is_polyglot = v;
        }
        if let Some(v) = self.language_breakdown {
            ctx.language_breakdown = v;
        }
        if let Some(v) = self.frontend_framework {
            ctx.frontend_framework = Some(v);
        }
        if let Some(v) = self.mobile_framework {
            ctx.mobile_framework = Some(v);
        }
        if let Some(v) = self.data_framework {
            ctx.data_framework = Some(v);
        }
        if let Some(v) = self.infra_tool {
            ctx.infra_tool = Some(v);
        }
        // Monorepo Intelligence (Phase 111)
        if let Some(v) = self.is_monorepo {
            ctx.is_monorepo = v;
        }
        if let Some(v) = self.monorepo_tool {
            ctx.monorepo_tool = Some(v);
        }
        if let Some(v) = self.sub_project_count {
            ctx.sub_project_count = v;
        }
        if let Some(v) = self.sub_projects {
            ctx.sub_projects = v;
        }
        // Project Scale (Phase 112)
        if let Some(v) = self.project_scale {
            ctx.project_scale = v;
        }
        if let Some(v) = self.total_file_count {
            ctx.total_file_count = v;
        }
        if let Some(v) = self.estimated_loc {
            ctx.estimated_loc = v;
        }
        // Distributed Architecture (Phase 113)
        if let Some(v) = self.architecture_patterns {
            ctx.architecture_patterns = v;
        }
        if let Some(v) = self.has_message_broker {
            ctx.has_message_broker = v;
        }
        if let Some(v) = self.message_broker_type {
            ctx.message_broker_type = Some(v);
        }
        if let Some(v) = self.has_service_mesh {
            ctx.has_service_mesh = v;
        }
        if let Some(v) = self.has_observability_stack {
            ctx.has_observability_stack = v;
        }
        if let Some(v) = self.has_api_gateway {
            ctx.has_api_gateway = v;
        }
        if let Some(v) = self.distributed_services_count {
            ctx.distributed_services_count = v;
        }
        // Advanced Scores (Phase 117)
        if let Some(v) = self.architecture_quality_score {
            ctx.architecture_quality_score = v;
        }
        if let Some(v) = self.scalability_score {
            ctx.scalability_score = v;
        }
        if let Some(v) = self.maintainability_score {
            ctx.maintainability_score = v;
        }
        if let Some(v) = self.technical_debt_score {
            ctx.technical_debt_score = v;
        }
        if let Some(v) = self.dev_ex_score {
            ctx.dev_ex_score = v;
        }
        if let Some(v) = self.ai_readiness_score {
            ctx.ai_readiness_score = v;
        }
        if let Some(v) = self.distributed_maturity_score {
            ctx.distributed_maturity_score = v;
        }
        // Auto-Mode Suggestions (Phase 119)
        if let Some(v) = self.suggested_model_tier {
            ctx.suggested_model_tier = Some(v);
        }
        if let Some(v) = self.suggested_agent_flags {
            ctx.suggested_agent_flags = v;
        }
        if let Some(v) = self.suggested_planning_strategy {
            ctx.suggested_planning_strategy = Some(v);
        }
        if let Some(v) = self.activate_reasoning_deep {
            ctx.activate_reasoning_deep = v;
        }
        if let Some(v) = self.activate_multimodal_for_init {
            ctx.activate_multimodal_for_init = v;
        }
        if let Some(v) = self.use_fast_mode {
            ctx.use_fast_mode = v;
        }
        if let Some(v) = self.agent_mode_rationale {
            ctx.agent_mode_rationale = Some(v);
        }
        // AI Context Files (Phase 122) — extend, not replace
        if let Some(v) = self.ai_context_files {
            ctx.ai_context_files.extend(v);
        }
        // Telemetry (accumulate, not overwrite)
        if let Some(v) = self.resource_detection_time_ms {
            ctx.resource_detection_time_ms = ctx.resource_detection_time_ms.max(v);
        }
        if let Some(v) = self.environment_scan_time_ms {
            ctx.environment_scan_time_ms = ctx.environment_scan_time_ms.max(v);
        }
        if let Some(v) = self.ide_context_integration_time_ms {
            ctx.ide_context_integration_time_ms = ctx.ide_context_integration_time_ms.max(v);
        }
        if let Some(v) = self.hicon_query_time_ms {
            ctx.hicon_query_time_ms = ctx.hicon_query_time_ms.max(v);
        }
    }
}

/// A workspace member (crate / npm package / etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMember {
    pub name: String,
    pub path: String,
    pub description: Option<String>,
}

// ─── Wave 0: Project Root Detection ──────────────────────────────────────────

/// Walk up from `start` looking for a workspace/project root.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    const MARKERS: &[&str] = &[
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "setup.py",
        "go.mod",
        "build.gradle",
        "pom.xml",
        "CMakeLists.txt",
        "Makefile",
    ];
    let mut current = start.to_path_buf();
    loop {
        for marker in MARKERS {
            if current.join(marker).exists() {
                return Some(current);
            }
        }
        if !current.pop() {
            break;
        }
    }
    None
}

// ─── Wave 1A: Filesystem Scanner ──────────────────────────────────────────────

/// Scan top-level directory structure and count source files.
pub async fn filesystem_scanner(root: &Path) -> ToolOutput {
    const SKIP: &[&str] = &[
        "target",
        "node_modules",
        ".git",
        ".cargo",
        "dist",
        "build",
        "__pycache__",
        ".venv",
        ".next",
        "out",
        "coverage",
        ".turbo",
        "vendor",
        "third_party",
    ];

    let mut top_dirs = vec![];
    let mut files_scanned: u32 = 0;
    let mut has_readme = false;

    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if path.is_dir() {
                if !name.starts_with('.') && !SKIP.contains(&name.as_str()) {
                    top_dirs.push(name.clone());
                    // Count files one level deep for telemetry
                    if let Ok(sub) = std::fs::read_dir(&path) {
                        files_scanned += sub.flatten().count() as u32;
                    }
                }
            } else {
                // Check for README at root level
                let lower = name.to_lowercase();
                if lower.starts_with("readme") {
                    has_readme = true;
                }
                files_scanned += 1;
            }
        }
    }

    top_dirs.sort();
    ToolOutput {
        top_dirs: Some(top_dirs),
        files_scanned: Some(files_scanned),
        has_readme: Some(has_readme),
        ..Default::default()
    }
}

// ─── Wave 1B: Project Type Detector ──────────────────────────────────────────

/// Detect the primary project type from well-known marker files.
pub fn type_detector(root: &Path) -> ToolOutput {
    let project_type = detect_project_type(root);
    ToolOutput {
        project_type: Some(project_type),
        ..Default::default()
    }
}

/// Standalone function (also used by tests).
pub fn detect_project_type(root: &Path) -> String {
    if root.join("Cargo.toml").exists() {
        if let Ok(c) = std::fs::read_to_string(root.join("Cargo.toml")) {
            if c.contains("[workspace]") {
                return "Rust Workspace".to_string();
            }
        }
        return "Rust".to_string();
    }
    if root.join("package.json").exists() {
        if let Ok(c) = std::fs::read_to_string(root.join("package.json")) {
            if c.contains("\"workspaces\"") {
                return "Node.js Monorepo".to_string();
            }
            if c.contains("\"next\"") {
                return "Next.js".to_string();
            }
            if c.contains("\"react\"") {
                return "React".to_string();
            }
        }
        return "Node.js".to_string();
    }
    if root.join("pyproject.toml").exists() || root.join("setup.py").exists() {
        return "Python".to_string();
    }
    if root.join("go.mod").exists() {
        return "Go".to_string();
    }
    if root.join("build.gradle").exists() {
        return "Java/Kotlin (Gradle)".to_string();
    }
    if root.join("pom.xml").exists() {
        return "Java (Maven)".to_string();
    }
    "Unknown".to_string()
}

// ─── Wave 2A: Metadata Reader ─────────────────────────────────────────────────

pub async fn metadata_reader(root: &Path, project_type: &str) -> ToolOutput {
    if project_type.starts_with("Rust") {
        read_cargo_metadata(root).await
    } else if project_type.contains("Node")
        || project_type.contains("React")
        || project_type.contains("Next")
    {
        read_npm_metadata(root).await
    } else if project_type == "Python" {
        read_python_metadata(root).await
    } else if project_type == "Go" {
        read_go_metadata(root).await
    } else {
        ToolOutput::default()
    }
}

async fn read_cargo_metadata(root: &Path) -> ToolOutput {
    let Ok(content) = tokio::fs::read_to_string(root.join("Cargo.toml")).await else {
        return ToolOutput::default();
    };

    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut description: Option<String> = None;
    let mut edition: Option<String> = None;
    let mut in_package = false;
    let mut in_workspace = false;
    let mut raw_members: Vec<String> = vec![];
    let mut collecting_members = false;
    let mut member_buf = String::new();

    for line in content.lines() {
        let t = line.trim();
        if t == "[package]" {
            in_package = true;
            in_workspace = false;
            collecting_members = false;
            continue;
        }
        if t == "[workspace]" {
            in_workspace = true;
            in_package = false;
            collecting_members = false;
            continue;
        }
        if t.starts_with('[') && !t.starts_with("[[") {
            in_package = false;
            in_workspace = false;
            collecting_members = false;
        }

        if in_package {
            if name.is_none() {
                name = parse_toml_str(t, "name");
            }
            if version.is_none() {
                version = parse_toml_str(t, "version");
            }
            if description.is_none() {
                description = parse_toml_str(t, "description");
            }
            if edition.is_none() {
                edition = parse_toml_str(t, "edition");
            }
        }

        if in_workspace {
            if t.starts_with("members") {
                member_buf.push_str(t);
                if t.contains(']') {
                    raw_members = extract_string_array(&member_buf);
                    member_buf.clear();
                } else {
                    collecting_members = true;
                }
            } else if collecting_members {
                member_buf.push(' ');
                member_buf.push_str(t);
                if t.contains(']') {
                    raw_members = extract_string_array(&member_buf);
                    member_buf.clear();
                    collecting_members = false;
                }
            }
        }
    }

    let members = expand_workspace_members(root, &raw_members).await;
    let stack = detect_rust_stack(&content);

    // Count dependencies
    let dep_count = count_cargo_deps(&content);

    ToolOutput {
        package_name: name,
        version,
        description,
        edition,
        members: Some(members),
        stack: Some(stack),
        dep_count: Some(dep_count),
        ..Default::default()
    }
}

fn count_cargo_deps(content: &str) -> u32 {
    let mut in_deps = false;
    let mut count = 0u32;
    for line in content.lines() {
        let t = line.trim();
        if t == "[dependencies]" || t == "[dev-dependencies]" || t == "[build-dependencies]" {
            in_deps = true;
            continue;
        }
        if t.starts_with('[') && !t.starts_with("[[") {
            in_deps = false;
        }
        if in_deps && !t.is_empty() && !t.starts_with('#') && t.contains('=') {
            count += 1;
        }
    }
    count
}

async fn expand_workspace_members(root: &Path, patterns: &[String]) -> Vec<WorkspaceMember> {
    let mut result = Vec::new();
    for pattern in patterns {
        if pattern.contains('*') {
            let base_part = pattern.split('*').next().unwrap_or("");
            let base = root.join(base_part.trim_end_matches('/'));
            if let Ok(mut rd) = tokio::fs::read_dir(&base).await {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    let path = entry.path();
                    if path.is_dir() && path.join("Cargo.toml").exists() {
                        let crate_name = read_crate_field(&path, "name").await;
                        let desc = read_crate_field(&path, "description").await;
                        let rel = path
                            .strip_prefix(root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .to_string();
                        result.push(WorkspaceMember {
                            name: crate_name.unwrap_or_else(|| {
                                path.file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string()
                            }),
                            path: rel,
                            description: desc,
                        });
                    }
                }
            }
        } else {
            let path = root.join(pattern);
            if path.exists() {
                let crate_name = read_crate_field(&path, "name").await;
                let desc = read_crate_field(&path, "description").await;
                result.push(WorkspaceMember {
                    name: crate_name.unwrap_or_else(|| pattern.clone()),
                    path: pattern.clone(),
                    description: desc,
                });
            }
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

async fn read_crate_field(crate_root: &Path, field: &str) -> Option<String> {
    let content = tokio::fs::read_to_string(crate_root.join("Cargo.toml"))
        .await
        .ok()?;
    let mut in_pkg = false;
    for line in content.lines() {
        let t = line.trim();
        if t == "[package]" {
            in_pkg = true;
            continue;
        }
        if t.starts_with('[') {
            in_pkg = false;
        }
        if in_pkg {
            if let Some(v) = parse_toml_str(t, field) {
                return Some(v);
            }
        }
    }
    None
}

fn detect_rust_stack(content: &str) -> Vec<String> {
    let candidates = [
        ("tokio", "tokio async"),
        ("axum", "axum web"),
        ("actix-web", "actix-web"),
        ("reqwest", "reqwest HTTP"),
        ("sqlx", "SQLx async"),
        ("rusqlite", "rusqlite / SQLite"),
        ("serde", "serde"),
        ("clap", "clap CLI"),
        ("ratatui", "ratatui TUI"),
        ("tonic", "tonic gRPC"),
        ("wasm-bindgen", "WASM"),
        ("tauri", "tauri desktop"),
        ("crossterm", "crossterm terminal"),
        ("tracing", "tracing observability"),
        ("thiserror", "thiserror"),
        ("anyhow", "anyhow"),
    ];
    candidates
        .iter()
        .filter(|(dep, _)| content.contains(dep))
        .map(|(_, label)| label.to_string())
        .collect()
}

async fn read_npm_metadata(root: &Path) -> ToolOutput {
    let Ok(content) = tokio::fs::read_to_string(root.join("package.json")).await else {
        return ToolOutput::default();
    };
    let dep_count = count_json_deps(&content);
    ToolOutput {
        package_name: extract_json_str(&content, "name"),
        version: extract_json_str(&content, "version"),
        description: extract_json_str(&content, "description"),
        license: extract_json_str(&content, "license"),
        dep_count: Some(dep_count),
        ..Default::default()
    }
}

fn count_json_deps(json: &str) -> u32 {
    // Count keys in "dependencies" + "devDependencies" sections (approximate)
    let mut count = 0u32;
    let mut in_deps = false;
    let mut brace_depth = 0i32;
    for line in json.lines() {
        let t = line.trim();
        if t.contains("\"dependencies\"") || t.contains("\"devDependencies\"") {
            in_deps = true;
        }
        if in_deps {
            brace_depth += t.chars().filter(|&c| c == '{').count() as i32;
            brace_depth -= t.chars().filter(|&c| c == '}').count() as i32;
            if brace_depth == 2 && t.starts_with('"') {
                count += 1;
            }
            if brace_depth <= 0 {
                in_deps = false;
                brace_depth = 0;
            }
        }
    }
    count
}

async fn read_python_metadata(root: &Path) -> ToolOutput {
    if let Ok(content) = tokio::fs::read_to_string(root.join("pyproject.toml")).await {
        return ToolOutput {
            package_name: find_toml_str(&content, "name"),
            version: find_toml_str(&content, "version"),
            description: find_toml_str(&content, "description"),
            license: find_toml_str(&content, "license"),
            ..Default::default()
        };
    }
    ToolOutput::default()
}

async fn read_go_metadata(root: &Path) -> ToolOutput {
    let Ok(content) = tokio::fs::read_to_string(root.join("go.mod")).await else {
        return ToolOutput::default();
    };
    // First non-comment line: `module github.com/user/repo`
    let module = content
        .lines()
        .find(|l| l.trim().starts_with("module "))
        .map(|l| l.trim().trim_start_matches("module ").trim().to_string());
    ToolOutput {
        package_name: module,
        ..Default::default()
    }
}

// ─── Wave 2B: Git Intelligence ────────────────────────────────────────────────

/// Advanced git analysis: branch, remote, commits, velocity, contributors, bus factor.
pub async fn git_intelligence(root: &Path) -> ToolOutput {
    let git = |args: &[&str]| -> Option<String> {
        std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout)
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                } else {
                    None
                }
            })
    };

    let branch = git(&["branch", "--show-current"]);
    if branch.is_none() {
        // Not a git repo
        return ToolOutput::default();
    }

    let remote = git(&["remote", "get-url", "origin"]);
    let last_commit = git(&["log", "-1", "--format=%h — %s"]);

    let status_summary = git(&["status", "--porcelain"]).map(|s| {
        let n = s.lines().filter(|l| !l.trim().is_empty()).count();
        if n == 0 {
            "working tree limpio".to_string()
        } else {
            format!("{n} archivos modificados")
        }
    });

    let total_commits = git(&["rev-list", "--count", "HEAD"]).and_then(|s| s.parse::<u32>().ok());

    // Contributor analysis (top 20 by commit count)
    let contributor_data = git(&["shortlog", "-sn", "--no-merges", "HEAD"]);
    let contributors: Vec<String> = contributor_data
        .as_deref()
        .unwrap_or("")
        .lines()
        .take(20)
        .filter_map(|l| {
            let parts: Vec<&str> = l.trim().splitn(2, '\t').collect();
            if parts.len() == 2 {
                Some(format!("{} ({})", parts[1].trim(), parts[0].trim()))
            } else {
                None
            }
        })
        .collect();

    // Bus factor: percentage of commits by top contributor
    let bus_factor = if contributors.len() >= 2 {
        // Rough heuristic: 1 + floor(contributors where top_n account for >80% of commits)
        Some(contributors.len().min(5) as u32)
    } else {
        Some(1)
    };

    // Commit velocity: commits in the last 30 days
    let recent_commits = git(&["rev-list", "--count", "--since=30.days.ago", "HEAD"])
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let commit_velocity_per_week = Some(recent_commits as f32 / 4.3); // ~4.3 weeks in 30 days

    ToolOutput {
        branch,
        remote,
        last_commit,
        status_summary,
        total_commits,
        contributors: Some(contributors),
        commit_velocity_per_week,
        bus_factor,
        ..Default::default()
    }
}

// ─── Wave 2C: CI/CD Detector ─────────────────────────────────────────────────

pub async fn cicd_detector(root: &Path) -> ToolOutput {
    let has_github_actions = root.join(".github").join("workflows").exists();
    let has_gitlab_ci = root.join(".gitlab-ci.yml").exists();
    let has_jenkins = root.join("Jenkinsfile").exists();
    let has_circle = root.join(".circleci").exists();
    let has_travis = root.join(".travis.yml").exists();
    let has_drone = root.join(".drone.yml").exists();
    let has_buildkite = root.join(".buildkite").exists();

    let ci_system = if has_github_actions {
        Some("GitHub Actions".to_string())
    } else if has_gitlab_ci {
        Some("GitLab CI".to_string())
    } else if has_jenkins {
        Some("Jenkins".to_string())
    } else if has_circle {
        Some("CircleCI".to_string())
    } else if has_travis {
        Some("Travis CI".to_string())
    } else if has_drone {
        Some("Drone CI".to_string())
    } else if has_buildkite {
        Some("Buildkite".to_string())
    } else {
        None
    };

    let has_ci = ci_system.is_some();

    ToolOutput {
        has_ci: Some(has_ci),
        ci_system,
        ..Default::default()
    }
}

// ─── Wave 2D: Docker Detector ─────────────────────────────────────────────────

pub async fn docker_detector(root: &Path) -> ToolOutput {
    let has_dockerfile = root.join("Dockerfile").exists()
        || root.join("Dockerfile.dev").exists()
        || root.join("Containerfile").exists();
    let has_compose = root.join("docker-compose.yml").exists()
        || root.join("docker-compose.yaml").exists()
        || root.join("compose.yml").exists();

    ToolOutput {
        has_docker: Some(has_dockerfile || has_compose),
        ..Default::default()
    }
}

// ─── Wave 2E: Security Scanner (light) ────────────────────────────────────────

pub async fn security_scanner(root: &Path) -> ToolOutput {
    let has_security_md = root.join("SECURITY.md").exists()
        || root.join("SECURITY.txt").exists()
        || root.join(".github").join("SECURITY.md").exists();

    // Audit/dependency checking configs
    let has_audit_config = root.join("deny.toml").exists()     // cargo-deny
        || root.join(".cargo-audit.toml").exists()             // cargo-audit
        || root.join(".snyk").exists()                         // Snyk
        || root.join("trivy.yaml").exists()                    // Trivy
        || root.join(".grype.yaml").exists(); // Grype

    // Check for license file
    let license = detect_license(root);

    ToolOutput {
        has_security_policy: Some(has_security_md),
        has_audit_config: Some(has_audit_config),
        license,
        ..Default::default()
    }
}

fn detect_license(root: &Path) -> Option<String> {
    let candidates = [
        "LICENSE",
        "LICENSE.md",
        "LICENSE.txt",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "COPYING",
    ];
    for name in &candidates {
        let path = root.join(name);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let first_lines: String = content.lines().take(5).collect::<Vec<_>>().join(" ");
                if first_lines.contains("MIT") {
                    return Some("MIT".to_string());
                }
                if first_lines.contains("Apache") {
                    return Some("Apache-2.0".to_string());
                }
                if first_lines.contains("GNU GENERAL PUBLIC") {
                    return Some("GPL".to_string());
                }
                if first_lines.contains("Mozilla") {
                    return Some("MPL-2.0".to_string());
                }
                if first_lines.contains("BSD") {
                    return Some("BSD".to_string());
                }
                if first_lines.contains("ISC") {
                    return Some("ISC".to_string());
                }
                return Some("Custom".to_string());
            }
            return Some("(present)".to_string());
        }
    }
    None
}

// ─── Wave 2F: Test Coverage Estimator ────────────────────────────────────────

pub async fn test_coverage_estimator(root: &Path, project_type: &str) -> ToolOutput {
    let (has_tests, coverage_est) = estimate_test_coverage(root, project_type).await;
    ToolOutput {
        has_tests: Some(has_tests),
        test_coverage_est: coverage_est,
        ..Default::default()
    }
}

async fn estimate_test_coverage(root: &Path, project_type: &str) -> (bool, Option<u8>) {
    if project_type.starts_with("Rust") {
        // Count #[test] and #[cfg(test)] occurrences in .rs files
        let test_count = count_rust_tests(root).await;
        let has_tests = test_count > 0;
        // Rough coverage estimate: tests > 50 → ~60%, tests > 200 → ~80%
        let est = if !has_tests {
            0
        } else if test_count < 20 {
            20
        } else if test_count < 50 {
            40
        } else if test_count < 200 {
            65
        } else {
            80
        };
        (has_tests, if has_tests { Some(est) } else { None })
    } else if project_type.contains("Node")
        || project_type.contains("React")
        || project_type.contains("Next")
    {
        // Check for jest/vitest/mocha config
        let has_jest = root.join("jest.config.js").exists()
            || root.join("jest.config.ts").exists()
            || root.join("vitest.config.ts").exists()
            || root.join(".mocharc.js").exists();
        // Check for __tests__ directories
        let has_test_dir = root.join("__tests__").exists()
            || root.join("test").exists()
            || root.join("tests").exists();
        let has_tests = has_jest || has_test_dir;
        (has_tests, if has_tests { Some(50) } else { None })
    } else if project_type == "Python" {
        let has_pytest = root.join("pytest.ini").exists()
            || root.join("setup.cfg").exists()
            || root.join("pyproject.toml").exists();
        let has_test_dir = root.join("tests").exists() || root.join("test").exists();
        let has_tests = has_pytest || has_test_dir;
        (has_tests, if has_tests { Some(45) } else { None })
    } else {
        let has_test_dir =
            root.join("test").exists() || root.join("tests").exists() || root.join("spec").exists();
        (has_test_dir, None)
    }
}

async fn count_rust_tests(root: &Path) -> u32 {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || count_pattern_in_dir(&root, "#[test]", &["target", ".git"]))
        .await
        .unwrap_or(0)
}

fn count_pattern_in_dir(dir: &Path, pattern: &str, skip: &[&str]) -> u32 {
    let mut count = 0u32;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return count;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if skip.contains(&name.as_ref()) {
            continue;
        }
        if path.is_dir() {
            count += count_pattern_in_dir(&path, pattern, skip);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                count += content.matches(pattern).count() as u32;
            }
        }
    }
    count
}

// ─── Wave 3A: Dependency Analyzer ────────────────────────────────────────────

pub async fn dependency_analyzer(root: &Path, project_type: &str) -> ToolOutput {
    // Check for lock files (indicates dep management is active)
    let has_lock = root.join("Cargo.lock").exists()
        || root.join("package-lock.json").exists()
        || root.join("yarn.lock").exists()
        || root.join("pnpm-lock.yaml").exists()
        || root.join("poetry.lock").exists()
        || root.join("go.sum").exists();

    if !has_lock && project_type.starts_with("Rust") {
        // Library crates may not have Cargo.lock — that's fine
    }

    ToolOutput::default() // dep_count already set by metadata_reader
}

// ─── Wave 3B: Architecture Detector ──────────────────────────────────────────

pub async fn architecture_detector(root: &Path, members: &[WorkspaceMember]) -> ToolOutput {
    let style = if !members.is_empty() {
        if members.len() > 5 {
            "monorepo".to_string()
        } else {
            "workspace".to_string()
        }
    } else {
        // Single crate/package: check directory layout for layered pattern
        let has_domain = root.join("domain").exists() || root.join("core").exists();
        let has_infra = root.join("infrastructure").exists() || root.join("infra").exists();
        let has_api = root.join("api").exists() || root.join("routes").exists();
        if has_domain && has_infra {
            "layered".to_string()
        } else if has_api {
            "api-first".to_string()
        } else {
            "monolith".to_string()
        }
    };

    // Complexity estimate based on member count and file count
    let complexity = if members.len() > 10 {
        70
    } else if members.len() > 5 {
        50
    } else {
        30
    } as u8;

    ToolOutput {
        architecture_style: Some(style),
        has_circular_deps: Some(false), // Static analysis would be needed for accurate detection
        complexity_score: Some(complexity),
        ..Default::default()
    }
}

// ─── Wave 4: Health Score Calculator (pure function) ─────────────────────────

/// Compute a composite health score (0-100) from the accumulated project context.
///
/// Components:
/// - CI/CD presence:     0-15 points
/// - Tests:              0-20 points
/// - Git activity:       0-15 points
/// - Security policy:    0-15 points
/// - Docker/containers:  0-5  points
/// - Architecture:       0-10 points
/// - Documentation:      0-10 points
/// - Complexity (inverse):0-10 points
pub fn health_score_calculator(ctx: &ProjectContext) -> (u8, Vec<String>, Vec<String>) {
    let mut score: u32 = 0;
    let mut issues = Vec::new();
    let mut recommendations = Vec::new();

    // CI/CD (0-15)
    if ctx.has_ci {
        score += 15;
    } else {
        issues.push("No CI/CD system detected".to_string());
        recommendations.push("Add GitHub Actions or GitLab CI for automated testing".to_string());
    }

    // Tests (0-20)
    if ctx.has_tests {
        let coverage = ctx.test_coverage_est.unwrap_or(30);
        let test_score = if coverage >= 80 {
            20
        } else if coverage >= 60 {
            16
        } else if coverage >= 40 {
            12
        } else {
            8
        };
        score += test_score;
        if coverage < 60 {
            issues.push(format!("Low estimated test coverage (~{}%)", coverage));
            recommendations.push("Increase test coverage to at least 60%".to_string());
        }
    } else {
        issues.push("No test suite detected".to_string());
        recommendations
            .push("Add tests using the appropriate framework for your project".to_string());
    }

    // Git activity (0-15)
    if ctx.branch.is_some() {
        let velocity = ctx.commit_velocity_per_week.unwrap_or(0.0);
        let git_score = if velocity >= 5.0 {
            15
        } else if velocity >= 2.0 {
            12
        } else if velocity >= 0.5 {
            8
        } else {
            4
        };
        score += git_score;
        if let Some(bf) = ctx.bus_factor {
            if bf <= 1 {
                issues.push("Bus factor of 1 — single point of failure".to_string());
                recommendations.push(
                    "Encourage more contributors to reduce knowledge concentration".to_string(),
                );
            }
        }
        if velocity < 1.0 && ctx.total_commits.unwrap_or(0) > 0 {
            issues.push("Low commit velocity (< 1/week)".to_string());
        }
    } else {
        issues.push("No git repository found".to_string());
        recommendations.push("Initialize a git repository for version control".to_string());
    }

    // Security policy (0-15)
    if ctx.has_security_policy {
        score += 10;
    } else {
        issues.push("No SECURITY.md found".to_string());
        recommendations.push("Add SECURITY.md with responsible disclosure policy".to_string());
    }
    if ctx.has_audit_config {
        score += 5;
    } else if ctx.project_type.starts_with("Rust") {
        recommendations.push("Add deny.toml for cargo-deny dependency auditing".to_string());
    }

    // Docker/containers (0-5)
    if ctx.has_docker {
        score += 5;
    }

    // Architecture quality (0-10)
    match ctx.architecture_style.as_deref() {
        Some("layered") | Some("api-first") => score += 10,
        Some("workspace") | Some("monorepo") => score += 8,
        Some("monolith") => {
            score += 5;
        }
        _ => {
            score += 3;
        }
    }

    // Documentation (0-10)
    if ctx.has_readme {
        score += 7;
    } else {
        issues.push("No README found".to_string());
        recommendations
            .push("Add a README.md with project overview and setup instructions".to_string());
    }
    if ctx.license.is_some() {
        score += 3;
    } else {
        recommendations.push("Add a LICENSE file to clarify usage terms".to_string());
    }

    // Complexity (inverse, 0-10)
    let complexity = ctx.complexity_score.unwrap_or(30);
    let complexity_score = if complexity < 30 {
        10
    } else if complexity < 50 {
        7
    } else if complexity < 70 {
        4
    } else {
        1
    };
    score += complexity_score;

    let final_score = score.min(100) as u8;
    (final_score, issues, recommendations)
}

// ─── TOML / JSON helpers (shared) ────────────────────────────────────────────

pub fn parse_toml_str(line: &str, key: &str) -> Option<String> {
    let rest = if line.starts_with(&format!("{key} =")) {
        line[key.len() + 2..].trim()
    } else if line.starts_with(&format!("{key}=")) {
        line[key.len() + 1..].trim()
    } else {
        return None;
    };
    let rest = rest.trim_start_matches('=').trim();
    if rest.len() >= 2
        && ((rest.starts_with('"') && rest.ends_with('"'))
            || (rest.starts_with('\'') && rest.ends_with('\'')))
    {
        Some(rest[1..rest.len() - 1].to_string())
    } else {
        None
    }
}

pub fn find_toml_str(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(v) = parse_toml_str(line.trim(), key) {
            return Some(v);
        }
    }
    None
}

pub fn extract_string_array(text: &str) -> Vec<String> {
    let start = text.find('[').unwrap_or(text.len());
    let end = text.rfind(']').unwrap_or(0);
    if start >= end {
        return vec![];
    }
    text[start + 1..end]
        .split(',')
        .filter_map(|s| {
            let s = s.trim().trim_matches('"').trim_matches('\'').trim();
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        })
        .collect()
}

pub fn extract_json_str(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)?;
    let rest = json[pos + needle.len()..].trim_start();
    let rest = rest.trim_start_matches(':').trim();
    if rest.starts_with('"') {
        let end = rest[1..].find('"')?;
        Some(rest[1..end + 1].to_string())
    } else {
        None
    }
}

// ─── Phase 122: AI Context File Scanner ───────────────────────────────────────

/// SOTA 2026: Discover which AI coding assistant configuration files exist in
/// the project root. This lets HALCON.md document the multi-tool environment
/// and lets the loader pick up all relevant context.
///
/// Checks for: HALCON.md, AGENTS.md, CLAUDE.md, .cursorrules, .cursor/rules/,
/// .github/copilot-instructions.md, .junie/guidelines.md, .continuerules,
/// GEMINI.md, .aider.conf.yml
pub async fn ai_context_file_scanner(root: &Path) -> ToolOutput {
    let root = root.to_path_buf();
    let result = tokio::task::spawn_blocking(move || scan_ai_context_files(&root))
        .await
        .unwrap_or_default();
    result
}

fn scan_ai_context_files(root: &Path) -> ToolOutput {
    /// Maps (relative_path_components, display_name)
    const KNOWN_FILES: &[(&[&str], &str)] = &[
        (&[".halcon", "HALCON.md"], "Halcon (generated)"),
        (&["HALCON.md"], "Halcon (root-level)"),
        (&["AGENTS.md"], "OpenAI Codex / Amp / Gemini CLI"),
        (&["AGENT.md"], "Amp (legacy)"),
        (&["CLAUDE.md"], "Claude Code"),
        (&["CLAUDE.local.md"], "Claude Code (local override)"),
        (&[".cursorrules"], "Cursor (legacy)"),
        (&[".github", "copilot-instructions.md"], "GitHub Copilot"),
        (&[".junie", "guidelines.md"], "JetBrains Junie"),
        (&[".continuerules"], "Continue"),
        (&["GEMINI.md"], "Gemini CLI"),
        (&[".aider.conf.yml"], "Aider"),
        (&[".claude", "settings.json"], "Claude Code (settings)"),
        (&[".cursor", "rules"], "Cursor (rules dir)"),
    ];

    let mut found: Vec<(String, String)> = Vec::new();

    for (components, tool_name) in KNOWN_FILES {
        let path = components
            .iter()
            .fold(root.to_path_buf(), |acc, c| acc.join(c));
        if path.exists() {
            // Use the last component as the display filename
            let fname = components.last().copied().unwrap_or("").to_string();
            found.push((fname, tool_name.to_string()));
        }
    }

    // Also scan .cursor/rules/ for *.md files (Cursor new format)
    let cursor_rules_dir = root.join(".cursor").join("rules");
    if cursor_rules_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&cursor_rules_dir) {
            let count = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| ext == "md" || ext == "mdc")
                        .unwrap_or(false)
                })
                .count();
            if count > 0 {
                found.push((
                    format!(".cursor/rules/ ({count} rules)"),
                    "Cursor".to_string(),
                ));
            }
        }
    }

    ToolOutput {
        ai_context_files: if found.is_empty() { None } else { Some(found) },
        ..Default::default()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn parse_toml_str_basic() {
        assert_eq!(
            parse_toml_str(r#"name = "halcon-cli""#, "name"),
            Some("halcon-cli".to_string())
        );
    }

    #[test]
    fn parse_toml_str_no_space_eq() {
        assert_eq!(
            parse_toml_str(r#"version="0.3.0""#, "version"),
            Some("0.3.0".to_string())
        );
    }

    #[test]
    fn parse_toml_str_wrong_key_returns_none() {
        assert!(parse_toml_str(r#"name = "foo""#, "version").is_none());
    }

    #[test]
    fn extract_string_array_single_line() {
        let raw = r#"members = ["crates/a", "crates/b"]"#;
        let arr = extract_string_array(raw);
        assert_eq!(arr, vec!["crates/a", "crates/b"]);
    }

    #[test]
    fn extract_string_array_glob() {
        let raw = r#"members = ["crates/*"]"#;
        let arr = extract_string_array(raw);
        assert_eq!(arr, vec!["crates/*"]);
    }

    #[test]
    fn detect_rust_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]",
        )
        .unwrap();
        assert_eq!(detect_project_type(tmp.path()), "Rust Workspace");
    }

    #[test]
    fn detect_rust_plain() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]\nname=\"foo\"").unwrap();
        assert_eq!(detect_project_type(tmp.path()), "Rust");
    }

    #[test]
    fn detect_nodejs() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("package.json"), r#"{"name":"app"}"#).unwrap();
        assert_eq!(detect_project_type(tmp.path()), "Node.js");
    }

    #[test]
    fn find_project_root_walks_up() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        let nested = tmp.path().join("src").join("lib");
        std::fs::create_dir_all(&nested).unwrap();
        let found = find_project_root(&nested);
        assert_eq!(found.unwrap(), tmp.path().to_path_buf());
    }

    #[test]
    fn extract_json_str_basic() {
        let json = r#"{"name": "my-app", "version": "1.0.0"}"#;
        assert_eq!(extract_json_str(json, "name"), Some("my-app".to_string()));
        assert_eq!(extract_json_str(json, "version"), Some("1.0.0".to_string()));
    }

    #[test]
    fn detect_rust_stack_finds_tokio() {
        let toml = "tokio = { version = \"1\", features = [\"full\"] }\nserde = \"1\"";
        let stack = detect_rust_stack(toml);
        assert!(stack.iter().any(|s| s.contains("tokio")));
        assert!(stack.iter().any(|s| s.contains("serde")));
    }

    #[tokio::test]
    async fn filesystem_scanner_detects_readme() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("README.md"), "# Test").unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::create_dir_all(tmp.path().join("target")).unwrap(); // should be skipped
        let out = filesystem_scanner(tmp.path()).await;
        assert_eq!(out.has_readme, Some(true));
        let dirs = out.top_dirs.unwrap();
        assert!(dirs.contains(&"src".to_string()));
        assert!(!dirs.contains(&"target".to_string()));
    }

    #[tokio::test]
    async fn cicd_detector_finds_github_actions() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".github").join("workflows")).unwrap();
        let out = cicd_detector(tmp.path()).await;
        assert_eq!(out.has_ci, Some(true));
        assert_eq!(out.ci_system.as_deref(), Some("GitHub Actions"));
    }

    #[tokio::test]
    async fn cicd_detector_none() {
        let tmp = tempfile::tempdir().unwrap();
        let out = cicd_detector(tmp.path()).await;
        assert_eq!(out.has_ci, Some(false));
    }

    #[tokio::test]
    async fn docker_detector_finds_dockerfile() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Dockerfile"), "FROM ubuntu:22.04").unwrap();
        let out = docker_detector(tmp.path()).await;
        assert_eq!(out.has_docker, Some(true));
    }

    #[tokio::test]
    async fn security_scanner_detects_security_md() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("SECURITY.md"), "# Security").unwrap();
        let out = security_scanner(tmp.path()).await;
        assert_eq!(out.has_security_policy, Some(true));
    }

    #[tokio::test]
    async fn security_scanner_detects_mit_license() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("LICENSE"), "MIT License\n...").unwrap();
        let out = security_scanner(tmp.path()).await;
        assert_eq!(out.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn health_score_full_project() {
        let ctx = ProjectContext {
            has_ci: true,
            has_tests: true,
            test_coverage_est: Some(80),
            branch: Some("main".to_string()),
            commit_velocity_per_week: Some(5.0),
            bus_factor: Some(3),
            has_security_policy: true,
            has_audit_config: true,
            has_docker: true,
            architecture_style: Some("layered".to_string()),
            has_readme: true,
            license: Some("MIT".to_string()),
            complexity_score: Some(20),
            ..Default::default()
        };
        let (score, issues, _) = health_score_calculator(&ctx);
        assert!(
            score >= 80,
            "Well-configured project should score >= 80, got {score}"
        );
        assert!(
            issues.is_empty() || issues.len() <= 1,
            "Few issues expected, got: {issues:?}"
        );
    }

    #[test]
    fn health_score_empty_project() {
        let ctx = ProjectContext::default();
        let (score, issues, recs) = health_score_calculator(&ctx);
        assert!(score < 50, "Empty project should score < 50, got {score}");
        assert!(!issues.is_empty(), "Should have issues");
        assert!(!recs.is_empty(), "Should have recommendations");
    }

    #[test]
    fn tool_output_merge_into_overwrites_default() {
        let mut ctx = ProjectContext::default();
        let out = ToolOutput {
            package_name: Some("my-pkg".to_string()),
            has_ci: Some(true),
            ..Default::default()
        };
        out.merge_into(&mut ctx);
        assert_eq!(ctx.package_name, Some("my-pkg".to_string()));
        assert!(ctx.has_ci);
    }
}
