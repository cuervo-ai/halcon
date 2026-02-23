# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] — SOTA Architecture + Permission Fixes (2026-02-23)

### Added

#### `halcon-agent-core` Crate — 10-Layer GDEM Architecture
- New standalone crate implementing Goal-Driven Execution Machine (GDEM) with 10 formal layers
- `AgentFsm` with states: Idle → Planning → Executing → Verifying → Converged / Error
- `UCB1Bandit` multi-armed bandit for strategy selection with `arm_stats()`, `record_outcome()`, `best_arm()`
- `GoalSpecParser` with `GoalSpec`, `KeywordPresence`, `ConfidenceScore` — typed goal specification
- `LoopCritic` in-loop goal verification: `Evidence` (tool_outputs, tools_called, assistant_text), `CriticVerdict`
- **127 tests pass** (was 74 after initial GDEM — +53 via Phase A+D hardening)
- Formal invariants (`invariants.rs`): I-1.1→I-5.2, proof methods PROVED/SIMULATED/ASSERTED
- `simulate_ucb1_convergence()`: deterministic proof that UCB1 converges on best arm (>85% fraction after 1000 rounds)
- Property-based tests with proptest: ConfidenceScore bounds, GAS monotonicity, UCB1 finiteness/infinity-for-unplayed

#### `halcon-sandbox` Crate — Execution Sandbox
- New standalone crate: macOS `sandbox-exec` + Linux `unshare` isolation
- Policy engine + executor with configurable resource limits (16 tests pass)

#### Session Metrics — GAS/RER/SCR/SID/ToolPR
- `SessionMetricsReport` with Goal Achievement Score (GAS): `0.6×confidence + 0.3×efficiency + 0.1×achieved_bonus`
- Tiers S/A/B/C/D, Runtime Efficiency Rate (RER), Success-to-Call Ratio (SCR), Skill-to-Invocation Density (SID)

#### SOTA Intent Architecture (IntentScorer + ModelRouter)
- `IntentScorer` multi-signal classifier: task_type, complexity, scope, reasoning_depth, suggested_max_rounds
- `ModelRouter` with `routing_bias_for()` — provider-aware model routing derived from IntentProfile
- Replaces keyword-only `TaskAnalyzer` with richer multi-dimensional intent profiling
- `IntentProfile.suggested_max_rounds()` caps UCB1 strategy's `max_rounds` (prevents over-allocation for conversational tasks)

#### Sub-Agent Pipeline Improvements
- `OrchestratorHeader` + `SubAgentTask` TUI activity lines — sub-agent progress visible in activity panel
- `Ctrl+B` toggles collapsed pill ↔ expanded tool+summary view for sub-agent results
- Context injection after sub-agent completion: sub-agent output injected into coordinator messages
- `PermissionAwaiter` callback: sub-agents route destructive tool permissions to TUI modal

### Fixed

#### Permission Modal (3 bugs resolved)
- **Silent timeout** (`permissions.rs`): When the 45-second TUI permission modal auto-denies (fail-closed), a `UiEvent::Warning` is now sent to the activity panel — user can see WHY the tool was denied even after missing the modal
- **Configurable timeout** (`permissions.rs`): TUI path now uses `config.tools.prompt_timeout_secs` (45s) instead of hardcoded 60s; stored as `tui_timeout_secs` with 30s floor
- **File path missing in delegation** (`delegation.rs`): `file_write` sub-agent instructions now include `Target file path: X` + `path="X"` directive — extracts from `expected_args.path` or infers via `infer_file_path()` (html→.html, python→.py, shell→.sh, etc.). Prevents sub-agents from generating content as text instead of calling file_write.

#### Orchestrator SOTA Gaps
- `allowed_tools` now filters tool definitions for sub-agents (sub-agents no longer see all 60+ tools)
- Sub-agent timeout capped at 200s (`SUB_AGENT_MAX_TIMEOUT_SECS=200`) — config `sub_agent_timeout_secs=200`
- `ConvergenceController` for sub-agents: max_rounds=6, stagnation_window=2, goal_coverage_threshold=0.10
- Multilingual keyword extraction: Spanish domain words translated to English for coverage matching (`estructura→structure`, `repositorio→repository`)
- `is_sub_agent: bool` field on `AgentContext` — sub-agent vs coordinator execution path separation

#### Tool Pipeline Fixes
- `native_search.rs`: uninitialized engine returns `is_error: true` (was false — caused model to retry infinitely)
- `executor.rs`: MCP pool connection errors reclassified as TRANSIENT (not deterministic) — enables recovery after temporary connection drops
- Tool output truncation: head+tail (60%+30%) UTF-8-safe — preserves both start AND end of long outputs

#### Agent Loop Fixes
- `LoopCritic`: uses `.rev().find()` for correct last-response extraction (not first)
- `ForcedSynthesis`: injects synthesis directive + `ForcedByOracle`, returns `NextRound` instead of immediately breaking
- UCB1 persistence: `match ... { Err(e) => warn!() }` instead of `let _ =` for visible error on DB failure
- Sub-agent `response_cache: None` — prevents caching of text-only "I will create..." responses as tool results

