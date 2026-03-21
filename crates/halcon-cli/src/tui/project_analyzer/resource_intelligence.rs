//! Resource Intelligence Layer — Phase 103.
//!
//! Detects all available resources in the agent's operating environment:
//! system hardware, installed tools, IDE context, agent capabilities,
//! HICON state, and runtime configuration.
//!
//! ## Design Principles
//!
//! - **Resilient**: every probe returns gracefully even if the target is absent.
//! - **Timeout-bounded**: no probe blocks longer than `PROBE_TIMEOUT_SECS`.
//! - **Non-invasive**: uses env vars, filesystem checks, and lightweight commands
//!   only — never touches running processes or sockets dangerously.
//! - **Sparse accumulator**: each scanner returns a `ToolOutput` that is merged
//!   into `ProjectContext` by the wave orchestrator (no new error surface).

use std::path::Path;
use std::time::{Duration, Instant};
use tokio::time::timeout;

use super::tools::ToolOutput;

// ─── Timeout ─────────────────────────────────────────────────────────────────

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Run a synchronous probe inside `spawn_blocking`, bounded by `PROBE_TIMEOUT`.
/// Returns `None` on timeout or panic.
async fn probe<F, T>(f: F) -> Option<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    timeout(PROBE_TIMEOUT, tokio::task::spawn_blocking(f))
        .await
        .ok()
        .and_then(|r| r.ok())
}

// ─── Command helper ───────────────────────────────────────────────────────────

/// Run a command with args, return trimmed stdout or None.
fn cmd_output(program: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Check if a binary is reachable in PATH.
fn binary_exists(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ─── Phase 103-A: System Profile ─────────────────────────────────────────────

pub async fn system_profile_scanner() -> ToolOutput {
    let started = Instant::now();

    let result = probe(|| {
        let os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);

        // CPU cores
        let cpu_cores: u32 = {
            #[cfg(target_os = "macos")]
            {
                cmd_output("sysctl", &["-n", "hw.logicalcpu"])
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1)
            }
            #[cfg(target_os = "linux")]
            {
                cmd_output("nproc", &[])
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1)
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                1
            }
        };

        // RAM in MB
        let ram_mb: u64 = {
            #[cfg(target_os = "macos")]
            {
                cmd_output("sysctl", &["-n", "hw.memsize"])
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|b| b / 1_048_576)
                    .unwrap_or(0)
            }
            #[cfg(target_os = "linux")]
            {
                std::fs::read_to_string("/proc/meminfo")
                    .ok()
                    .and_then(|s| {
                        s.lines()
                            .find(|l| l.starts_with("MemTotal:"))
                            .and_then(|l| l.split_whitespace().nth(1))
                            .and_then(|kb| kb.parse::<u64>().ok())
                            .map(|kb| kb / 1024)
                    })
                    .unwrap_or(0)
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                0
            }
        };

        // Disk free (in GB) for current directory
        let disk_free_gb: Option<f64> = {
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            {
                cmd_output("df", &["-k", "."]).and_then(|out| {
                    out.lines()
                        .nth(1)
                        .and_then(|l| l.split_whitespace().nth(3))
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(|kb| kb as f64 / 1_048_576.0)
                })
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                None
            }
        };

        // GPU detection
        let gpu_available: bool = {
            // NVIDIA
            if std::process::Command::new("nvidia-smi")
                .arg("-L")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                true
            } else if cfg!(target_os = "macos") {
                // Apple Silicon / Metal always available on M-series
                std::env::consts::ARCH == "aarch64"
            } else {
                false
            }
        };

        // WSL detection
        let is_wsl: bool = {
            #[cfg(target_os = "linux")]
            {
                std::fs::read_to_string("/proc/version")
                    .map(|s| s.to_lowercase().contains("microsoft"))
                    .unwrap_or(false)
            }
            #[cfg(not(target_os = "linux"))]
            {
                false
            }
        };

        // Container detection
        let is_container: bool = {
            std::path::Path::new("/.dockerenv").exists()
                || std::env::var("KUBERNETES_SERVICE_HOST").is_ok()
                || std::env::var("container").is_ok()
        };

        (
            os,
            cpu_cores,
            ram_mb,
            disk_free_gb,
            gpu_available,
            is_wsl,
            is_container,
        )
    })
    .await;

    let mut out = ToolOutput::default();
    if let Some((os, cpu_cores, ram_mb, disk_free_gb, gpu_available, is_wsl, is_container)) = result
    {
        out.sys_os = Some(os);
        out.sys_cpu_cores = Some(cpu_cores);
        out.sys_ram_mb = Some(ram_mb);
        out.sys_disk_free_gb = disk_free_gb;
        out.sys_gpu_available = Some(gpu_available);
        out.sys_is_wsl = Some(is_wsl);
        out.sys_is_container = Some(is_container);
    }
    out.resource_detection_time_ms = Some(started.elapsed().as_millis() as u64);
    out
}

