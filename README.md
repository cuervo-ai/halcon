<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)"  srcset="img/halcon-logo.png">
    <source media="(prefers-color-scheme: light)" srcset="img/halcon-logo-bg.png">
    <img alt="Halcon CLI" src="img/halcon-logo-bg.png" width="220">
  </picture>
</p>

<p align="center">
  <em>AI-native terminal agent — routes intelligently, acts decisively</em>
</p>

<hr/>

<p align="center">
  <a href="https://github.com/cuervo-ai/halcon-cli/actions/workflows/ci.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/cuervo-ai/halcon-cli/ci.yml?style=flat-square&label=CI&logo=github" alt="CI">
  </a>
  <a href="https://github.com/cuervo-ai/halcon-cli/releases/latest">
    <img src="https://img.shields.io/github/v/release/cuervo-ai/halcon-cli?style=flat-square&logo=rust&label=release&color=FF6B00" alt="Latest release">
  </a>
  <img src="https://img.shields.io/badge/Rust-1.80+-orange?style=flat-square&logo=rust" alt="Rust 1.80+">
  <a href="LICENSE">
    <img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square" alt="License">
  </a>
  <a href="https://github.com/cuervo-ai/halcon-cli/actions/workflows/devsecops.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/cuervo-ai/halcon-cli/devsecops.yml?style=flat-square&label=security&logo=shield&color=22c55e" alt="Security">
  </a>
</p>

<p align="center">
  <a href="QUICKSTART.md">Quickstart</a> ·
  <a href="docs/">Documentation</a> ·
  <a href="https://github.com/cuervo-ai/halcon-cli/releases">Releases</a> ·
  <a href="https://github.com/cuervo-ai/halcon-cli/issues">Issues</a>
</p>

---

Halcon is a production-grade AI terminal agent written in Rust across 19 crates (~77 K LOC). Each request is routed through a **Boundary Decision Engine** — intent classification, SLA budget calibration, model selection — before the first LLM call. A **FASE-2 security gate** enforces 18 catastrophic-pattern guards at the tool layer, independent of any agent configuration. The result: complex multi-step sessions complete in fewer rounds with deterministic safety guarantees.

<p align="center">
  <img alt="Halcon CLI TUI — activity timeline, working memory, conversational overlay" src="img/uxui.png" width="800">
</p>

---

## Table of Contents