### Changed

#### Architecture Refactor — Clean Module Boundaries
- `repl/agent.rs` → `repl/agent/` module (provider_round, budget_guards, round_setup, convergence_phase, etc.)
- `repl/reasoning_engine.rs` → `repl/application/reasoning_engine.rs`
- `repl/strategy_selector.rs` → `repl/domain/strategy_selector.rs`
- `repl/task_analyzer.rs` → `repl/domain/task_analyzer.rs`
- `SessionManager` extracted from `repl/mod.rs` → `repl/session_manager.rs` (13 new tests)
- `ModelRouter` per-round: `forced_routing_bias` field on `LoopState` — single-round override without strategy mutation

### Tests
- **3404 total tests pass** (was 3396 before permission fixes, +8 new tests this session)
- New in this session: `file_write_with_explicit_path_uses_expected_args`, `file_write_infers_html/python_path`, `non_file_write_tools_have_no_path_hint`, `infer_html/python/shell_variants`, `infer_default_for_unknown`
- UCB1 closed-loop tests (Phase 9): `reward_pipeline_feeds_ucb1_strategy_learning`, `repeated_high_rewards_make_strategy_dominant`, `low_reward_does_not_mark_as_success`, `ucb1_total_experience_count_increments`

---

## [Unreleased] — Phase 78-80: HALCON V3 Plugin Suite (2026-02-19)

### Added

#### Plugin System V3 — 7 Plugins, 33 Herramientas
- Complete plugin infrastructure: `plugin_manifest.rs`, `plugin_registry.rs`, `plugin_circuit_breaker.rs`, `plugin_cost_tracker.rs`, `plugin_permission_gate.rs`, `capability_index.rs`, `capability_resolver.rs`
- UCB1 bandit per-plugin reward tracking with `record_reward()` + `select_best_for_capability()`
- BM25 `CapabilityIndex` with `exact_match()` fallback for deterministic plugin tool resolution
- `BatchVerdict::SuspendPlugin` in supervisor.rs — Gate 0 fires before existing batch gates
- `plugin_adjusted_reward()` — `(0.90 × base + 0.10 × plugin_success_rate).clamp(0.0, 1.0)`
- Plugin registry wired into `AgentContext` and `executor.rs` pre/post hooks

#### New Plugin: `halcon-otel-tracer` (Arquitectura — 5 herramientas)
- `trace_coverage_scan` — Mide cobertura de trazado: `#[tracing::instrument]`, spans manuales, OTel JS SDK, opentelemetry Python, Go otel spans
- `metric_inventory` — Inventario de métricas: `counter!`/`histogram!`/`gauge!` macros en Rust, MeterProvider en TS, Prometheus
- `log_pattern_scan` — Analiza patrones de logging: structured vs unstructured ratio, hotspots de `println!`/`console.log`
- `otel_compliance_check` — Verifica 7 puntos de cumplimiento OTel: exportadores, resource detection, W3C TraceContext, sampler
- `observability_health_report` — Score holístico 0-100: Trazado (40%), Métricas (30%), Logging (20%), Pipeline (10%)
- **Hallazgo real en HALCON**: 1% cobertura de trazado (3/205 archivos), 18 llamadas `println!`, 0% OTel → Grade D (16/100)

#### New Plugin: `halcon-perf-analyzer` (Frontend — 5 herramientas)
- `bundle_size_analyzer` — Indicadores de bundle JS/TS: importaciones dinámicas, barrel exports, librerías sin tree-shaking
- `lazy_loading_audit` — Auditoría de code-splitting: React.lazy, Suspense, React.memo, useCallback/useMemo, preload hints
- `render_blocking_scan` — Detección de recursos bloqueantes: `<script>` sin async/defer, inline `<style>` >2KB, Google Fonts sin font-display:swap
- `image_optimization_check` — Verificación de imágenes: >200KB, missing loading='lazy', alt attrs, width/height, WebP/AVIF
- `perf_health_report` — Score 0-100: Bundle Size (30%), Code Splitting (25%), Resource Loading (25%), Asset Optimization (20%)
- **Resultado en website/src**: Grade A (98/100)

#### New Plugin: `halcon-schema-oracle` (Backend — 5 herramientas)
- `db_schema_analyzer` — Analiza esquemas desde archivos SQL, Diesel `schema.rs`, entidades SeaORM
- `migration_health` — Auditoría de migraciones: reversibilidad, DROP sin Down, NOT NULL sin DEFAULT
- `index_advisor` — Sugerencias de índices: FKs sin índice, columnas filtradas frecuentemente, genera CREATE INDEX SQL
- `query_pattern_scan` — Patrones peligrosos: SELECT *, N+1 queries, joins cartesianos, SQL injection por concatenación
- `schema_health_report` — Score 0-100: Schema Richness (30%), Migraciones (25%), Query Safety (25%), FK Coverage (20%)
- **Nota**: HALCON usa SQL embebido en constantes Rust (no archivos .sql) — plugin reporta 0 tablas correctamente