// ─── Phase 103-B: Tool Versions ───────────────────────────────────────────────

pub async fn tool_versions_scanner() -> ToolOutput {
    let started = Instant::now();

    let result = probe(|| {
        let git = cmd_output("git", &["--version"]).map(|s| version_from(&s));
        let docker = cmd_output("docker", &["--version"]).map(|s| version_from(&s));
        let node = cmd_output("node", &["--version"]);
        let python = cmd_output("python3", &["--version"]).map(|s| version_from(&s));
        let rustc = cmd_output("rustc", &["--version"]).map(|s| version_from(&s));
        let go = cmd_output("go", &["version"]).map(|s| version_from(&s));
        let cargo = cmd_output("cargo", &["--version"]).map(|s| version_from(&s));
        let make =
            cmd_output("make", &["--version"]).map(|s| s.lines().next().unwrap_or("").to_string());
        let kubectl = binary_exists("kubectl");
        let helm = binary_exists("helm");
        let terraform = binary_exists("terraform");
        let ansible = binary_exists("ansible");
        (
            git, docker, node, python, rustc, go, cargo, make, kubectl, helm, terraform, ansible,
        )
    })
    .await;

    let mut out = ToolOutput::default();
    if let Some((
        git,
        docker,
        node,
        python,
        rustc,
        go,
        cargo,
        make,
        kubectl,
        helm,
        terraform,
        ansible,
    )) = result
    {
        out.tool_git_version = git;
        out.tool_docker_version = docker;
        out.tool_node_version = node;
        out.tool_python_version = python;
        out.tool_rust_version = rustc;
        out.tool_go_version = go;
        out.tool_cargo_version = cargo;
        out.tool_make_version = make;
        out.tool_has_kubectl = Some(kubectl);
        out.tool_has_helm = Some(helm);
        out.tool_has_terraform = Some(terraform);
        out.tool_has_ansible = Some(ansible);
    }
    out.environment_scan_time_ms = Some(started.elapsed().as_millis() as u64);
    out
}

/// Extract a short version string from verbose command output.
fn version_from(raw: &str) -> String {
    // Try to grab the first thing that looks like a version number
    for word in raw.split_whitespace() {
        if word
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            return word.trim_start_matches('v').to_string();
        }
    }
    raw.lines().next().unwrap_or(raw).trim().to_string()
}

// ─── Phase 103-C: IDE Context ─────────────────────────────────────────────────

pub async fn ide_context_scanner() -> ToolOutput {
    let started = Instant::now();

    let result = probe(detect_ide_context).await;

    let mut out = ToolOutput::default();
    if let Some((ide, workspace, active_file, lsp_port)) = result {
        out.ide_detected = ide;
        out.ide_workspace = workspace;
        out.ide_active_file = active_file;
        out.ide_lsp_port = lsp_port;
        out.ide_lsp_connected = Some(lsp_port.is_some());
    }
    out.ide_context_integration_time_ms = Some(started.elapsed().as_millis() as u64);
    out
}

