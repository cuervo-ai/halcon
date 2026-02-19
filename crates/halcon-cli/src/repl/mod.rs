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
pub mod strategy_selector;
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
        let reflector = if config.reflexion.enabled {
            registry
                .get(&provider)
                .cloned()
                .map(|p| {
                    reflexion::Reflector::new(p, model.clone())
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
            #[cfg(feature = "tui")]
            ctrl_rx: None,
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

    /// Run the interactive REPL loop.
    pub async fn run(&mut self) -> Result<()> {
        // Warm L1 cache from L2 on startup.
        if let Some(ref cache) = self.response_cache {
            cache.warm_l1().await;
        }

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
                    // Fix: resolve the planning model from the active provider rather than
                    // using config.general.default_model (which may be from a different
                    // provider, e.g. "claude-sonnet-4-5" when using deepseek).
                    // Prefer the model the user explicitly requested if valid; otherwise
                    // fall back to the provider's first tool-supporting model.
                    let planner_model = if p.validate_model(&self.model).is_ok() {
                        self.model.clone()
                    } else {
                        p.supported_models()
                            .iter()
                            .filter(|m| m.supports_tools)
                            .max_by_key(|m| m.context_window)
                            .map(|m| m.id.clone())
                            .unwrap_or_else(|| self.model.clone())
                    };
                    tracing::debug!(
                        provider = p.name(),
                        model = %planner_model,
                        "LlmPlanner resolved model for provider"
                    );
                    Some(planner::LlmPlanner::new(
                        Arc::clone(&p),
                        planner_model,
                    ).with_max_replans(self.config.planning.max_replans))
                } else {
                    None
                };

                // Skip model selection when user explicitly set --model on the CLI.
                let selector = if self.config.agent.model_selection.enabled && !self.explicit_model {
                    let mut sel = model_selector::ModelSelector::new(
                        self.config.agent.model_selection.clone(),
                        &self.registry,
                    ).with_provider_scope(p.name());

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

                // FASE 3.1: POST-LOOP reasoning evaluation (when reasoning engine enabled).
                if let Some(ref mut engine) = self.reasoning_engine {
                    if let Some(ref analysis) = reasoning_analysis {
                        let evaluation = engine.post_loop(analysis, &result);

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
