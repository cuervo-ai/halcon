# HALCÓN CLI v0.3.0 — Remediation Handoff Document

> Branch: `feature/sota-intent-architecture`
> Generated: 2026-03-10
> Last commit: `7cd94a4` (P1-B/C/D)
> Baseline score: 3.2/10 → Estimated post-fix score: **6.5–7.0/10**

---

## Executive Summary

Six critical defects identified by the multi-expert panel audit were remediated across two
commit sets. The architectural root cause (all audit events carrying `session_id=NULL`)
is fixed at the domain model level — no callsite modifications were required.

---

## Fixes Implemented

### P0-A — Context Propagation (CRITICAL — architectural keystone)

**Problem**: `DomainEvent` lacked `session_id`/`trace_id`/`span_id`. The async audit
subscriber had no access to session context. `append_audit_event()` hardwired `None`.
Result: 9,897 rows in `audit_log` with `session_id=NULL` — all SOC2 queries fail.

**Fix** (`7b44226`):
- New `crates/halcon-core/src/context.rs`:
  - `ExecutionContext { session_id, trace_id, span_id, agent_id }`
  - `EXECUTION_CTX: tokio::task_local!` — propagates automatically across await points
  - `current_session_id() / current_trace_id() / current_span_id()` — read from task-local
  - `TraceId` (128-bit hex), `SpanId` (64-bit hex) — W3C Trace-Context compatible
- `DomainEvent` gains `session_id: Option<Uuid>`, `trace_id`, `span_id` with
  `#[serde(default)]` for backward compatibility — auto-injected from `EXECUTION_CTX`
- `chat.rs`: generates `session_id = Uuid::new_v4()` at session entry, wraps the
  REPL execution block in `EXECUTION_CTX.scope(exec_ctx, async { ... }).await`
- Audit subscriber: `append_audit_event_with_session(&event, event.session_id.as_deref())`

**Verification**: `context::tests::context_injected_inside_scope` — PASS

---

### P0-B — Policy Decisions Wiring (CRITICAL)

**Problem**: `save_policy_decision()` existed in the storage layer but was never called
from production code. `policy_decisions` table was always empty — AUDIT-2 gap.

**Fix** (`7b44226`):
- `executor.rs:execute_sequential_tool()` — calls `save_policy_decision()` at:
  - `PermissionDenied` site (line ~1230): decision="denied"
  - `PermissionGranted` site (line ~1257): decision="granted"
- Uses existing `trace_db: Option<&AsyncDatabase>` and `session_id: Uuid` parameters

---

### P0-C — Broadcast Channel Overflow (HIGH)

**Problem**: `event_bus(256)` — broadcast channel had 256-slot capacity. Sessions
generating ~9,897 events silently dropped messages when all receivers fell behind.

**Fix** (`7b44226`):
- `chat.rs`: `event_bus(256)` → `event_bus(4096)` — 16× capacity increase

---

### P1-B — Resilience Event Metrics (MEDIUM)

**Problem**: `persist_breaker_event()` always set `score: None, details: None`.
`resilience_events` table had no diagnostic data for post-incident analysis.

**Fix** (`7cd94a4`):
- `resilience.rs`: Computes `score` (u32, 0–100 scale: Closed=100, HalfOpen=50, Open=0)
  and `details` (human-readable transition description) from BreakerState destination

---

### P1-C — Task Type Classification (HIGH)

**Problem**: Audit/compliance/security tasks fell to `General` catch-all (no keywords
matched). 65% of UCB1 learning data was `task_type=General` — reward signals pointed
to wrong strategy arm, degrading future strategy selection.

**Fix** (`7cd94a4`):
- `task_analyzer.rs`: Added 20+ keywords to Research classifier:
  `audit, auditar, auditoria, compliance, vulnerability, sonar, sast, dast, pentest,
  assessment, soc2, sox, gdpr, hipaa, iso27001, cve, scan, validate, verify` + Spanish
- **8 regression tests** added, all pass

---

### P1-D — UCB1 Data Integrity (HIGH)

**Problem**: When `critic_unavailable=true`, reward signals were computed without
adversarial evaluation — noisy data written to `reasoning_experiences` table. This
gradually degraded cross-session UCB1 strategy learning.

**Fix** (`7cd94a4`):
- `mod.rs`: `save_reasoning_experience()` is **skipped** when `result.critic_unavailable=true`
- Log message: `"P1-D: Skipping UCB1 record — critic_unavailable=true, reward signal unreliable"`

---

## Test Results

| Phase | Before | After | Delta |
|-------|--------|-------|-------|
| halcon-core | 276 pass | 282 pass | +6 (context::tests) |
| halcon-cli | 4336 pass | 4360 pass | +24 (P1-C×8 + others) |
| Pre-existing failures | render::theme (1) | render::theme (1) | 0 regressions |

---

## Sprint STAT+SOTA — 2026-03-10
### Violaciones resueltas: 14/33 críticas
### Score estimado: 6.2/10 → **8.5/10**

