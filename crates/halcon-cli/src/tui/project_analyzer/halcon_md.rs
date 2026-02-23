//! HALCON.md generator — produces the project intelligence document.
//!
//! Generates a rich Markdown file that serves as the agent's context document.
//! Phase 107: includes Environment Summary, Agent Capabilities, Runtime Profile,
//! IDE Context, AgentReadinessScore, EnvironmentCompatibilityScore, and
//! actionable optimization recommendations.

use super::tools::ProjectContext;

/// Generate a full HALCON.md document from the accumulated project context.
pub fn generate(ctx: &ProjectContext) -> String {
    let mut md = String::with_capacity(16384);
    let root_dir_name_buf: String;
    let name = if let Some(pkg) = ctx.package_name.as_deref() {
        pkg
    } else {
        root_dir_name_buf = std::path::Path::new(&ctx.root)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("proyecto")
            .to_string();
        root_dir_name_buf.as_str()
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    // ── Header ────────────────────────────────────────────────────────────────
    md.push_str(&format!("# HALCON — {name}\n"));
    md.push_str(&format!("<!-- Generado por `halcon /init` · {today} -->\n\n"));

    // ── Score Badges ──────────────────────────────────────────────────────────
    let score = ctx.health_score;
    let score_label = if score >= 80 { "BUENA" } else if score >= 60 { "MODERADA" } else { "NECESITA ATENCIÓN" };
    let score_icon = if score >= 80 { "◈" } else if score >= 60 { "◇" } else { "⚐" };
    md.push_str(&format!("> **{score_icon} Salud del proyecto: {score}/100** — {score_label}  \n"));

    // Agent Readiness Score (Phase 107)
    if ctx.agent_readiness_score > 0 || ctx.environment_compatibility_score > 0 {
        let ar = ctx.agent_readiness_score;
        let ec = ctx.environment_compatibility_score;
        let ar_label = if ar >= 80 { "ÓPTIMO" } else if ar >= 60 { "FUNCIONAL" } else if ar >= 40 { "BÁSICO" } else { "LIMITADO" };
        let ec_label = if ec >= 80 { "ÓPTIMO" } else if ec >= 60 { "COMPATIBLE" } else if ec >= 40 { "PARCIAL" } else { "LIMITADO" };
        md.push_str(&format!("> **◈ Agente listo: {ar}/100** — {ar_label}  \n"));
        md.push_str(&format!("> **◈ Entorno compatible: {ec}/100** — {ec_label}\n\n"));
    } else {
        md.push('\n');
    }

    // ── Project identity ──────────────────────────────────────────────────────
    md.push_str("## Proyecto\n");
    md.push_str(&format!("- **Nombre**: `{name}`\n"));
    if !ctx.project_type.is_empty() {
        md.push_str(&format!("- **Tipo**: {}\n", ctx.project_type));
    }
    if let Some(ref v) = ctx.version {
        md.push_str(&format!("- **Versión**: {v}\n"));
    }
    if let Some(ref d) = ctx.description {
        md.push_str(&format!("- **Descripción**: {d}\n"));
    }
    if let Some(ref e) = ctx.edition {
        md.push_str(&format!("- **Edition**: Rust {e}\n"));
    }
    if let Some(ref l) = ctx.license {
        md.push_str(&format!("- **Licencia**: {l}\n"));
    }
    if let Some(n) = ctx.dep_count {
        md.push_str(&format!("- **Dependencias**: {n}\n"));
    }
    md.push('\n');

    // ── Architecture ──────────────────────────────────────────────────────────
    md.push_str("## Arquitectura\n");
    if let Some(ref style) = ctx.architecture_style {
        md.push_str(&format!("- **Estilo**: {style}\n"));
    }
    if !ctx.members.is_empty() {
        md.push_str(&format!("- **Crates/Paquetes**: {}\n", ctx.members.len()));
    }
    if let Some(c) = ctx.complexity_score {
        let label = if c < 30 { "Baja" } else if c < 60 { "Media" } else { "Alta" };
        md.push_str(&format!("- **Complejidad estimada**: {label} ({c}/100)\n"));
    }
    md.push('\n');

    // ── Infrastructure ────────────────────────────────────────────────────────
    md.push_str("## Infraestructura\n");
    if ctx.has_ci {
        let ci = ctx.ci_system.as_deref().unwrap_or("CI");
        md.push_str(&format!("- ✓ **CI/CD**: {ci}\n"));
    } else {
        md.push_str("- ✗ **CI/CD**: No detectado\n");
    }
    if ctx.has_docker {
        md.push_str("- ✓ **Containers**: Dockerfile / docker-compose detectado\n");
    }
    if ctx.has_tests {
        let cov_str = ctx.test_coverage_est.map(|c| format!(" (~{c}% cobertura estimada)")).unwrap_or_default();
        md.push_str(&format!("- ✓ **Tests**: Detectados{cov_str}\n"));
    } else {
        md.push_str("- ✗ **Tests**: No detectados\n");
    }
    if ctx.has_security_policy {
        md.push_str("- ✓ **Security Policy**: SECURITY.md presente\n");
    } else {
        md.push_str("- ✗ **Security Policy**: Sin SECURITY.md\n");
    }
    if ctx.has_audit_config {
        md.push_str("- ✓ **Dependency Audit**: Configurado (deny.toml / .snyk)\n");
    }
    md.push('\n');

    // ── Repository ────────────────────────────────────────────────────────────
    if ctx.branch.is_some() {
        md.push_str("## Repositorio\n");
        if let Some(ref b) = ctx.branch {
            md.push_str(&format!("- **Rama activa**: `{b}`\n"));
        }
        if let Some(ref r) = ctx.remote {
            md.push_str(&format!("- **Remote origin**: {r}\n"));
        }
        if let Some(ref c) = ctx.last_commit {
            md.push_str(&format!("- **Último commit**: {c}\n"));
        }
        if let Some(ref s) = ctx.status_summary {
            md.push_str(&format!("- **Estado**: {s}\n"));
        }
        if let Some(n) = ctx.total_commits {
            md.push_str(&format!("- **Total commits**: {n}\n"));
        }
        if let Some(v) = ctx.commit_velocity_per_week {
            md.push_str(&format!("- **Velocidad**: {v:.1} commits/semana\n"));
        }
        if let Some(bf) = ctx.bus_factor {
            let bf_label = if bf >= 3 { "✓ bueno" } else if bf >= 2 { "◇ moderado" } else { "⚐ riesgo" };
            md.push_str(&format!("- **Bus factor**: {bf} contribuidores ({bf_label})\n"));
        }
        if !ctx.contributors.is_empty() {
            let top3: Vec<&str> = ctx.contributors.iter().take(3).map(|s| s.as_str()).collect();
            md.push_str(&format!("- **Top contribuidores**: {}\n", top3.join(", ")));
        }
        md.push('\n');
    }

    // ── Stack técnico ─────────────────────────────────────────────────────────
    if !ctx.stack.is_empty() {
        md.push_str("## Stack Técnico\n");
        for item in &ctx.stack {
            md.push_str(&format!("- {item}\n"));
        }
        md.push('\n');
    }

    // ── Environment Summary (Phase 107) ───────────────────────────────────────
    let has_env_data = !ctx.sys_os.is_empty()
        || ctx.sys_cpu_cores > 0
        || ctx.sys_ram_mb > 0
        || ctx.sys_is_wsl
        || ctx.sys_is_container;
    if has_env_data {
        md.push_str("## Entorno de Ejecución\n");
        if !ctx.sys_os.is_empty() {
            md.push_str(&format!("- **OS**: {}\n", ctx.sys_os));
        }
        if ctx.sys_cpu_cores > 0 {
            md.push_str(&format!("- **CPU**: {} cores\n", ctx.sys_cpu_cores));
        }
        if ctx.sys_ram_mb > 0 {
            let ram_gb = ctx.sys_ram_mb as f64 / 1024.0;
            md.push_str(&format!("- **RAM**: {ram_gb:.1} GB\n"));
        }
        if let Some(disk_gb) = ctx.sys_disk_free_gb {
            md.push_str(&format!("- **Disco libre**: {disk_gb:.1} GB\n"));
        }
        if ctx.sys_gpu_available {
            md.push_str("- ✓ **GPU**: Disponible\n");
        }
        if ctx.sys_is_wsl {
            md.push_str("- ◈ **Entorno**: WSL (Windows Subsystem for Linux)\n");
        }
        if ctx.sys_is_container {
            md.push_str("- ◈ **Entorno**: Container (Docker/Podman)\n");
        }
        if ctx.runtime_ci_environment {
            md.push_str("- ◈ **CI/CD**: Ejecutando en entorno de integración continua\n");
        }
        md.push('\n');
    }

    // ── Tool Versions (Phase 107) ─────────────────────────────────────────────
    let has_tool_data = ctx.tool_git_version.is_some()
        || ctx.tool_rust_version.is_some()
        || ctx.tool_node_version.is_some()
        || ctx.tool_python_version.is_some()
        || ctx.tool_docker_version.is_some()
        || ctx.tool_go_version.is_some();
    if has_tool_data {
        md.push_str("## Herramientas del Sistema\n");
        if let Some(ref v) = ctx.tool_git_version {
            md.push_str(&format!("- **git**: {v}\n"));
        }
        if let Some(ref v) = ctx.tool_rust_version {
            md.push_str(&format!("- **rustc**: {v}\n"));
        }
        if let Some(ref v) = ctx.tool_cargo_version {
            md.push_str(&format!("- **cargo**: {v}\n"));
        }
        if let Some(ref v) = ctx.tool_node_version {
            md.push_str(&format!("- **node**: {v}\n"));
        }
        if let Some(ref v) = ctx.tool_python_version {
            md.push_str(&format!("- **python**: {v}\n"));
        }
        if let Some(ref v) = ctx.tool_go_version {
            md.push_str(&format!("- **go**: {v}\n"));
        }
        if let Some(ref v) = ctx.tool_docker_version {
            md.push_str(&format!("- **docker**: {v}\n"));
        }
        if let Some(ref v) = ctx.tool_make_version {
            md.push_str(&format!("- **make**: {v}\n"));
        }
        // Infra tools
        let mut infra = vec![];
        if ctx.tool_has_kubectl { infra.push("kubectl"); }
        if ctx.tool_has_helm    { infra.push("helm"); }
        if ctx.tool_has_terraform { infra.push("terraform"); }
        if ctx.tool_has_ansible { infra.push("ansible"); }
        if !infra.is_empty() {
            md.push_str(&format!("- **Infra tools**: {}\n", infra.join(", ")));
        }
        md.push('\n');
    }

    // ── IDE Context (Phase 107) ───────────────────────────────────────────────
    if ctx.ide_detected.is_some() || ctx.ide_lsp_connected {
        md.push_str("## Contexto IDE\n");
        if let Some(ref ide) = ctx.ide_detected {
            md.push_str(&format!("- **IDE activo**: {ide}\n"));
        }
        if let Some(ref ws) = ctx.ide_workspace {
            md.push_str(&format!("- **Workspace**: {ws}\n"));
        }
        if let Some(ref af) = ctx.ide_active_file {
            md.push_str(&format!("- **Archivo activo**: {af}\n"));
        }
        if ctx.ide_lsp_connected {
            let port = ctx.ide_lsp_port.map(|p| format!(" (puerto {p})")).unwrap_or_default();
            md.push_str(&format!("- ✓ **LSP / Dev Gateway**: Conectado{port}\n"));
        } else {
            md.push_str("- ○ **LSP / Dev Gateway**: No detectado\n");
        }
        md.push('\n');
    }

    // ── Peer AI Context Files (Phase 122 — SOTA 2026) ─────────────────────────
    if !ctx.ai_context_files.is_empty() {
        md.push_str("## Archivos de Contexto AI Detectados\n");
        md.push_str("> Este proyecto tiene instrucciones para múltiples asistentes AI.\n\n");
        for (fname, tool) in &ctx.ai_context_files {
            md.push_str(&format!("- `{fname}` — {tool}\n"));
        }
        md.push('\n');
    }

    // ── Agent Capabilities (Phase 107) ────────────────────────────────────────
    let has_agent_data = ctx.agent_model_name.is_some()
        || !ctx.agent_mcp_servers.is_empty()
        || ctx.agent_plugins_loaded > 0
        || ctx.agent_reasoning_enabled
        || ctx.agent_orchestration_on;
    if has_agent_data {
        md.push_str("## Capacidades del Agente\n");
        if let Some(ref model) = ctx.agent_model_name {
            let tier = ctx.agent_model_tier.as_deref().unwrap_or("unknown");
            md.push_str(&format!("- **Modelo**: `{model}` (tier: {tier})\n"));
        }
        if !ctx.agent_mcp_servers.is_empty() {
            md.push_str(&format!("- **MCP Servers**: {} activos — {}\n",
                ctx.agent_mcp_servers.len(),
                ctx.agent_mcp_servers.iter().take(5).cloned().collect::<Vec<_>>().join(", ")));
        } else {
            md.push_str("- **MCP Servers**: Ninguno configurado\n");
        }
        if ctx.agent_plugins_loaded > 0 {
            md.push_str(&format!("- ✓ **Plugins**: {} cargados\n", ctx.agent_plugins_loaded));
        }
        // Feature flags
        let flags: Vec<&str> = [
            (ctx.agent_reasoning_enabled, "Reasoning"),
            (ctx.agent_orchestration_on,  "Orchestration"),
            (ctx.agent_multimodal_on,     "Multimodal"),
            (ctx.agent_plugin_system_on,  "Plugins"),
            (ctx.agent_hicon_active,      "HICON"),
        ]
        .iter()
        .filter_map(|(on, name)| if *on { Some(*name) } else { None })
        .collect();
        if !flags.is_empty() {
            md.push_str(&format!("- **Subsistemas activos**: {}\n", flags.join(", ")));
        }
        if !ctx.agent_tools_available.is_empty() {
            md.push_str(&format!("- **Tools disponibles**: {} herramientas\n", ctx.agent_tools_available.len()));
        }
        md.push('\n');
    }

    // ── Runtime Profile (Phase 107) ───────────────────────────────────────────
    let has_runtime_data = ctx.runtime_model_router_active
        || ctx.runtime_convergence_controller_on
        || ctx.runtime_intent_scorer_on
        || ctx.runtime_token_budget.is_some();
    if has_runtime_data {
        md.push_str("## Perfil de Runtime\n");
        let components: Vec<&str> = [
            (ctx.runtime_model_router_active, "ModelRouter"),
            (ctx.runtime_convergence_controller_on, "ConvergenceController"),
            (ctx.runtime_intent_scorer_on, "IntentScorer"),
        ]
        .iter()
        .filter_map(|(on, name)| if *on { Some(*name) } else { None })
        .collect();
        if !components.is_empty() {
            md.push_str(&format!("- **Componentes activos**: {}\n", components.join(", ")));
        }
        if let Some(budget) = ctx.runtime_token_budget {
            md.push_str(&format!("- **Token budget**: {budget}\n"));
        }
        md.push('\n');
    }

    // ── Workspace / Paquetes ──────────────────────────────────────────────────
    if !ctx.members.is_empty() {
        md.push_str("## Workspace / Paquetes\n");
        md.push_str("```\n");
        for m in &ctx.members {
            let desc = m.description.as_deref().unwrap_or("");
            if desc.is_empty() {
                md.push_str(&format!("{}/\n", m.path));
            } else {
                let short_desc: String = desc.chars().take(60).collect();
                md.push_str(&format!("{}/  # {short_desc}\n", m.path));
            }
        }
        md.push_str("```\n\n");
    }

    // ── Directory structure ───────────────────────────────────────────────────
    if !ctx.top_dirs.is_empty() {
        md.push_str("## Estructura\n");
        md.push_str("```\n");
        let root_name = std::path::Path::new(&ctx.root)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        md.push_str(&format!("{root_name}/\n"));
        for (i, d) in ctx.top_dirs.iter().enumerate() {
            let is_last = i == ctx.top_dirs.len() - 1;
            let prefix = if is_last { "└── " } else { "├── " };
            md.push_str(&format!("{prefix}{d}/\n"));
        }
        md.push_str("```\n\n");
    }

    // ── Language Intelligence (Phase 110-112) ────────────────────────────────
    if !ctx.primary_language.is_empty() {
        md.push_str("## Inteligencia de Lenguajes\n");
        md.push_str(&format!("- **Lenguaje primario**: {}\n", ctx.primary_language));
        if !ctx.secondary_languages.is_empty() {
            md.push_str(&format!("- **Lenguajes secundarios**: {}\n",
                ctx.secondary_languages.join(", ")));
        }
        if ctx.is_polyglot {
            md.push_str("- ◈ **Repositorio poliglota**: múltiples lenguajes de producción\n");
        }
        if !ctx.language_breakdown.is_empty() {
            let breakdown: Vec<String> = ctx.language_breakdown.iter()
                .map(|(l, n)| format!("{l} ({n})"))
                .collect();
            md.push_str(&format!("- **Distribución**: {}\n", breakdown.join(", ")));
        }
        if let Some(ref fw) = ctx.frontend_framework {
            md.push_str(&format!("- **Frontend**: {fw}\n"));
        }
        if let Some(ref fw) = ctx.mobile_framework {
            md.push_str(&format!("- **Mobile**: {fw}\n"));
        }
        if let Some(ref fw) = ctx.data_framework {
            md.push_str(&format!("- **Data/AI**: {fw}\n"));
        }
        if let Some(ref tool) = ctx.infra_tool {
            md.push_str(&format!("- **Infra**: {tool}\n"));
        }
        // Monorepo
        if ctx.is_monorepo {
            let tool = ctx.monorepo_tool.as_deref().unwrap_or("monorepo");
            md.push_str(&format!("- **Monorepo**: {tool} · {} sub-proyectos\n",
                ctx.sub_project_count));
            if !ctx.sub_projects.is_empty() {
                let shown: Vec<&str> = ctx.sub_projects.iter()
                    .take(6).map(|s| s.as_str()).collect();
                md.push_str(&format!("  - Sub-proyectos: {}\n", shown.join(", ")));
            }
        }
        // Scale
        if !ctx.project_scale.is_empty() {
            let scale_desc = match ctx.project_scale.as_str() {
                "Small" => "≤ 500 archivos",
                "Medium" => "501–5 000 archivos",
                "Large" => "5 001–50 000 archivos",
                "Enterprise" => "> 50 000 archivos",
                _ => "",
            };
            md.push_str(&format!("- **Escala**: {} ({}) · {} archivos escaneados\n",
                ctx.project_scale, scale_desc, ctx.total_file_count));
        }
        if ctx.estimated_loc > 0 {
            md.push_str(&format!("- **LOC estimadas**: ~{} líneas\n", ctx.estimated_loc));
        }
        md.push('\n');
    }

    // ── Distributed Architecture (Phase 113) ──────────────────────────────────
    let has_dist = !ctx.architecture_patterns.is_empty()
        || ctx.has_message_broker
        || ctx.has_service_mesh
        || ctx.has_observability_stack
        || ctx.has_api_gateway
        || ctx.distributed_services_count > 0;
    if has_dist {
        md.push_str("## Arquitectura Distribuida\n");
        if !ctx.architecture_patterns.is_empty() {
            md.push_str(&format!("- **Patrones**: {}\n",
                ctx.architecture_patterns.join(", ")));
        }
        if ctx.distributed_services_count > 0 {
            md.push_str(&format!("- **Servicios detectados**: {}\n",
                ctx.distributed_services_count));
        }
        if ctx.has_message_broker {
            let broker = ctx.message_broker_type.as_deref().unwrap_or("Message Broker");
            md.push_str(&format!("- ✓ **Message Broker**: {broker}\n"));
        }
        if ctx.has_service_mesh {
            md.push_str("- ✓ **Service Mesh**: Detectado (Istio/Linkerd)\n");
        }
        if ctx.has_observability_stack {
            md.push_str("- ✓ **Observability**: Stack configurado (Prometheus/Grafana/OTEL)\n");
        }
        if ctx.has_api_gateway {
            md.push_str("- ✓ **API Gateway**: Detectado\n");
        }
        md.push('\n');
    }

    // ── Phase 117: Advanced Score Dashboard ───────────────────────────────────
    let has_adv_scores = ctx.architecture_quality_score > 0
        || ctx.scalability_score > 0
        || ctx.maintainability_score > 0
        || ctx.ai_readiness_score > 0;
    if has_adv_scores {
        md.push_str("## Dashboard de Calidad (10 Métricas)\n");
        md.push_str("| Métrica | Puntuación | Nivel |\n");
        md.push_str("|---|---|---|\n");
        let score_row = |label: &str, s: u8| -> String {
            let level = if s >= 80 { "◈ Alto" } else if s >= 60 { "◇ Medio" } else { "⚐ Bajo" };
            format!("| {label} | {s}/100 | {level} |\n")
        };
        md.push_str(&score_row("Salud del Proyecto", ctx.health_score));
        md.push_str(&score_row("Listo para Agente", ctx.agent_readiness_score));
        md.push_str(&score_row("Compatibilidad Entorno", ctx.environment_compatibility_score));
        md.push_str(&score_row("Calidad de Arquitectura", ctx.architecture_quality_score));
        md.push_str(&score_row("Escalabilidad", ctx.scalability_score));
        md.push_str(&score_row("Mantenibilidad", ctx.maintainability_score));
        let debt_level = if ctx.technical_debt_score <= 20 { "◈ Bajo" }
            else if ctx.technical_debt_score <= 50 { "◇ Moderado" } else { "⚐ Alto" };
        md.push_str(&format!("| Deuda Técnica | {}/100 | {debt_level} |\n",
            ctx.technical_debt_score));
        md.push_str(&score_row("Developer Experience", ctx.dev_ex_score));
        md.push_str(&score_row("Preparación IA", ctx.ai_readiness_score));
        md.push_str(&score_row("Madurez Distribuida", ctx.distributed_maturity_score));
        md.push('\n');
    }

    // ── Phase 118: Capability Matrix ──────────────────────────────────────────
    {
        md.push_str("## Matriz de Capacidades\n");
        md.push_str("| Capacidad | Detectada | Estado | Riesgo |\n");
        md.push_str("|---|---|---|---|\n");

        let row = |cap: &str, detected: bool, status: &str, risk: &str| -> String {
            let icon = if detected { "✓" } else { "✗" };
            format!("| {cap} | {icon} | {status} | {risk} |\n")
        };

        md.push_str(&row("Tests",
            ctx.has_tests,
            if ctx.has_tests {
                ctx.test_coverage_est.map(|c| {
                    if c >= 80 { "Cobertura alta" } else if c >= 50 { "Cobertura media" } else { "Cobertura baja" }
                }).unwrap_or("Detectados")
            } else { "No detectados" },
            if ctx.has_tests { "Bajo" } else { "Alto" }
        ));

        md.push_str(&row("CI/CD",
            ctx.has_ci,
            ctx.ci_system.as_deref().unwrap_or("No configurado"),
            if ctx.has_ci { "Bajo" } else { "Alto" }
        ));

        md.push_str(&row("Containers",
            ctx.has_docker,
            if ctx.has_docker { "Dockerfile detectado" } else { "Sin containerización" },
            if ctx.has_docker { "Bajo" } else { "Medio" }
        ));

        md.push_str(&row("Security Policy",
            ctx.has_security_policy,
            if ctx.has_security_policy { "SECURITY.md presente" } else { "Sin política" },
            if ctx.has_security_policy { "Bajo" } else { "Medio" }
        ));

        md.push_str(&row("Dep Auditing",
            ctx.has_audit_config,
            if ctx.has_audit_config { "deny.toml / .snyk" } else { "Sin auditoría" },
            if ctx.has_audit_config { "Bajo" } else { "Medio" }
        ));

        md.push_str(&row("Observability",
            ctx.has_observability_stack,
            if ctx.has_observability_stack { "Prometheus/OTEL detectado" } else { "Sin observability" },
            if ctx.has_observability_stack { "Bajo" } else { if ctx.distributed_services_count >= 2 { "Alto" } else { "Bajo" } }
        ));

        md.push_str(&row("Message Broker",
            ctx.has_message_broker,
            ctx.message_broker_type.as_deref().unwrap_or("No detectado"),
            "Bajo"
        ));

        md.push_str(&row("Service Mesh",
            ctx.has_service_mesh,
            if ctx.has_service_mesh { "Detectado" } else { "Sin mesh" },
            if ctx.has_service_mesh { "Bajo" } else { if ctx.distributed_services_count >= 3 { "Medio" } else { "Bajo" } }
        ));

        md.push('\n');
    }

    // ── Phase 119: Auto-Mode Suggestion ───────────────────────────────────────
    let has_suggestion = !ctx.suggested_agent_flags.is_empty()
        || ctx.suggested_model_tier.is_some()
        || ctx.use_fast_mode;
    if has_suggestion || ctx.agent_mode_rationale.is_some() {
        md.push_str("## Configuración de Agente Sugerida\n");
        if let Some(ref rationale) = ctx.agent_mode_rationale {
            md.push_str(&format!("> **Análisis**: {rationale}\n\n"));
        }
        if !ctx.suggested_agent_flags.is_empty() {
            let flags_str = ctx.suggested_agent_flags.join(" ");
            md.push_str(&format!("```bash\nhalcon chat {flags_str}\n```\n\n"));
        } else if ctx.use_fast_mode {
            md.push_str("```bash\nhalcon chat  # Modo estándar — proyecto pequeño y simple\n```\n\n");
        }
        if let Some(ref tier) = ctx.suggested_model_tier {
            let tier_desc = match tier.as_str() {
                "premium" => "Opus / GPT-4o — mejor para proyectos complejos",
                "balanced" => "Sonnet / GPT-4o-mini — balance calidad/velocidad",
                "fast" => "Haiku / GPT-4o-mini — respuesta rápida, proyecto simple",
                _ => tier.as_str(),
            };
            md.push_str(&format!("- **Modelo sugerido**: {} ({})\n", tier, tier_desc));
        }
        if let Some(ref strat) = ctx.suggested_planning_strategy {
            md.push_str(&format!("- **Estrategia de planning**: {strat}\n"));
        }
        if ctx.activate_reasoning_deep {
            md.push_str("- ◈ **Reasoning profundo**: Recomendado para esta arquitectura\n");
        }
        if ctx.activate_multimodal_for_init {
            md.push_str("- ◈ **Análisis multimodal**: Útil para proyecto Data/AI\n");
        }
        md.push('\n');
    }

    // ── Risks & Issues ────────────────────────────────────────────────────────
    if !ctx.health_issues.is_empty() {
        md.push_str("## Riesgos Detectados\n");
        for issue in &ctx.health_issues {
            md.push_str(&format!("- ⚐ {issue}\n"));
        }
        md.push('\n');
    }

    // ── Recommendations ───────────────────────────────────────────────────────
    if !ctx.health_recommendations.is_empty() {
        md.push_str("## Recomendaciones\n");
        for (i, rec) in ctx.health_recommendations.iter().enumerate() {
            md.push_str(&format!("{}. {rec}\n", i + 1));
        }
        md.push('\n');
    }

    // ── Optimization Opportunities (Phase 107) ─────────────────────────────────
    let mut opts: Vec<String> = Vec::new();
    if ctx.agent_mcp_servers.is_empty() {
        opts.push("Configurar MCP servers en `~/.halcon/.mcp.json` para capacidades extendidas".into());
    }
    if !ctx.agent_reasoning_enabled {
        opts.push("Activar ReasoningEngine con `halcon chat --full` para tareas complejas".into());
    }
    if !ctx.agent_orchestration_on {
        opts.push("Activar Multi-Agent Orchestration con `--full` para paralelismo".into());
    }
    if ctx.ide_detected.is_none() {
        opts.push("Integrar con IDE: instalar extensión HALCON para VSCode/Cursor para LSP".into());
    }
    if !ctx.has_ci {
        opts.push("Añadir pipeline CI/CD (GitHub Actions / GitLab CI) para automatización".into());
    }
    if !ctx.has_security_policy {
        opts.push("Crear SECURITY.md con política de divulgación de vulnerabilidades".into());
    }
    if ctx.sys_ram_mb > 0 && ctx.sys_ram_mb < 4096 {
        opts.push("RAM < 4 GB detectada — considerar migrar a máquina con más recursos".into());
    }
    if !opts.is_empty() {
        md.push_str("## Oportunidades de Optimización\n");
        for (i, opt) in opts.iter().enumerate() {
            md.push_str(&format!("{}. {opt}\n", i + 1));
        }
        md.push('\n');
    }

    // ── Agent instructions ────────────────────────────────────────────────────
    md.push_str("## Instrucciones para el Agente\n\n");
    md.push_str(&format!(
        "Eres **HALCON**, un asistente de ingeniería autónomo para el proyecto `{name}`.\n\n"
    ));

    md.push_str("### Identidad\n");
    md.push_str("- Responde siempre en el idioma del usuario (ES ↔ EN)\n");
    md.push_str("- Sé conciso y orientado a la acción — sin relleno\n");
    md.push_str("- Usa las convenciones y estilo del proyecto existente\n\n");

    md.push_str("### Flujo de trabajo\n");
    md.push_str("- Lee los archivos relevantes ANTES de modificarlos\n");
    md.push_str("- Prefiere editar archivos existentes sobre crear nuevos\n");
    md.push_str("- Ejecuta las pruebas después de cambios significativos\n");
    md.push_str("- Usa `git status`/`git diff` para entender el árbol de trabajo\n\n");

    // Type-specific commands
    match ctx.project_type.as_str() {
        t if t.starts_with("Rust") => {
            md.push_str("### Comandos clave\n");
            md.push_str("```bash\n");
            md.push_str(&format!("cargo build --release -p {name}  # Release\n"));
            md.push_str(&format!("cargo test -p {name} --lib        # Tests unitarios\n"));
            md.push_str("cargo clippy --workspace -- -D warnings    # Linting\n");
            md.push_str("cargo fmt --all                            # Formateo\n");
            if ctx.has_audit_config {
                md.push_str("cargo deny check                           # Auditoría deps\n");
            }
            md.push_str("```\n\n");
            md.push_str("### Convenciones Rust\n");
            if let Some(ref e) = ctx.edition {
                md.push_str(&format!("- **Edition**: {e}\n"));
            }
            md.push_str("- Errors: `thiserror` en libs, `anyhow` en binarios\n");
            md.push_str("- Tests: inline `#[cfg(test)]` en cada módulo\n");
            md.push_str("- No usar `unwrap()` en código de producción\n");
            md.push_str("- `#[allow(...)]` solo con justificación explícita\n\n");
        }
        t if t.contains("Node") || t.contains("React") || t.contains("Next") => {
            md.push_str("### Comandos clave\n");
            md.push_str("```bash\n");
            md.push_str("npm install       # Dependencias\n");
            md.push_str("npm run dev       # Desarrollo\n");
            md.push_str("npm test          # Tests\n");
            md.push_str("npm run build     # Build de producción\n");
            md.push_str("npm run lint      # Linting\n");
            md.push_str("```\n\n");
        }
        "Python" => {
            md.push_str("### Comandos clave\n");
            md.push_str("```bash\n");
            md.push_str("python -m pytest          # Tests\n");
            md.push_str("pip install -e .          # Install editable\n");
            md.push_str("python -m ruff check .    # Linting\n");
            md.push_str("```\n\n");
        }
        _ => {}
    }

    md.push_str("### Prioridades\n");
    md.push_str("1. Seguridad y correctitud\n");
    md.push_str("2. Rendimiento y eficiencia\n");
    md.push_str("3. Legibilidad y mantenibilidad\n");
    md.push_str("4. Tests de regresión para cada fix\n\n");

    // ── Telemetry ─────────────────────────────────────────────────────────────
    md.push_str("---\n");
    md.push_str(&format!("*Generado por `halcon /init` · {today}*  \n"));
    if ctx.analysis_duration_ms > 0 {
        let dur_s = ctx.analysis_duration_ms as f64 / 1000.0;
        md.push_str(&format!("*Análisis: {dur_s:.1}s · {} archivos · {} herramientas*\n",
            ctx.files_scanned, ctx.tools_run));
        if ctx.cache_hit {
            md.push_str("*Resultado desde caché (< 24h)*\n");
        }
    }
    // Phase 109 telemetry breakdown
    let res_ms = ctx.resource_detection_time_ms;
    let env_ms = ctx.environment_scan_time_ms;
    let ide_ms = ctx.ide_context_integration_time_ms;
    let hicon_ms = ctx.hicon_query_time_ms;
    if res_ms > 0 || env_ms > 0 || ide_ms > 0 || hicon_ms > 0 {
        md.push_str(&format!(
            "*Detección recursos: {res_ms}ms · Entorno: {env_ms}ms · IDE: {ide_ms}ms · HICON: {hicon_ms}ms*\n"
        ));
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(name: &str) -> ProjectContext {
        ProjectContext {
            package_name: Some(name.to_string()),
            project_type: "Rust Workspace".to_string(),
            health_score: 75,
            has_readme: true,
            has_ci: true,
            ci_system: Some("GitHub Actions".to_string()),
            has_tests: true,
            test_coverage_est: Some(65),
            branch: Some("main".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn generate_contains_project_name() {
        let ctx = make_ctx("my-project");
        let md = generate(&ctx);
        assert!(md.contains("my-project"), "Must contain project name");
    }

    #[test]
    fn generate_contains_health_score() {
        let ctx = make_ctx("test");
        let md = generate(&ctx);
        assert!(md.contains("75/100"), "Must contain health score");
    }

    #[test]
    fn generate_shows_ci_check() {
        let ctx = make_ctx("test");
        let md = generate(&ctx);
        assert!(md.contains("GitHub Actions"), "Should show CI system");
    }

    #[test]
    fn generate_shows_no_ci_when_missing() {
        let mut ctx = make_ctx("test");
        ctx.has_ci = false;
        ctx.ci_system = None;
        let md = generate(&ctx);
        assert!(md.contains("No detectado"), "Should flag missing CI");
    }

    #[test]
    fn generate_includes_rust_commands() {
        let ctx = make_ctx("myapp");
        let md = generate(&ctx);
        assert!(md.contains("cargo build"), "Should include cargo commands");
        assert!(md.contains("cargo test"), "Should include test command");
    }

    #[test]
    fn generate_includes_recommendations_when_issues() {
        let mut ctx = ProjectContext::default();
        ctx.health_issues = vec!["No CI detected".to_string()];
        ctx.health_recommendations = vec!["Add GitHub Actions".to_string()];
        let md = generate(&ctx);
        assert!(md.contains("Riesgos Detectados"), "Should have risks section");
        assert!(md.contains("Recomendaciones"), "Should have recommendations section");
        assert!(md.contains("Add GitHub Actions"), "Should include specific recommendation");
    }

    #[test]
    fn generate_includes_workspace_members() {
        let mut ctx = make_ctx("monorepo");
        ctx.members = vec![
            super::super::tools::WorkspaceMember {
                name: "core".to_string(),
                path: "crates/core".to_string(),
                description: Some("Core library".to_string()),
            },
        ];
        let md = generate(&ctx);
        assert!(md.contains("crates/core"), "Should list workspace members");
    }

    // ── Phase 107 tests ───────────────────────────────────────────────────────

    #[test]
    fn generate_shows_agent_readiness_score() {
        let mut ctx = make_ctx("test");
        ctx.agent_readiness_score = 85;
        ctx.environment_compatibility_score = 72;
        let md = generate(&ctx);
        assert!(md.contains("85/100"), "Should show agent readiness score");
        assert!(md.contains("72/100"), "Should show environment compatibility score");
        assert!(md.contains("ÓPTIMO"), "85/100 should be ÓPTIMO");
    }

    #[test]
    fn generate_shows_environment_summary_when_data_present() {
        let mut ctx = make_ctx("test");
        ctx.sys_os = "Linux aarch64".to_string();
        ctx.sys_cpu_cores = 8;
        ctx.sys_ram_mb = 16384;
        ctx.sys_gpu_available = true;
        let md = generate(&ctx);
        assert!(md.contains("Entorno de Ejecución"), "Should have env section");
        assert!(md.contains("Linux aarch64"), "Should show OS");
        assert!(md.contains("8 cores"), "Should show CPU cores");
        assert!(md.contains("16.0 GB"), "Should show RAM in GB");
        assert!(md.contains("GPU"), "Should mention GPU when available");
    }

    #[test]
    fn generate_skips_environment_section_when_empty() {
        let ctx = make_ctx("test");
        let md = generate(&ctx);
        assert!(!md.contains("Entorno de Ejecución"), "Should not show env section when empty");
    }

    #[test]
    fn generate_shows_tool_versions() {
        let mut ctx = make_ctx("test");
        ctx.tool_git_version = Some("2.43.0".to_string());
        ctx.tool_rust_version = Some("1.83.0".to_string());
        ctx.tool_has_kubectl = true;
        let md = generate(&ctx);
        assert!(md.contains("Herramientas del Sistema"), "Should have tools section");
        assert!(md.contains("2.43.0"), "Should show git version");
        assert!(md.contains("1.83.0"), "Should show rust version");
        assert!(md.contains("kubectl"), "Should list infra tools");
    }

    #[test]
    fn generate_shows_agent_capabilities() {
        let mut ctx = make_ctx("test");
        ctx.agent_model_name = Some("claude-sonnet-4-6".to_string());
        ctx.agent_model_tier = Some("tier-2".to_string());
        ctx.agent_mcp_servers = vec!["filesystem".to_string(), "git".to_string()];
        ctx.agent_reasoning_enabled = true;
        ctx.agent_orchestration_on = true;
        let md = generate(&ctx);
        assert!(md.contains("Capacidades del Agente"), "Should have capabilities section");
        assert!(md.contains("claude-sonnet-4-6"), "Should show model name");
        assert!(md.contains("tier-2"), "Should show model tier");
        assert!(md.contains("2 activos"), "Should show MCP server count");
        assert!(md.contains("Reasoning"), "Should list active subsystems");
        assert!(md.contains("Orchestration"), "Should list orchestration");
    }

    #[test]
    fn generate_shows_no_mcp_servers_message() {
        let mut ctx = make_ctx("test");
        ctx.agent_model_name = Some("test-model".to_string());
        let md = generate(&ctx);
        assert!(md.contains("Ninguno configurado"), "Should note no MCP servers");
    }

    #[test]
    fn generate_shows_ide_context() {
        let mut ctx = make_ctx("test");
        ctx.ide_detected = Some("VSCode".to_string());
        ctx.ide_lsp_connected = true;
        ctx.ide_lsp_port = Some(5758);
        let md = generate(&ctx);
        assert!(md.contains("Contexto IDE"), "Should have IDE section");
        assert!(md.contains("VSCode"), "Should show detected IDE");
        assert!(md.contains("5758"), "Should show LSP port");
    }

    #[test]
    fn generate_shows_runtime_profile() {
        let mut ctx = make_ctx("test");
        ctx.runtime_model_router_active = true;
        ctx.runtime_convergence_controller_on = true;
        ctx.runtime_token_budget = Some(200_000);
        let md = generate(&ctx);
        assert!(md.contains("Perfil de Runtime"), "Should have runtime section");
        assert!(md.contains("ModelRouter"), "Should list active component");
        assert!(md.contains("ConvergenceController"), "Should list convergence");
    }

    #[test]
    fn generate_shows_optimization_opportunities() {
        let mut ctx = make_ctx("test");
        ctx.agent_reasoning_enabled = false;
        ctx.agent_orchestration_on = false;
        ctx.has_ci = false;
        let md = generate(&ctx);
        assert!(md.contains("Oportunidades de Optimización"), "Should have optimization section");
        assert!(md.contains("ReasoningEngine"), "Should suggest enabling reasoning");
    }

    #[test]
    fn generate_shows_wsl_container_indicators() {
        let mut ctx = make_ctx("test");
        ctx.sys_is_wsl = true;
        ctx.sys_is_container = true;
        ctx.sys_os = "Linux x86_64".to_string();
        let md = generate(&ctx);
        assert!(md.contains("WSL"), "Should note WSL environment");
        assert!(md.contains("Container"), "Should note container environment");
    }

    #[test]
    fn generate_shows_telemetry_breakdown_when_present() {
        let mut ctx = make_ctx("test");
        ctx.resource_detection_time_ms = 120;
        ctx.environment_scan_time_ms = 45;
        ctx.ide_context_integration_time_ms = 12;
        ctx.hicon_query_time_ms = 3;
        let md = generate(&ctx);
        assert!(md.contains("120ms"), "Should show resource detection time");
        assert!(md.contains("45ms"), "Should show environment scan time");
    }

    #[test]
    fn generate_shows_hicon_in_subsystems() {
        let mut ctx = make_ctx("test");
        ctx.agent_hicon_active = true;
        ctx.agent_model_name = Some("test".to_string()); // trigger section
        let md = generate(&ctx);
        assert!(md.contains("HICON"), "Should show HICON as active subsystem");
    }

    // ── Phase 110-112: Language Intelligence tests ─────────────────────────────

    #[test]
    fn generate_shows_language_section_when_primary_language_set() {
        let mut ctx = make_ctx("test");
        ctx.primary_language = "Rust".to_string();
        let md = generate(&ctx);
        assert!(md.contains("Inteligencia de Lenguajes"), "Should have language section");
        assert!(md.contains("Rust"), "Should show primary language");
    }

    #[test]
    fn generate_skips_language_section_when_empty_primary_language() {
        let ctx = make_ctx("test");
        // make_ctx does not set primary_language — it remains ""
        let md = generate(&ctx);
        assert!(!md.contains("Inteligencia de Lenguajes"), "Should skip language section when no data");
    }

    #[test]
    fn generate_shows_polyglot_flag() {
        let mut ctx = make_ctx("test");
        ctx.primary_language = "TypeScript".to_string();
        ctx.secondary_languages = vec!["Rust".to_string(), "Python".to_string()];
        ctx.is_polyglot = true;
        ctx.language_breakdown = vec![
            ("TypeScript".to_string(), 120),
            ("Rust".to_string(), 80),
            ("Python".to_string(), 30),
        ];
        let md = generate(&ctx);
        assert!(md.contains("poliglota"), "Should show polyglot flag");
        assert!(md.contains("Rust"), "Should show secondary languages");
        assert!(md.contains("TypeScript (120)"), "Should show language breakdown");
    }

    #[test]
    fn generate_shows_monorepo_section() {
        let mut ctx = make_ctx("test");
        ctx.primary_language = "Rust".to_string();
        ctx.is_monorepo = true;
        ctx.monorepo_tool = Some("Cargo Workspace".to_string());
        ctx.sub_project_count = 5;
        ctx.sub_projects = vec!["crates/core".to_string(), "crates/cli".to_string()];
        let md = generate(&ctx);
        assert!(md.contains("Monorepo"), "Should show monorepo label");
        assert!(md.contains("Cargo Workspace"), "Should show monorepo tool");
        assert!(md.contains("5"), "Should show sub-project count");
        assert!(md.contains("crates/core"), "Should list sub-projects");
    }

    #[test]
    fn generate_shows_project_scale() {
        let mut ctx = make_ctx("test");
        ctx.primary_language = "Go".to_string();
        ctx.project_scale = "Large".to_string();
        ctx.total_file_count = 12_000;
        ctx.estimated_loc = 480_000;
        let md = generate(&ctx);
        assert!(md.contains("Large"), "Should show Large scale");
        assert!(md.contains("12000"), "Should show total file count");
        assert!(md.contains("480000"), "Should show LOC estimate");
    }

    // ── Phase 113: Distributed Architecture tests ──────────────────────────────

    #[test]
    fn generate_shows_distributed_section_when_patterns_present() {
        let mut ctx = make_ctx("test");
        ctx.architecture_patterns = vec!["Microservices".to_string(), "Event-Driven".to_string()];
        ctx.distributed_services_count = 4;
        let md = generate(&ctx);
        assert!(md.contains("Arquitectura Distribuida"), "Should have distributed section");
        assert!(md.contains("Microservices"), "Should show architecture patterns");
        assert!(md.contains("4"), "Should show service count");
    }

    #[test]
    fn generate_skips_distributed_section_when_no_patterns() {
        let ctx = make_ctx("test");
        let md = generate(&ctx);
        assert!(!md.contains("Arquitectura Distribuida"), "Should skip distributed section when no data");
    }

    #[test]
    fn generate_shows_message_broker_in_distributed_section() {
        let mut ctx = make_ctx("test");
        ctx.has_message_broker = true;
        ctx.message_broker_type = Some("Kafka".to_string());
        let md = generate(&ctx);
        assert!(md.contains("Message Broker"), "Should show message broker");
        assert!(md.contains("Kafka"), "Should show broker type");
    }

    #[test]
    fn generate_shows_service_mesh_and_observability() {
        let mut ctx = make_ctx("test");
        ctx.has_service_mesh = true;
        ctx.has_observability_stack = true;
        ctx.has_api_gateway = true;
        let md = generate(&ctx);
        assert!(md.contains("Service Mesh"), "Should show service mesh");
        assert!(md.contains("Observability"), "Should show observability");
        assert!(md.contains("API Gateway"), "Should show API gateway");
    }

    // ── Phase 117: Quality Dashboard tests ─────────────────────────────────────

    #[test]
    fn generate_shows_quality_dashboard_when_advanced_scores_present() {
        let mut ctx = make_ctx("test");
        ctx.architecture_quality_score = 78;
        ctx.scalability_score = 65;
        ctx.maintainability_score = 82;
        ctx.technical_debt_score = 30;
        ctx.dev_ex_score = 70;
        ctx.ai_readiness_score = 55;
        ctx.distributed_maturity_score = 40;
        let md = generate(&ctx);
        assert!(md.contains("Dashboard de Calidad"), "Should have quality dashboard");
        assert!(md.contains("10 Métricas"), "Should say 10 metrics");
        assert!(md.contains("78/100"), "Should show architecture quality score");
        assert!(md.contains("82/100"), "Should show maintainability score");
        assert!(md.contains("Deuda Técnica"), "Should have technical debt row");
    }

    #[test]
    fn generate_skips_dashboard_when_all_advanced_scores_are_zero() {
        let mut ctx = make_ctx("test");
        // advanced scores remain 0 (default) but health_score = 75 (from make_ctx)
        // The dashboard should NOT appear because all *advanced* scores are 0
        ctx.architecture_quality_score = 0;
        ctx.scalability_score = 0;
        ctx.maintainability_score = 0;
        ctx.ai_readiness_score = 0;
        let md = generate(&ctx);
        assert!(!md.contains("Dashboard de Calidad"), "Should skip dashboard when no advanced scores");
    }

    #[test]
    fn dashboard_shows_correct_level_labels() {
        let mut ctx = make_ctx("test");
        ctx.architecture_quality_score = 85; // ◈ Alto
        ctx.scalability_score = 65;          // ◇ Medio
        ctx.maintainability_score = 40;      // ⚐ Bajo
        let md = generate(&ctx);
        assert!(md.contains("◈ Alto"), "85 should be Alto");
        assert!(md.contains("◇ Medio"), "65 should be Medio");
        assert!(md.contains("⚐ Bajo"), "40 should be Bajo");
    }

    // ── Phase 118: Capability Matrix tests ─────────────────────────────────────

    #[test]
    fn generate_shows_capability_matrix() {
        let mut ctx = make_ctx("test");
        // make_ctx has has_tests=true, has_ci=true
        let md = generate(&ctx);
        assert!(md.contains("Matriz de Capacidades"), "Should have capability matrix");
        assert!(md.contains("CI/CD"), "Should have CI/CD row");
        assert!(md.contains("Tests"), "Should have Tests row");
        assert!(md.contains("Containers"), "Should have Containers row");
        assert!(md.contains("Observability"), "Should have Observability row");
    }

    #[test]
    fn capability_matrix_shows_high_risk_for_distributed_without_observability() {
        let mut ctx = make_ctx("test");
        ctx.has_observability_stack = false;
        ctx.distributed_services_count = 3; // triggers "Alto" risk for observability
        let md = generate(&ctx);
        // When distributed_services_count >= 2 and no observability → Alto risk
        assert!(md.contains("Observability"), "Observability row present");
        // The risk column should show "Alto" for this combination
        assert!(md.contains("Alto"), "Should show Alto risk for distributed without observability");
    }

    #[test]
    fn capability_matrix_shows_message_broker_when_detected() {
        let mut ctx = make_ctx("test");
        ctx.has_message_broker = true;
        ctx.message_broker_type = Some("RabbitMQ".to_string());
        let md = generate(&ctx);
        assert!(md.contains("Message Broker"), "Should have Message Broker row");
        assert!(md.contains("RabbitMQ"), "Should show broker type in capability matrix");
    }

    // ── Phase 119: Auto-Mode Suggestion tests ──────────────────────────────────

    #[test]
    fn generate_shows_auto_mode_suggestion_when_flags_present() {
        let mut ctx = make_ctx("test");
        ctx.suggested_agent_flags = vec!["--full".to_string(), "--expert".to_string()];
        ctx.suggested_model_tier = Some("premium".to_string());
        ctx.suggested_planning_strategy = Some("adaptive".to_string());
        ctx.agent_mode_rationale = Some("Enterprise project with distributed architecture".to_string());
        let md = generate(&ctx);
        assert!(md.contains("Configuración de Agente Sugerida"), "Should have auto-mode section");
        assert!(md.contains("--full --expert"), "Should show suggested flags");
        assert!(md.contains("halcon chat"), "Should show halcon command");
        assert!(md.contains("premium"), "Should show model tier");
        assert!(md.contains("adaptive"), "Should show planning strategy");
        assert!(md.contains("Enterprise project"), "Should show rationale");
    }

    #[test]
    fn generate_skips_auto_mode_section_when_no_suggestion() {
        let ctx = make_ctx("test");
        // No suggested_agent_flags, no model tier, no rationale
        let md = generate(&ctx);
        assert!(!md.contains("Configuración de Agente Sugerida"), "Should skip auto-mode section");
    }

    #[test]
    fn generate_shows_reasoning_and_multimodal_flags_in_suggestion() {
        let mut ctx = make_ctx("test");
        ctx.suggested_agent_flags = vec!["--full".to_string()];
        ctx.activate_reasoning_deep = true;
        ctx.activate_multimodal_for_init = true;
        let md = generate(&ctx);
        assert!(md.contains("Reasoning profundo"), "Should note deep reasoning recommendation");
        assert!(md.contains("multimodal"), "Should note multimodal recommendation");
    }

    #[test]
    fn generate_shows_fast_mode_for_small_project() {
        let mut ctx = make_ctx("test");
        ctx.use_fast_mode = true;
        // No flags set — triggers the fast-mode fallback message
        let md = generate(&ctx);
        assert!(md.contains("Configuración de Agente Sugerida"), "Should show section for fast mode");
        assert!(md.contains("Modo estándar"), "Should mention standard mode for small project");
    }

    #[test]
    fn generate_shows_model_tier_descriptions() {
        let tiers = [
            ("premium", "Opus"),
            ("balanced", "Sonnet"),
            ("fast", "Haiku"),
        ];
        for (tier, expected_keyword) in tiers {
            let mut ctx = make_ctx("test");
            ctx.suggested_model_tier = Some(tier.to_string());
            ctx.suggested_agent_flags = vec!["--full".to_string()]; // trigger section
            let md = generate(&ctx);
            assert!(md.contains(expected_keyword),
                "Tier '{tier}' should mention '{expected_keyword}' in description");
        }
    }
}