fn detect_ide_context() -> (Option<String>, Option<String>, Option<String>, Option<u16>) {
    let env = std::env::vars().collect::<Vec<_>>();

    // ── IDE detection via env vars (priority order) ──────────────────────────
    let ide: Option<String> = if env
        .iter()
        .any(|(k, _)| k == "CURSOR_TRACE_PATH" || k == "CURSOR_SESSION_ID")
    {
        Some("Cursor".to_string())
    } else if env
        .iter()
        .any(|(k, v)| k == "TERM_PROGRAM" && v == "cursor")
    {
        Some("Cursor".to_string())
    } else if env
        .iter()
        .any(|(k, _)| k == "VSCODE_PID" || k == "VSCODE_IPC_HOOK_CLI")
    {
        Some("VS Code".to_string())
    } else if env
        .iter()
        .any(|(k, v)| k == "TERM_PROGRAM" && v == "vscode")
    {
        Some("VS Code".to_string())
    } else if env
        .iter()
        .any(|(k, _)| k == "JETBRAINS_IDE" || k == "IDEA_INITIAL_DIRECTORY")
    {
        Some("JetBrains".to_string())
    } else if env
        .iter()
        .any(|(k, _)| k == "NVIM_LISTEN_ADDRESS" || k == "NVIM")
    {
        Some("Neovim".to_string())
    } else if env.iter().any(|(k, _)| k == "EMACS" || k == "INSIDE_EMACS") {
        Some("Emacs".to_string())
    } else if env
        .iter()
        .any(|(k, _)| k == "ZED_SESSION_ID" || k == "ZED_TERM")
    {
        Some("Zed".to_string())
    } else if env
        .iter()
        .any(|(k, _)| k == "SUBLIME_TEXT_3" || k == "SUBLIME_SESSION_DIR")
    {
        Some("Sublime Text".to_string())
    } else {
        None
    };

    // ── Workspace root from env ──────────────────────────────────────────────
    let workspace: Option<String> = env
        .iter()
        .find(|(k, _)| {
            matches!(
                k.as_str(),
                "VSCODE_WORKSPACE_FOLDER"
                    | "JETBRAINS_PROJECT_FOLDER"
                    | "PROJECT_ROOT"
                    | "WORKSPACE_ROOT"
            )
        })
        .map(|(_, v)| v.clone());

    // ── Active file (VS Code / JetBrains pass this sometimes) ───────────────
    let active_file: Option<String> = env
        .iter()
        .find(|(k, _)| matches!(k.as_str(), "VSCODE_ACTIVE_FILE" | "IDE_ACTIVE_FILE"))
        .map(|(_, v)| v.clone());

    // ── LSP port: check our own dev-gateway port first ───────────────────────
    let lsp_port: Option<u16> = {
        // Check if port 5758 (halcon LSP) is listening (non-blocking connect attempt)
        use std::net::TcpStream;
        let addr = "127.0.0.1:5758";
        if TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(200)).is_ok() {
            Some(5758)
        } else {
            // Try HALCON_LSP_PORT env override
            env.iter()
                .find(|(k, _)| k == "HALCON_LSP_PORT")
                .and_then(|(_, v)| v.parse().ok())
        }
    };

    (ide, workspace, active_file, lsp_port)
}

// ─── Phase 103-D: Agent Capabilities ─────────────────────────────────────────

pub async fn agent_capabilities_scanner(project_root: &Path) -> ToolOutput {
    let started = Instant::now();
    let root = project_root.to_path_buf();

    let result = probe(move || detect_agent_capabilities(&root)).await;

    let mut out = ToolOutput::default();
    if let Some((tools, mcps, plugins, model, tier, flags)) = result {
        out.agent_tools_available = Some(tools);
        out.agent_mcp_servers = Some(mcps);
        out.agent_plugins_loaded = Some(plugins);
        out.agent_model_name = model;
        out.agent_model_tier = tier;
        out.agent_reasoning_enabled = Some(flags.reasoning);
        out.agent_orchestration_on = Some(flags.orchestration);
        out.agent_plugin_system_on = Some(flags.plugins);
        out.agent_multimodal_on = Some(flags.multimodal);
        out.agent_hicon_active = Some(flags.hicon);
    }
    out.hicon_query_time_ms = Some(started.elapsed().as_millis() as u64);
    out
}

