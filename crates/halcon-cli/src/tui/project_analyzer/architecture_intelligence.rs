//! Architecture Intelligence Scanner — Phases 113-119
//!
//! Detects distributed-system patterns, computes Phase 117 composite scores,
//! and produces Phase 119 auto-mode suggestions.
//!
//! All detection uses only file *presence* and simple string-contains checks
//! on small config files — never AST parsing or heavyweight analysis.

use std::{collections::HashSet, fs, path::Path};

use super::tools::{ProjectContext, ToolOutput};

// ─── Architecture pattern detection ──────────────────────────────────────────

/// Signals that suggest a microservices architecture.
fn detect_microservices(root: &Path) -> bool {
    // Multiple Dockerfiles in sub-directories
    let mut dockerfile_dirs = 0u32;
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("Dockerfile").exists() {
                dockerfile_dirs += 1;
            }
        }
    }
    if dockerfile_dirs >= 2 {
        return true;
    }

    // docker-compose with 3+ services hints at microservices
    let compose_content = read_compose(root);
    if let Some(c) = &compose_content {
        let service_count = c.matches("image:").count() + c.matches("build:").count();
        if service_count >= 3 {
            return true;
        }
    }

    false
}

fn detect_event_driven(root: &Path) -> bool {
    let compose = read_compose(root);
    if let Some(c) = &compose {
        for kw in &["kafka", "rabbitmq", "nats", "pulsar", "redis", "activemq"] {
            if c.to_lowercase().contains(kw) {
                return true;
            }
        }
    }
    // Check for event-related directory names
    for name in &[
        "events",
        "messages",
        "queue",
        "consumers",
        "producers",
        "handlers",
    ] {
        if root.join(name).is_dir() {
            return true;
        }
    }
    false
}

fn detect_cqrs(root: &Path) -> bool {
    root.join("commands").is_dir() && root.join("queries").is_dir()
}

fn detect_ddd(root: &Path) -> bool {
    let domain = root.join("domain").is_dir() || root.join("src/domain").is_dir();
    let app = root.join("application").is_dir() || root.join("src/application").is_dir();
    let infra = root.join("infrastructure").is_dir() || root.join("src/infrastructure").is_dir();
    domain && (app || infra)
}

fn detect_hexagonal(root: &Path) -> bool {
    (root.join("ports").is_dir() && root.join("adapters").is_dir())
        || (root.join("src/ports").is_dir() && root.join("src/adapters").is_dir())
}

fn detect_message_broker(root: &Path) -> Option<String> {
    let compose = read_compose(root);
    if let Some(c) = &compose {
        let c_low = c.to_lowercase();
        for (kw, name) in &[
            ("kafka", "Kafka"),
            ("rabbitmq", "RabbitMQ"),
            ("nats", "NATS"),
            ("pulsar", "Pulsar"),
            ("activemq", "ActiveMQ"),
        ] {
            if c_low.contains(kw) {
                return Some(name.to_string());
            }
        }
    }
    None
}

fn detect_service_mesh(root: &Path) -> bool {
    let mesh_files = [
        ".istio",
        "istio.yaml",
        "istio-config.yaml",
        "linkerd.yml",
        "linkerd-config.yaml",
        "consul-mesh.hcl",
        "kuma-mesh.yaml",
    ];
    mesh_files.iter().any(|f| root.join(f).exists())
}

fn detect_observability_stack(root: &Path) -> bool {
    let obs_markers = [
        "prometheus.yml",
        "prometheus.yaml",
        "grafana", // directory
        "jaeger-config.yml",
        "otel-collector-config.yaml",
        ".otel",
        "opentelemetry-config.yaml",
        "datadog.yaml",
        "alertmanager.yml",
    ];
    obs_markers.iter().any(|f| {
        let p = root.join(f);
        p.exists()
    })
}

fn detect_api_gateway(root: &Path) -> bool {
    let gw_markers = [
        "nginx.conf",
        "nginx.yaml",
        "traefik.toml",
        "traefik.yaml",
        "kong.yml",
        "kong.yaml",
        "gateway", // directory named "gateway"
        "api-gateway",
    ];
    gw_markers.iter().any(|f| root.join(f).exists())
}

