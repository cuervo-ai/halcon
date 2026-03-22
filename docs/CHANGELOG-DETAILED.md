# Detailed Changelog — Halcon CLI

---

## v0.3.0 — 2026-03-14

### Cenzontle SSO (Zuclubit OAuth 2.1)

- `halcon login` command — initiates OAuth 2.1 PKCE flow via Zuclubit identity provider
- Loopback callback server on `localhost:9876` — receives auth code after browser redirect
- Token storage in system keychain (`keyring` v3 — macOS Keychain / Linux Secret Service / Windows Credential Manager)
- Proactive token refresh: auto-refreshes access token 5 minutes before expiry
- `CENZONTLE_ACCESS_TOKEN` env var bypass for CI/non-interactive environments
- `halcon auth sso-login cenzontle` long form (same as `halcon login`)
- `halcon auth logout cenzontle` — revokes token and removes from keychain
- `halcon auth status` now shows Cenzontle login state, expiry time, and refresh status
- `cenzontle` provider in `halcon status` output
- `OAuthManager::ensure_token()` in `halcon-mcp/src/oauth.rs` — PKCE S256 with sha2 + base64 + rand
- `HALCON_MCP_CLIENT_SECRET` env var bypass for CI pipelines

### HybridIntentClassifier (Phases 1–6)

- **Phase 1**: Core heuristic classifier with pattern matching and keyword scoring
- **Phase 2**: Embedding layer — TF-IDF hash projections (FNV-1a, 384 dimensions), `PrototypeStore` with cosine similarity
- **Phase 3**: Hybrid fusion — weighted combination of heuristic and embedding scores
- **Phase 4**: `AnthropicLlmLayer` — claude-haiku-4-5 deliberation via `reqwest::blocking::Client` in isolated thread (avoids tokio conflict). Activates when `confidence < 0.40` and `query.len() >= 10`. Timeout: 2000ms. Fast path at ≥0.88 skips LLM entirely.
- **Phase 5**: `DynamicPrototypeStore` — EMA centroid updates (α=0.10), UCB1 bandit per TaskType, versioned JSON persistence (`prototypes_v{N}.json`), ring buffer feedback queue (cap 256). Auto-feedback from traces: `LowConfidence` if confidence < 0.50, `LlmDisagreement` if LLM and heuristic disagree.
- **Phase 6**: `AmbiguityAnalyzer` — detects `NarrowMargin` (<0.05), `HighEntropy` (>0.75 after softmax), `PrototypeConflict`, `CrossDomainSignals` (≥3 domains). `LlmDeliberation` strategy distinct from `LlmFallback`. Cost guardrail: extra `classify_scores()` call only runs when `enable_llm && enable_embedding`.
- New `ClassificationTrace` fields: `ambiguity_detected`, `ambiguity_reason`, `classification_margin`, `score_entropy`, `llm_used`, `llm_latency_us`, `prototype_version`, `ucb_score`
- 58 total HybridIntentClassifier tests

### Compliance Audit Export (SOC 2)

- `halcon audit list` — list all auditable sessions
- `halcon audit export --format jsonl|csv|pdf` — full SOC 2 export
- `halcon audit verify <session-id>` — verify HMAC-SHA256 chain; exits 1 if tampered
- Event taxonomy: 9 SOC 2 event types mapped from 4 SQLite tables (audit_log, policy_decisions, resilience_events, execution_loop_events)
- HMAC-SHA256 chain with key stored in `audit_hmac_key` table
- PDF export: A4 format, 3 sections (cover + event timeline + breakdown), using `printpdf 0.7`
- 7 new unit tests covering JSONL, CSV, integrity chain, and PDF output
- New module: `crates/halcon-cli/src/audit/` (7 files)

### Declarative Sub-Agent Registry

- `.halcon/agents/*.md` (project scope) and `~/.halcon/agents/*.md` (user scope)
- YAML frontmatter parsing (no gray_matter dependency — manual `---` splitting + serde_yaml)
- Required fields: `name` (kebab-case), `description`. Optional: `tools`, `model`, `max_turns` (1–100), `skills`
- Skills: `.halcon/skills/*.md` and `~/.halcon/skills/*.md` (project overrides user on collision)
- Model aliases: `haiku`, `sonnet`, `opus`
- Batch validation with levenshtein typo suggestions
- `halcon agents list [--verbose]` and `halcon agents validate [paths...]`
- Routing manifest injected into parent agent system prompt when `enable_agent_registry=true`
- `PolicyConfig::enable_agent_registry = false` feature flag
- `SubAgentTask::system_prompt_prefix: Option<String>` new field
- 79 new tests including integration tests with `tempfile::TempDir`

### MCP Ecosystem (OAuth + Tool Search + HTTP SSE)

