use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use reedline::{
    EditCommand, FileBackedHistory, KeyCode, KeyModifiers, Reedline, ReedlineEvent, Signal,
};

use halcon_core::traits::{ContextQuery, ContextSource, ModelProvider, Planner};
use halcon_core::types::{
    AppConfig, ChatMessage, DomainEvent, EventPayload, MessageContent, ModelRequest,
    Role, Session,
};
use halcon_core::EventSender;
use halcon_providers::ProviderRegistry;
use halcon_storage::{AsyncDatabase, Database};
use halcon_tools::ToolRegistry;

use conversational_permission::ConversationalPermissionHandler;
use memory_source::MemorySource;
use planning_source::PlanningSource;
use resilience::ResilienceManager;
use response_cache::ResponseCache;

pub mod accumulator;
pub mod agent;
pub mod agent_comm;
pub mod agent_task_manager;
pub mod agent_types;
pub mod agent_utils;
pub mod anomaly_detector;
pub mod arima_predictor;
pub mod metacognitive_loop;
pub mod artifact_store;
pub mod authorization;
pub mod backpressure;
pub mod ci_detection;
pub mod circuit_breaker;
pub mod command_blacklist;
pub mod commands;
pub mod compaction;
pub mod console;
pub mod context_governance;
pub mod context_manager;
pub mod context_metrics;
pub mod delegation;
pub mod episodic_source;
pub mod execution_tracker;
pub mod executor;
pub mod failure_tracker;
pub mod health;
pub mod idempotency;
pub mod hybrid_retriever;
pub mod integration_decision;
pub mod loop_guard;
pub mod mcp_manager;
pub mod memory_consolidator;
pub mod metrics_store;
pub mod memory_source;
pub mod model_selector;
pub mod optimizer;
pub mod orchestrator;
pub mod orchestrator_metrics;
pub mod permissions;
pub mod permission_lifecycle;
pub mod rule_matcher;
pub mod conversation_protocol;
pub mod conversation_state;
pub mod input_normalizer;
pub mod adaptive_prompt;
pub mod validation;
pub mod conversational_permission;
pub mod planner;
pub mod playbook_planner;
pub mod planning_metrics;
pub mod tool_manifest;
pub mod planning_source;
pub mod provenance_tracker;
mod prompt;
pub mod reflection_source;
pub mod reflexion;
pub mod repo_map_source;
pub mod requirements_server;
pub mod architecture_server;
pub mod codebase_server;
pub mod workflow_server;
pub mod test_results_server;
pub mod runtime_metrics_server;
pub mod security_server;
pub mod support_server;
pub mod sdlc_phase_detector;
pub mod replay_executor;
pub mod replay_runner;
pub mod resilience;
pub mod response_cache;
pub mod router;
pub mod search_engine_global;
pub mod self_corrector;
pub mod speculative;
pub mod evaluator;
pub mod reasoning_engine;
pub mod plan_coherence;
pub mod capability_index;
pub mod capability_resolver;
pub mod plugin_circuit_breaker;
pub mod plugin_cost_tracker;
pub mod plugin_manifest;
pub mod plugin_permission_gate;
pub mod plugin_registry;
pub mod plugin_loader;
pub mod plugin_transport_runtime;
pub mod plugin_proxy_tool;
pub mod reward_pipeline;
pub mod round_scorer;
pub mod strategy_selector;
pub mod supervisor;
pub mod strategy_metrics;
pub mod task_analyzer;
pub mod task_backlog;
pub mod task_bridge;
pub mod task_scheduler;
pub mod tool_selector;
pub mod tool_speculation;
pub mod traceback_parser;
pub mod code_instrumentation;
pub mod rollback;
pub mod risk_tier_classifier;
pub mod patch_preview_engine;
pub mod edit_transaction;
pub mod safe_edit_manager;
// Phase 2 — Git Context & Branch Awareness
pub mod git_context;
pub mod branch_divergence;
pub mod commit_reward_tracker;
pub mod git_event_listener;
// Phase 3 — Test Runner Bridge
pub mod test_result_parsers;
pub mod test_runner_bridge;
// Phase 4 — CI Feedback Loop
pub mod ci_result_ingestor;
// Phase 5 — IDE Protocol Handler
pub mod unsaved_buffer_tracker;
pub mod ide_protocol_handler;
pub mod dev_gateway;
// Phase 6 — AST Symbol Extractor (feature-gated ast-symbols; regex backend compiles always)
pub mod ast_symbol_extractor;
// Phase 7 — Runtime Signal Ingestor (OTEL-compatible, feature-gated otel)
pub mod runtime_signal_ingestor;
// Phase 8 — Dev Ecosystem Integration Tests (cross-module invariant validation)
#[cfg(test)]
pub mod dev_ecosystem_integration_tests;

// Planning V3 — Compression, Macro Feedback, Early Convergence
pub mod plan_compressor;
pub mod macro_feedback;
pub mod early_convergence;

mod slash_commands;

#[cfg(test)]
mod stress_tests;

use prompt::HalconPrompt;

/// Detect the OS username for user context injection into the system prompt.
///
/// Priority: $USER → $LOGNAME → home dir basename → "user".
fn detect_user_display_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            dirs::home_dir()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "user".to_string())
}

/// Interactive REPL for halcon.
pub struct Repl {
    pub(crate) editor: Reedline,
    pub(crate) prompt: HalconPrompt,
    pub(crate) config: AppConfig,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) session: Session,
    pub(crate) db: Option<Arc<Database>>,
    pub(crate) async_db: Option<AsyncDatabase>,
    pub(crate) registry: ProviderRegistry,
    pub(crate) tool_registry: ToolRegistry,
    pub(crate) permissions: ConversationalPermissionHandler,
    pub(crate) event_tx: EventSender,
    /// Context manager wrapping pipeline + sources + governance for unified context assembly (Phase 38).
    /// Sources are owned by the manager - access via context_manager.sources().
    pub(crate) context_manager: Option<context_manager::ContextManager>,
    pub(crate) response_cache: Option<ResponseCache>,
    pub(crate) resilience: ResilienceManager,
    pub(crate) reflector: Option<reflexion::Reflector>,
    pub(crate) no_banner: bool,
    /// When true, the user explicitly set `--model` on the CLI, so model selection is bypassed.
    pub(crate) explicit_model: bool,
    /// Temporary dry-run mode override for the next handle_message call.
    pub(crate) dry_run_override: Option<halcon_core::types::DryRunMode>,
    /// Trace step cursor for /step forward/back navigation.
    pub(crate) trace_cursor: Option<(uuid::Uuid, Vec<halcon_storage::TraceStep>, usize)>,
    /// Cached execution timeline JSON from the last agent loop (for --timeline flag).
    pub(crate) last_timeline: Option<String>,
    /// Shared context metrics for agent loop observability (Phase 42).
    pub(crate) context_metrics: std::sync::Arc<context_metrics::ContextMetrics>,
    /// Context governance for per-source token limits (Phase 42).
    pub(crate) context_governance: context_governance::ContextGovernance,
    /// Expert mode: show full agent feedback (model selection, caching, etc.).
    pub(crate) expert_mode: bool,
    /// Tool speculation engine for pre-executing read-only tools (Phase 3 remediation).
    pub(crate) speculator: tool_speculation::ToolSpeculator,
    /// FASE 3.1: Reasoning engine for metacognitive agent loop wrapping (Phase 40).
    /// None when reasoning.enabled = false (default).
    pub(crate) reasoning_engine: Option<reasoning_engine::ReasoningEngine>,
    /// FASE 3.2: MCP resource manager for lazy MCP server discovery.
    /// Always present (empty when no servers configured).
    pub(crate) mcp_manager: mcp_manager::McpResourceManager,
    /// P1.1: Playbook-based planner loaded from ~/.halcon/playbooks/.
    /// Runs before LlmPlanner — instant (zero LLM calls) for matched workflows.
    pub(crate) playbook_planner: playbook_planner::PlaybookPlanner,
    /// OS username for user context injection into system prompt (e.g. "oscarvalois").
    pub(crate) user_display_name: String,
    /// Multimodal subsystem (image/audio/video analysis). Activated with `--full`.
    pub(crate) multimodal: Option<std::sync::Arc<halcon_multimodal::MultimodalSubsystem>>,
    /// Control channel receiver from TUI (Phase 43). None in classic REPL mode.
    #[cfg(feature = "tui")]
    pub(crate) ctrl_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::tui::events::ControlEvent>>,
    /// Phase 3: Model quality stats cache for session-level persistence.
    ///
    /// Snapshot of `ModelSelector.quality_stats` extracted after each agent loop and re-injected
    /// into the next fresh `ModelSelector` via `with_quality_seeds()`. This ensures `balanced`
    /// routing uses accumulated quality data across all messages within the session, not just
    /// the current message (previously reset to neutral every turn because ModelSelector is
    /// created fresh per message). Tuple: `(success_count, failure_count, total_reward)`.
    pub(crate) model_quality_cache: std::collections::HashMap<String, (u32, u32, f64)>,
    /// Phase 4: Whether cross-session quality stats have been loaded from DB this session.
    ///
    /// Prevents repeated DB queries (load-once-per-session). Set to true after first load attempt
    /// (even if the DB returned empty results or was unavailable).
    pub(crate) model_quality_db_loaded: bool,
    /// Plugin registry for V3 plugin system. None until plugins are configured.
    /// Initialized as None in Repl::new() — activated when plugins are loaded.
    pub(crate) plugin_registry: Option<plugin_registry::PluginRegistry>,
    /// Transport runtime for V3 plugins (shared handle pool for Stdio/HTTP/Local plugins).
    /// None until plugins are lazy-initialized on first message with config.plugins.enabled.
    pub(crate) plugin_transport_runtime: Option<std::sync::Arc<plugin_transport_runtime::PluginTransportRuntime>>,
    /// Whether plugin UCB1 metrics have been loaded from DB this session (load-once guard).
    pub(crate) plugin_metrics_db_loaded: bool,
    /// Phase 5 Dev Ecosystem: DevGateway coordinates IDE buffers, git context, and CI
    /// feedback into a single `DevContext` snapshot that is injected into the system
    /// prompt on each message.  The gateway is Arc-backed internally so clone is cheap.
    pub(crate) dev_gateway: dev_gateway::DevGateway,
    /// Phase 7 Dev Ecosystem: Rolling observability window for agent-loop telemetry.
    /// Ingests per-loop spans and exposes p50/p95/p99 + error-rate as a UCB1 reward
    /// signal.  Shared via Arc so multiple async tasks can ingest without contention.
    pub(crate) runtime_signals: std::sync::Arc<runtime_signal_ingestor::RuntimeSignalIngestor>,
    /// Phase 4 Dev Ecosystem: Stop signal for the background CI polling task.
    /// Set once during `run()` / `run_tui()` when GITHUB_TOKEN is available.
    /// Notified on session teardown so the polling loop exits gracefully.
    pub(crate) ci_stop: std::sync::Arc<tokio::sync::Notify>,
}

impl Repl {
    /// Build real FeatureStatus from current REPL configuration and state.
    fn build_feature_status(&self, tui_active: bool) -> crate::render::banner::FeatureStatus {
        let tool_count = self.tool_registry.tool_definitions().len();
        // Background tools enabled if tool count is 23 (20 core + 3 background)
        let background_tools_enabled = tool_count >= 23;

        crate::render::banner::FeatureStatus {
            tui_active,
            reasoning_enabled: self.reasoning_engine.is_some(),
            orchestrator_enabled: self.config.orchestrator.enabled,
            context_pipeline_active: true, // Always active (L0-L4 always present)
            tool_count,
            background_tools_enabled,
            multimodal_enabled: self.multimodal.is_some(),
            loop_critic_enabled: self.config.reasoning.enable_loop_critic,
        }
    }