- [Quickstart](#quickstart)
- [Features](#features)
- [Installation](#installation)
- [Commands](#commands)
- [Providers](#providers)
- [Tools](#tools)
- [Agent Loop](#agent-loop)
- [Memory Systems](#memory-systems)
- [MCP Integration](#mcp-integration)
- [TUI](#tui)
- [Configuration](#configuration)
- [Security](#security)
- [Architecture](#architecture)
- [Contributing](#contributing)
- [License](#license)

---

## Quickstart

```sh
# 1. Install (macOS / Linux)
curl -fsSL https://raw.githubusercontent.com/cuervo-ai/halcon-cli/main/scripts/install-binary.sh | sh

# 2. Configure your API key
export ANTHROPIC_API_KEY="sk-ant-..."
# or: halcon auth login anthropic

# 3. Start a session
halcon
```

Run a one-shot task without entering the REPL:

```sh
halcon "refactor the auth module to use the new TokenStore API"
```

---

## Features

<table>
<tr>
<td width="33%">
<b>Boundary Decision Engine</b><br/>
Classifies intent, calibrates SLA round budgets, and selects routing mode before the first LLM call. IntentPipeline reconciles IntentScorer + BoundaryDecisionEngine into a single <code>effective_max_rounds</code> (fixes dual-pipeline drift).
</td>
<td width="33%">
<b>60+ Native Tools</b><br/>
File ops, bash, git, grep, glob, web fetch, web search, HTTP, Docker, SQL, JSON transform, OpenAPI validation, test execution, code metrics, linting, secret scanning — all with typed schemas and risk tiers.
</td>
<td width="33%">
<b>FASE-2 Security Gate</b><br/>
18 catastrophic patterns (rm -rf, credential exfil, fork bombs…) enforced in <code>halcon-core/src/security.rs</code> — a single source of truth shared by bash.rs and command_blacklist.rs. Path-independent: fires regardless of agent routing.
</td>
</tr>
<tr>
<td>
<b>7-Tier Context Engine</b><br/>
L0 hot buffer → L1 sliding window → L2 cold store → L3 compressed archive. Token accountant allocates budget per tier; ToolOutputElider prunes low-value outputs. Zstd compression + delta encoding for older messages.
</td>
<td>
<b>Three Memory Subsystems</b><br/>
(1) HALCON.md — 4-scope persistent instructions with hot-reload (<100ms) and <code>@import</code> resolution. (2) Auto-memory — event-triggered scoring with LRU-capped files. (3) Vector memory — TF-IDF + cosine similarity + MMR retrieval.
</td>
<td>
<b>MCP Server + Client</b><br/>
Run Halcon as an MCP endpoint (<code>halcon mcp serve</code>) over stdio or HTTP (axum, SSE, Bearer auth, session TTL). Connect to any MCP server with OAuth 2.1 + PKCE, 3-scope config, and fuzzy tool search.
</td>
</tr>
<tr>
<td>
<b>Sub-Agent Registry</b><br/>
Declarative <code>.halcon/agents/*.md</code> with YAML frontmatter (name, model alias, max_turns, skills). 3-scope discovery. Kebab-case validation with Levenshtein typo suggestions. Routing manifest injected into parent system prompt.
</td>
<td>
<b>Lifecycle Hooks</b><br/>
Shell commands or Rhai sandboxed scripts on 6 events: UserPromptSubmit, PreToolUse, PostToolUse, PostToolUseFailure, Stop, SessionEnd. Exit codes: 0=Allow, 2=Deny (stdout→reason), other=Warn+continue.
</td>
<td>
<b>Professional TUI</b><br/>
ratatui 3-zone layout — prompt editor, activity timeline (virtual scroll), status bar. 11 custom widgets including conversational permission overlay, agent badges, context budget visualizer, and toast notifications.
</td>
</tr>
</table>

---

## Installation

### Recommended — one-line installer

**macOS / Linux:**
```sh
curl -fsSL https://raw.githubusercontent.com/cuervo-ai/halcon-cli/main/scripts/install-binary.sh | sh
```

**Windows (PowerShell):**
```powershell
iwr -useb https://raw.githubusercontent.com/cuervo-ai/halcon-cli/main/scripts/install-binary.ps1 | iex
```

The installer detects your platform and architecture, verifies SHA-256 checksums, and configures your shell PATH. Supported targets: `x86_64-linux-musl`, `aarch64-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-windows-msvc`.

### Homebrew

```sh
brew tap cuervo-ai/tap && brew install halcon
```

### Cargo

```sh
cargo install --git https://github.com/cuervo-ai/halcon-cli --features tui --locked
```

<details>
<summary><b>Build from source</b></summary>

```sh
git clone https://github.com/cuervo-ai/halcon-cli.git
cd halcon-cli

cargo build --release --features tui -p halcon-cli
# binary at: target/release/halcon
```

**Feature flags:**

| Flag | Default | Effect |
|------|---------|--------|
| `tui` | ✓ | ratatui multi-panel TUI |
| `color-science` | ✓ | momoto perceptual color metrics |
| `headless` | — | disables TUI, forces classic render |
| `vendored-openssl` | — | static OpenSSL for musl targets |

</details>

<details>
<summary><b>Manual binary download + verification</b></summary>

Download from [Releases](https://github.com/cuervo-ai/halcon-cli/releases/latest). All artifacts are signed with [cosign](https://sigstore.dev) keyless signing.

```sh
# Verify signature
cosign verify-blob --signature halcon-*.tar.gz.sig --certificate halcon-*.tar.gz.pem halcon-*.tar.gz

# Verify checksum
sha256sum -c halcon-*.tar.gz.sha256
```

</details>

---

## Commands

```
halcon [OPTIONS] [PROMPT]                        interactive REPL (or one-shot with prompt)
halcon chat [--tui] [--orchestrate] [...]        explicit chat mode with flags
halcon init [--force]                            project initialization wizard
halcon auth login|logout|status PROVIDER         API key management (OS keychain)
halcon config show|get|set|path                  configuration management
halcon status                                    runtime state
halcon doctor                                    system diagnostics
halcon update [--check] [--force]                self-update
halcon theme                                     theme generation

halcon agents list|validate                      sub-agent registry management
halcon memory list|search|prune|stats|clear      persistent memory management
halcon tools list|validate|doctor|add|remove     tool registry management
halcon audit export|list|verify                  SOC 2 audit log management
halcon metrics show|export|prune|decide          performance baseline management
halcon trace export SESSION_ID                   session export
halcon replay SESSION_ID [--verify]              deterministic replay

halcon mcp add|remove|list|get|auth|serve        MCP server management
halcon lsp                                       Language Server Protocol (stdio)
halcon plugin list|install|remove|status         plugin management
```

<details>
<summary><b>Global flags</b></summary>

```
--model MODEL          override model for this session
--provider PROVIDER    override provider (anthropic|openai|ollama|deepseek|gemini|claude-code)
--verbose              debug logging
--log-level LEVEL      trace|debug|info|warn|error
--config PATH          alternate config file
--no-banner            suppress startup banner
--mode MODE            interactive|json-rpc (json-rpc used by VS Code extension)
--max-turns N          max agent loop turns
--trace-json PATH      write JSON trace to file
```

</details>

<details>
<summary><b>Chat flags</b></summary>

```
--tui                  3-zone ratatui TUI mode
--orchestrate          multi-agent orchestration
--tasks                task tracking panel
--reflexion            self-improvement loop
--metrics              performance metrics overlay
--timeline             activity timeline
--expert               advanced diagnostics
--trace-out PATH       write session trace
--trace-in PATH        replay from trace
```

</details>

---

## Providers

| Provider | Models | Transport | Vision | Tool Use |
|----------|--------|-----------|:------:|:--------:|
| **Anthropic** | Claude Opus 4.6, Sonnet 4.6, Haiku 4.5 | SSE streaming | ✓ | ✓ |
| **OpenAI** | GPT-4o, o1, o3-mini | SSE streaming | ✓ | ✓ |
| **Ollama** | Llama, Mistral, Qwen, Phi, CodeLlama… | NDJSON streaming | ✓ | ✓ |
| **DeepSeek** | DeepSeek Coder, Chat, Reasoner | OpenAI-compat | — | ✓ |
| **Google Gemini** | Gemini Pro, Flash, Ultra | SSE streaming | ✓ | ✓ |
| **Claude Code** | claude CLI subprocess (NDJSON bridge) | Stdio JSON-RPC | — | ✓ |
| **OpenAI-compat** | Any OpenAI-compatible API | SSE streaming | ✓ | ✓ |
| **Echo** | Debug / testing | Sync | — | — |
| **Replay** | Deterministic trace reproduction | Offline | — | — |

Configure via `halcon auth login PROVIDER` or environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OLLAMA_HOST`, `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`).

---

## Tools

Halcon ships **60+ native tools** with typed JSON schemas, per-tool `RiskTier`, and per-directory allow-lists.

<details>
<summary><b>Full tool inventory</b></summary>

**File Operations (7)**
`file_read` · `file_write` · `file_edit` · `file_delete` · `directory_tree` · `file_inspect` · `file_diff`

**Shell & System (5)**
`bash` (FASE-2 guarded) · `glob` · `env_inspect` · `process_list` · `port_check`

**Background Jobs (3)**
`background_start` · `background_output` · `background_kill`

**Search & Content (5)**
`grep` · `web_fetch` · `web_search` · `native_search` (BM25 + PageRank + semantic) · `semantic_grep`

**Git (8)**
`git_status` · `git_diff` · `git_log` · `git_add` · `git_commit` · `git_blame` · `git_branch` · `git_stash`

**Data & Transform (6)**
`json_transform` · `json_schema_validate` · `sql_query` · `template_engine` · `test_data_gen` · `openapi_validate`

**Code Quality (7)**
`execute_test` · `test_run` · `code_coverage` · `code_metrics` · `lint_check` · `perf_analyze` · `dependency_graph`

**Infrastructure (9)**
`docker_tool` · `process_monitor` · `make_tool` · `dep_check` · `http_probe` · `http_request` · `task_track` · `ci_logs` · `checksum`

**Security (2)**
`secret_scan` · `path_security`

**Utilities (9)**
`url_parse` · `regex_test` · `token_count` · `parse_logs` · `changelog_gen` · `archive` · `diff_apply` · `patch_apply` · `fuzzy_find`

**Memory (1)**
`search_memory` — semantic search over auto-memory and vector store

</details>

**Risk tiers** — enforced at the executor layer before any execution:

| Tier | Examples | Behavior |
|------|----------|----------|
| `ReadOnly` | `file_read`, `grep`, `git_status` | Runs without confirmation |
| `ReadWrite` | `git_add`, `task_track` | Runs without confirmation |
| `Destructive` | `bash`, `file_write`, `git_commit` | Requires confirmation (configurable) |

Destructive tools are blocked from parallel execution (`execute_parallel_batch` guard) — they run sequentially to prevent race conditions.

---

## Agent Loop

<details>
<summary><b>Per-round phases</b></summary>

```
round_setup → provider_round → post_batch → convergence_phase → result_assembly → checkpoint
```

Each phase is a separate module under `crates/halcon-cli/src/repl/agent/`:

| Phase | Module | Responsibility |
|-------|--------|---------------|
| Pre-loop | `setup.rs` | Context assembly, HALCON.md load, hooks, memory injection |
| Round setup | `round_setup.rs` | HALCON.md hot-reload (per round), tool selection |
| Provider round | `provider_round.rs` | LLM API call with retry + circuit breaker |
| Post-batch | `post_batch.rs` | Tool execution (sequential + parallel DAG), FASE-2 gate |
| Convergence | `convergence_phase.rs` | SynthesisGate → TerminationOracle → RoutingAdaptor |
| Result assembly | `result_assembly.rs` | Output construction, auto-memory scoring |
| Checkpoint | `checkpoint.rs` | Session persistence, trace recording |

</details>

<details>
<summary><b>Boundary Decision Engine (pre-loop)</b></summary>

Before any LLM call, `IntentPipeline::resolve()` runs:

1. `BoundaryDecisionEngine` classifies the query → `BoundaryDecision` (routing mode, SLA budget)
2. `InputNormalizer` strips zero-width chars, detects language (EN/ES/Mixed), normalizes query
3. `IntentPipeline` reconciles intent score with boundary decision → `ResolvedIntent { effective_max_rounds }`
4. `ConvergenceController::new_with_budget()` is initialized with the pre-reconciled budget

**Routing modes:** `QuickAnswer` · `Balanced` · `DeepAnalysis` (constitutional constraint: never escalated down)

**Escalation triggers** (`RoutingAdaptor`):
- T1: security signals detected in round feedback
- T2: tool failure rate ≥ 60%
- T3: evidence coverage < 25% at round ≥ 4
- T4: combined convergence score > 0.90 at round ≥ 3

</details>

<details>
<summary><b>Tool execution safety</b></summary>

Two security layers run independently:

1. **FASE-2 path gate** (`executor.rs`) — 18 catastrophic patterns from `halcon_core::security::CATASTROPHIC_PATTERNS` checked before execution. Cannot be bypassed by agent configuration or hooks.

2. **DANGEROUS_COMMAND_PATTERNS** (12 patterns) — checked in `command_blacklist.rs`. Both lists compile from `halcon-core/src/security.rs` — single source of truth.

**Bash orchestration rules:**
- Core runtime tools (`bash`, `file_read`, `grep`) are never stripped from `cached_tools` post-delegation
- `run_command` → `bash` alias resolved via `tool_aliases::canonicalize()` before tool-surface narrowing
- `2>/dev/null` patterns use `^` anchoring to avoid false positives on legitimate redirects

</details>

---

## Memory Systems

Halcon uses three complementary memory systems that layer on top of each other.

### 1. HALCON.md — Persistent Instructions

4-scope hierarchy (last-wins injection order):

| Scope | Path | Notes |
|-------|------|-------|
| Local | `./HALCON.local.md` | git-ignored, dev overrides |
| User | `~/.halcon/HALCON.md` | personal preferences |
| Project | `.halcon/HALCON.md` + `.halcon/rules/*.md` | path-glob filtered rules |
| Managed | `/etc/halcon/HALCON.md` | operator policy, highest LLM weight |

Features: `@import` resolution (max depth 3, cycle detection, 64 KiB cap), hot-reload via `notify::recommended_watcher` (FSEvents/inotify, <100ms), YAML `paths:` glob filtering in rules.

### 2. Auto-Memory

Automatically captures knowledge during sessions. Files: `.halcon/memory/MEMORY.md` (180-line LRU cap) + `.halcon/memory/<topic>.md` (50-entry cap per topic).

| Trigger | Importance Score |
|---------|-----------------|
| User correction | 1.0 |
| Error recovery | 0.5 + magnitude |
| Tool pattern discovered | 0.6 |
| Task success | 0.2 + complexity |

Threshold: `memory_importance_threshold = 0.3` (configurable). Background write after `result_assembly` — never blocks response.

```sh
halcon memory search "authentication patterns"
halcon memory list --type code_snippet --limit 20
halcon memory stats
halcon memory clear project   # or: user
```

### 3. Vector Memory

TF-IDF hash embeddings + cosine similarity + MMR (max marginal relevance) retrieval, backed by `VectorMemoryStore` in `halcon-context`. Surfaced via the `search_memory` tool and `halcon memory search`.

---

## MCP Integration

### Halcon as MCP Server

```sh
# Claude Code / any MCP client (stdio transport)
claude mcp add halcon -- halcon mcp serve

# HTTP server with Bearer auth
halcon mcp serve --transport http --port 7777
# → HALCON_MCP_SERVER_API_KEY=<auto-generated 48-char hex key>
```

The HTTP server is built on axum and supports SSE streaming, `Mcp-Session-Id` session management with TTL expiry, audit tracing, and Bearer token auth. All Halcon tools are exposed as MCP tools.

### Halcon as MCP Client

Connect to external MCP servers:

```sh
# Add a server
halcon mcp add filesystem --command npx @modelcontextprotocol/server-filesystem /path

# Add HTTP server with OAuth
halcon mcp add my-api --url https://api.example.com/mcp
halcon mcp auth my-api   # OAuth 2.1 + PKCE flow, token stored in keychain

# List active connections
halcon mcp list
```

**Config** (`~/.halcon/mcp.toml`):
```toml
[[servers]]
name    = "filesystem"
command = ["npx", "@modelcontextprotocol/server-filesystem", "/home/user"]

[[servers]]
name      = "my-api"
url       = "https://api.example.com/mcp"
auth.type = "bearer"
auth.env  = "MY_API_TOKEN"
```

3-scope config: local > project > user. `${VAR:-default}` env expansion at connection time.

**Tool discovery** — `ToolSearchIndex` (nucleo-matcher fuzzy search) defers tool listing above 10% context threshold to avoid token waste.

---

## TUI

Enable with `halcon chat --tui` or `halcon --tui`.

<p align="center">
  <img alt="Halcon TUI — 3-zone layout with activity timeline and working memory" src="img/uxui.png" width="700">
</p>

**3-zone layout:**

| Zone | Content |
|------|---------|
| Left | Activity timeline — scrollable tool calls, agent badges, round markers |
| Center | Prompt editor (tui-textarea, multiline) + response stream |
| Right | Working memory panel — context budget, session stats |

**Keyboard shortcuts:**
```
Enter          submit prompt
Shift+Enter    newline in prompt
Tab            cycle focus zones
Ctrl+C         cancel in-progress request
Ctrl+L         clear activity timeline
Ctrl+Y         copy last response to clipboard
↑/↓/PgUp/PgDn scroll activity timeline
Esc            dismiss modal / overlay
```

**Features:** virtual scroll (handles thousands of activity entries without lag), conversational permission overlay (inline tool approval — no blocking modal), sub-agent progress badges, context token budget bar, toast notifications.

---

## Configuration

### Initial setup

```sh
halcon init          # interactive wizard
halcon auth login anthropic
halcon config show   # verify
```

### Config hierarchy (last wins)

```
CLI flags → env vars → ./.halcon/config.toml → ~/.halcon/config.toml → defaults
```

### `~/.halcon/config.toml` reference

```toml
[general]
default_provider = "anthropic"
default_model    = "claude-sonnet-4-6"
max_tokens       = 8192
temperature      = 0.0

[models.providers.anthropic]
enabled       = true
api_key_env   = "ANTHROPIC_API_KEY"
default_model = "claude-sonnet-4-6"

[models.providers.ollama]
enabled       = true
api_base      = "http://localhost:11434"
default_model = "llama3.2"

[tools]
confirm_destructive  = true
timeout_secs         = 120
allowed_directories  = ["/home/user/projects"]
blocked_patterns     = ["**/.env", "**/.env.*", "**/*.key", "**/*.pem"]

[security]
pii_detection          = true
pii_action             = "warn"   # warn | block | redact
audit_enabled          = true
audit_retention_days   = 90

[storage]
max_sessions          = 1000
max_session_age_days  = 90
```

See [docs/technical/CONFIGURATION_EXAMPLES.md](docs/technical/CONFIGURATION_EXAMPLES.md) for the full field reference.

### Environment variables

```sh
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
DEEPSEEK_API_KEY=sk-...
GEMINI_API_KEY=...
OLLAMA_HOST=http://localhost:11434

HALCON_MODEL=claude-sonnet-4-6
HALCON_PROVIDER=anthropic
HALCON_LOG=debug
HALCON_MCP_SERVER_API_KEY=...   # MCP HTTP server auth key
```

---

## Security

**FASE-2 gate** — 18 catastrophic patterns in `halcon-core/src/security.rs`:
- Filesystem destruction: `rm -rf /`, `format C:`, `dd if=/dev/zero`
- Credential exfiltration: curl/wget piped to remote hosts with tokens
- Fork bombs: `:(){ :|:& };:`
- Kernel module loading, `/proc/sysrq-trigger`, raw disk access

**DANGEROUS_COMMAND_PATTERNS** — 12 named G7 patterns (crypto miners, reverse shells, privilege escalation). Same source file — both `bash.rs` and `command_blacklist.rs` compile from this single truth.

**PII Detection** — configurable warn/block/redact on inputs and outputs.

**TBAC** (Tool-Based Access Control) — every tool declares its `PermissionLevel` (ReadOnly / ReadWrite / Destructive) and `AllowedDirectories`. Violations reject before execution.

**Keychain** — API keys stored in OS keychain (macOS Keychain, Linux Secret Service via D-Bus, Windows Credential Manager). Never written to config files unless explicitly overridden.

**Audit log** — append-only SQLite audit trail with HMAC-SHA256 chain validation (`halcon audit verify SESSION_ID`). SOC 2-compatible export in JSONL / CSV format.

See [SECURITY.md](SECURITY.md) for the vulnerability disclosure policy.

---

## Architecture

<details>
<summary><b>19-crate workspace</b></summary>

```
halcon-cli/
├── crates/
│   ├── halcon-cli/         # binary — REPL, TUI, commands, agent loop (337 files)
│   ├── halcon-core/        # domain types, traits, security patterns — zero I/O
│   ├── halcon-providers/   # AI adapters: Anthropic, OpenAI, Ollama, DeepSeek, Gemini, ClaudeCode
│   ├── halcon-tools/       # 60+ tool implementations (75 files)
│   ├── halcon-mcp/         # MCP client + HTTP server, OAuth 2.1, tool search (13 files)
│   ├── halcon-context/     # 7-tier context engine, embeddings, vector store (18 files)
│   ├── halcon-storage/     # SQLite persistence, migrations, audit, cache, metrics (33 files)
│   ├── halcon-runtime/     # DAG executor for parallel tool batches (21 files)
│   ├── halcon-search/      # BM25 + PageRank search engine (35 files)
│   ├── halcon-agent-core/  # GDEM experimental loop (26 files)
│   ├── halcon-multimodal/  # Image / audio / document processing (20 files)
│   ├── halcon-api/         # HTTP API types + axum server (25 files)
│   ├── halcon-auth/        # keychain, OAuth device flow, JWT
│   ├── halcon-security/    # guardrails, PII detection
│   ├── halcon-files/       # file access controls, 12 format handlers
│   ├── halcon-client/      # async typed HTTP + WebSocket SDK
│   ├── halcon-sandbox/     # rlimit / seccomp sandboxing
│   ├── halcon-desktop/     # egui control plane desktop app
│   └── halcon-integrations/# plugin extensibility framework
├── config/
│   └── default.toml        # built-in defaults
├── docs/
└── scripts/
```

</details>

<details>
<summary><b>Domain boundaries</b></summary>

`halcon-core` is a strict boundary — zero I/O, zero async, zero network imports. All 32 domain modules (`repl/domain/`, `repl/decision_engine/`) compile with no infrastructure dependencies.

Layer ordering (dependencies flow downward only):
```
halcon-cli   (binary, I/O, commands)
    ↓
halcon-providers, halcon-tools, halcon-mcp, halcon-context, halcon-storage
    ↓
halcon-core  (pure domain — types, traits, events)
```

</details>

<details>
<summary><b>Agent loop module map</b></summary>

```
crates/halcon-cli/src/repl/
├── agent/
│   ├── mod.rs              # run_agent_loop() — main entry (2,537 lines)
│   ├── loop_state.rs       # LoopState (62 fields) — refactor target B3
│   ├── context.rs          # AgentContext decomposed into 3 sub-structs
│   ├── round_setup.rs      # per-round init, HALCON.md hot-reload
│   ├── provider_round.rs   # LLM API call, retry, circuit breaker
│   ├── post_batch.rs       # tool execution results, round diagnostics
│   ├── convergence_phase.rs# SynthesisGate → TerminationOracle → RoutingAdaptor
│   ├── result_assembly.rs  # output construction, auto-memory scoring
│   ├── checkpoint.rs       # session persistence, trace recording
│   └── tests.rs            # 4,307 tests
├── decision_engine/
│   ├── intent_pipeline.rs  # IntentPipeline::resolve()
│   ├── routing_adaptor.rs  # 4-trigger escalation
│   ├── policy_store.rs     # runtime SLA constants
│   ├── domain_detector.rs
│   └── complexity_estimator.rs
├── domain/
│   ├── convergence_controller.rs
│   ├── termination_oracle.rs
│   ├── synthesis_gate.rs
│   ├── adaptive_policy.rs
│   ├── strategy_weights.rs
│   ├── round_feedback.rs
│   └── ...
├── auto_memory/            # scorer.rs, writer.rs, injector.rs
├── instruction_store/      # HALCON.md 4-scope loader, hot-reload
├── hooks/                  # lifecycle hooks — shell + Rhai
├── agent_registry/         # sub-agent loader, validator, skills
└── vector_memory_source.rs # VectorMemoryStore context source
```

</details>

---

## Contributing

Read [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) for the full workflow.

```sh
# Clone and build
git clone https://github.com/cuervo-ai/halcon-cli.git
cd halcon-cli
cargo build --features tui -p halcon-cli

# Run the test suite (4,300+ tests, ~2 min on macOS M-series)
cargo test --workspace --no-default-features

# Lint
cargo clippy --workspace --no-default-features -- -D warnings
cargo fmt --all -- --check
```

**Commit format** ([Conventional Commits](https://www.conventionalcommits.org/)):
```
feat(scope): short description
fix(scope): short description
refactor|docs|test|chore|ci(scope): short description
```

**Branch strategy:**
- `feature/*` → open PR to `main`
- CI runs on Linux (PR gate) + macOS (post-merge to `main`)
- Security checks (gitleaks, cargo-deny) run on every PR

---

## License

Halcon CLI is distributed under the **[Apache License 2.0](LICENSE)**.

---

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)"  srcset="img/cuervo-cloud-logo.png">
    <source media="(prefers-color-scheme: light)" srcset="img/cuervo-logo-2.png">
    <img alt="Cuervo AI" src="img/cuervo-logo-2.png" width="72">
  </picture>
  <br/>
  <sub>Built by <a href="https://github.com/cuervo-ai">Cuervo AI</a></sub>
</p>