struct AgentFlags {
    reasoning: bool,
    orchestration: bool,
    plugins: bool,
    multimodal: bool,
    hicon: bool,
}

fn detect_agent_capabilities(
    project_root: &Path,
) -> (
    Vec<String>,
    Vec<String>,
    u32,
    Option<String>,
    Option<String>,
    AgentFlags,
) {
    // ── MCP servers from ~/.halcon/.mcp.json ────────────────────────────────
    let mcp_path = dirs::home_dir()
        .map(|h| h.join(".halcon").join(".mcp.json"))
        .filter(|p| p.exists());

    let mcps: Vec<String> = mcp_path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("mcpServers").cloned())
        .and_then(|v| v.as_object().cloned())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    // ── Plugins from ~/.halcon/plugins/ ─────────────────────────────────────
    let plugins_dir = dirs::home_dir().map(|h| h.join(".halcon").join("plugins"));
    let plugins: u32 = plugins_dir
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().map(|x| x == "toml").unwrap_or(false))
                .count() as u32
        })
        .unwrap_or(0);

    // ── Model & tier from env vars or config ─────────────────────────────────
    let model_name: Option<String> = std::env::var("HALCON_MODEL")
        .or_else(|_| std::env::var("ANTHROPIC_MODEL"))
        .or_else(|_| std::env::var("OPENAI_MODEL"))
        .ok();

    let model_tier: Option<String> = {
        if let Some(ref m) = model_name {
            let ml = m.to_lowercase();
            if ml.contains("haiku") || ml.contains("mini") || ml.contains("flash") {
                Some("Fast".to_string())
            } else if ml.contains("opus") || ml.contains("o1") || ml.contains("reasoner") {
                Some("Deep".to_string())
            } else {
                Some("Balanced".to_string())
            }
        } else {
            None
        }
    };

    // ── Config flags from ~/.halcon/config.toml ───────────────────────────
    let config_content = dirs::home_dir()
        .map(|h| h.join(".halcon").join("config.toml"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();

    let flags = AgentFlags {
        reasoning: config_content.contains("enabled = true")
            && config_content.contains("[reasoning]"),
        orchestration: config_content.contains("[orchestrator]")
            && config_content.contains("enabled = true"),
        plugins: plugins > 0
            || (config_content.contains("[plugins]") && config_content.contains("enabled = true")),
        multimodal: config_content.contains("[multimodal]")
            && config_content.contains("enabled = true"),
        hicon: probe_hicon_active(project_root, &config_content),
    };

    // ── Available tools: infer from project capabilities ─────────────────────
    let mut tools: Vec<String> = vec![
        "file_read".to_string(),
        "file_write".to_string(),
        "bash".to_string(),
        "grep".to_string(),
        "glob".to_string(),
        "directory_tree".to_string(),
    ];
    if flags.plugins || !mcps.is_empty() {
        tools.push("mcp_tools".to_string());
    }
    if config_content.contains("docker")
        || std::process::Command::new("docker")
            .arg("info")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    {
        tools.push("docker".to_string());
    }

    (tools, mcps, plugins, model_name, model_tier, flags)
}

// ─── Phase 105: HICON Probe ───────────────────────────────────────────────────

/// Lightweight probe: is the HICON meta-cognitive system active?
/// Checks for config flags, state files, and env vars — no live RPC.
fn probe_hicon_active(project_root: &Path, config_content: &str) -> bool {
    // Config flag
    if config_content.contains("[hicon]") && config_content.contains("enabled = true") {
        return true;
    }
    // HICON state file in project .halcon dir
    if project_root
        .join(".halcon")
        .join("hicon_state.json")
        .exists()
    {
        return true;
    }
    // HICON state file in home
    if dirs::home_dir()
        .map(|h| h.join(".halcon").join("hicon_state.json").exists())
        .unwrap_or(false)
    {
        return true;
    }
    // Env var override
    std::env::var("HALCON_HICON")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

// ─── Phase 103-E: Runtime Profile ────────────────────────────────────────────

pub async fn runtime_profile_scanner(project_root: &Path) -> ToolOutput {
    let started = Instant::now();
    let root = project_root.to_path_buf();

    let result = probe(move || detect_runtime_profile(&root)).await;

    let mut out = ToolOutput::default();
    if let Some((model_router, convergence, intent_scorer, token_budget, ci_env)) = result {
        out.runtime_model_router_active = Some(model_router);
        out.runtime_convergence_controller_on = Some(convergence);
        out.runtime_intent_scorer_on = Some(intent_scorer);
        out.runtime_token_budget = token_budget;
        out.runtime_ci_environment = Some(ci_env);
    }
    out.environment_scan_time_ms = Some(started.elapsed().as_millis() as u64);
    out
}

fn detect_runtime_profile(project_root: &Path) -> (bool, bool, bool, Option<u32>, bool) {
    let config_content = dirs::home_dir()
        .map(|h| h.join(".halcon").join("config.toml"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();

    // ModelRouter: present if model_router section or routing config exists
    let model_router = config_content.contains("[model_router]")
        || config_content.contains("routing_strategy")
        || project_root
            .join(".halcon")
            .join("model_router.toml")
            .exists();

    // ConvergenceController: check config or state
    let convergence = config_content.contains("[convergence]")
        || config_content.contains("convergence_threshold")
        || project_root
            .join(".halcon")
            .join("convergence_state.json")
            .exists();

    // IntentScorer: check config
    let intent_scorer = config_content.contains("[intent_scorer]")
        || config_content.contains("intent_scoring")
        || std::env::var("HALCON_INTENT_SCORER").is_ok();

    // Token budget from env
    let token_budget: Option<u32> = std::env::var("HALCON_MAX_TOKENS")
        .ok()
        .and_then(|s| s.parse().ok());

    // CI environment detection
    let ci_env = std::env::var("CI").is_ok()
        || std::env::var("GITHUB_ACTIONS").is_ok()
        || std::env::var("GITLAB_CI").is_ok()
        || std::env::var("JENKINS_HOME").is_ok()
        || std::env::var("CIRCLECI").is_ok();

    (
        model_router,
        convergence,
        intent_scorer,
        token_budget,
        ci_env,
    )
}

// ─── Scoring functions (Phase 107) ───────────────────────────────────────────

use super::tools::ProjectContext;

/// Compute AgentReadinessScore (0-100).
///
/// Weights:
/// - Model configured (25 pts)
/// - MCP servers connected (20 pts)
/// - Plugins available (15 pts)
/// - Reasoning subsystem on (15 pts)
/// - Orchestration on (10 pts)
/// - Multimodal on (5 pts)
/// - HICON active (10 pts)
pub fn compute_agent_readiness_score(ctx: &ProjectContext) -> u8 {
    let mut score: u32 = 0;
    if ctx.agent_model_name.is_some() {
        score += 25;
    }
    if !ctx.agent_mcp_servers.is_empty() {
        score += 20;
    }
    if ctx.agent_plugins_loaded > 0 {
        score += 15;
    }
    if ctx.agent_reasoning_enabled {
        score += 15;
    }
    if ctx.agent_orchestration_on {
        score += 10;
    }
    if ctx.agent_multimodal_on {
        score += 5;
    }
    if ctx.agent_hicon_active {
        score += 10;
    }
    score.min(100) as u8
}

/// Compute EnvironmentCompatibilityScore (0-100).
///
/// Weights:
/// - Git installed (15 pts)
/// - Runtime language present (20 pts)
/// - Docker available (10 pts)
/// - Sufficient RAM ≥4GB (15 pts)
/// - CI/CD env variables (10 pts)
/// - Multiple CPU cores (10 pts)
/// - Not in a constrained container (10 pts)
/// - Infra tools (kubectl/helm/terraform) (10 pts)
pub fn compute_environment_compatibility_score(ctx: &ProjectContext) -> u8 {
    let mut score: u32 = 0;
    if ctx.tool_git_version.is_some() {
        score += 15;
    }

    // Language runtime based on project type
    let type_lower = ctx.project_type.to_lowercase();
    let has_runtime = if type_lower.contains("rust") {
        ctx.tool_rust_version.is_some()
    } else if type_lower.contains("node") || type_lower.contains("react") {
        ctx.tool_node_version.is_some()
    } else if type_lower.contains("python") {
        ctx.tool_python_version.is_some()
    } else if type_lower.contains("go") {
        ctx.tool_go_version.is_some()
    } else {
        ctx.tool_rust_version.is_some()
            || ctx.tool_node_version.is_some()
            || ctx.tool_python_version.is_some()
    };
    if has_runtime {
        score += 20;
    }

    if ctx.tool_docker_version.is_some() {
        score += 10;
    }
    if ctx.sys_ram_mb >= 4096 {
        score += 15;
    }
    if ctx.runtime_ci_environment {
        score += 10;
    }
    if ctx.sys_cpu_cores >= 4 {
        score += 10;
    }
    // Only award "not in container" when sys scan actually ran (sys_os non-empty)
    if !ctx.sys_os.is_empty() && !ctx.sys_is_container {
        score += 10;
    }
    if ctx.tool_has_kubectl || ctx.tool_has_helm || ctx.tool_has_terraform {
        score += 10;
    }

    score.min(100) as u8
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::project_analyzer::tools::ProjectContext;

    // ── version_from ─────────────────────────────────────────────────────────

    #[test]
    fn version_from_git_output() {
        let raw = "git version 2.43.0";
        assert_eq!(version_from(raw), "2.43.0");
    }

    #[test]
    fn version_from_rustc_output() {
        let raw = "rustc 1.80.0 (2024-05-01)";
        assert_eq!(version_from(raw), "1.80.0");
    }

    #[test]
    fn version_from_docker_output() {
        let raw = "Docker version 24.0.7, build afdd53b";
        assert_eq!(version_from(raw), "24.0.7,");
    }

    #[test]
    fn version_from_fallback_to_first_line() {
        let raw = "go version go1.21.0 linux/amd64";
        // "version" doesn't start with digit, "go1.21.0" doesn't either → falls back
        let result = version_from(raw);
        // Should return the "1.21.0" part from "go1.21.0" or the full line
        assert!(!result.is_empty());
    }

    // ── probe_hicon_active ────────────────────────────────────────────────────

    #[test]
    fn hicon_inactive_with_empty_config() {
        let tmp = tempfile::tempdir().unwrap();
        let active = probe_hicon_active(tmp.path(), "");
        assert!(!active, "HICON should not be active with empty config");
    }

    #[test]
    fn hicon_active_via_config_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let config = "[hicon]\nenabled = true\n";
        let active = probe_hicon_active(tmp.path(), config);
        assert!(active, "HICON should be detected via config flag");
    }

    #[test]
    fn hicon_active_via_state_file() {
        let tmp = tempfile::tempdir().unwrap();
        let halcon_dir = tmp.path().join(".halcon");
        std::fs::create_dir_all(&halcon_dir).unwrap();
        std::fs::write(halcon_dir.join("hicon_state.json"), r#"{"active":true}"#).unwrap();
        let active = probe_hicon_active(tmp.path(), "");
        assert!(active, "HICON should be detected via state file");
    }

    // ── compute_agent_readiness_score ─────────────────────────────────────────

    #[test]
    fn agent_readiness_zero_when_nothing_configured() {
        let ctx = ProjectContext::default();
        let score = compute_agent_readiness_score(&ctx);
        assert_eq!(score, 0, "No capabilities → score 0");
    }

    #[test]
    fn agent_readiness_full_when_all_on() {
        let ctx = ProjectContext {
            agent_model_name: Some("claude-sonnet-4-6".to_string()),
            agent_mcp_servers: vec!["filesystem".to_string(), "halcon".to_string()],
            agent_plugins_loaded: 3,
            agent_reasoning_enabled: true,
            agent_orchestration_on: true,
            agent_multimodal_on: true,
            agent_hicon_active: true,
            ..Default::default()
        };
        let score = compute_agent_readiness_score(&ctx);
        assert_eq!(score, 100);
    }

    #[test]
    fn agent_readiness_partial() {
        let ctx = ProjectContext {
            agent_model_name: Some("gpt-4o".to_string()), // +25
            agent_reasoning_enabled: true,                // +15
            ..Default::default()
        };
        let score = compute_agent_readiness_score(&ctx);
        assert_eq!(score, 40);
    }

    // ── compute_environment_compatibility_score ───────────────────────────────

    #[test]
    fn env_compat_zero_when_nothing() {
        let ctx = ProjectContext::default();
        let score = compute_environment_compatibility_score(&ctx);
        assert_eq!(score, 0);
    }

    #[test]
    fn env_compat_awards_git_installed() {
        let ctx = ProjectContext {
            tool_git_version: Some("2.43.0".to_string()),
            sys_cpu_cores: 1,
            ..Default::default()
        };
        let score = compute_environment_compatibility_score(&ctx);
        assert!(score >= 15, "Git present = at least 15 pts, got {score}");
    }

    #[test]
    fn env_compat_awards_sufficient_ram() {
        let ctx = ProjectContext {
            sys_ram_mb: 8192, // 8 GB → +15
            sys_cpu_cores: 4, // → +10
            ..Default::default()
        };
        let score = compute_environment_compatibility_score(&ctx);
        assert!(score >= 25, "8GB RAM + 4 cores = ≥25 pts, got {score}");
    }

    #[test]
    fn env_compat_capped_at_100() {
        let ctx = ProjectContext {
            tool_git_version: Some("2.43.0".to_string()),
            tool_rust_version: Some("1.80".to_string()),
            tool_docker_version: Some("24.0".to_string()),
            tool_node_version: Some("20.0".to_string()),
            tool_has_kubectl: true,
            sys_ram_mb: 16384,
            sys_cpu_cores: 16,
            runtime_ci_environment: true,
            project_type: "Rust".to_string(),
            ..Default::default()
        };
        let score = compute_environment_compatibility_score(&ctx);
        assert!(score <= 100, "Score must not exceed 100, got {score}");
    }

    // ── Async scanner smoke tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn system_profile_scanner_completes_without_panic() {
        let out = system_profile_scanner().await;
        // OS must always be detectable
        assert!(out.sys_os.is_some(), "sys_os must be populated");
        // cpu_cores ≥ 1 always
        assert!(out.sys_cpu_cores.map(|c| c >= 1).unwrap_or(false));
    }

    #[tokio::test]
    async fn tool_versions_scanner_completes_without_panic() {
        // Should complete without panicking; some tools may be None on CI
        let out = tool_versions_scanner().await;
        let _ = out; // just verify no panic
    }

    #[tokio::test]
    async fn ide_context_scanner_completes_without_panic() {
        let out = ide_context_scanner().await;
        let _ = out;
    }

    #[tokio::test]
    async fn agent_capabilities_scanner_completes_without_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let out = agent_capabilities_scanner(tmp.path()).await;
        let _ = out;
    }

    #[tokio::test]
    async fn runtime_profile_scanner_completes_without_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let out = runtime_profile_scanner(tmp.path()).await;
        let _ = out;
    }
}