fn count_distributed_services(root: &Path) -> u32 {
    // Count Dockerfiles in sub-directories
    let mut count = 0u32;
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("Dockerfile").exists() {
                count += 1;
            }
        }
    }
    // Also parse docker-compose for distinct service blocks
    if let Some(c) = read_compose(root) {
        let svc_count = (c.matches("image:").count() + c.matches("build:").count()) as u32;
        count = count.max(svc_count);
    }
    count
}

// ─── Helper: read docker-compose ──────────────────────────────────────────────

fn read_compose(root: &Path) -> Option<String> {
    for name in &[
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ] {
        let p = root.join(name);
        if p.exists() {
            return fs::read_to_string(p).ok();
        }
    }
    None
}

// ─── Phase 117: Composite Scoring Functions ───────────────────────────────────

/// Architecture quality: rewards clean separation-of-concerns patterns.
pub fn compute_architecture_quality_score(ctx: &ProjectContext) -> u8 {
    let mut score: i32 = 50; // Neutral baseline

    // Explicit architecture patterns are positive signals
    let patterns = &ctx.architecture_patterns;
    if patterns.contains(&"DDD".to_string()) {
        score += 15;
    }
    if patterns.contains(&"Hexagonal".to_string()) {
        score += 15;
    }
    if patterns.contains(&"CQRS".to_string()) {
        score += 10;
    }
    if patterns.contains(&"Microservices".to_string()) {
        score += 5;
    }

    // Clean project structure indicators
    if ctx.has_tests {
        score += 10;
    }
    if ctx.has_security_policy {
        score += 5;
    }
    if ctx.has_audit_config {
        score += 5;
    }
    if ctx.has_ci {
        score += 5;
    }

    // Circular deps are a negative
    if ctx.has_circular_deps {
        score -= 20;
    }

    // Complexity penalty
    if let Some(c) = ctx.complexity_score {
        if c > 80 {
            score -= 10;
        }
    }

    score.clamp(0, 100) as u8
}

/// Scalability score: rewards horizontal-scaling readiness.
pub fn compute_scalability_score(ctx: &ProjectContext) -> u8 {
    let mut score: i32 = 40;

    if ctx.has_docker {
        score += 20;
    }
    if ctx.has_message_broker {
        score += 15;
    }
    if ctx.has_service_mesh {
        score += 10;
    }
    if ctx.has_api_gateway {
        score += 10;
    }
    if ctx.has_observability_stack {
        score += 5;
    }
    if ctx.distributed_services_count >= 3 {
        score += 10;
    }

    // K8s / Terraform readiness
    if ctx.tool_has_kubectl {
        score += 5;
    }
    if ctx.tool_has_terraform {
        score += 5;
    }
    if ctx.tool_has_helm {
        score += 5;
    }

    // Large projects that lack Docker are penalised
    if !ctx.has_docker && ctx.project_scale == "Large" {
        score -= 10;
    }
    if !ctx.has_docker && ctx.project_scale == "Enterprise" {
        score -= 15;
    }

    score.clamp(0, 100) as u8
}

/// Maintainability score: rewards test coverage, docs, CI, linting.
pub fn compute_maintainability_score(ctx: &ProjectContext) -> u8 {
    let mut score: i32 = 40;

    if ctx.has_tests {
        score += 20;
    }
    if ctx.has_ci {
        score += 15;
    }
    if ctx.has_readme {
        score += 10;
    }
    if ctx.has_security_policy {
        score += 5;
    }
    if ctx.has_audit_config {
        score += 5;
    }

    // Coverage estimate bonus
    if let Some(cov) = ctx.test_coverage_est {
        if cov >= 80 {
            score += 15;
        } else if cov >= 50 {
            score += 8;
        } else if cov >= 20 {
            score += 3;
        }
    }

    // Bus-factor penalty
    if let Some(bf) = ctx.bus_factor {
        if bf == 1 {
            score -= 10;
        }
    }

    // Circular deps hurt maintainability
    if ctx.has_circular_deps {
        score -= 15;
    }

    score.clamp(0, 100) as u8
}