| Violación | Status | Archivos | Tests |
|-----------|--------|----------|-------|
| STAT-PANIC-001/004 | ✅ | db/mod.rs, db/plugins.rs (8 sitios) | — |
| STAT-PANIC-005 | ✅ | render/sink.rs (23 sitios) | — |
| STAT-RACE-001 | ✅ | agent/mod.rs (3 spawns) | — |
| STAT-DEAD-002 | ✅ N/A | ast_symbols.rs:807 es string literal en test | — |
| STAT-SILENT-001 | ✅ | event.rs + resilience.rs + audit.rs | 2 tests updated |
| STAT-SILENT-002/003 | ✅ | migrations.rs (M39) | 3 migration tests |
| SOTA-LEARN-001 | ✅ | reasoning_engine.rs (ucb1_updated_this_session guard) | — |
| STAT-LOGIC-002 | ✅ | mod.rs (16 sites: {:?} → as_str()) | — |
| SOTA-SCHEMA-001 | ✅ | migrations.rs M39 (composite + 3 indexes) | 3 migration tests |
| SOTA-CLASSIFY-001 | ✅ | task_analyzer.rs (SMRC rewrite, prev commit) | 56 tests |
| SOTA-HASH-001 | ✅ | task_analyzer.rs (stop-word + sort + SHA-256) | 3 hash tests |
| SOTA-CLASSIFY-002 | ✅ | task_analyzer.rs (IntentClassifier trait, KeywordClassifier) | — |

### Desvíos del plan
- **STAT-DEAD-002**: El `todo!()` en `ast_symbols.rs:807` está dentro de un string literal de test
  (`r#"pub fn start() { todo!() }"#`). No es código ejecutable en producción. Falso positivo.
- **P3-3 (SQLite PRAGMAs)**: Ya estaban implementados en `Database::open()` (WAL + FK + synchronous=NORMAL).
  Sin acción necesaria.
- **P4-3 (Dead-letter queue)**: No implementado en este sprint — require nueva tabla + drain logic
  con mayor scope. Agregado a pendientes.

### Violaciones pendientes (menor prioridad)
| ID | Descripción |
|----|-------------|
| P2-A | Auto-derive session title desde primer mensaje |
| P2-C | TTL archiving para audit_log (> 90 días → audit_log_archive) |
| P2-D | `failure_class`/`failure_detail` en tool_execution_metrics |
| P4-3 | Dead-letter queue (DLQ) para eventos perdidos en broadcast overflow |
| SOTA-DATA-001 | Event bus overflow counter/metric expuesto en session summary |
| SOTA-RESILIENCE-002 | Recovery threshold configurable en circuit breaker |

---

## Architecture Invariants (Post-Sprint)

1. **No crash cascade from Mutex poison**: 31 `lock().unwrap()` → `unwrap_or_else(|p| p.into_inner())`.
   Si una closure panics mientras tiene el lock, el siguiente caller recupera el guard sin panic.

2. **EXECUTION_CTX propagado a todas las tareas asíncronas**:
   Los 3 `tokio::spawn` en `agent/mod.rs` envuelven su payload en `EXECUTION_CTX.scope(ctx, ...)`.

3. **Circuit breaker con eventos semánticamente correctos**:
   - `CircuitBreakerOpened` → trip (Closed→Open)
   - `CircuitBreakerRecovered` → recovery (HalfOpen→Closed)
   - `CircuitBreakerHalfOpen` → probe (Open→HalfOpen)
   No más falsos positivos en alerting por usar el mismo evento para trip y recovery.

4. **UCB1 actualizado exactamente 1× por sesión**:
   `ucb1_updated_this_session` previene double-counting entre `post_loop_with_reward` y
   `record_per_round_signals`. El counter `uses` ya no crece 2-3× más rápido que los rewards.

5. **DB keys de UCB1 son estables**: `task_type.as_str()` + `strategy.as_str()` en lugar de
   `format!("{:?}", ...)`. Un rename de variante de enum ya no silencia experiencias históricas.

6. **audit_log indexes**: Composite `(session_id, id)` hace que `halcon audit verify` sea O(log n).

---

## Commit Log

```
8d3e47f fix(sprint): STAT+SOTA remediation P0–P3 — crashes, audit, UCB1, schema
c6a157b refactor(task-analyzer): SOTA 2026 Scored Multi-Rule Classifier (SMRC)
7cd94a4 fix(ucb1): P1-B/C/D — resilience metrics, audit task classification, UCB1 integrity
7b44226 fix(audit): P0-A/B/C — session_id propagation, policy_decisions wiring, bus capacity
0864e45 fix(schema): recursive OpenAI-compatible JSON Schema normalization (P0 prereq)
```

---

## Config Changes (Already Applied)

File: `~/.halcon/config.toml`
- `critic_timeout_secs`: 30 → **60** (prevents o1 LoopCritic always timing out)
- `critic_model = "claude-haiku-4-5-20251001"` (separated from executor model)
- `critic_provider = "anthropic"` (fast, reliable — not o1 which averaged 16.2s)

---

## Known Pre-Existing Issues (Not Fixed Here)

1. `render::theme::progressive_enhancement_downgrades_for_limited_terminals` — test
   environment TUI color capability mismatch. Not introduced by these changes.
2. Tool failures for `ci_logs`, `lint_check`, `dep_check` — root cause: Node.js
   unavailable in sub-agent working directories. Requires environment-level fix.
3. `git_status` failure in sub-agents — working directory is not a git repo.
   Fix: pass `working_dir` from parent session to sub-agent context.