#### Previously Added Plugins (Phase 79)
- `halcon-dev-sentinel` — 4 herramientas de seguridad: secret scanning, dependency audit, SAST, OWASP top 10
- `halcon-dependency-auditor` — 4 herramientas: auditoría Cargo.lock/package-lock.json, licencias, CVE
- `halcon-ui-inspector` — 5 herramientas: componentes UI, accesibilidad WCAG, rendimiento de renders
- `halcon-api-sculptor` — 5 herramientas: análisis REST/GraphQL, contratos OpenAPI, seguridad de endpoints

#### SOTA Meta-Cognition (Phases 73-78)
- `ReasoningEngine` + UCB1 `StrategySelector` — aprendizaje multi-armed bandit entre sesiones
- `LoopCritic` — evaluación autónoma de resultados del agente con umbral de confianza 0.80
- `RoundScorer` — puntuación por ronda: progress_delta×0.35 + tool_efficiency×0.30 + coherence×0.20 + token_score×0.15
- `PlanCoherenceChecker` — detección de drift semántico con umbral 0.70
- G1-G10 compliance gaps cerrados (Phantom Retry, Critic Separation, UCB1 Multi-Dim, ForceReplanNow, etc.)
- `StopCondition::EnvironmentError` + `StopCondition::CostBudget` para halts deterministas
- P0-A/B/C MCP dead-loop fixes: detección de servidores MCP caídos, circuit breaker, halt automático
- P1-A Parallel batch failure escalation, P1-B Compaction timeout escalation
- P2-C Cost budget hard stop, P2-D Deduplication visibility

### Fixed
- GOTCHA `extract_inline_attr` word boundary: `name="` coincidía dentro de `classname="` — fixed using `" name="` prefix
- GOTCHA BM25 IDF con documento único: idf = ln(4/3) ≈ 0.288 < MIN_PLUGIN_SCORE=0.5 → `exact_match()` bypass
- `Mutex<PluginRegistry>` en executor: `try_lock()` pattern para acceso concurrente en parallel batch

---

## [0.1.0] - 2026-02-14

### Added

#### Core Features
- Initial release of Cuervo CLI - AI-powered terminal assistant
- Multi-provider support (Anthropic Claude, OpenAI, DeepSeek, Ollama)
- Interactive REPL with rich terminal UI
- Full-featured TUI mode with multi-panel interface
- Model Context Protocol (MCP) integration
- Comprehensive tool system (file operations, git, directory tree, etc.)

#### Architecture
- Modular workspace architecture with 14 crates
- Async-first design with Tokio runtime
- Event-driven orchestration system
- Context management with automatic summarization
- Semantic memory with vector storage
- Audit logging and provenance tracking

#### TUI/UX
- Three-zone layout (Prompt, Activity, Status)
- Syntax highlighting for code blocks
- Real-time token usage and cost tracking
- Overlay system (Command Palette, Search, Help)
- Adaptive theming with color science (Momoto integration)
- Keyboard shortcuts and vim-style navigation
- Circuit breaker for API rate limiting
- Graceful degradation and error recovery

#### Security
- PII detection and redaction
- Sandbox mode for tool execution
- Dry-run mode for testing
- Keyring integration for secure credential storage
- Audit trail for all AI interactions
- Configurable safety guardrails

#### Distribution System
- One-line installation for Linux/macOS/Windows
- Automated cross-platform binary releases (6 targets)
- SHA256 checksum verification
- Automatic PATH configuration
- Fallback installation methods (cargo-binstall, cargo install)
- GitHub Actions CI/CD pipeline
- Comprehensive installation documentation

#### Documentation
- Quick start guide (5-minute setup)
- Complete installation guide with troubleshooting
- Visual installation examples
- Release process documentation
- Testing and validation guides
- API documentation and examples

#### Testing
- 1486+ passing tests across workspace
- Integration tests for core functionality
- TUI component tests
- Tool audit tests
- Installation script validation

### Technical Details

**Supported Platforms:**
- Linux x86_64 (glibc)
- Linux x86_64 (musl/Alpine)
- Linux ARM64
- macOS Intel (x86_64)
- macOS Apple Silicon (M1/M2/M3/M4)
- Windows x64

**Performance:**
- Optimized release builds (LTO, strip, size optimization)
- Lazy loading of heavy dependencies
- Streaming responses for real-time output
- Efficient context window management

**Developer Experience:**
- Hot-reloadable configuration
- Extensive logging with tracing
- Developer tools (stress tests, replay runner)
- Modular architecture for easy extension

---

[0.1.0]: https://github.com/cuervo-ai/cuervo-cli/releases/tag/v0.1.0