/// Technical debt score: HIGHER = MORE DEBT (inverse, 0 = clean).
pub fn compute_technical_debt_score(ctx: &ProjectContext) -> u8 {
    let mut debt: i32 = 0;

    if !ctx.has_tests {
        debt += 20;
    }
    if !ctx.has_ci {
        debt += 15;
    }
    if ctx.has_circular_deps {
        debt += 25;
    }
    if !ctx.has_security_policy {
        debt += 10;
    }
    if !ctx.has_readme {
        debt += 5;
    }

    if let Some(cov) = ctx.test_coverage_est {
        if cov < 20 {
            debt += 15;
        } else if cov < 50 {
            debt += 8;
        }
    }

    if let Some(c) = ctx.complexity_score {
        if c > 80 {
            debt += 15;
        } else if c > 60 {
            debt += 8;
        }
    }

    if let Some(bf) = ctx.bus_factor {
        if bf == 1 {
            debt += 10;
        }
    }

    debt.clamp(0, 100) as u8
}

/// Developer Experience score.
pub fn compute_dev_ex_score(ctx: &ProjectContext) -> u8 {
    let mut score: i32 = 40;

    if ctx.has_ci {
        score += 15;
    }
    if ctx.has_docker {
        score += 10;
    }
    if ctx.has_readme {
        score += 10;
    }
    if ctx.has_tests {
        score += 10;
    }

    // Good version control signals
    if ctx.branch.is_some() {
        score += 5;
    }
    if ctx
        .commit_velocity_per_week
        .map(|v| v > 1.0)
        .unwrap_or(false)
    {
        score += 5;
    }

    // IDE integration
    if ctx.ide_detected.is_some() {
        score += 5;
    }
    if ctx.ide_lsp_connected {
        score += 5;
    }
    if ctx.agent_orchestration_on {
        score += 5;
    }

    // Bus factor risk hurts DX
    if let Some(bf) = ctx.bus_factor {
        if bf == 1 {
            score -= 10;
        }
    }

    score.clamp(0, 100) as u8
}

/// AI Readiness: how well-positioned the project is for AI agent augmentation.
pub fn compute_ai_readiness_score(ctx: &ProjectContext) -> u8 {
    let mut score: i32 = 30;

    // Agent capabilities
    if ctx.agent_reasoning_enabled {
        score += 15;
    }
    if ctx.agent_orchestration_on {
        score += 10;
    }
    if ctx.agent_multimodal_on {
        score += 5;
    }
    if ctx.agent_plugin_system_on {
        score += 5;
    }
    if ctx.agent_hicon_active {
        score += 5;
    }
    if !ctx.agent_mcp_servers.is_empty() {
        score += 5;
    }
    if ctx.agent_plugins_loaded > 0 {
        score += 5;
    }

    // Good project hygiene aids AI understanding
    if ctx.has_tests {
        score += 5;
    }
    if ctx.has_readme {
        score += 5;
    }
    if ctx.has_ci {
        score += 5;
    }

    // Complexity penalty
    if let Some(c) = ctx.complexity_score {
        if c > 80 {
            score -= 10;
        }
    }

    // Data/AI framework is a positive signal
    if ctx.data_framework.is_some() {
        score += 5;
    }

    score.clamp(0, 100) as u8
}

/// Distributed Maturity: readiness for production distributed system operation.
pub fn compute_distributed_maturity_score(ctx: &ProjectContext) -> u8 {
    let mut score: i32 = 10;

    if ctx.has_docker {
        score += 15;
    }
    if ctx.has_message_broker {
        score += 15;
    }
    if ctx.has_service_mesh {
        score += 15;
    }
    if ctx.has_observability_stack {
        score += 20;
    }
    if ctx.has_api_gateway {
        score += 10;
    }
    if ctx.has_ci {
        score += 10;
    }
    if ctx.has_security_policy {
        score += 5;
    }

    // K8s tooling
    if ctx.tool_has_kubectl {
        score += 5;
    }
    if ctx.tool_has_helm {
        score += 5;
    }

    // Scale matters
    if ctx.project_scale == "Enterprise" {
        score += 10;
    } else if ctx.project_scale == "Large" {
        score += 5;
    }

    score.clamp(0, 100) as u8
}

// ─── Phase 119: Auto-Mode Suggestion ─────────────────────────────────────────

/// Suggested agent configuration for this project.
#[derive(Debug, Clone)]
pub struct AgentSuggestion {
    pub model_tier: Option<String>,
    pub agent_flags: Vec<String>,
    pub planning_strategy: Option<String>,
    pub activate_reasoning_deep: bool,
    pub activate_multimodal: bool,
    pub use_fast_mode: bool,
    pub rationale: String,
}

