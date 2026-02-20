# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