- `OAuthManager` — PKCE S256, keychain storage, loopback callback, proactive refresh
- `MergedMcpConfig::load()` — 3-scope TOML (local > project > user), `${VAR:-default}` env expansion
- `ToolSearchIndex` — nucleo-matcher 0.3, deferred mode threshold (default 10% context), `rebuild_index()` for `list_changed` notifications
- `HttpTransport` — POST JSON-RPC Bearer auth, SSE listener (tokio task), 401 → actionable error
- `halcon mcp add/remove/list/get/auth` CLI subcommands
- 92 halcon-mcp tests

### Halcon as MCP Server

- `halcon mcp serve` — stdio transport (for Claude desktop / IDE)
- `halcon mcp serve --transport http --port 7777` — HTTP+SSE with Bearer auth
- `Mcp-Session-Id` session management with TTL expiry
- Audit tracing on all tool calls via `tracing::info!(mcp_server.tool_call)`
- `HALCON_MCP_SERVER_API_KEY` Bearer token auth; auto-generates 48-char hex key on first start
- FASE-2 guards active regardless of MCP call path
- 14 tests in `halcon-mcp/src/http_server.rs`

### Semantic Memory Vector Store

- `VectorMemoryStore` — TF-IDF hash projections (FNV-1a, DIMS=384), brute-force cosine similarity, MMR retrieval (λ=0.7)
- `MEMORY.md` section parsing and JSON index persistence (`MEMORY.vindex.json`)
- `load_from_standard_locations()` — finds MEMORY.md in CWD, `.halcon/`, and `~/.halcon/`
- `SearchMemoryTool` — agent-accessible `search_memory(query, top_k?)` tool
- `VectorMemorySource` — `ContextSource` implementation (priority 25) for pipeline injection
- `enable_semantic_memory` and `semantic_memory_top_k` in `PolicyConfig`
- Sub-1ms retrieval on 200-entry index

### VS Code Extension MVP

- TypeScript extension: `halcon-vscode/`
- `binary_resolver.ts` — resolves bundled binary for 4 platforms, config override, PATH fallback
- `halcon_process.ts` — JSON-RPC subprocess bridge, ping/pong health check, auto-restart (5× exponential backoff)
- `context_collector.ts` — active file (≤50KB), diagnostics (error+warn, cap 50), git branch/staged/unstaged
- `webview_panel.ts` — singleton xterm.js WebviewPanel, CSP nonce, tool indicator, edit proposal UI
- `diff_applier.ts` — `halcon-diff:` content provider, `vscode.diff` editor, `workspace.applyEdit`
- 5 commands: openPanel, askAboutSelection, editFile, newSession, cancelTask
- Keybindings: `Ctrl+Shift+H` (open panel), `Ctrl+Shift+A` (ask about selection)
- `halcon --mode json-rpc --max-turns N` — NDJSON protocol: `token/tool_call/tool_result/done/error` events
- `<vscode_context>` XML block injected into user messages with file/diagnostics/git context

### Lifecycle Hooks

- Pre/post tool execution hooks
- Session start/end hooks
- Custom shell command hooks
- `crates/halcon-cli/src/repl/hooks/` (3 files)

### Agent Execution Hardening (BUG-007 fix)

- **Synthesis premature strip** — fixed `all(tool_name.is_none())` → `!any(tool_name.is_some())` in `agent/mod.rs:1606-1609` — prevents false positive on mixed plans with coordination steps
- **Zero-tool drift** — `orchestrator.rs:982-984` — zero-tool success treated as soft-fail except for synthesis summaries
- **Synthesis whitelist** — `is_synthesis_summary()` with 6-keyword list: synthesis, summariz, conclus, final, review, analysis complete
- 7 regression tests covering BUG-007 scenarios

### Cron Scheduler

- `halcon schedule list/add/remove` — cron-based agent task management
- `--cron "0 9 * * 1-5"` standard cron expression support
- `halcon-cli/src/commands/schedule.rs`

### Control Plane API

- `halcon serve` — HTTP/WebSocket server (default port 9000)
- REST endpoints: 7 chat session handlers
- WebSocket: `ChatStreamToken`, `ConversationCompleted`, `ExecutionFailed`, `PermissionRequired`
- Bearer auth via `HALCON_API_TOKEN`
- Session persistence to SQLite

### Bug Fixes

- `McpServerConfig` → `McpServeConfig` rename — fixed 29 compile errors from duplicate type
- Sub-agent orphan permission modals: `confirm_destructive=false` + auto-approve in `ui_event_handler.rs`
- Sub-agent description leak in pill labels (now `"Coder [3/3]"` not raw instruction slice)
- Sub-agent spinner completion: consistent `task_id_to_step` lookup
- `HybridConfig` margin_threshold and entropy_threshold respected in all code paths

### Test Coverage

- 12,670 tests passing, 0 failures
- 1 known pre-existing failure: `render::theme::progressive_enhancement_downgrades_for_limited_terminals` (ratatui color comparison, not our code)

---

## v0.2.x — 2026-02-25

See [CHANGELOG.md](../CHANGELOG.md) for v0.2.x entries.