pub fn suggest_agent_configuration(ctx: &ProjectContext) -> AgentSuggestion {
    let mut flags: Vec<String> = Vec::new();
    let mut reasons: Vec<&str> = Vec::new();

    // Enterprise / distributed → full expert mode
    let is_enterprise = ctx.project_scale == "Enterprise" || ctx.project_scale == "Large";
    let is_distributed = ctx.distributed_services_count >= 3 || ctx.has_service_mesh;
    let is_complex = ctx.architecture_patterns.len() >= 2;
    // Large monorepo (5+ sub-projects) is inherently complex even at Medium scale
    let is_complex_monorepo = ctx.is_monorepo && ctx.sub_project_count >= 5;
    // Polyglot with 2+ secondary languages adds significant coordination overhead
    let is_polyglot_complex = ctx.is_polyglot && ctx.secondary_languages.len() >= 2;

    if is_enterprise || is_distributed || is_complex || is_complex_monorepo || is_polyglot_complex {
        flags.push("--full".to_string());
        flags.push("--expert".to_string());
        if is_enterprise {
            reasons.push("enterprise scale project");
        }
        if is_distributed {
            reasons.push("distributed architecture detected");
        }
        if is_complex {
            reasons.push("complex architecture patterns");
        }
        if is_complex_monorepo {
            reasons.push("large monorepo with many sub-projects");
        }
        if is_polyglot_complex {
            reasons.push("polyglot repository");
        }
    }

    // Deep reasoning for architectural complexity
    let activate_reasoning_deep = is_enterprise
        || is_distributed
        || is_complex
        || is_complex_monorepo
        || ctx.architecture_quality_score < 50;

    // Multimodal for data/AI or if images might be relevant
    let activate_multimodal = ctx.data_framework.is_some()
        || ctx.primary_language == "Python"
        || ctx.primary_language == "R"
        || ctx.primary_language == "Julia";
    if activate_multimodal {
        reasons.push("Data/AI project — multimodal analysis useful");
    }

    // Fast mode for small/simple projects
    let use_fast_mode = ctx.project_scale == "Small"
        && ctx.architecture_patterns.is_empty()
        && ctx.distributed_services_count == 0
        && !ctx.is_polyglot;

    // Planning strategy
    let planning_strategy = if is_enterprise || is_distributed || is_complex_monorepo {
        Some("adaptive".to_string())
    } else if ctx.project_scale == "Medium" {
        Some("balanced".to_string())
    } else {
        None
    };

    // Model tier
    let model_tier = if is_enterprise || is_distributed || is_complex_monorepo {
        Some("premium".to_string())
    } else if ctx.project_scale == "Small" {
        Some("fast".to_string())
    } else {
        Some("balanced".to_string())
    };

    let rationale = if reasons.is_empty() {
        "Standard project — default mode suitable".to_string()
    } else {
        reasons.join(", ")
    };

    AgentSuggestion {
        model_tier,
        agent_flags: flags,
        planning_strategy,
        activate_reasoning_deep,
        activate_multimodal,
        use_fast_mode,
        rationale,
    }
}

// ─── Main scanner entry point ─────────────────────────────────────────────────

pub async fn architecture_intelligence_scanner(root: &Path) -> ToolOutput {
    let root = root.to_path_buf();
    let result = tokio::task::spawn_blocking(move || scan_architecture_intelligence(&root))
        .await
        .unwrap_or_default();
    result
}