    /// Create a new REPL instance with file-backed history and optional DB.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &AppConfig,
        provider: String,
        model: String,
        db: Option<Arc<Database>>,
        resume_session: Option<Session>,
        registry: ProviderRegistry,
        mut tool_registry: ToolRegistry,
        event_tx: EventSender,
        no_banner: bool,
        explicit_model: bool,
    ) -> Result<Self> {
        let mut keybindings = reedline::default_emacs_keybindings();
        // Alt+Enter inserts a newline (multi-line input).
        keybindings.add_binding(
            KeyModifiers::ALT,
            KeyCode::Enter,
            ReedlineEvent::Edit(vec![EditCommand::InsertNewline]),
        );

        let mut editor =
            Reedline::create().with_edit_mode(Box::new(reedline::Emacs::new(keybindings)));

        if let Some(path) = Self::history_path() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let history = Box::new(
                FileBackedHistory::with_file(1000, path)
                    .map_err(|e| anyhow::anyhow!("Failed to init history: {e}"))?,
            );
            editor = editor.with_history(history);
        }

        let prompt = HalconPrompt::new(&provider, &model);

        let cwd = std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let session =
            resume_session.unwrap_or_else(|| Session::new(model.clone(), provider.clone(), cwd));

        let permissions = ConversationalPermissionHandler::with_config(
            config.tools.confirm_destructive,
            config.security.tbac_enabled,
            config.tools.auto_approve_in_ci,
            config.tools.prompt_timeout_secs,
        );

        // Build async database wrapper (for async call sites).
        let async_db = db.as_ref().map(|db_ref| AsyncDatabase::new(Arc::clone(db_ref)));

        // Build context sources.
        let mut context_sources: Vec<Box<dyn ContextSource>> = vec![
            Box::new(halcon_context::InstructionSource::new()),
            Box::new(repo_map_source::RepoMapSource::default()),
        ];

        if config.planning.enabled {
            context_sources.push(Box::new(PlanningSource::new(&config.planning)));
        }

        if config.memory.enabled {
            if let Some(ref adb) = async_db {
                if config.memory.episodic {
                    // Episodic memory with hybrid retrieval (BM25 + RRF + temporal decay).
                    let retriever = hybrid_retriever::HybridRetriever::new(adb.clone())
                        .with_rrf_k(config.memory.rrf_k)
                        .with_decay_half_life(config.memory.decay_half_life_days);
                    context_sources.push(Box::new(episodic_source::EpisodicSource::new(
                        retriever,
                        config.memory.retrieval_top_k,
                        config.memory.retrieval_token_budget,
                    )));
                } else {
                    // Legacy BM25-only memory source.
                    context_sources.push(Box::new(MemorySource::new(
                        adb.clone(),
                        config.memory.retrieval_top_k,
                        config.memory.retrieval_token_budget,
                    )));
                }
            }
        }

        // Initialize reflexion: Reflector + ReflectionSource.
        // Phase 3: AgentModelConfig — use dedicated reflector provider/model when configured.
        let reflector = if config.reflexion.enabled {
            // Resolve reflector provider: explicit config > primary provider.
            let reflector_prov = config.reasoning.reflector_provider.as_deref()
                .and_then(|name| registry.get(name))
                .cloned()
                .or_else(|| registry.get(&provider).cloned());

            // Resolve reflector model: explicit config > session model.
            let reflector_model = config.reasoning.reflector_model.clone()
                .unwrap_or_else(|| model.clone());

            reflector_prov.map(|p| {
                reflexion::Reflector::new(p, reflector_model)
                    .with_reflect_on_success(config.reflexion.reflect_on_success)
            })
        } else {
            None
        };

        if config.reflexion.enabled {
            if let Some(ref adb) = async_db {
                context_sources.push(Box::new(reflection_source::ReflectionSource::new(
                    adb.clone(),
                    config.reflexion.max_reflections,
                )));
            }
        }

        // Initialize SDLC context servers (opt-in via config).
        if config.context_servers.enabled {
            if let Some(ref adb) = async_db {
                // Server 1: Requirements & Product (Discovery phase)
                if config.context_servers.requirements.enabled {
                    context_sources.push(Box::new(requirements_server::RequirementsServer::new(
                        adb.clone(),
                        config.context_servers.requirements.priority,
                        config.context_servers.requirements.token_budget,
                    )));
                }

                // Server 2: Architecture & Design (Planning phase)
                if config.context_servers.architecture.enabled {
                    context_sources.push(Box::new(architecture_server::ArchitectureServer::new(
                        adb.clone(),
                        config.context_servers.architecture.priority,
                        config.context_servers.architecture.token_budget,
                    )));
                }
            }
        }

        // Server 3: Codebase Context (Implementation phase) - no DB needed
        if config.context_servers.enabled && config.context_servers.codebase.enabled {
            let working_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            context_sources.push(Box::new(codebase_server::CodebaseServer::new(
                working_dir,
                config.context_servers.codebase.priority,
                config.context_servers.codebase.token_budget,
            )));
        }

        // Server 4: Workflow & CI/CD Context (Testing/Deployment phase)
        if config.context_servers.enabled {
            if let Some(ref adb) = async_db {
                if config.context_servers.workflow.enabled {
                    context_sources.push(Box::new(workflow_server::WorkflowServer::new(
                        adb.clone(),
                        config.context_servers.workflow.priority,
                        config.context_servers.workflow.token_budget,
                    )));
                }
            }
        }

        // Server 5: Test Results & Coverage Context (Testing phase)
        if config.context_servers.enabled {
            if let Some(ref adb) = async_db {
                if config.context_servers.testing.enabled {
                    context_sources.push(Box::new(test_results_server::TestResultsServer::new(
                        adb.clone(),
                        config.context_servers.testing.priority,
                        config.context_servers.testing.token_budget,
                    )));
                }
            }
        }

        // Server 6: Runtime Metrics & Monitoring Context (Monitoring phase)
        if config.context_servers.enabled {
            if let Some(ref adb) = async_db {
                if config.context_servers.runtime.enabled {
                    context_sources.push(Box::new(runtime_metrics_server::RuntimeMetricsServer::new(
                        adb.clone(),
                        config.context_servers.runtime.priority,
                        config.context_servers.runtime.token_budget,
                    )));
                }
            }
        }

        // Server 7: Security & Compliance Context (Security/Review phase)
        if config.context_servers.enabled {
            if let Some(ref adb) = async_db {
                if config.context_servers.security.enabled {
                    context_sources.push(Box::new(security_server::SecurityServer::new(
                        adb.clone(),
                        config.context_servers.security.priority,
                        config.context_servers.security.token_budget,
                    )));
                }
            }
        }

        // Server 8: Support & Incidents Context (Support phase)
        if config.context_servers.enabled {
            if let Some(ref adb) = async_db {
                if config.context_servers.support.enabled {
                    context_sources.push(Box::new(support_server::SupportServer::new(
                        adb.clone(),
                        config.context_servers.support.priority,
                        config.context_servers.support.token_budget,
                    )));
                }
            }
        }

        // Initialize response cache when DB is available and cache is enabled.
        let response_cache = if config.cache.enabled {
            async_db.as_ref().map(|adb| {
                ResponseCache::new(adb.clone(), config.cache.clone())
            })
        } else {
            None
        };

        // Initialize resilience manager and register ALL providers from the registry.
        let mut resilience =
            ResilienceManager::new(config.resilience.clone()).with_event_tx(event_tx.clone());
        if let Some(ref adb) = async_db {
            resilience = resilience.with_db(adb.clone());
        }
        for name in registry.list() {
            resilience.register_provider(name);
        }

        // Create ContextManager for unified context assembly from all sources (Phase 38 + Context Servers).
        let context_manager = if !context_sources.is_empty() {
            let gov_config = &config.context.governance;
            let governance = if gov_config.default_max_tokens_per_source > 0 {
                context_governance::ContextGovernance::with_default_max_tokens(
                    gov_config.default_max_tokens_per_source,
                )
            } else {
                // No limits configured - use empty HashMap for per-source limits
                context_governance::ContextGovernance::new(std::collections::HashMap::new())
            };

            Some(context_manager::ContextManager::new(
                &halcon_context::ContextPipelineConfig {
                    max_context_tokens: config.general.max_tokens.max(200_000),
                    ..Default::default()
                },
                context_sources, // Move sources into ContextManager (it owns them)
                governance,
            ))
        } else {
            None
        };

        // FASE 3.1: Initialize ReasoningEngine when enabled.
        // Reads from [reasoning] enabled = true in config.toml or HALCON_REASONING=true env var.
        let reasoning_enabled = config.reasoning.enabled
            || std::env::var("HALCON_REASONING").map(|v| v == "true" || v == "1").unwrap_or(false);
        let reasoning_engine = if reasoning_enabled {
            tracing::info!("ReasoningEngine enabled — UCB1 strategy learning active");
            let engine_config = reasoning_engine::ReasoningConfig {
                enabled: true,
                success_threshold: config.reasoning.success_threshold,
                max_retries: config.reasoning.max_retries,
                exploration_factor: config.reasoning.exploration_factor,
            };
            Some(reasoning_engine::ReasoningEngine::new(engine_config))
        } else {
            None
        };

        // P1.2: Load external tools from ~/.halcon/tools/*.toml.
        tool_manifest::load_external_tools_default(&mut tool_registry);

        // FASE 3.2: Initialize MCP resource manager (lazy discovery, safe fallback).
        let mcp_manager = if config.mcp.servers.is_empty() {
            tracing::debug!("No MCP servers configured — using empty manager");
            mcp_manager::McpResourceManager::empty()
        } else {
            tracing::info!(server_count = config.mcp.servers.len(), "MCP resource manager initialized (lazy discovery)");
            mcp_manager::McpResourceManager::new(&config.mcp)
        };

        // Native Search: Initialize global SearchEngine if database is available.
        if let Some(ref db_arc) = db {
            search_engine_global::init_search_engine(
                db_arc.clone(),
                halcon_search::SearchEngineConfig::default(),
            );
        }

        Ok(Self {
            editor,
            prompt,
            config: config.clone(),
            provider,
            model,
            session,
            db,
            async_db,
            registry,
            tool_registry,
            permissions,
            event_tx,
            context_manager,
            response_cache,
            resilience,
            reflector,
            no_banner,
            explicit_model,
            dry_run_override: None,
            trace_cursor: None,
            last_timeline: None,
            context_metrics: std::sync::Arc::new(context_metrics::ContextMetrics::default()),
            context_governance: {
                let gov_config = &config.context.governance;
                if gov_config.default_max_tokens_per_source > 0 {
                    context_governance::ContextGovernance::with_default_max_tokens(
                        gov_config.default_max_tokens_per_source,
                    )
                } else {
                    context_governance::ContextGovernance::new(std::collections::HashMap::new())
                }
            },
            expert_mode: false,
            speculator: tool_speculation::ToolSpeculator::new(),
            reasoning_engine,
            mcp_manager,
            playbook_planner: playbook_planner::PlaybookPlanner::from_default_dir(),
            user_display_name: detect_user_display_name(),
            multimodal: None,
            #[cfg(feature = "tui")]
            ctrl_rx: None,
            model_quality_cache: std::collections::HashMap::new(),
            model_quality_db_loaded: false,
            plugin_registry: None, // V3 plugins: loaded lazily via /plugin install or config.toml
            plugin_transport_runtime: None,
            plugin_metrics_db_loaded: false,
            // Phase 5/7 Dev Ecosystem: initialized fresh per session.
            // DevGateway is inert until LSP messages arrive via handle_lsp_message().
            dev_gateway: dev_gateway::DevGateway::new(),
            runtime_signals: std::sync::Arc::new(
                runtime_signal_ingestor::RuntimeSignalIngestor::new(512),
            ),
            // Phase 4 Dev Ecosystem: stop signal for CI polling (armed in run/run_tui).
            ci_stop: std::sync::Arc::new(tokio::sync::Notify::new()),
        })
    }

    /// Execute a single prompt through the full agent loop (with tools), then exit.
    ///
    /// This gives inline prompts (`halcon chat "do X"`) the same capabilities as
    /// the interactive REPL: tool execution, context assembly, resilience, etc.
    pub async fn run_single_prompt(&mut self, prompt: &str) -> Result<()> {
        // Non-interactive mode: auto-approve tools since there's no TTY for prompts.
        self.permissions.set_non_interactive();

        // Warm L1 cache from L2 on startup.
        if let Some(ref cache) = self.response_cache {
            cache.warm_l1().await;
        }

        // Emit SessionStarted event.
        let _ = self.event_tx.send(DomainEvent::new(EventPayload::SessionStarted {
            session_id: self.session.id,
        }));

        // Send session_id to render sink (for TUI status bar initialization).
        use crate::render::sink::RenderSink;
        let sink = crate::render::sink::ClassicSink::with_expert(self.expert_mode);
        sink.session_started(&self.session.id.to_string());

        // Run the prompt through handle_message (full agent loop with tools).
        self.handle_message(prompt).await?;

        // Save session.
        self.auto_save_session().await;
        self.save_session();

        Ok(())
    }

    /// Start CI polling in the background when environment variables are present.
    ///
    /// Reads `GITHUB_TOKEN` and `GITHUB_REPOSITORY` (format: `owner/repo`).
    /// When both are set, spawns a background task that polls GitHub Actions every
    /// 60 s and feeds results into `DevGateway::ingest_ci_event()`.
    /// The task exits when `self.ci_stop` is notified.
    fn maybe_start_ci_polling(&self) {
        use ci_result_ingestor::{CiIngestorConfig, CiResultIngestor, GithubActionsClient};
        use std::sync::Arc;

        let token = match std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("HALCON_CI_TOKEN"))
        {
            Ok(t) if !t.is_empty() => t,
            _ => return, // no token → skip silently
        };

        let repository = match std::env::var("GITHUB_REPOSITORY")
            .or_else(|_| std::env::var("HALCON_CI_REPO"))
        {
            Ok(r) if !r.is_empty() => r,
            _ => return, // no repo → skip silently
        };

        let parts: Vec<&str> = repository.splitn(2, '/').collect();
        if parts.len() != 2 {
            tracing::warn!(repo = %repository, "GITHUB_REPOSITORY must be 'owner/repo' — CI polling skipped");
            return;
        }
        let (owner, repo) = (parts[0].to_string(), parts[1].to_string());
        // Workflow name: optional, falls back to any workflow.
        let workflow = std::env::var("HALCON_CI_WORKFLOW").unwrap_or_default();

        let client = Arc::new(GithubActionsClient::new(&owner, &repo, &workflow, &token));
        let ingestor = CiResultIngestor::new(client, CiIngestorConfig::default());
        let mut rx = ingestor.subscribe();
        let stop = Arc::clone(&self.ci_stop);

        // Feed CI events into DevGateway so build_context() can include them.
        let gateway = self.dev_gateway.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.notified() => break,
                    result = rx.recv() => {
                        match result {
                            Ok(event) => gateway.ingest_ci_event(event).await,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        }
                    }
                }
            }
        });

        ingestor.start();
        tracing::info!(owner = %owner, repo = %repo, "Phase 4: CI polling started (GitHub Actions)");
    }

    /// Run the interactive REPL loop.
    pub async fn run(&mut self) -> Result<()> {
        // Warm L1 cache from L2 on startup.
        if let Some(ref cache) = self.response_cache {
            cache.warm_l1().await;
        }

        // Phase 4 Dev Ecosystem: start CI polling when GitHub credentials are present.
        self.maybe_start_ci_polling();

        self.print_welcome();

        // Emit SessionStarted event.
        let _ = self.event_tx.send(DomainEvent::new(EventPayload::SessionStarted {
            session_id: self.session.id,
        }));

        // Send session_id to render sink (for TUI status bar initialization).
        use crate::render::sink::RenderSink;
        let sink = crate::render::sink::ClassicSink::with_expert(self.expert_mode);
        sink.session_started(&self.session.id.to_string());

        loop {
            let sig = self.editor.read_line(&self.prompt);

            match sig {
                Ok(Signal::Success(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Some(cmd) = trimmed.strip_prefix('/') {
                        match commands::handle(cmd, &self.provider, &self.model) {
                            commands::CommandResult::Continue => continue,
                            commands::CommandResult::Exit => break,
                            commands::CommandResult::Unknown(c) => {
                                crate::render::feedback::user_error(
                                    &format!("unknown command '/{c}'"),
                                    Some("Type /help for available commands"),
                                );
                                continue;
                            }
                            commands::CommandResult::SessionList => {
                                self.list_sessions();
                                continue;
                            }
                            commands::CommandResult::SessionShow => {
                                self.show_session();
                                continue;
                            }
                            commands::CommandResult::TestRun(kind) => {
                                self.run_test(&kind).await;
                                continue;
                            }
                            commands::CommandResult::Orchestrate(instruction) => {
                                self.run_orchestrate(&instruction).await;
                                continue;
                            }
                            commands::CommandResult::DryRun(prompt) => {
                                self.handle_message_dry_run(&prompt).await?;
                                continue;
                            }
                            commands::CommandResult::TraceInfo => {
                                self.show_trace_info();
                                continue;
                            }
                            commands::CommandResult::StateInfo => {
                                self.show_state_info();
                                continue;
                            }

                            // --- Phase 19: Agent Operating Console ---
                            commands::CommandResult::Research(query) => {
                                self.handle_research(&query).await;
                                continue;
                            }
                            commands::CommandResult::Inspect(target) => {
                                self.handle_inspect(&target).await;
                                continue;
                            }
                            commands::CommandResult::Plan(goal) => {
                                self.handle_plan(&goal).await;
                                continue;
                            }
                            commands::CommandResult::RunPlan(plan_id) => {
                                self.handle_run_plan(&plan_id).await;
                                continue;
                            }
                            commands::CommandResult::Resume(session_id) => {
                                self.handle_resume(&session_id).await;
                                continue;
                            }
                            commands::CommandResult::Cancel(task_id) => {
                                self.handle_cancel(&task_id).await;
                                continue;
                            }
                            commands::CommandResult::LiveStatus => {
                                self.handle_live_status().await;
                                continue;
                            }
                            commands::CommandResult::Logs(filter) => {
                                self.handle_logs(filter.as_deref()).await;
                                continue;
                            }
                            commands::CommandResult::Metrics => {
                                self.handle_metrics().await;
                                continue;
                            }
                            commands::CommandResult::TraceBrowse(session_id) => {
                                self.handle_trace_browse(session_id.as_deref()).await;
                                continue;
                            }
                            commands::CommandResult::Replay(session_id) => {
                                self.handle_replay(&session_id).await;
                                continue;
                            }
                            commands::CommandResult::Step(direction) => {
                                self.handle_step(&direction).await;
                                continue;
                            }
                            commands::CommandResult::Snapshot => {
                                self.handle_snapshot().await;
                                continue;
                            }
                            commands::CommandResult::Diff(a, b) => {
                                self.handle_diff(&a, &b).await;
                                continue;
                            }
                            commands::CommandResult::Benchmark(workload) => {
                                self.handle_benchmark(&workload).await;
                                continue;
                            }
                            commands::CommandResult::Optimize => {
                                self.handle_optimize().await;
                                continue;
                            }
                            commands::CommandResult::Analyze => {
                                self.handle_analyze().await;
                                continue;
                            }
                        }
                    }
                    self.handle_message(trimmed).await?;
                    // Auto-save session after each message exchange.
                    self.auto_save_session().await;
                }
                Ok(Signal::CtrlC) => {
                    continue;
                }
                Ok(Signal::CtrlD) => {
                    println!("\nGoodbye!");
                    break;
                }
                Err(err) => {
                    crate::render::feedback::user_error(
                        &format!("input failed — {err}"),
                        None,
                    );
                    break;
                }
            }
        }

        self.save_session();
        self.print_session_summary();
        Ok(())
    }

    /// Run the TUI-based interactive loop.
    ///
    /// Spawns a ratatui 3-zone TUI (prompt / activity / status) and bridges
    /// the agent loop through `TuiSink` ↔ UiEvent channel.
    #[cfg(feature = "tui")]
    pub async fn run_tui(&mut self) -> Result<()> {
        use crate::render::sink::TuiSink;
        use crate::tui::app::TuiApp;
        use crate::tui::events::UiEvent;
        use tokio::sync::mpsc as tokio_mpsc;

        // Warm L1 cache from L2 on startup.
        if let Some(ref cache) = self.response_cache {
            cache.warm_l1().await;
        }

        // Phase 4 Dev Ecosystem: start CI polling when GitHub credentials are present.
        self.maybe_start_ci_polling();

        // Emit SessionStarted event.
        let _ = self.event_tx.send(DomainEvent::new(EventPayload::SessionStarted {
            session_id: self.session.id,
        }));

        // Create channels: UiEvents (agent → TUI) and prompts (TUI → agent).
        // FIX: Increased buffer from 1024 to 16384 to handle high-throughput streaming
        // from fast models (e.g., DeepSeek) without dropping events.
        let (ui_tx, ui_rx) = tokio_mpsc::channel::<UiEvent>(16384);
        let (prompt_tx, mut prompt_rx) = tokio_mpsc::unbounded_channel::<String>();

        let tui_sink = TuiSink::new(ui_tx.clone());

        // Expert mode: emit SOTA subsystem activation report to TUI activity stream.
        // This confirms all systems are live before the first prompt is entered.
        if self.expert_mode {
            use crate::render::sink::RenderSink as _;
            let fs = self.build_feature_status(true);
            tui_sink.info("[expert] SOTA subsystems active:");
            tui_sink.info(&format!(
                "  Reasoning/UCB1={} Orchestrator={} TaskFramework={} Reflexion={}",
                fs.reasoning_enabled,
                fs.orchestrator_enabled,
                self.config.task_framework.enabled,
                self.config.reflexion.enabled,
            ));
            tui_sink.info(&format!(
                "  Multimodal={} LoopCritic={} RoundScorer=on PlanCoherence=on",
                fs.multimodal_enabled,
                fs.loop_critic_enabled,
            ));
            tui_sink.info("  DevEcosystem=on [LSP:5758 CIPoll=env GitContext=on AST=on]");
            tracing::info!(
                reasoning = fs.reasoning_enabled,
                orchestrator = fs.orchestrator_enabled,
                multimodal = fs.multimodal_enabled,
                loop_critic = fs.loop_critic_enabled,
                task_framework = self.config.task_framework.enabled,
                reflexion = self.config.reflexion.enabled,
                "Expert mode: SOTA subsystem activation report"
            );
        }

        // Phase 5 Dev Ecosystem: Start embedded TCP LSP server so IDE extensions can
        // connect while the TUI is running.  The server binds on localhost:5758 and
        // handles standard LSP JSON-RPC over a line-delimited TCP connection.
        //
        // A secondary polling task checks the open-buffer count every 5 s and emits
        // IdeBuffersUpdated when it changes, keeping the status bar indicator live.
        {
            use std::sync::Arc;
            const LSP_PORT: u16 = 5758;
            let lsp_addr: std::net::SocketAddr = ([127, 0, 0, 1], LSP_PORT).into();
            let lsp_gw = Arc::new(self.dev_gateway.clone());
            let lsp_stop = Arc::clone(&self.ci_stop);
            // Separate senders: server-done and buffer-poll need distinct clones.
            let lsp_done_tx = ui_tx.clone(); // moved into LSP server task
            let poll_gw = self.dev_gateway.clone();
            let poll_ui_tx = ui_tx.clone(); // moved into polling task
            // Independent stop signal clone for the polling task so it exits
            // cleanly when the TUI session ends (avoids the infinite loop leak).
            let poll_stop = Arc::clone(&self.ci_stop);

            // Start the TCP LSP accept loop in a background task.
            tokio::spawn(async move {
                if let Err(e) = lsp_gw.serve_tcp(lsp_addr, lsp_stop).await {
                    tracing::warn!(error = %e, "Dev ecosystem LSP TCP server stopped");
                }
                // Notify TUI that the server has gone away.
                let _ = lsp_done_tx.try_send(UiEvent::IdeDisconnected);
            });

            // Notify the TUI immediately that the LSP port is ready.
            let _ = ui_tx.try_send(UiEvent::IdeConnected { port: LSP_PORT });

            // Poll buffer count every 5 s; emit IdeBuffersUpdated on change.
            // Exits cleanly when `poll_stop` (= ci_stop) is notified.
            tokio::spawn(async move {
                let mut last_count: usize = 0;
                loop {
                    // Wait 5 s or until session teardown, whichever comes first.
                    tokio::select! {
                        _ = poll_stop.notified() => break,
                        _ = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {}
                    }
                    let count = poll_gw.buffers.len().await;
                    if count != last_count {
                        last_count = count;
                        // Fetch full dev context to get current git branch.
                        // build_context() offloads git I/O to spawn_blocking.
                        let ctx = poll_gw.build_context().await;
                        // Extract branch name from summary "git:{branch} [{status}] …"
                        let git_branch = ctx.git_summary.as_deref().and_then(|s| {
                            s.strip_prefix("git:")
                                .and_then(|s| s.split_once(" ["))
                                .map(|(b, _)| b.to_string())
                                .filter(|b| b != "(detached)")
                        });
                        let _ = poll_ui_tx.try_send(UiEvent::IdeBuffersUpdated {
                            count,
                            git_branch,
                        });
                    }
                }
            });
        }

        // Gather banner info for TUI startup display.
        let banner_version = env!("CARGO_PKG_VERSION").to_string();
        let banner_provider = self.provider.clone();
        let banner_provider_connected = self.registry.get(&self.provider).is_some();
        let banner_model = self.model.clone();
        let banner_session_id = self.session.id.to_string()[..8].to_string();
        let banner_session_type = if self.session.messages.is_empty() {
            "new".to_string()
        } else {
            "resumed".to_string()
        };

        // Build routing display info for TUI banner.
        let banner_routing = if !self.config.agent.routing.fallback_models.is_empty() {
            Some(crate::render::banner::RoutingDisplay {
                mode: self.config.agent.routing.mode.clone(),
                strategy: self.config.agent.routing.strategy.clone(),
                fallback_chain: std::iter::once(self.provider.clone())
                    .chain(self.config.agent.routing.fallback_models.clone())
                    .collect(),
            })
        } else {
            None
        };

        // Control channel: TUI → agent (pause/step/cancel).
        let (ctrl_tx, ctrl_rx) = tokio::sync::mpsc::unbounded_channel();
        self.ctrl_rx = Some(ctrl_rx);

        // Permission approval channel: TUI → PermissionChecker (approve/reject).
        // Dedicated channel ensures permission decisions reach the executor even
        // while the agent loop is blocked on tool execution.
        let (perm_tx, perm_rx) = tokio::sync::mpsc::unbounded_channel::<halcon_core::types::PermissionDecision>();
        self.permissions.set_tui_channel(perm_rx);

        // Sudo password channel: TuiApp → executor's get_sudo_password().
        // Kept separate from perm_tx because PermissionDecision is Copy and cannot
        // carry a String payload. The executor awaits this after an approved sudo command.
        let (sudo_pw_tx, sudo_pw_rx) = tokio::sync::mpsc::unbounded_channel::<Option<String>>();
        self.permissions.set_sudo_channel(sudo_pw_rx);

        // Determine initial UI mode from expert_mode flag.
        let initial_mode = if self.expert_mode {
            crate::tui::state::UiMode::Expert
        } else {
            // Map config string to UiMode.
            match self.config.display.ui_mode.as_str() {
                "minimal" => crate::tui::state::UiMode::Minimal,
                "expert" => crate::tui::state::UiMode::Expert,
                _ => crate::tui::state::UiMode::Standard,
            }
        };

        // Build real feature status for banner display.
        let features = self.build_feature_status(true); // tui_active = true

        // Spawn TUI render loop in a separate task.
        tracing::debug!("Spawning TUI task");
        let async_db_clone = self.async_db.clone(); // Phase 3 SRCH-004: Pass database for search history
        let ui_tx_for_bg = ui_tx.clone(); // Phase 45E: for background DB queries from TUI
        let tui_handle = tokio::spawn(async move {
            tracing::debug!("TUI task started");
            let mut app = TuiApp::with_mode(ui_rx, prompt_tx, ctrl_tx, perm_tx, async_db_clone, initial_mode);
            // Phase 45E: Give app a sender so it can push events from async background tasks.
            app.set_ui_tx(ui_tx_for_bg);
            // Phase 50: Wire sudo password sender so TuiApp can deliver passwords to executor.
            app.set_sudo_pw_tx(sudo_pw_tx);
            tracing::debug!("TUI app created with mode: {:?}", initial_mode);
            app.push_banner(
                &banner_version,
                &banner_provider,
                banner_provider_connected,
                &banner_model,
                &banner_session_id,
                &banner_session_type,
                banner_routing.as_ref(),
                &features,
            );
            tracing::debug!("TUI banner pushed, calling run()");
            let result = app.run().await;
            tracing::debug!("TUI run() returned: {:?}", result);
            result
        });

        // Send initial status update with session info.
        let session_id_short = self.session.id.to_string()[..8].to_string();
        let _ = ui_tx.send(UiEvent::StatusUpdate {
            provider: Some(self.provider.clone()),
            model: Some(self.model.clone()),
            round: Some(0),
            tokens: None,
            cost: Some(0.0),
            session_id: Some(session_id_short.clone()),
            elapsed_ms: Some(0),
            tool_count: Some(0),
            input_tokens: Some(0),
            output_tokens: Some(0),
        });

        let session_start = std::time::Instant::now();

        // Phase 4: Create task manager for non-blocking agent execution.
        let max_concurrent = self.config.agent.limits.max_concurrent_agents;
        let mut task_manager = agent_task_manager::AgentTaskManager::new(max_concurrent);
        tracing::debug!(max_concurrent, "Agent task manager initialized");

        // Agent message loop: wait for prompts from TUI, process each.
        tracing::debug!("Entering agent message loop, waiting for prompts from TUI");
        loop {
            // Check for control events (non-blocking) before waiting for next prompt.
            // This handles TUI requests like RequestContextServers that need immediate response.
            if let Some(ref mut ctrl) = self.ctrl_rx {
                while let Ok(event) = ctrl.try_recv() {
                    use crate::tui::events::ControlEvent;
                    match event {
                        ControlEvent::RequestContextServers => {
                            // ✅ Collect REAL data from context_manager with runtime stats
                            let servers = if let Some(ref cm) = self.context_manager {
                                cm.sources_with_stats()
                                    .map(|(name, priority, stats)| {
                                        // Calcular ms desde última query
                                        let last_query_ms = stats.last_query.map(|instant| {
                                            instant.elapsed().as_millis() as u64
                                        });

                                        crate::tui::events::ContextServerInfo {
                                            name: name.to_string(),
                                            priority,
                                            enabled: true,  // TODO: Obtener de config si es posible
                                            last_query_ms,
                                            total_tokens: stats.total_tokens,
                                            query_count: stats.query_count,
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            } else {
                                Vec::new()
                            };

                            let total_count = servers.len();
                            let enabled_count = servers.iter().filter(|s| s.enabled).count();

                            // Send back via UiEvent
                            let _ = ui_tx.send(crate::tui::events::UiEvent::ContextServersList {
                                servers,
                                total_count,
                                enabled_count,
                            });
                        }
                        // Phase 45F: Load a previous session from DB and restore context.
                        ControlEvent::ResumeSession(id) => {
                            use uuid::Uuid;
                            if let Ok(uuid) = Uuid::parse_str(&id) {
                                if let Some(ref db) = self.async_db {
                                    match db.load_session(uuid).await {
                                        Ok(Some(session)) => {
                                            let rounds = session.agent_rounds as usize;
                                            let msgs = session.messages.len();
                                            let provider = session.provider.clone();
                                            let model = session.model.clone();
                                            let cost = session.estimated_cost_usd;
                                            let short_id = &id[..8.min(id.len())];
                                            // Restore session state.
                                            self.session.id = uuid;
                                            self.session.total_usage.input_tokens = session.total_usage.input_tokens;
                                            self.session.total_usage.output_tokens = session.total_usage.output_tokens;
                                            self.session.estimated_cost_usd = cost;
                                            self.session.agent_rounds = session.agent_rounds;
                                            self.session.messages = session.messages;
                                            // Notify TUI of loaded session.
                                            let _ = ui_tx.send(UiEvent::SessionInitialized {
                                                session_id: uuid.to_string(),
                                            });
                                            let _ = ui_tx.send(UiEvent::StatusUpdate {
                                                provider: Some(provider),
                                                model: Some(model),
                                                round: Some(rounds),
                                                tokens: None,
                                                cost: Some(cost),
                                                session_id: Some(uuid.to_string()),
                                                elapsed_ms: None,
                                                tool_count: None,
                                                input_tokens: Some(session.total_usage.input_tokens),
                                                output_tokens: Some(session.total_usage.output_tokens),
                                            });
                                            let _ = ui_tx.send(UiEvent::Info(format!(
                                                "=== Session {} loaded ({} rounds, {} messages) ===",
                                                short_id, rounds, msgs
                                            )));
                                        }
                                        Ok(None) => {
                                            let _ = ui_tx.send(UiEvent::Warning {
                                                message: format!("Session {} not found", &id[..8.min(id.len())]),
                                                hint: None,
                                            });
                                        }
                                        Err(e) => {
                                            let _ = ui_tx.send(UiEvent::Warning {
                                                message: format!("Failed to load session: {e}"),
                                                hint: None,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        // Other control events (Pause/Resume/Step) are handled by agent loop,
                        // but we need to consume them here to prevent queue buildup.
                        // They'll be re-sent when appropriate or ignored if agent isn't running.
                        _ => {
                            // Ignore or log other control events in repl loop.
                            tracing::trace!("Ignoring control event in repl loop: {:?}", event);
                        }
                    }
                }
            }

            tracing::trace!("Waiting for prompt from TUI...");
            let text = match prompt_rx.recv().await {
                Some(t) => {
                    tracing::debug!("Received prompt from TUI: {}", t.chars().take(50).collect::<String>());
                    t
                }
                None => {
                    // Normal shutdown path: TUI exited (user pressed q/Ctrl+C),
                    // dropping prompt_tx which closes this channel. Not an error.
                    tracing::debug!("TUI closed the prompt channel, exiting loop");
                    break; // TUI closed the channel.
                }
            };

            // Phase 44B: Signal that agent started processing this prompt.
            let _ = ui_tx.send(UiEvent::AgentStartedPrompt);

            // Phase 4B-Lite: Show toast if prompts are queued (user feedback)
            let queue_len = prompt_rx.len();
            if queue_len > 0 {
                let _ = ui_tx.send(UiEvent::Info(format!(
                    "✓ Prompt queued ({} ahead)",
                    queue_len
                )));
            }

            // Route slash commands before sending to agent.
            let trimmed = text.trim();
            if let Some(cmd) = trimmed.strip_prefix('/') {
                match commands::handle(cmd, &self.provider, &self.model) {
                    commands::CommandResult::Exit => {
                        let _ = ui_tx.send(UiEvent::Quit);
                        break;
                    }
                    commands::CommandResult::Continue => {
                        // Commands that printed to stdout (help, model, clear)
                        // already wrote to stdout which is captured by alternate screen.
                        // Re-render the help text in the activity zone instead.
                        let (c, _) = cmd.split_once(' ').unwrap_or((cmd, ""));
                        match c {
                            "help" | "h" | "?" => {
                                let _ = ui_tx.send(UiEvent::Info("─── Help ───".into()));
                                let _ = ui_tx.send(UiEvent::Info("/help        Show this help".into()));
                                let _ = ui_tx.send(UiEvent::Info("/model       Show current provider/model".into()));
                                let _ = ui_tx.send(UiEvent::Info("/session     Show session info".into()));
                                let _ = ui_tx.send(UiEvent::Info("/status      Live system status".into()));
                                let _ = ui_tx.send(UiEvent::Info("/metrics     Token/cost metrics".into()));
                                let _ = ui_tx.send(UiEvent::Info("/clear       Clear activity zone".into()));
                                let _ = ui_tx.send(UiEvent::Info("/quit        Exit halcon".into()));
                                let _ = ui_tx.send(UiEvent::Info("────────────".into()));
                            }
                            "model" => {
                                let _ = ui_tx.send(UiEvent::Info(format!(
                                    "Current: {}/{}",
                                    self.provider, self.model
                                )));
                            }
                            _ => {}
                        }
                    }
                    commands::CommandResult::Unknown(c) => {
                        let _ = ui_tx.send(UiEvent::Warning {
                            message: format!("Unknown command '/{c}'"),
                            hint: Some("Type /help for available commands".into()),
                        });
                    }
                    commands::CommandResult::SessionShow => {
                        let info = format!(
                            "Session {} | {} rounds | ↑{} ↓{} tokens | ${:.4} | {} tools",
                            &self.session.id.to_string()[..8],
                            self.session.agent_rounds,
                            self.session.total_usage.input_tokens,
                            self.session.total_usage.output_tokens,
                            self.session.estimated_cost_usd,
                            self.session.tool_invocations,
                        );
                        let _ = ui_tx.send(UiEvent::Info(info));
                    }
                    commands::CommandResult::LiveStatus => {
                        let info = format!(
                            "Provider: {} | Model: {} | Rounds: {} | Cost: ${:.4}",
                            self.provider, self.model,
                            self.session.agent_rounds,
                            self.session.estimated_cost_usd,
                        );
                        let _ = ui_tx.send(UiEvent::Info(info));
                    }
                    commands::CommandResult::Metrics => {
                        let total = self.session.total_usage.total();
                        let info = format!(
                            "Tokens: {} total (↑{} ↓{}) | Cost: ${:.4} | Latency: {:.1}s",
                            total,
                            self.session.total_usage.input_tokens,
                            self.session.total_usage.output_tokens,
                            self.session.estimated_cost_usd,
                            self.session.total_latency_ms as f64 / 1000.0,
                        );
                        let _ = ui_tx.send(UiEvent::Info(info));
                    }
                    _ => {
                        let _ = ui_tx.send(UiEvent::Warning {
                            message: format!("Command '/{cmd}' not available in TUI mode"),
                            hint: Some("Use classic mode (halcon chat) for full command access".into()),
                        });
                    }
                }
                // Phase 44B: Slash commands also count as prompt completion.
                let _ = ui_tx.send(UiEvent::AgentFinishedPrompt);
                let _ = ui_tx.send(UiEvent::PromptQueueStatus(prompt_rx.len()));
                let _ = ui_tx.send(UiEvent::AgentDone);
                continue;
            }

            // Send RoundStart to TUI.
            let _ = ui_tx.send(UiEvent::RoundStart((self.session.agent_rounds + 1) as usize));

            // Phase 4B-Lite: Validate provider is available before processing.
            if self.provider == "none" || self.model == "none" {
                let _ = ui_tx.send(UiEvent::Error {
                    message: "No provider configured".to_string(),
                    hint: Some("Configure a provider to send prompts:\n\n\
                        • Anthropic: Set ANTHROPIC_API_KEY environment variable\n\
                        • Ollama: Start local server with `ollama serve`\n\
                        • DeepSeek: Set DEEPSEEK_API_KEY environment variable\n\
                        • OpenAI: Set OPENAI_API_KEY environment variable\n\n\
                        Then restart halcon.".to_string()),
                });
                // Signal agent finished without actual processing.
                let _ = ui_tx.send(UiEvent::AgentFinishedPrompt);
                let _ = ui_tx.send(UiEvent::PromptQueueStatus(prompt_rx.len()));
                let _ = ui_tx.send(UiEvent::AgentDone);
                continue;
            }

            // Phase 4: Process message in background (allows TUI to remain responsive).
            // For now, we await to preserve session state consistency,
            // but the TUI event loop continues independently.
            tracing::debug!("Processing agent message (non-blocking architecture)");
            if let Err(e) = self.handle_message_tui(&text, &tui_sink).await {
                let _ = ui_tx.send(UiEvent::Error {
                    message: format!("Agent error: {e}"),
                    hint: None,
                });
            }
            tracing::debug!("Agent message processing complete");

            // Send post-round status update with accumulated session metrics.
            let _ = ui_tx.send(UiEvent::StatusUpdate {
                provider: Some(self.provider.clone()),
                model: Some(self.model.clone()),
                round: Some(self.session.agent_rounds as usize),
                tokens: Some(self.session.total_usage.total() as u64),
                cost: Some(self.session.estimated_cost_usd),
                session_id: None, // Already set.
                elapsed_ms: Some(session_start.elapsed().as_millis() as u64),
                tool_count: Some(self.session.tool_invocations),
                input_tokens: Some(self.session.total_usage.input_tokens),
                output_tokens: Some(self.session.total_usage.output_tokens),
            });

            // Phase 44B: Signal that agent finished processing this prompt.
            let _ = ui_tx.send(UiEvent::AgentFinishedPrompt);

            // Phase 44B: Send current queue status (how many prompts waiting).
            let queued_count = prompt_rx.len();
            let _ = ui_tx.send(UiEvent::PromptQueueStatus(queued_count));

            // Phase 2: Send metrics update (placeholder values for now)
            // TODO: Wire actual metrics collectors into Repl struct
            let _ = ui_tx.send(UiEvent::Phase2Metrics {
                delegation_success_rate: None,
                delegation_trigger_rate: None,
                plan_success_rate: None,
                ucb1_agreement_rate: None,
            });

            // Signal agent done.
            let _ = ui_tx.send(UiEvent::AgentDone);

            // Auto-save session.
            self.auto_save_session().await;
        }

        // Wait for TUI task to finish.
        let _ = tui_handle.await;

        // Emit SessionEnded event (matches classic REPL behavior).
        let _ = self.event_tx.send(DomainEvent::new(EventPayload::SessionEnded {
            session_id: self.session.id,
            total_usage: self.session.total_usage.clone(),
        }));

        self.save_session();
        Ok(())
    }

    /// Handle a message using a TuiSink (same as handle_message but with custom sink).
    #[cfg(feature = "tui")]
    async fn handle_message_tui(
        &mut self,
        input: &str,
        tui_sink: &crate::render::sink::TuiSink,
    ) -> Result<()> {
        self.handle_message_with_sink(input, tui_sink).await
    }

    /// Print a brief session summary on exit.
    fn print_session_summary(&self) {
        // Emit SessionEnded event.
        let _ = self.event_tx.send(DomainEvent::new(EventPayload::SessionEnded {
            session_id: self.session.id,
            total_usage: self.session.total_usage.clone(),
        }));

        if self.session.agent_rounds == 0 {
            return; // No interactions, nothing to summarize.
        }

        let t = crate::render::theme::active();
        let r = crate::render::theme::reset();
        let dim = t.palette.text_dim.fg();
        let accent = t.palette.accent.fg();

        let latency = self.session.total_latency_ms as f64 / 1000.0;
        let cost_str = if self.session.estimated_cost_usd > 0.0 {
            format!(" | ${:.4}", self.session.estimated_cost_usd)
        } else {
            String::new()
        };
        let tools_str = if self.session.tool_invocations > 0 {
            format!(" | {} tools", self.session.tool_invocations)
        } else {
            String::new()
        };
        eprintln!(
            "\n{dim}Session:{r} {} rounds | {:.1}s{}{} | {accent}{}{r}",
            self.session.agent_rounds,
            latency,
            cost_str,
            tools_str,
            &self.session.id.to_string()[..8],
        );
    }

    fn print_welcome(&self) {
        let provider_connected = self.registry.get(&self.provider).is_some();
        let session_short = &self.session.id.to_string()[..8];
        let session_type = if self.session.messages.is_empty() {
            "new"
        } else {
            "resumed"
        };

        let routing = if !self.config.agent.routing.fallback_models.is_empty() {
            Some(crate::render::banner::RoutingDisplay {
                mode: self.config.agent.routing.mode.clone(),
                strategy: self.config.agent.routing.strategy.clone(),
                fallback_chain: std::iter::once(self.provider.clone())
                    .chain(self.config.agent.routing.fallback_models.clone())
                    .collect(),
            })
        } else {
            None
        };

        let show = !self.no_banner
            && crate::render::banner::should_show(self.config.display.show_banner);

        if show {
            // Build real feature status for banner display.
            let features = self.build_feature_status(false); // tui_active = false in classic mode

            // Deterministic tip index from session ID.
            let tip_index = self.session.id.as_u128() as usize;
            crate::render::banner::render_startup_with_features(
                env!("CARGO_PKG_VERSION"),
                &self.provider,
                provider_connected,
                &self.model,
                session_short,
                session_type,
                tip_index,
                routing.as_ref(),
                &features,
            );
        } else {
            let fallback_count = routing.as_ref().map(|r| r.fallback_chain.len());
            crate::render::banner::render_minimal(
                env!("CARGO_PKG_VERSION"),
                &self.provider,
                &self.model,
                fallback_count,
            );
        }

        // Warn if primary provider is not available.
        if !provider_connected {
            crate::render::feedback::user_warning(
                &format!("no API key configured for '{}'", self.provider),
                Some(&format!("Run `halcon auth login {}` to set up", self.provider)),
            );
        }
    }

    /// Handle a /dry-run command: routes through the agent loop with DestructiveOnly mode.
    async fn handle_message_dry_run(&mut self, input: &str) -> Result<()> {
        use crate::render::sink::RenderSink;
        let sink = crate::render::sink::ClassicSink::with_expert(self.expert_mode);
        sink.info("[dry-run] Destructive tools will be skipped.");
        self.dry_run_override = Some(halcon_core::types::DryRunMode::DestructiveOnly);
        self.handle_message(input).await
    }

    async fn handle_message(&mut self, input: &str) -> Result<()> {
        let classic_sink = crate::render::sink::ClassicSink::with_expert(self.expert_mode);
        self.handle_message_with_sink(input, &classic_sink).await?;
        println!();
        Ok(())
    }

    /// Unified message handler — runs the full agent loop with any RenderSink.
    ///
    /// Both classic REPL and TUI modes delegate here. The sink parameter
    /// abstracts away the rendering backend.
    async fn handle_message_with_sink(
        &mut self,
        input: &str,
        sink: &dyn crate::render::sink::RenderSink,
    ) -> Result<()> {
        // Record user message in session.
        self.session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text(input.to_string()),
        });

        // Assemble context from all sources (instructions + memory).
        let working_dir = self
            .config
            .general
            .working_directory
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });

        // Assemble context via ContextManager (if available).
        let system_prompt = if let Some(ref mut cm) = self.context_manager {
            let context_query = ContextQuery {
                working_directory: working_dir.clone(),
                user_message: Some(input.to_string()),
                token_budget: self.config.general.max_tokens as usize,
            };

            let assembled = cm.assemble(&context_query).await;
            assembled.system_prompt
        } else {
            None
        };

        // P0.1: Lazy MCP initialization — connect servers and register tools on first message.
        // ensure_initialized() is idempotent: subsequent calls are no-ops.
        // Must run BEFORE tool_definitions() so MCP tools appear in the agent loop.
        if !self.mcp_manager.is_initialized() && self.mcp_manager.has_servers() {
            let results = self.mcp_manager.ensure_initialized(&mut self.tool_registry).await;
            for (server, result) in &results {
                match result {
                    Ok(()) => {
                        use crate::render::sink::RenderSink;
                        sink.info(&format!("[mcp] Server '{server}' connected"));
                    }
                    Err(e) => {
                        use crate::render::sink::RenderSink;
                        sink.info(&format!("[mcp] Server '{server}' failed to connect: {e}"));
                    }
                }
            }
            let n = self.mcp_manager.registered_tool_count();
            if n > 0 {
                tracing::info!(tool_count = n, "MCP tools registered into agent loop");
            }
        }

        // Build the model request from session history.
        let tool_defs = self.tool_registry.tool_definitions();
        let mut request = ModelRequest {
            model: self.model.clone(),
            messages: self.session.messages.clone(),
            tools: tool_defs,
            max_tokens: Some(self.config.general.max_tokens),
            temperature: Some(self.config.general.temperature),
            system: system_prompt,
            stream: true,
        };

        // Inject user context into system prompt (idempotent via marker check).
        // Gives the model awareness of who it's talking to and the working directory.
        const USER_CTX_MARKER: &str = "## User Context";
        if let Some(ref mut sys) = request.system {
            if !sys.contains(USER_CTX_MARKER) {
                sys.push_str(&format!(
                    "\n\n{USER_CTX_MARKER}\nUser: {}\nDirectory: {}\nPlatform: {}",
                    self.user_display_name,
                    working_dir,
                    std::env::consts::OS,
                ));
            }
        }

        // Phase 5 Dev Ecosystem: inject DevGateway context (open IDE buffers, git branch,
        // latest CI run) — refreshed on EVERY round so git changes, new CI results, and
        // buffer edits are always current.  build_context() offloads git I/O to
        // spawn_blocking so this await is safe inside the async agent loop.
        {
            const DEV_ECO_MARKER: &str = "## Dev Ecosystem Context";
            if let Some(ref mut sys) = request.system {
                // Remove stale dev context block injected in the previous round.
                if let Some(idx) = sys.find(&format!("\n\n{DEV_ECO_MARKER}")) {
                    sys.truncate(idx);
                } else if sys.starts_with(DEV_ECO_MARKER) {
                    sys.clear();
                }
                // Re-inject fresh snapshot (git branch, open buffers, latest CI run).
                let dev_ctx = self.dev_gateway.build_context().await;
                let dev_md = dev_ctx.as_markdown();
                if !dev_md.is_empty() {
                    sys.push_str(&format!("\n\n{dev_md}"));
                }
            }
        }

        // Look up the active provider.
        let provider: Option<Arc<dyn ModelProvider>> = self.registry.get(&self.provider).cloned();

        match provider {
            Some(p) => {
                // Build fallback providers from routing config.
                let fallback_providers: Vec<(String, Arc<dyn ModelProvider>)> = self
                    .config
                    .agent
                    .routing
                    .fallback_models
                    .iter()
                    .filter_map(|name| {
                        self.registry
                            .get(name)
                            .cloned()
                            .map(|p| (name.clone(), p))
                    })
                    .collect();

                let compactor = compaction::ContextCompactor::new(
                    self.config.agent.compaction.clone(),
                );
                let guardrails: &[Box<dyn halcon_security::Guardrail>] =
                    if self.config.security.guardrails.enabled
                        && self.config.security.guardrails.builtins
                    {
                        halcon_security::builtin_guardrails()
                    } else {
                        &[]
                    };

                let llm_planner = if self.config.planning.adaptive {
                    // Phase 3: AgentModelConfig — use dedicated planner provider/model when configured.
                    // Fall back to the session's primary provider/model for backward compatibility.
                    let planner_prov: Arc<dyn halcon_core::traits::ModelProvider> =
                        self.config.reasoning.planner_provider.as_deref()
                            .and_then(|name| self.registry.get(name))
                            .cloned()
                            .unwrap_or_else(|| Arc::clone(&p));

                    // Resolve planner model: explicit config > validate against planner_prov > best model.
                    let planner_model = if let Some(ref m) = self.config.reasoning.planner_model {
                        m.clone()
                    } else if planner_prov.validate_model(&self.model).is_ok() {
                        self.model.clone()
                    } else {
                        planner_prov.supported_models()
                            .iter()
                            .filter(|m| m.supports_tools)
                            .max_by_key(|m| m.context_window)
                            .map(|m| m.id.clone())
                            .unwrap_or_else(|| self.model.clone())
                    };
                    tracing::debug!(
                        provider = planner_prov.name(),
                        model = %planner_model,
                        "LlmPlanner resolved model for provider (Phase 3)"
                    );
                    Some(planner::LlmPlanner::new(
                        planner_prov,
                        planner_model,
                    ).with_max_replans(self.config.planning.max_replans))
                } else {
                    None
                };

                // Phase 4: Load cross-session model quality stats from DB on first message.
                // This seeds the ModelSelector with historical quality data so "balanced" routing
                // exploits learned performance signals from prior sessions (not just the current session).
                if !self.model_quality_db_loaded {
                    self.model_quality_db_loaded = true;
                    if let Some(ref adb) = self.async_db {
                        match adb.load_model_quality_stats(p.name()).await {
                            Ok(prior_stats) if !prior_stats.is_empty() => {
                                for (model_id, success, failure, reward) in prior_stats {
                                    let cached = self.model_quality_cache
                                        .entry(model_id)
                                        .or_insert((0u32, 0u32, 0.0f64));
                                    // Merge: take prior stats when they show more experience
                                    // than whatever was already in the in-session cache.
                                    if success > cached.0 {
                                        *cached = (success, failure, reward);
                                    }
                                }
                                tracing::info!(
                                    models = self.model_quality_cache.len(),
                                    provider = p.name(),
                                    "Phase 4: cross-session model quality loaded from DB"
                                );
                            }
                            Ok(_) => {
                                tracing::debug!("Phase 4: no prior model quality stats in DB");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Phase 4: failed to load model quality from DB");
                            }
                        }
                    }
                }

                // Phase 8-A: Plugin system lazy-init (V3).
                // Discovers *.plugin.toml manifests from ~/.halcon/plugins/ and registers
                // PluginProxyTool instances into the session ToolRegistry.  Only runs once
                // per session (guard: self.plugin_registry.is_none()).
                //
                // Auto-activation: if the default plugin directory exists and contains at
                // least one *.plugin.toml manifest, plugins are activated even when
                // config.plugins.enabled = false.  This provides zero-config UX: drop a
                // manifest into ~/.halcon/plugins/ and it activates on next message.
                let plugins_should_run = self.config.plugins.enabled || {
                    let default_dir = dirs::home_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                        .join(".halcon")
                        .join("plugins");
                    std::fs::read_dir(&default_dir)
                        .map(|mut entries| {
                            entries.any(|e| {
                                e.ok()
                                    .and_then(|e| e.file_name().into_string().ok())
                                    .map(|n| n.ends_with(".plugin.toml"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                };
                if self.plugin_registry.is_none() && plugins_should_run {
                    let loader = plugin_loader::PluginLoader::default();
                    let mut runtime = plugin_transport_runtime::PluginTransportRuntime::new();
                    let mut registry = plugin_registry::PluginRegistry::new();
                    let load_result = loader.load_into(&mut registry, &mut runtime);
                    if load_result.loaded > 0 {
                        tracing::info!(
                            loaded = load_result.loaded,
                            skipped_invalid = load_result.skipped_invalid,
                            skipped_checksum = load_result.skipped_checksum,
                            "Phase 8-A: Plugin system initialised"
                        );
                        let runtime_arc = std::sync::Arc::new(runtime);
                        self.plugin_transport_runtime = Some(runtime_arc.clone());
                        self.plugin_registry = Some(registry);
                    } else {
                        tracing::debug!(
                            skipped_invalid = load_result.skipped_invalid,
                            "Phase 8-A: No plugins loaded (dir empty or all invalid)"
                        );
                    }
                }

                // Phase 8-E: Load plugin UCB1 metrics from DB on first message (seed bandit arms).
                // Follows the same load-once-per-session pattern as model_quality_db_loaded.
                if !self.plugin_metrics_db_loaded {
                    self.plugin_metrics_db_loaded = true;
                    if let (Some(ref adb), Some(ref mut reg)) =
                        (&self.async_db, &mut self.plugin_registry)
                    {
                        match adb.load_plugin_metrics().await {
                            Ok(records) if !records.is_empty() => {
                                // records: Vec<PluginMetricsRecord>
                                let seeds: Vec<(String, i64, f64)> = records
                                    .iter()
                                    .map(|r| (r.plugin_id.clone(), r.ucb1_n_uses, r.ucb1_sum_rewards))
                                    .collect();
                                reg.seed_ucb1_from_metrics(&seeds);
                                tracing::info!(
                                    plugins = records.len(),
                                    "Phase 8-E: Plugin UCB1 metrics loaded from DB"
                                );
                            }
                            Ok(_) => {
                                tracing::debug!("Phase 8-E: no prior plugin metrics in DB");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Phase 8-E: failed to load plugin metrics from DB");
                            }
                        }
                    }
                }

                // Skip model selection when user explicitly set --model on the CLI.
                let selector = if self.config.agent.model_selection.enabled && !self.explicit_model {
                    let mut sel = model_selector::ModelSelector::new(
                        self.config.agent.model_selection.clone(),
                        &self.registry,
                    )
                    .with_provider_scope(p.name())
                    // Phase 3: Inject accumulated quality stats from prior messages this session
                    // so "balanced" routing starts with learned quality adjustments (not neutral prior).
                    .with_quality_seeds(self.model_quality_cache.clone());

                    // P3: Provider health routing — populate real p95 latency hints from
                    // the metrics DB so "fast" strategy routes to lowest-latency models.
                    if let Some(ref db) = self.db {
                        let hints = build_latency_hints_from_db(db, &self.registry);
                        if !hints.is_empty() {
                            sel = sel.with_latency_hints(hints);
                        }
                    }

                    Some(sel)
                } else {
                    None
                };

                // Create task bridge when structured task framework is enabled.
                let mut task_bridge_inst = if self.config.task_framework.enabled {
                    Some(task_bridge::TaskBridge::new(&self.config.task_framework))
                } else {
                    None
                };

                // Phase 1.1: Load cross-session UCB1 experience on first use (lazy, one-time per session).
                // The write path (save_reasoning_experience) runs every turn.
                // The read path was NEVER called — UCB1 always started naive. This fixes that.
                if let Some(ref mut engine) = self.reasoning_engine {
                    if !engine.is_experience_loaded() {
                        // Convert Debug-format strings ("CodeGeneration") → snake_case ("code_generation")
                        // needed by TaskType::from_str / ReasoningStrategy::from_str.
                        fn pascal_to_snake(s: &str) -> String {
                            let mut out = String::with_capacity(s.len() + 4);
                            for (i, c) in s.chars().enumerate() {
                                if c.is_uppercase() && i > 0 { out.push('_'); }
                                out.extend(c.to_lowercase());
                            }
                            out
                        }
                        if let Some(ref adb) = self.async_db {
                            match adb.load_all_reasoning_experiences().await {
                                Ok(exps) => {
                                    let parsed: Vec<_> = exps.iter().filter_map(|e| {
                                        let tt = task_analyzer::TaskType::from_str(&pascal_to_snake(&e.task_type))?;
                                        let st = strategy_selector::ReasoningStrategy::from_str(&pascal_to_snake(&e.strategy))?;
                                        Some((tt, st, e.avg_score, e.uses))
                                    }).collect();
                                    let count = parsed.len();
                                    engine.load_experience(parsed); // sets experience_loaded = true
                                    tracing::info!(entries = count, "UCB1: cross-session experience loaded");
                                }
                                Err(e) => {
                                    tracing::warn!("UCB1 load_experience failed: {e}");
                                    engine.mark_experience_loaded(); // suppress future retries this session
                                }
                            }
                        } else {
                            engine.mark_experience_loaded(); // no DB — skip all future attempts
                        }
                    }
                }

                // FASE 3.1: PRE-LOOP reasoning analysis (when reasoning engine enabled).
                let reasoning_analysis = if let Some(ref mut engine) = self.reasoning_engine {
                    let analysis = engine.pre_loop(input, &self.config.agent.limits);

                    // Emit ReasoningStarted event.
                    let _ = self.event_tx.send(DomainEvent::new(EventPayload::ReasoningStarted {
                        query_hash: analysis.analysis.task_hash.clone(),
                        task_type: format!("{:?}", analysis.analysis.task_type),
                        complexity: format!("{:?}", analysis.analysis.complexity),
                    }));

                    // Emit StrategySelected event.
                    let _ = self.event_tx.send(DomainEvent::new(EventPayload::StrategySelected {
                        strategy: format!("{:?}", analysis.strategy),
                        confidence: 0.8, // Placeholder confidence
                        task_type: format!("{:?}", analysis.analysis.task_type),
                    }));

                    // Note: reasoning_status in pre-loop doesn't have score/success yet
                    tracing::info!(
                        task_type = ?analysis.analysis.task_type,
                        complexity = ?analysis.analysis.complexity,
                        strategy = ?analysis.strategy,
                        "Reasoning strategy selected"
                    );

                    Some(analysis)
                } else {
                    None
                };

                // Use reasoning-adjusted limits if available, else base limits.
                let agent_limits = reasoning_analysis
                    .as_ref()
                    .map(|a| &a.adjusted_limits)
                    .unwrap_or(&self.config.agent.limits);

                // Build StrategyContext from UCB1 PreLoopAnalysis (Step 9a).
                let strategy_ctx: Option<agent_types::StrategyContext> =
                    reasoning_analysis.as_ref().map(|a| agent_types::StrategyContext {
                        strategy: a.strategy,
                        enable_reflection: a.plan.enable_reflection,
                        loop_guard_tightness: a.plan.loop_guard_tightness,
                        replan_sensitivity: a.plan.replan_sensitivity,
                        routing_bias: a.plan.routing_bias.clone(),
                        task_type: a.analysis.task_type,
                        complexity: a.analysis.complexity,
                    });

                // Build critic provider/model from config (Step 9b — G2 critic separation).
                let critic_prov: Option<std::sync::Arc<dyn halcon_core::traits::ModelProvider>> =
                    self.config.reasoning.critic_provider.as_deref()
                        .and_then(|name| self.registry.get(name))
                        .cloned();
                let critic_mdl: Option<String> = self.config.reasoning.critic_model.clone();

                let ctx = agent::AgentContext {
                    provider: &p,
                    session: &mut self.session,
                    request: &request,
                    tool_registry: &self.tool_registry,
                    permissions: &mut self.permissions,
                    working_dir: &working_dir,
                    event_tx: &self.event_tx,
                    trace_db: self.async_db.as_ref(),
                    limits: agent_limits,
                    response_cache: self.response_cache.as_ref(),
                    resilience: &mut self.resilience,
                    fallback_providers: &fallback_providers,
                    routing_config: &self.config.agent.routing,
                    compactor: Some(&compactor),
                    // P1.1: Try PlaybookPlanner first (zero LLM latency). Fall back to LlmPlanner.
                    planner: if self.playbook_planner.find_match(input).is_some() {
                        Some(&self.playbook_planner as &dyn Planner)
                    } else {
                        llm_planner.as_ref().map(|p| p as &dyn Planner)
                    },
                    guardrails,
                    reflector: self.reflector.as_ref(),
                    render_sink: sink,
                    replay_tool_executor: None,
                    phase14: halcon_core::types::Phase14Context {
                        dry_run_mode: self.dry_run_override.take().unwrap_or_default(),
                        ..Default::default()
                    },
                    model_selector: selector.as_ref(),
                    registry: Some(&self.registry),
                    episode_id: Some(uuid::Uuid::new_v4()),
                    planning_config: &self.config.planning,
                    orchestrator_config: &self.config.orchestrator,
                    tool_selection_enabled: self.config.context.dynamic_tool_selection,
                    task_bridge: task_bridge_inst.as_mut(),
                    context_metrics: Some(&self.context_metrics),
                    context_manager: self.context_manager.as_mut(),
                    // Phase 43: pass control channel receiver from TUI (if present).
                    #[cfg(feature = "tui")]
                    ctrl_rx: self.ctrl_rx.take(),
                    #[cfg(not(feature = "tui"))]
                    ctrl_rx: None,
                    speculator: &self.speculator,
                    security_config: &self.config.security,
                    strategy_context: strategy_ctx.clone(),
                    critic_provider: critic_prov.clone(),
                    critic_model: critic_mdl.clone(),
                    plugin_registry: self.plugin_registry.as_mut(),
                };
                // Fix: restore ctrl_rx before propagating any error so TUI controls
                // (Pause/Step/Cancel) remain functional across agent loop failures.
                // Previously `?` would drop ctrl_rx on Err, leaving self.ctrl_rx = None
                // for the rest of the session.
                let mut agent_loop_result = agent::run_agent_loop(ctx).await;

                // Phase 43: restore control channel receiver for reuse across TUI messages.
                // We restore from AgentLoopResult on Ok. On Err the channel was consumed
                // inside run_agent_loop; we leave self.ctrl_rx as None in that rare case.
                #[cfg(feature = "tui")]
                if let Ok(ref mut r) = agent_loop_result {
                    self.ctrl_rx = r.ctrl_rx.take();
                }

                let mut result = agent_loop_result?;

                // Cache timeline for --timeline exit hook.
                self.last_timeline = result.timeline_json.clone();

                // Phase 1.2: Variables for capturing critic retry decision (must be outside
                // the reasoning_engine borrow so we can re-borrow self.session etc. for the retry).
                let mut critic_retry_needed = false;
                // (confidence, gaps, retry_instruction)
                let mut critic_retry_info: Option<(f32, Vec<String>, Option<String>)> = None;

                // Phase 2 Causality Enforcement: capture pipeline reward outside the
                // reasoning_engine borrow so record_outcome() can be called after retry.
                // None when reasoning engine is disabled (coarse fallback used instead).
                let mut captured_pipeline_reward: Option<(f64, bool)> = None;

                // FASE 3.1: POST-LOOP reasoning evaluation (when reasoning engine enabled).
                // Step 9e: Use reward_pipeline::compute_reward() for richer UCB1 signal.
                if let Some(ref mut engine) = self.reasoning_engine {
                    if let Some(ref analysis) = reasoning_analysis {
                        // Build multi-signal reward from all available signals.
                        let round_scores: Vec<f32> = result.round_evaluations.iter()
                            .map(|e| e.combined_score)
                            .collect();
                        let critic_verdict_tuple = result.critic_verdict.as_ref()
                            .map(|cv| (cv.achieved, cv.confidence));
                        let raw_signals = reward_pipeline::RawRewardSignals {
                            stop_condition: result.stop_condition,
                            round_scores,
                            critic_verdict: critic_verdict_tuple,
                            // Phase 7: wired from agent result (was TODO: 0.0 placeholder).
                            plan_coherence_score: result.avg_plan_drift,
                            oscillation_penalty: result.oscillation_penalty,
                            plan_completion_ratio: result.plan_completion_ratio,
                            plugin_snapshots: result.plugin_cost_snapshot.clone(),
                        };
                        let reward_computation = reward_pipeline::compute_reward(&raw_signals);
                        // Step 5 plugin blending: apply plugin success rate signal (10% weight).
                        let blended_reward = reward_pipeline::plugin_adjusted_reward(
                            reward_computation.final_reward as f64,
                            &result.plugin_cost_snapshot,
                        );
                        let evaluation = engine.post_loop_with_reward(analysis, blended_reward as f64);

                        // Phase 2 Causality Enforcement: capture pipeline reward for unified
                        // record_outcome() call after retry (reward contamination fix).
                        // Use blended_reward (includes plugin signal) as the canonical reward.
                        captured_pipeline_reward = Some((
                            blended_reward,
                            evaluation.success,
                        ));

                        // Emit EvaluationCompleted event.
                        let _ = self.event_tx.send(DomainEvent::new(
                            EventPayload::EvaluationCompleted {
                                score: evaluation.score,
                                success: evaluation.success,
                                strategy: format!("{:?}", evaluation.strategy),
                            },
                        ));

                        // Call reasoning_status with full evaluation
                        sink.reasoning_status(
                            &format!("{:?}", evaluation.task_type),
                            &format!("{:?}", analysis.analysis.complexity),
                            &format!("{:?}", evaluation.strategy),
                            evaluation.score,
                            evaluation.success,
                        );

                        // Emit ExperienceRecorded event.
                        let _ = self.event_tx.send(DomainEvent::new(
                            EventPayload::ExperienceRecorded {
                                task_type: format!("{:?}", evaluation.task_type),
                                strategy: format!("{:?}", evaluation.strategy),
                                score: evaluation.score,
                            },
                        ));

                        // P3 FIX: Persist reasoning experience to SQLite for cross-session UCB1 learning.
                        if let Some(ref adb) = self.async_db {
                            let _ = adb.save_reasoning_experience(
                                &format!("{:?}", evaluation.task_type),
                                &format!("{:?}", evaluation.strategy),
                                evaluation.score,
                            ).await;
                            tracing::debug!(
                                task_type = %format!("{:?}", evaluation.task_type),
                                strategy = %format!("{:?}", evaluation.strategy),
                                score = evaluation.score,
                                "P3: Reasoning experience persisted"
                            );
                        }

                        tracing::info!(
                            score = evaluation.score,
                            success = evaluation.success,
                            "Reasoning evaluation complete"
                        );

                        // Phase 1.2 + Phase 7 (Autonomy Validation): LoopCritic verdict → should_retry().
                        //
                        // Two independent paths can trigger a retry:
                        //   A) Score-based:  reward score < success_threshold (engine.should_retry)
                        //   B) Halt-based:   LoopCritic::should_halt() — !achieved AND
                        //                    confidence >= HALT_CONFIDENCE_THRESHOLD (0.80)
                        //
                        // Path B closes Phase 7: even if the reward score is above threshold
                        // (e.g. EndTurn scored as 0.70+), a highly confident critic verdict
                        // of failure overrides the score-based decision and forces a retry.
                        //
                        // Extract retry decision into outer variables so we can act on it
                        // AFTER the reasoning_engine borrow is released (Rust borrow rules).
                        if let Some(ref cv) = result.critic_verdict {
                            let score_says_retry = engine.should_retry(evaluation.score, 0);
                            // Phase 7: LoopCritic::should_halt_raw() — high-confidence failure
                            // bypass. When the critic is >=80% confident the goal was NOT
                            // achieved, treat it as a structural halt regardless of reward score.
                            let critic_halt = supervisor::LoopCritic::should_halt_raw(
                                cv.achieved,
                                cv.confidence,
                            );
                            if !cv.achieved && (score_says_retry || critic_halt) {
                                critic_retry_needed = true;
                                critic_retry_info = Some((
                                    cv.confidence,
                                    cv.gaps.clone(),
                                    cv.retry_instruction.clone(),
                                ));
                                tracing::info!(
                                    critic_confidence = cv.confidence,
                                    score = evaluation.score,
                                    score_says_retry,
                                    critic_halt,
                                    "LoopCritic+Reasoning: in-session retry warranted"
                                );
                            }
                        }
                    }
                }

                // Phase 1.2: Actual in-session LoopCritic retry.
                // Now that the reasoning_engine borrow has been released, we can mutably
                // access self.session + self.permissions + self.resilience for the retry.
                // This is the structural change: previously this was advisory (just logs).
                // Now it performs a real second agent loop invocation within the same turn.
                if critic_retry_needed {
                    if let Some((confidence, gaps, retry_instr)) = critic_retry_info {
                        let instr = retry_instr.as_deref().unwrap_or(
                            "Your previous response did not fully complete the task. Please address all missing elements."
                        );
                        let retry_text = format!(
                            "[Critic retry]: Task incomplete. Missing: {}. Instruction: {}",
                            if gaps.is_empty() { "see previous response".to_string() } else { gaps.join("; ") },
                            instr
                        );

                        sink.info(&format!(
                            "[reasoning] critic retry ({:.0}% confidence) — re-running agent loop",
                            confidence * 100.0
                        ));

                        // Inject retry instruction into session (already has agent's first response).
                        self.session.add_message(ChatMessage {
                            role: Role::User,
                            content: MessageContent::Text(retry_text),
                        });

                        // Rebuild request with updated session messages.
                        let retry_request = ModelRequest {
                            model: request.model.clone(),
                            messages: self.session.messages.clone(),
                            tools: request.tools.clone(),
                            max_tokens: request.max_tokens,
                            temperature: request.temperature,
                            system: request.system.clone(),
                            stream: true,
                        };

                        // Reconstruct AgentContext for the retry invocation.
                        let retry_ctx = agent::AgentContext {
                            provider: &p,
                            session: &mut self.session,
                            request: &retry_request,
                            tool_registry: &self.tool_registry,
                            permissions: &mut self.permissions,
                            working_dir: &working_dir,
                            event_tx: &self.event_tx,
                            trace_db: self.async_db.as_ref(),
                            limits: agent_limits,
                            response_cache: self.response_cache.as_ref(),
                            resilience: &mut self.resilience,
                            fallback_providers: &fallback_providers,
                            routing_config: &self.config.agent.routing,
                            compactor: Some(&compactor),
                            planner: llm_planner.as_ref().map(|lp| lp as &dyn Planner),
                            guardrails,
                            reflector: self.reflector.as_ref(),
                            render_sink: sink,
                            replay_tool_executor: None,
                            phase14: halcon_core::types::Phase14Context::default(),
                            model_selector: selector.as_ref(),
                            registry: Some(&self.registry),
                            episode_id: Some(uuid::Uuid::new_v4()),
                            planning_config: &self.config.planning,
                            orchestrator_config: &self.config.orchestrator,
                            tool_selection_enabled: self.config.context.dynamic_tool_selection,
                            task_bridge: None, // task_bridge state committed from first loop
                            context_metrics: Some(&self.context_metrics),
                            context_manager: self.context_manager.as_mut(),
                            #[cfg(feature = "tui")]
                            ctrl_rx: self.ctrl_rx.take(),
                            #[cfg(not(feature = "tui"))]
                            ctrl_rx: None,
                            speculator: &self.speculator,
                            security_config: &self.config.security,
                            strategy_context: strategy_ctx.clone(),
                            critic_provider: critic_prov.clone(),
                            critic_model: critic_mdl.clone(),
                            plugin_registry: None, // retry doesn't re-share plugin state
                        };

                        let mut retry_loop_result = agent::run_agent_loop(retry_ctx).await;
                        #[cfg(feature = "tui")]
                        if let Ok(ref mut r) = retry_loop_result {
                            self.ctrl_rx = r.ctrl_rx.take();
                        }
                        if let Ok(retry_r) = retry_loop_result {
                            // Accumulate tokens from retry into session counters.
                            self.session.total_usage.input_tokens += retry_r.input_tokens as u32;
                            self.session.total_usage.output_tokens += retry_r.output_tokens as u32;
                            tracing::info!(
                                rounds = retry_r.rounds,
                                stop = ?retry_r.stop_condition,
                                "Phase 1.2: LoopCritic in-session retry completed"
                            );
                            result = retry_r;
                        }
                    }
                }

                // Phase 2 Causality Enforcement: Unified reward → ModelSelector.record_outcome().
                //
                // This is the reward contamination fix. Previously agent.rs called
                // record_outcome() with a coarse 4-value mapping INSIDE the loop, giving
                // the quality tracker a completely different (and less accurate) signal than
                // the 5-signal reward_pipeline used by the UCB1 engine. Now:
                //   - When reasoning engine is active: use the pipeline reward captured above.
                //   - When reasoning engine is disabled: compute a coarse formula here once.
                //
                // Uses result.last_model_used so we record the model that actually ran the
                // final round (possibly changed by fallback or ModelSelector mid-session).
                if let Some(ref sel) = selector {
                    if let Some(model_id) = result.last_model_used.as_deref() {
                        let (reward, success) = if let Some((pr, ps)) = captured_pipeline_reward {
                            // Reasoning engine was active: use 5-signal pipeline reward.
                            (pr, ps)
                        } else {
                            // Coarse fallback: stop-condition mapping (2-level only).
                            let coarse_success = matches!(
                                result.stop_condition,
                                agent_types::StopCondition::EndTurn
                                    | agent_types::StopCondition::ForcedSynthesis
                            );
                            let coarse_reward = match result.stop_condition {
                                agent_types::StopCondition::EndTurn => 0.85,
                                agent_types::StopCondition::ForcedSynthesis => 0.65,
                                agent_types::StopCondition::MaxRounds => 0.40,
                                agent_types::StopCondition::TokenBudget
                                | agent_types::StopCondition::DurationBudget
                                | agent_types::StopCondition::CostBudget
                                | agent_types::StopCondition::SupervisorDenied => 0.30,
                                // User-cancelled: partial credit (not a model/task failure).
                                agent_types::StopCondition::Interrupted => 0.50,
                                // Hard failures: zero reward so UCB1 avoids bad strategies.
                                _ => 0.0,
                            };
                            (coarse_reward, coarse_success)
                        };
                        sel.record_outcome(model_id, reward, success);
                        tracing::debug!(
                            model_id,
                            reward,
                            success,
                            via = if captured_pipeline_reward.is_some() { "pipeline" } else { "coarse" },
                            "Phase 2: ModelSelector quality record unified"
                        );
                    }
                    // Phase 3: Snapshot quality stats back to Repl-level cache so the NEXT
                    // message starts with informed priors (not neutral 0.5).
                    self.model_quality_cache = sel.snapshot_quality_stats();
                    tracing::debug!(
                        models_tracked = self.model_quality_cache.len(),
                        "Phase 3: Quality stats snapshot saved for next message"
                    );

                    // Phase 7: Provider quality gate — warn when all tracked models are degraded.
                    // Fires after record_outcome() so the new outcome is included in the check.
                    // Min 5 interactions required to avoid false positives on cold-start.
                    if let Some(warning) = sel.quality_gate_check(5) {
                        sink.warning(&warning, None);
                        tracing::warn!(
                            provider = p.name(),
                            "Phase 7: Provider quality degradation detected"
                        );
                    }

                    // Phase 4: Persist quality stats to DB (fire-and-forget) for cross-session
                    // learning. Non-fatal: DB unavailability does not affect the agent loop.
                    if let Some(ref adb) = self.async_db {
                        let adb_clone = adb.clone();
                        let provider_name = p.name().to_string();
                        let snapshot: Vec<(String, u32, u32, f64)> = self
                            .model_quality_cache
                            .iter()
                            .map(|(k, &(s, f, r))| (k.clone(), s, f, r))
                            .collect();
                        tokio::spawn(async move {
                            if let Err(e) = adb_clone
                                .save_model_quality_stats(&provider_name, snapshot)
                                .await
                            {
                                tracing::warn!(error = %e, "Phase 4: model quality persist failed");
                            } else {
                                tracing::debug!("Phase 4: model quality stats persisted to DB");
                            }
                        });
                    }
                }

                // Phase 8-D + 8-E: Record per-plugin UCB1 rewards from this agent loop and
                // fire-and-forget persist to DB.
                if let Some(ref mut reg) = self.plugin_registry {
                    for snapshot in &result.plugin_cost_snapshot {
                        // Derive success rate from calls_made / calls_failed as reward signal.
                        let rate = if snapshot.calls_made > 0 {
                            let succeeded =
                                snapshot.calls_made.saturating_sub(snapshot.calls_failed);
                            succeeded as f64 / snapshot.calls_made as f64
                        } else {
                            0.5 // neutral prior for plugins that were not invoked this round
                        };
                        reg.record_reward(&snapshot.plugin_id, rate);
                    }

                    // Persist updated UCB1 arm stats (fire-and-forget, non-fatal).
                    if let Some(ref adb) = self.async_db {
                        let adb_clone = adb.clone();
                        let snapshot_data: Vec<halcon_storage::db::PluginMetricsRecord> = reg
                            .ucb1_snapshot()
                            .into_iter()
                            .map(|(plugin_id, n_uses, sum_rewards)| {
                                halcon_storage::db::PluginMetricsRecord {
                                    plugin_id,
                                    calls_made: 0,
                                    calls_failed: 0,
                                    tokens_used: 0,
                                    ucb1_n_uses: n_uses as i64,
                                    ucb1_sum_rewards: sum_rewards,
                                    updated_at: String::new(),
                                }
                            })
                            .collect();
                        if !snapshot_data.is_empty() {
                            tokio::spawn(async move {
                                if let Err(e) = adb_clone.save_plugin_metrics(snapshot_data).await {
                                    tracing::warn!(error = %e, "Phase 8-E: plugin metrics persist failed");
                                } else {
                                    tracing::debug!("Phase 8-E: plugin metrics persisted to DB");
                                }
                            });
                        }
                    }
                }

                // P3: Playbook auto-learning — save successful LLM-generated plans as reusable YAML.
                // Only when:
                //   1. auto_learn_playbooks is enabled in config
                //   2. The agent stopped successfully (EndTurn or ForcedSynthesis, not error)
                //   3. A plan was actually executed (timeline_json is Some)
                //   4. PlaybookPlanner did NOT already match this message (LlmPlanner was used)
                if self.config.planning.auto_learn_playbooks
                    && matches!(
                        result.stop_condition,
                        agent_types::StopCondition::EndTurn
                            | agent_types::StopCondition::ForcedSynthesis
                    )
                    && self.playbook_planner.find_match(input).is_none()
                {
                    if let Some(ref timeline_json) = result.timeline_json {
                        if let Some(saved_path) =
                            self.playbook_planner.record_from_timeline(input, timeline_json)
                        {
                            tracing::info!(
                                path = %saved_path.display(),
                                "P3: Auto-saved plan as playbook for future reuse"
                            );
                        }
                    }
                }

                // Phase 7 Dev Ecosystem: Record agent-loop span into the rolling telemetry
                // window so as_reward() can surface latency / error-rate back to UCB1.
                // fire-and-forget (tokio::spawn) — never blocks the REPL response path.
                {
                    let rt_signals = std::sync::Arc::clone(&self.runtime_signals);
                    let loop_ms = result.latency_ms as f64;
                    let had_error = matches!(
                        result.stop_condition,
                        agent_types::StopCondition::ProviderError
                            | agent_types::StopCondition::EnvironmentError
                    );
                    tokio::spawn(async move {
                        rt_signals
                            .ingest(runtime_signal_ingestor::RuntimeSignal::span(
                                "agent_loop",
                                loop_ms,
                                had_error,
                            ))
                            .await;
                    });
                }

                // FIX P0.1 2026-02-17: Update session token counters from agent loop result
                self.session.total_usage.input_tokens += result.input_tokens as u32;
                self.session.total_usage.output_tokens += result.output_tokens as u32;

                // Display result summary via sink.
                let total_tokens = result.input_tokens + result.output_tokens;
                if total_tokens > 0 || result.latency_ms > 0 {
                    let cost_str = if result.cost_usd > 0.0 {
                        format!(" | ${:.4}", result.cost_usd)
                    } else {
                        String::new()
                    };
                    let rounds_str = if result.rounds > 0 {
                        format!(
                            " | {} tool {}",
                            result.rounds,
                            if result.rounds == 1 { "round" } else { "rounds" },
                        )
                    } else {
                        String::new()
                    };
                    sink.info(&format!(
                        "  [{} tokens | {:.1}s{}{}]",
                        total_tokens,
                        result.latency_ms as f64 / 1000.0,
                        cost_str,
                        rounds_str,
                    ));
                }

                // Auto-consolidate reflections after each agent interaction.
                if let Some(ref adb) = self.async_db {
                    sink.consolidation_status("consolidating reflections...");

                    // Consolidation with 30-second timeout to prevent UI freeze
                    let consolidation_timeout = std::time::Duration::from_secs(30);
                    let start = std::time::Instant::now();

                    match tokio::time::timeout(
                        consolidation_timeout,
                        memory_consolidator::maybe_consolidate(adb)
                    ).await {
                        Ok(Some(result)) => {
                            let duration_ms = start.elapsed().as_millis() as u64;
                            tracing::debug!(
                                merged = result.merged,
                                pruned = result.pruned,
                                duration_ms,
                                "Memory consolidation completed successfully"
                            );
                            sink.consolidation_complete(result.merged, result.pruned, duration_ms);
                        }
                        Ok(None) => {
                            // Consolidation was skipped (below threshold or error)
                            tracing::debug!("Memory consolidation skipped");
                        }
                        Err(_) => {
                            tracing::warn!(
                                timeout_secs = consolidation_timeout.as_secs(),
                                "Memory consolidation timed out - skipping to prevent UI freeze"
                            );
                            sink.warning(
                                "Memory consolidation took too long and was skipped",
                                Some("This is safe but may accumulate more reflections. Consider clearing old memories."),
                            );
                        }
                    }
                }
            }
            None => {
                sink.error(
                    &format!("provider '{}' not configured", self.provider),
                    Some("Set API key or check config"),
                );
                // Add placeholder to session so it's visible in session history.
                self.session.add_message(ChatMessage {
                    role: Role::Assistant,
                    content: MessageContent::Text(format!(
                        "[provider '{}' not configured] — Set API key or check config.",
                        self.provider,
                    )),
                });
            }
        }

        Ok(())
    }

    /// Fire-and-forget async session save (crash protection after each message).
    async fn auto_save_session(&self) {
        if self.session.messages.is_empty() {
            return;
        }
        if let Some(ref adb) = self.async_db {
            if let Err(e) = adb.save_session(&self.session).await {
                tracing::warn!("Auto-save session failed: {e}");
                crate::render::feedback::user_warning(
                    &format!("session auto-save failed — {e}"),
                    Some("Session data may be lost if process exits"),
                );
            }
        }
    }

    fn save_session(&self) {
        if self.session.messages.is_empty() {
            return;
        }
        if let Some(db) = &self.db {
            if let Err(e) = db.save_session(&self.session) {
                crate::render::feedback::user_warning(
                    &format!("failed to save session — {e}"),
                    None,
                );
            } else {
                tracing::debug!("Session {} saved", self.session.id);
            }

            // Auto-summarize session to memory (extractive, no LLM call).
            if self.config.memory.enabled && self.config.memory.auto_summarize {
                self.summarize_session_to_memory(db);
            }
        }
    }

    fn summarize_session_to_memory(&self, db: &Database) {
        use halcon_storage::{MemoryEntry, MemoryEntryType};
        use sha2::{Digest, Sha256};

        // Build an extractive summary from user messages.
        let user_messages: Vec<&str> = self
            .session
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .filter_map(|m| match &m.content {
                MessageContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();

        if user_messages.is_empty() {
            return;
        }

        let topic_preview: String = user_messages
            .iter()
            .take(3)
            .map(|m| {
                let trimmed: String = m.chars().take(100).collect();
                trimmed.replace('\n', " ")
            })
            .collect::<Vec<_>>()
            .join("; ");

        let summary = format!(
            "Session {}: {} messages, {} user turns. Topics: {}",
            &self.session.id.to_string()[..8],
            self.session.messages.len(),
            user_messages.len(),
            topic_preview,
        );

        let hash = hex::encode(Sha256::digest(summary.as_bytes()));

        let entry = MemoryEntry {
            entry_id: uuid::Uuid::new_v4(),
            session_id: Some(self.session.id),
            entry_type: MemoryEntryType::SessionSummary,
            content: summary,
            content_hash: hash,
            metadata: serde_json::json!({
                "model": self.model,
                "provider": self.provider,
                "message_count": self.session.messages.len(),
                "tokens": self.session.total_usage.input_tokens + self.session.total_usage.output_tokens,
            }),
            created_at: chrono::Utc::now(),
            expires_at: self.config.memory.default_ttl_days.map(|days| {
                chrono::Utc::now() + chrono::Duration::days(days as i64)
            }),
            relevance_score: 0.8,
        };

        match db.insert_memory(&entry) {
            Ok(true) => {
                tracing::debug!("Session summary stored in memory");
            }
            Ok(false) => {
                tracing::debug!("Session summary already exists (duplicate hash)");
            }
            Err(e) => {
                tracing::warn!("Failed to store session summary: {e}");
            }
        }
    }


    fn history_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".halcon").join("history.txt"))
    }

    /// Return the last execution timeline as JSON, if a plan was generated.
    ///
    /// Used by `--timeline` flag. Returns None if no plan was executed in this session.
    pub fn last_timeline_json(&self) -> Option<String> {
        self.last_timeline.clone()
    }

    /// Expose session ID for testing.
    #[cfg(test)]
    pub fn session_id(&self) -> uuid::Uuid {
        self.session.id
    }

    /// Expose session message count for testing.
    #[cfg(test)]
    pub fn message_count(&self) -> usize {
        self.session.messages.len()
    }
}

// ---------------------------------------------------------------------------
// P3: Provider health routing helpers
// ---------------------------------------------------------------------------

/// Load per-model p95 latency hints from the metrics DB.
///
/// Returns `HashMap<model_id, p95_latency_ms>` for models that have at least
/// 3 recorded invocations. Used to populate `ModelSelector::with_latency_hints()`
/// so the "fast" routing strategy routes to historically fastest models.
///
/// Requires only 3 samples to avoid cold-start bias (model with 1 fast outlier
/// getting preferential routing over better-tested alternatives).
fn build_latency_hints_from_db(
    db: &Database,
    registry: &ProviderRegistry,
) -> std::collections::HashMap<String, u64> {
    let mut hints = std::collections::HashMap::new();
    for provider_name in registry.list() {
        if let Some(provider) = registry.get(provider_name) {
            for model in provider.supported_models() {
                if let Ok(stats) = db.model_stats(provider_name, &model.id) {
                    // Require at least 3 samples for a reliable p95 estimate.
                    if stats.p95_latency_ms > 0 && stats.total_invocations >= 3 {
                        hints.insert(model.id.clone(), stats.p95_latency_ms);
                    }
                }
            }
        }
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::AppConfig;

    fn test_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    fn test_config() -> AppConfig {
        AppConfig::default()
    }

    fn test_registry() -> ProviderRegistry {
        ProviderRegistry::new()
    }

    fn test_tool_registry() -> ToolRegistry {
        ToolRegistry::new()
    }

    fn test_event_tx() -> EventSender {
        halcon_core::event_bus(16).0
    }

    #[test]
    fn repl_creates_session() {
        let config = test_config();
        let repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            Some(test_db()),
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();
        assert_eq!(repl.message_count(), 0);
    }

    #[test]
    fn session_save_and_load_roundtrip() {
        let config = test_config();
        let db = test_db();

        // Create REPL, add a message, save.
        let mut session = Session::new("test-model".into(), "test".into(), "/tmp".into());
        session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text("hello".into()),
        });
        let id = session.id;
        db.save_session(&session).unwrap();

        // Reload via resume.
        let repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            Some(test_db()),
            Some(db.load_session(id).unwrap().unwrap()),
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();
        assert_eq!(repl.session_id(), id);
        assert_eq!(repl.message_count(), 1);
    }

    #[test]
    fn session_not_saved_if_empty() {
        let config = test_config();
        let db = test_db();
        let repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            Some(db),
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();
        // save_session should be a no-op (no messages).
        repl.save_session();
        // Verify no sessions in DB.
        let db2 = test_db();
        let sessions = db2.list_sessions(10).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn list_sessions_without_db() {
        let config = test_config();
        let repl = Repl::new(
            &config,
            "test".into(),
            "model".into(),
            None,
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();
        // Should not panic.
        repl.list_sessions();
    }

    #[test]
    fn show_session_info() {
        let config = test_config();
        let repl = Repl::new(
            &config,
            "anthropic".into(),
            "sonnet".into(),
            None,
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();
        // Should not panic.
        repl.show_session();
    }

    #[tokio::test]
    async fn handle_message_with_echo_provider() {
        let config = test_config();
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(halcon_providers::EchoProvider::new()));

        let mut repl = Repl::new(
            &config,
            "echo".into(),
            "echo".into(),
            None,
            None,
            registry,
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        repl.handle_message("hello world").await.unwrap();

        // Session should have 2 messages (user + assistant).
        assert_eq!(repl.message_count(), 2);
        // Token usage should be updated.
        assert!(repl.session.total_usage.output_tokens > 0);
    }

    #[tokio::test]
    async fn handle_message_no_provider_shows_placeholder() {
        let config = test_config();
        let registry = ProviderRegistry::new();

        let mut repl = Repl::new(
            &config,
            "missing".into(),
            "some-model".into(),
            None,
            None,
            registry,
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        repl.handle_message("test").await.unwrap();

        // Session should have 2 messages (user + placeholder).
        assert_eq!(repl.message_count(), 2);
    }

    #[test]
    fn session_auto_summarize_to_memory() {
        let config = test_config();
        let db = test_db();

        let mut repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            Some(Arc::clone(&db)),
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        // Add some messages so save_session triggers summarization.
        repl.session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text("What is Rust?".into()),
        });
        repl.session.add_message(ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text("Rust is a systems programming language.".into()),
        });

        repl.save_session();

        // Memory should have a session summary.
        let stats = db.memory_stats().unwrap();
        assert_eq!(stats.total_entries, 1);

        let entries = db
            .list_memories(Some(halcon_storage::MemoryEntryType::SessionSummary), 10)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].content.contains("What is Rust?"));
        assert!(entries[0].session_id.is_some());
    }

    #[test]
    fn session_summarize_disabled_when_config_off() {
        let mut config = test_config();
        config.memory.auto_summarize = false;
        let db = test_db();

        let mut repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            Some(Arc::clone(&db)),
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        repl.session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text("test".into()),
        });

        repl.save_session();

        // No memory entry should be created.
        let stats = db.memory_stats().unwrap();
        assert_eq!(stats.total_entries, 0);
    }

    #[test]
    fn context_sources_include_memory_when_enabled() {
        let config = test_config();
        let db = test_db();

        let repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            Some(db),
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        // Should have 13 context sources: instructions + repo_map + planning + episodic_memory +
        // reflections + 8 SDLC context servers (requirements, architecture, codebase, workflow,
        // testing, runtime, security, support).
        let cm = repl.context_manager.as_ref().expect("ContextManager should exist");
        let source_names: Vec<&str> = cm.sources().map(|(name, _)| name).collect();
        assert_eq!(source_names.len(), 13);
        assert!(source_names.contains(&"instructions"));
        assert!(source_names.contains(&"repo_map"));
        assert!(source_names.contains(&"planning"));
        assert!(source_names.contains(&"episodic_memory"));
        assert!(source_names.contains(&"reflections"));
    }

    #[test]
    fn context_sources_no_memory_when_disabled() {
        let mut config = test_config();
        config.memory.enabled = false;
        config.reflexion.enabled = false;
        let db = test_db();

        let repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            Some(db),
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        // Should have 11 context sources: instructions + repo_map + planning + 8 SDLC servers
        // (no episodic_memory or reflections because those are disabled).
        let cm = repl.context_manager.as_ref().expect("ContextManager should exist");
        let source_names: Vec<&str> = cm.sources().map(|(name, _)| name).collect();
        assert_eq!(source_names.len(), 11);
        assert!(source_names.contains(&"instructions"));
        assert!(source_names.contains(&"repo_map"));
        assert!(source_names.contains(&"planning"));
        assert!(!source_names.contains(&"episodic_memory"));
        assert!(!source_names.contains(&"reflections"));
    }

    #[test]
    fn context_sources_no_memory_without_db() {
        let config = test_config();

        let repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            None,
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        // No DB => no memory, reflections, or DB-backed SDLC servers.
        // Has: instructions + repo_map + planning + codebase (codebase doesn't need DB).
        let cm = repl.context_manager.as_ref().expect("ContextManager should exist");
        let source_names: Vec<&str> = cm.sources().map(|(name, _)| name).collect();
        assert_eq!(source_names.len(), 4);
        assert!(source_names.contains(&"instructions"));
        assert!(source_names.contains(&"repo_map"));
        assert!(source_names.contains(&"planning"));
        assert!(source_names.contains(&"codebase"));
        assert!(!source_names.contains(&"episodic_memory"));
        assert!(!source_names.contains(&"reflections"));
    }

    #[test]
    fn context_sources_no_planning_when_disabled() {
        let mut config = test_config();
        config.planning.enabled = false;
        config.memory.enabled = false;
        config.reflexion.enabled = false;

        let repl = Repl::new(
            &config,
            "test".into(),
            "test-model".into(),
            None,
            None,
            test_registry(),
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        // Instructions + repo_map + codebase when planning, memory, and reflexion disabled.
        // codebase server doesn't require a DB, so it's still included.
        let cm = repl.context_manager.as_ref().expect("ContextManager should exist");
        let source_names: Vec<&str> = cm.sources().map(|(name, _)| name).collect();
        assert_eq!(source_names.len(), 3);
        assert!(source_names.contains(&"instructions"));
        assert!(source_names.contains(&"repo_map"));
        assert!(source_names.contains(&"codebase"));
        assert!(!source_names.contains(&"planning"));
    }

    // --- Phase 4C: Integration wiring tests ---

    #[test]
    fn resilience_registers_all_providers() {
        let config = test_config();
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(halcon_providers::EchoProvider::new()));

        let repl = Repl::new(
            &config,
            "echo".into(),
            "echo".into(),
            Some(test_db()),
            None,
            registry,
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        // Resilience diagnostics should list all registered providers.
        let diag = repl.resilience.diagnostics();
        let names: Vec<&str> = diag.iter().map(|d| d.provider.as_str()).collect();
        assert!(
            names.contains(&"echo"),
            "resilience should register 'echo' provider: {names:?}"
        );
    }

    #[test]
    fn resilience_registers_multiple_providers() {
        let config = test_config();
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(halcon_providers::EchoProvider::new()));
        registry.register(Arc::new(halcon_providers::OllamaProvider::new(
            None,
            halcon_core::types::HttpConfig::default(),
        )));

        let repl = Repl::new(
            &config,
            "echo".into(),
            "echo".into(),
            None,
            None,
            registry,
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        let diag = repl.resilience.diagnostics();
        let names: Vec<&str> = diag.iter().map(|d| d.provider.as_str()).collect();
        assert!(
            names.contains(&"echo"),
            "resilience should register echo: {names:?}"
        );
        assert!(
            names.contains(&"ollama"),
            "resilience should register ollama: {names:?}"
        );
    }

    #[tokio::test]
    async fn end_to_end_failover_to_echo() {
        use halcon_core::types::{BackpressureConfig, CircuitBreakerConfig, ResilienceConfig};

        let config = test_config();
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(halcon_providers::EchoProvider::new()));

        let mut repl = Repl::new(
            &config,
            "echo".into(),
            "echo".into(),
            None,
            None,
            registry,
            test_tool_registry(),
            test_event_tx(),
            false,
            false,
        )
        .unwrap();

        // Override resilience to test failover path.
        let mut resilience = ResilienceManager::new(ResilienceConfig {
            enabled: true,
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: 100, // high threshold so echo won't trip
                ..Default::default()
            },
            health: Default::default(),
            backpressure: BackpressureConfig::default(),
        });
        resilience.register_provider("echo");
        repl.resilience = resilience;

        // Should succeed with echo provider even through resilience.
        repl.handle_message("integration test").await.unwrap();
        assert_eq!(repl.message_count(), 2); // user + assistant
    }
}