fn scan_architecture_intelligence(root: &Path) -> ToolOutput {
    // Pattern detection
    let mut patterns: HashSet<String> = HashSet::new();

    if detect_microservices(root) {
        patterns.insert("Microservices".to_string());
    }
    if detect_event_driven(root) {
        patterns.insert("Event-Driven".to_string());
    }
    if detect_cqrs(root) {
        patterns.insert("CQRS".to_string());
    }
    if detect_ddd(root) {
        patterns.insert("DDD".to_string());
    }
    if detect_hexagonal(root) {
        patterns.insert("Hexagonal".to_string());
    }

    let architecture_patterns: Vec<String> = {
        let mut v: Vec<String> = patterns.into_iter().collect();
        v.sort();
        v
    };

    let broker = detect_message_broker(root);
    let has_message_broker = broker.is_some();
    let has_service_mesh = detect_service_mesh(root);
    let has_observability_stack = detect_observability_stack(root);
    let has_api_gateway = detect_api_gateway(root);
    let distributed_services_count = count_distributed_services(root);

    ToolOutput {
        architecture_patterns: Some(architecture_patterns),
        has_message_broker: Some(has_message_broker),
        message_broker_type: broker,
        has_service_mesh: Some(has_service_mesh),
        has_observability_stack: Some(has_observability_stack),
        has_api_gateway: Some(has_api_gateway),
        distributed_services_count: Some(distributed_services_count),
        ..Default::default()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_dir(root: &Path, name: &str) -> std::path::PathBuf {
        let p = root.join(name);
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn make_file(p: &Path, name: &str) {
        fs::write(p.join(name), b"x").unwrap();
    }

    // ── Pattern detection ──────────────────────────────────────────────────────

    #[test]
    fn detects_microservices_multiple_dockerfiles() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let a = make_dir(root, "service-a");
        let b = make_dir(root, "service-b");
        make_file(&a, "Dockerfile");
        make_file(&b, "Dockerfile");
        assert!(detect_microservices(root));
    }

    #[test]
    fn not_microservices_single_dockerfile() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "Dockerfile");
        // Only root Dockerfile, no sub-dir Dockerfiles
        assert!(!detect_microservices(root));
    }

    #[test]
    fn detects_event_driven_from_compose() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("docker-compose.yml"),
            b"services:\n  kafka:\n    image: confluentinc/cp-kafka\n",
        )
        .unwrap();
        assert!(detect_event_driven(root));
    }

    #[test]
    fn detects_cqrs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_dir(root, "commands");
        make_dir(root, "queries");
        assert!(detect_cqrs(root));
    }

    #[test]
    fn not_cqrs_without_both_dirs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_dir(root, "commands");
        assert!(!detect_cqrs(root));
    }

    #[test]
    fn detects_ddd_domain_application() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_dir(root, "domain");
        make_dir(root, "application");
        assert!(detect_ddd(root));
    }

    #[test]
    fn detects_hexagonal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_dir(root, "ports");
        make_dir(root, "adapters");
        assert!(detect_hexagonal(root));
    }

    #[test]
    fn detects_service_mesh_istio() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "istio.yaml");
        assert!(detect_service_mesh(root));
    }

    #[test]
    fn detects_observability_prometheus() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "prometheus.yml");
        assert!(detect_observability_stack(root));
    }

    #[test]
    fn detects_api_gateway_nginx() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        make_file(root, "nginx.conf");
        assert!(detect_api_gateway(root));
    }

    // ── Phase 117: Scoring ─────────────────────────────────────────────────────

    fn ctx_with_tests() -> ProjectContext {
        ProjectContext {
            has_tests: true,
            has_ci: true,
            has_readme: true,
            has_security_policy: true,
            has_audit_config: true,
            project_scale: "Medium".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn technical_debt_zero_for_clean_project() {
        let ctx = ctx_with_tests();
        let score = compute_technical_debt_score(&ctx);
        assert_eq!(score, 0);
    }

    #[test]
    fn technical_debt_high_for_legacy_project() {
        let ctx = ProjectContext {
            has_tests: false,
            has_ci: false,
            has_circular_deps: true,
            has_security_policy: false,
            ..Default::default()
        };
        let score = compute_technical_debt_score(&ctx);
        assert!(score >= 60, "Expected high debt, got {score}");
    }

    #[test]
    fn maintainability_high_for_tested_project() {
        let ctx = ctx_with_tests();
        let score = compute_maintainability_score(&ctx);
        assert!(score >= 70, "Expected high maintainability, got {score}");
    }

    #[test]
    fn scalability_high_for_k8s_stack() {
        let ctx = ProjectContext {
            has_docker: true,
            has_message_broker: true,
            has_service_mesh: true,
            tool_has_kubectl: true,
            tool_has_helm: true,
            distributed_services_count: 5,
            ..Default::default()
        };
        let score = compute_scalability_score(&ctx);
        assert!(score >= 80, "Expected high scalability, got {score}");
    }

    #[test]
    fn architecture_quality_with_ddd_and_tests() {
        let mut ctx = ctx_with_tests();
        ctx.architecture_patterns = vec!["DDD".to_string(), "Hexagonal".to_string()];
        let score = compute_architecture_quality_score(&ctx);
        assert!(score >= 80, "Expected high arch quality, got {score}");
    }

    #[test]
    fn architecture_quality_penalizes_circular_deps() {
        let ctx = ProjectContext {
            has_circular_deps: true,
            has_tests: true,
            has_ci: true,
            ..Default::default()
        };
        let q1 = compute_architecture_quality_score(&ctx);
        let ctx2 = ProjectContext {
            has_circular_deps: false,
            has_tests: true,
            has_ci: true,
            ..Default::default()
        };
        let q2 = compute_architecture_quality_score(&ctx2);
        assert!(q1 < q2, "Circular deps should lower quality: {q1} vs {q2}");
    }

    #[test]
    fn ai_readiness_high_with_all_subsystems() {
        let ctx = ProjectContext {
            agent_reasoning_enabled: true,
            agent_orchestration_on: true,
            agent_multimodal_on: true,
            agent_hicon_active: true,
            agent_plugin_system_on: true,
            has_tests: true,
            has_readme: true,
            has_ci: true,
            ..Default::default()
        };
        let score = compute_ai_readiness_score(&ctx);
        assert!(score >= 80, "Expected high AI readiness, got {score}");
    }

    #[test]
    fn distributed_maturity_zero_for_bare_project() {
        let ctx = ProjectContext::default();
        let score = compute_distributed_maturity_score(&ctx);
        assert!(
            score <= 20,
            "Expected low distributed maturity, got {score}"
        );
    }

    // ── Phase 119: Auto-mode suggestions ──────────────────────────────────────

    #[test]
    fn suggests_full_expert_for_enterprise() {
        let ctx = ProjectContext {
            project_scale: "Enterprise".to_string(),
            ..Default::default()
        };
        let suggestion = suggest_agent_configuration(&ctx);
        assert!(suggestion.agent_flags.contains(&"--full".to_string()));
        assert!(suggestion.agent_flags.contains(&"--expert".to_string()));
        assert!(suggestion.activate_reasoning_deep);
    }

    #[test]
    fn suggests_fast_mode_for_small_simple() {
        let ctx = ProjectContext {
            project_scale: "Small".to_string(),
            architecture_patterns: vec![],
            distributed_services_count: 0,
            is_polyglot: false,
            ..Default::default()
        };
        let suggestion = suggest_agent_configuration(&ctx);
        assert!(suggestion.use_fast_mode);
        assert!(suggestion.agent_flags.is_empty());
    }

    #[test]
    fn suggests_multimodal_for_data_project() {
        let ctx = ProjectContext {
            data_framework: Some("PyTorch/TensorFlow".to_string()),
            primary_language: "Python".to_string(),
            project_scale: "Medium".to_string(),
            ..Default::default()
        };
        let suggestion = suggest_agent_configuration(&ctx);
        assert!(suggestion.activate_multimodal);
    }

    #[test]
    fn suggests_premium_model_for_distributed() {
        let ctx = ProjectContext {
            has_service_mesh: true,
            distributed_services_count: 5,
            project_scale: "Large".to_string(),
            ..Default::default()
        };
        let suggestion = suggest_agent_configuration(&ctx);
        assert_eq!(suggestion.model_tier.as_deref(), Some("premium"));
    }

    #[test]
    fn suggestion_rationale_is_non_empty() {
        let ctx = ProjectContext {
            project_scale: "Enterprise".to_string(),
            ..Default::default()
        };
        let suggestion = suggest_agent_configuration(&ctx);
        assert!(!suggestion.rationale.is_empty());
    }

    // ── Message broker detection ───────────────────────────────────────────────

    #[test]
    fn detects_kafka_from_compose() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("docker-compose.yml"),
            b"services:\n  kafka:\n    image: confluentinc/cp-kafka:latest\n",
        )
        .unwrap();
        let broker = detect_message_broker(root);
        assert_eq!(broker.as_deref(), Some("Kafka"));
    }

    #[test]
    fn detects_rabbitmq_from_compose() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("docker-compose.yml"),
            b"services:\n  rabbitmq:\n    image: rabbitmq:3-management\n",
        )
        .unwrap();
        let broker = detect_message_broker(root);
        assert_eq!(broker.as_deref(), Some("RabbitMQ"));
    }

    #[test]
    fn no_broker_when_no_compose() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let broker = detect_message_broker(root);
        assert!(broker.is_none());
    }
}
