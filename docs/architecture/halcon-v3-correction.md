# Halcon v3 — Corrección formal de arquitectura de frontera

> **Status**: `DRAFT` → pending sign-off de 3 arquitectos para promover a `FROZEN`.
> **Version**: `0.1.0`.
> **Audience**: Principal architects, platform engineers, contributors al repositorio `cuervo-cli`.
> **Scope**: rol de Halcon dentro del ecosistema CUERVO (Halcon ∪ Paloma ∪ Tordo ∪ Cenzontle).
> **Normative**: este documento es la **fuente de verdad arquitectónica** del sistema. Cualquier PR que viole una obligación declarada aquí DEBE ser rechazado o escalado a decisión de arquitectura formal.

---

## Índice

1. [Propósito y alcance](#1-propósito-y-alcance)
2. [El problema estructural](#2-el-problema-estructural)
3. [Ecosistema CUERVO — arquitectura de frontera objetivo](#3-ecosistema-cuervo--arquitectura-de-frontera-objetivo)
4. [Rol correcto de Halcon](#4-rol-correcto-de-halcon)
5. [Obligaciones formales (Ω-01 .. Ω-20)](#5-obligaciones-formales-ω-01--ω-20)
6. [Invariantes verificables (I-H1 .. I-H14)](#6-invariantes-verificables-i-h1--i-h14)
7. [Contratos cross-boundary requeridos](#7-contratos-cross-boundary-requeridos)
8. [Anti-patrones a eliminar obligatoriamente](#8-anti-patrones-a-eliminar-obligatoriamente)
9. [Plan de corrección por ciclos](#9-plan-de-corrección-por-ciclos)
10. [Enforcement en CI/CD](#10-enforcement-en-cicd)
11. [Promesa de producto honesta](#11-promesa-de-producto-honesta)
12. [Matriz de cumplimiento](#12-matriz-de-cumplimiento)
13. [Glosario](#13-glosario)
14. [ADRs relacionados](#14-adrs-relacionados)

---

## 1. Propósito y alcance

Este documento formaliza las **obligaciones de diseño** que Halcon debe cumplir para integrarse correctamente con Paloma, Tordo y Cenzontle como parte del ecosistema CUERVO. No es una propuesta ni un roadmap general: es un contrato arquitectónico cuyas obligaciones son **ejecutables por CI** y **verificables por evidencia en código**.

### 1.1 Qué NO es este documento

- No es un README comercial.
- No es una lista de mejoras opcionales.
- No es un roadmap de features.
- No propone cambios a Paloma/Tordo/Cenzontle salvo que sean **prerequisito estructural** de una obligación de Halcon.

### 1.2 Autoridad

Este documento es autoritativo cuando haya conflicto con:
- README de Halcon.
- Comentarios en código.
- Decisiones ad-hoc de PR.
- Documentación legacy en `docs/` pre-v3.

En caso de ambigüedad, gana el código real del repositorio; este documento describe hacia dónde el código debe converger.

### 1.3 Alcance

Cubre:
- Topología de componentes Halcon ↔ {Paloma, Tordo, Cenzontle, paloma-ledger}.
- Ownership por capability.
- Wire contracts formales en los bordes.
- Invariantes verificables por propiedad.
- Plan de transición Ciclos 0-6.

No cubre:
- Detalles internos de Paloma/Tordo/Cenzontle (tienen sus propios ADRs).
- Decisiones de UX CLI (fuera de alcance de arquitectura de frontera).
- Features futuras del agent plane.

---

## 2. El problema estructural

### 2.1 Hipótesis central (confirmada por código)

**Halcon fue concebido como thin planner y se implementó como monolito agéntico in-process.** Resultado observable en el repositorio:

1. `halcon-providers/src/router/paloma_adapter.rs:25` importa `paloma_pipeline::Pipeline` y la **instancia** localmente (`Pipeline::new(PipelineConfig::default())`). Paloma fue diseñada como servicio HTTP content-isolated; Halcon la invoca como crate con acceso al content.
2. `halcon-providers/src/{anthropic,openai,deepseek,gemini,bedrock,vertex,azure_foundry,ollama,claude_code}/` son **9 adapters HTTP directos a LLMs**. Cenzontle es el gateway canónico por diseño del ecosistema; Halcon lo bypassea.
3. `grep -r tordo crates → 0`: Halcon **desconoce** Tordo. Tordo fue diseñado explícitamente como execution fabric durable al que Halcon debía delegar.
4. `halcon-api/src/server/handlers/tasks.rs` expone `POST /tasks` con ejecución de DAGs — **duplica conceptualmente** `tordo-api /v1/jobs`.
5. `halcon-storage/src/audit.rs` firma eventos HMAC locales; no replica a ningún ledger central.
6. `grep schema_version crates → 0` en Halcon: **cero wire types versionados**. Tordo y Paloma sí los tienen (`tordo-contracts::plan` incluye `schema_version`, `plan_version`).
7. Sin `scripts/check_boundaries.sh` hasta Ciclo 0 de este plan: **cero enforcement automático**. Paloma y Tordo sí lo tienen.

### 2.2 Tipos de violación de frontera presentes

| Tipo | Ejemplo concreto | Consecuencia observable |
|------|------------------|-------------------------|
| **Semántica** | Halcon "posee" capabilities que no le corresponden (routing, budget, execution durable, audit canonical). | Duplicación de lógica; drift multi-tenant; hallucinations bajo fallo. |
| **Runtime** | Halcon instancia `paloma_pipeline::Pipeline::new()` en proceso. | INV-6 content-isolation violada en runtime; estado de budget no reconciliable con Paloma real. |
| **Contrato** | Wire types sin `schema_version`; Halcon usa `paloma-types` (crate interno). | Evolución imposible; release coupling. |
| **Gobierno** | No CI boundary check; no cargo-deny policy. | Regresión silenciosa inevitable. |

### 2.3 Incidentes observables causados por el problema estructural

Registrados en sesión `9f9a0cf0` del 2026-04-17 (ver `docs/architecture/adr/ADR-HALCON-001-capabilities-scope.md` §Apéndice A):

- Empty SSE stream de Azure Container Apps (backend de Cenzontle) interpretado como éxito con `success=1, tokens=0/0, text=""` por parser local de Halcon.
- `directory_tree` EACCES en subcarpeta causó `LoopGuardStagnationDetected` en round 0 → `tools_suppressed=65` en round 1 → modelo alucinó "Salud del Proyecto: 12/100" sin evidencia.
- Planning timeout fijo 15s fallaba consistentemente con modelos BALANCED (DeepSeek-V3.2) que necesitan >20s.
- `should_trigger_replan` con `replan_sensitivity=1.0` efectiva_rounds=1 disparaba replan sobre un único round fallido.

Ninguno era bug aislado: todos son síntomas de la misma violación estructural.

---

## 3. Ecosistema CUERVO — arquitectura de frontera objetivo

### 3.1 Diagrama de planos ortogonales

```
┌────────────────────── USER / OPERATOR ───────────────────────┐
│      halcon chat --tui    //    halcon mcp-server            │
└─────────────────────────┬─────────────────────────────────────┘
                          │
             ┌────────────▼─────────────┐
             │   HALCON (agent plane)    │
             │                           │
             │  Capabilities owned:      │
             │   • agent.plan            │
             │   • agent.sandbox         │
             │   • session.conversation  │
             │   • tui.render            │
             │   • plan.build_and_sign   │
             │   • tool.execute_local    │
             │                           │
             │  Never owns:              │
             │   ✗ routing               │
             │   ✗ inference gateway     │
             │   ✗ budget state          │
             │   ✗ durable execution     │
             │   ✗ retry policy          │
             │   ✗ audit SSOT            │
             └──┬────────────────────┬───┘
                │                    │
       ExecutionPlan (HMAC)      MCP model_call
                │                    │
                ▼                    ▼
        ┌──────────────┐    ┌────────────────┐
        │    TORDO     │    │   CENZONTLE    │
        │ (execution)  │    │  (data plane)  │
        │              │    │                │
        │ POST /v1/jobs│    │  MCP server    │
        │ SSE events   │    │  model_call    │
        │ Postgres Q   │    │  tool_call     │
        │ Replay det.  │    │  cap.negotiate │
        └──┬───────────┘    └──┬─────────────┘
           │                   │
           │  per-step         │  POST /v1/route
           │  model_call       │  POST /v1/outcome
           └──► Cenzontle      ▼
                         ┌──────────────┐
                         │    PALOMA    │
                         │  (control)   │
                         │              │
                         │  Thompson    │
                         │  Budget      │
                         │  Plan sign   │
                         │  INV-6       │
                         └──┬───────────┘
                            │
                            │  audit events
                            ▼
                      ┌──────────────────┐
                      │  PALOMA-LEDGER   │
                      │  (audit SSOT)    │
                      │                  │
                      │  Append-only     │
                      │  Hash-chained    │
                      │  Compliance qry  │
                      └──────────────────┘
```

### 3.2 Planos (planes) — descripción normativa

| Plano | Owner | Concerns | Qué NO contiene |
|-------|-------|----------|-----------------|
| **Agent plane** | Halcon | Reasoning, planning, tool sandbox, TUI/REPL, session UX | Routing, inference, durable execution, audit canonical |
| **Control plane** | Paloma | Routing decision, budget lifecycle, Thompson Sampling, plan signing | Content, execution, inference |
| **Data plane** | Cenzontle | MCP gateway, LLM inference, SSE parsing, stall detection, capability negotiation, outcome relay | Agent reasoning, durable execution |
| **Execution plane** | Tordo | Durable job queue, step dispatch, retry, replay, artifacts | Routing, inference, agent reasoning |
| **Audit plane** | paloma-ledger | Immutable append-only log, compliance queries | Mutación, ejecución, decisión |

### 3.3 Regla estructural — acceso a Paloma

**Halcon NO debe tener conexión HTTP directa a Paloma `/v1/route` ni `/v1/outcome`.** Paloma se accede vía Cenzontle por diseño del ecosistema:
- Cenzontle internamente llama `/v1/route` (código real en `cenzontle/packages/backend/src/modules/paloma/paloma-client.service.ts`).
- Cenzontle internamente llama `/v1/outcome` tras ejecutar inferencia.
- Halcon sólo ve el resultado via MCP.

**Excepción controlada — sólo lectura**: `GET /v1/ledger/session/:id/usage` puede ser directo (display de cost en UI, cache 60s). Marcado como `read-only` en contract. **No es decisión**, es observabilidad.

---

## 4. Rol correcto de Halcon

### 4.1 Responsabilidades permitidas (agent plane)

Halcon es dueño exclusivo de:

1. **Reasoning local y planning** — generar `ExecutionPlan` a partir del prompt del usuario y del contexto de sesión.
2. **Tool sandbox** — ejecución local de tools (filesystem, bash, MCP-hosted) con gating de permisos.
3. **Session UX** — TUI, REPL, render, panel de progreso, keybindings.
4. **Session conversation state** — histórico local `~/.halcon/halcon.db` (último N mensajes).
5. **Plan emission** — firmar y enviar `ExecutionPlan` a Tordo cuando aplique.
6. **Consume streams tipados** — `Stream<ChunkKind>` del MCP de Cenzontle; `Stream<JobEvent>` de Tordo.
7. **Integración con developer tools locales** — LSP, git, file ops, AST.
8. **Permission pipeline** — 3-gate check local (hook → rule → risk) antes de ejecutar tools.

### 4.2 Responsabilidades prohibidas

1. ❌ **No instancia** componentes internos de Paloma (`Pipeline`, `BudgetStore`, `HealthTracker`, `RegistrySnapshot`, `ThompsonSamplingPolicy`).
2. ❌ **No decide routing**: ningún `IntelligentRouter::route()` ni `PalomaRouter::route()` en production path.
3. ❌ **No habla con LLMs directamente** en production: requiere gateway Cenzontle.
4. ❌ **No parsea SSE crudo** en production: consume `Stream<ChunkKind>` tipado.
5. ❌ **No ejecuta DAGs durables** localmente: delega a Tordo cuando se requiere durabilidad.
6. ❌ **No expone API de jobs**: `tordo-api` es único productor.
7. ❌ **No retries** en production path.
8. ❌ **No calcula pricing oficial**: consulta ledger (read-only display).
9. ❌ **No mantiene budget state authoritative**: consulta Paloma vía Cenzontle.
10. ❌ **No firma plans sin spec**: sólo plans tipados, versionados, canonicalizados.

---

## 5. Obligaciones formales (Ω-01 .. Ω-20)

Cada obligación se deriva de un hallazgo concreto. Estado se actualiza vía PR que cierra la obligación y este documento se re-versiona.

| # | Obligación | Propiedad | Owner | Estado | Enforcement |
|---|-----------|-----------|-------|--------|-------------|
| Ω-01 | Eliminar crates internos de Paloma de `halcon-providers`; consumir vía HTTP a Cenzontle (que internamente usa Paloma) | P-UNIQ-OWNER(routing) | Halcon | `pending` | `scripts/check_boundaries.sh` §[2] |
| Ω-02 | 9 adapters LLM bajo `#[cfg(feature="dev-providers")]`; production default sólo `cenzontle-mcp` | P-GATEWAY-UNIQUENESS | Halcon | `pending` | `scripts/check_boundaries.sh` §[1,7] |
| Ω-03 | Añadir `tordo-contracts` como dep + delegar ejecución durable | P-DURABLE-EXECUTION | Halcon | `pending` | Integration test `SIGKILL → replay` |
| Ω-04 | CI boundary enforcement activo | P-ENFORCEMENT | Halcon | **`in-progress`** (warn mode activo) | `.github/workflows/ci.yml: boundary-check` |
| Ω-05 | `schema_version` en todo wire DTO cross-boundary | P-VERSIONED-CONTRACT | Halcon | `pending` | CI grep §[4] + schemathesis |
| Ω-06 | Halcon no parsea SSE crudo en production; consume `ChunkKind` tipado | P-PARSE-AT-GATEWAY | Cenzontle (producer) + Halcon (consumer) | `pending` | CI grep `text/event-stream` en production |
| Ω-07 | Empty stream → error tipado; detector sin guard `round > 0` | P-NO-SILENT-FAIL | Cenzontle + Halcon | **`done`** | Test `p0_empty_stream_terminates_cleanly` + `replan_sensitivity_one_respects_hard_min_two_rounds` |
| Ω-08 | Planning timeout adaptativo por tier | Product correctness | Halcon | **`done`** | Test `adaptive_timeout_tests::*` (4 tests) |
| Ω-09 | Audit events replicados a paloma-ledger (prerequisito: ledger service) | P-AUDIT-SSOT | Paloma (prereq) + Halcon | `pending` | Integration test `session → ledger` |
| Ω-10 | MCP `capability.negotiate(model_id)` formal | P-CAPABILITY-CONTRACT | Cenzontle (prereq) + Halcon | `pending` | Contract test MCP |
| Ω-11 | Production path NO usa `halcon-runtime::executor`; usa Tordo | P-DURABLE-STATE | Halcon + Tordo | `pending` | Chaos test `SIGKILL → resume` |
| Ω-12 | Jerarquía única de retry: Tordo > Cenzontle > Paloma fallback. Halcon NO retries | P-BOUNDED-RETRY | Halcon | `pending` | TLA+ retry FSM + grep |
| Ω-13 | `RouteRequestMetadata` no incluye content (typestate) | P-CONTENT-ISOLATION (INV-6) | Halcon | `pending` | Compile-time |
| Ω-14 | Unidad de replay canónica: `(tenant_id, session_id, plan_id, step_seq)` | P-REPLAY-UNIT | Tordo + Halcon | `pending` | TLA+ spec + 1000 fixture replay |
| Ω-15 | Halcon `cost_table` marcado display-only; SSOT = paloma-ledger | P-LEDGER-SSOT | Paloma + Halcon | `pending` | Reconciliation job |
| Ω-16 | Halcon depende sólo de `paloma-boundary` (DTOs wire-stable) | P-STABLE-SHARED-TYPES | Paloma (separación) + Halcon (migración) | `pending` | `cargo tree` + CI |
| Ω-17 | Audit sink activo en production (no condicional a feature dormant) | P-FEATURE-INTEGRITY | Halcon | **`done`** | `audit_events.db` rows > 0 tras sesión |
| Ω-18 | Outcome reporting a Paloma vía Cenzontle (no local) | P-FEEDBACK-CLOSURE | Cenzontle + Halcon | `pending` | SLO `|outcomes| ≥ |sessions|` |
| Ω-19 | Feature flags `cfg(feature="X")` sólo si X declarada en Cargo.toml | P-NO-DEAD-FEATURE-FLAG | Halcon | **`done`** | `scripts/check_boundaries.sh` §[5] |
| Ω-20 | Trace W3C propagation obligatoria en todo HTTP client | P-TRACE-CONTINUITY | Halcon | `pending` | OTel collector validation |

### 5.1 Estado agregado

| Estado | Conteo |
|--------|-------|
| **done** | 4 (Ω-07, Ω-08, Ω-17, Ω-19) |
| **in-progress** | 1 (Ω-04, warn mode) |
| **pending** | 15 |

**Total obligaciones: 20.** **Completadas: 20% (4/20).** **Bloqueadas por prereq externo: 4 (Ω-09, Ω-10, Ω-15, Ω-16).**

---

## 6. Invariantes verificables (I-H1 .. I-H14)

Cada invariante tiene definición precisa, condición de violación, y método de verificación.

### I-H1. Gateway uniqueness
- **Definición**: `∀ proceso halcon en production: ∀ conexión c → c.dest ∈ {cenzontle_mcp, tordo_api, paloma_ledger, sso}`.
- **Violación**: conexión directa a upstream LLM.
- **Verificación**: `strings $(which halcon) | grep -E "api\.(anthropic|openai|deepseek)\.com"` debe ser empty en release default.
- **Estado**: INCUMPLIDO.

### I-H2. No routing logic in production
- **Definición**: `build(--release, sin dev-providers): ∄ symbol ∈ {IntelligentRouter::route, PalomaRouter::route, route_and_reserve}`.
- **Violación**: símbolo presente.
- **Verificación**: `nm target/release/halcon | grep -E "IntelligentRouter|PalomaRouter" → empty`.
- **Estado**: INCUMPLIDO.

### I-H3. No budget state local
- **Definición**: Halcon no mantiene estructuras autoritativas de budget; `PalomaReservation` es handle opaco de HTTP.
- **Verificación**: typestate — `PalomaReservation` sólo construible por deserialización; no `::new()` público.
- **Estado**: INCUMPLIDO.

### I-H4. Plan integrity from emission
- **Definición**: `∀ ExecutionPlan p emitido por Halcon → verify_hmac(p) ∧ p.schema_version ∈ SupportedVersions ∧ p.plan_id único`.
- **Verificación**: property test con 10k shrinks de plan mutations; 100% rechazo.
- **Estado**: INCUMPLIDO (Halcon no emite plans todavía).

### I-H5. Content isolation at routing boundary
- **Definición**: `∀ RouteRequestMetadata: fields ∩ {messages, prompt, response, content, text} = ∅`.
- **Verificación**: typestate Rust — `RouteRequestMetadata` sin campo de texto.
- **Estado**: VIOLADO (`paloma_adapter.rs` recibe `req.messages`).

### I-H6. No silent failure on stream
- **Definición**: `∀ MCP stream consumed: chunks_received = 0 → emit AgentLoopResult::StreamError(ErrorClass::StreamStalled)`.
- **Verificación**: property test con mock MCP stream vacío.
- **Estado**: PARCIAL (detector local activo post Ω-07; gateway gateway signal pendiente Ω-06).

### I-H7. No local retries in production
- **Definición**: `∀ módulo production: ∄ loop de retry sobre MCP/HTTP`.
- **Verificación**: AST grep + `#[cfg(feature="dev-providers")]` gate.
- **Estado**: INCUMPLIDO (RetryPolicy presente en production).

### I-H8. Replay unit well-defined
- **Definición**: unidad canónica `(tenant_id, session_id, plan_id, step_seq)`; Halcon no persiste ExecutionState local.
- **Verificación**: chaos test SIGKILL → resume via Tordo SSE.
- **Estado**: INCUMPLIDO.

### I-H9. Versioned wire contracts
- **Definición**: `∀ DTO cross-boundary → "schema_version" ∈ fields(DTO)`.
- **Verificación**: CI grep + derive macro + `#[serde(deny_unknown_fields)]`.
- **Estado**: INCUMPLIDO (0 hits).

### I-H10. Audit replication bounded delay
- **Definición**: `∀ AuditEvent e generado en t: ◇_{t+5min} (e ∈ ledger ∨ e ∈ outbox_local)`.
- **Verificación**: SLO + reconciliation job.
- **Estado**: PARCIAL (local sí, ledger no existe aún).

### I-H11. Feedback closure via Cenzontle
- **Definición**: outcome → Paloma `/v1/outcome` sólo desde Cenzontle; Halcon NO reporta directo.
- **Verificación**: grep `record_success` en Halcon production → 0.
- **Estado**: VIOLADO.

### I-H12. Trace continuity
- **Definición**: `∀ HTTP call c → c.headers["traceparent"] bien formado (W3C §3.2)`.
- **Verificación**: middleware assertion + OTel collector.
- **Estado**: INCUMPLIDO.

### I-H13. Feature flag integrity
- **Definición**: `∀ cfg_feature F en código → F ∈ Cargo.toml.features`.
- **Verificación**: `scripts/check_boundaries.sh §[5]`.
- **Estado**: **DONE** (check activo, warnings documentados).

### I-H14. Deprecated path sunset
- **Definición**: `∀ #[deprecated] → incluye since="X.Y.Z" + note con fecha de hard removal`.
- **Verificación**: clippy `deprecated_missing_note`.
- **Estado**: INCUMPLIDO.

---

## 7. Contratos cross-boundary requeridos

Detalle de request/response schemas, error model, idempotency, retry semantics, replay semantics, observability, auth, owner y validación en §6 del documento principal. Puntos clave:

### 7.1 Halcon → Tordo: `POST /v1/jobs`
- **Request**: `tordo_contracts::ExecutionPlan` con `schema_version`, `plan_version`, `plan_id` Uuid v7, HMAC/Ed25519 signature.
- **Idempotency**: `plan_id` único; repetición devuelve job existente.
- **Retry**: sólo para `ErrorClass::Transient`.
- **Owner**: Tordo.

### 7.2 Halcon ← Tordo: `GET /v1/jobs/:id/events` (SSE)
- **Event DTO**: `tordo_contracts::JobEvent` con `schema_version`, `seq` monotónico.
- **Resumption**: `Last-Event-Id` header.
- **Heartbeat**: cada 15s.

### 7.3 Halcon → Cenzontle MCP: `model_call`
- **Response**: stream tipado `ChunkKind::{TextDelta, ThinkingDelta, ToolUseStart, ToolUseDelta, Usage, Completed{reason}, Heartbeat{stage, elapsed_ms}, Error{class}}`.
- **Retry**: Cenzontle retries upstream; Halcon **no** retries.

### 7.4 Halcon → Cenzontle MCP: `capability.negotiate`
- **Response**: `ModelCapability { tool_result_format, context_window, supports_streaming, ... }`.
- **Cache**: 5 min TTL.

### 7.5 Halcon → paloma-ledger: `POST /v1/audit/ingest`
- **Request**: batch `AuditEvent[]` con HMAC per-tenant.
- **Idempotency**: `event_id` único.
- **Retry**: exp backoff con outbox local persistente.

### 7.6 Halcon → paloma-ledger: `GET /v1/ledger/session/:id/usage` (read-only)
- Display UX; cache 60s; no decision data.

---

## 8. Anti-patrones a eliminar obligatoriamente

Los 13 anti-patrones están detallados en el documento de corrección principal. Los 4 críticos:

| AP | Evidencia | Corrección | Criterio verificable |
|----|-----------|-----------|---------------------|
| **AP-H1** Router in-process | `paloma_adapter.rs:74` | `cargo rm` crates paloma-internal + HTTP client | `cargo tree | grep paloma-pipeline → empty` |
| **AP-H2** LLM SDK en production | `halcon-providers/{anthropic,openai,...}` | Feature gate `dev-providers` | `strings halcon | grep api.anthropic.com → empty` |
| **AP-H3** Empty-response heurística local | `provider_round.rs:1585` (fixed parcialmente Ω-07) | Confiar en `ChunkKind::Error(StreamStalled)` | grep `output_tokens == 0 &&` → 0 |
| **AP-H4** `halcon-api /tasks` duplica `tordo-api /v1/jobs` | `halcon-api/src/server/handlers/tasks.rs` | Eliminar endpoints de jobs | Route inventory ∩ tordo-api = ∅ |

---

## 9. Plan de corrección por ciclos

### Ciclo 0 — Freeze de invariantes (2 semanas)
**Objetivo**: ADRs firmados + spec versionada.
**Entregables**:
- Este documento (`halcon-v3-correction.md`) ✅
- ADR-HALCON-001 "Capabilities scope" ✅
- `scripts/check_boundaries.sh` en CI (warn mode) ✅
- TLA+ spec `halcon_plan_lifecycle.tla` (placeholder)
- Alloy model `halcon_ownership.als` (placeholder)

**Criterio de salida**: 3 arquitectos firman; Git tag `halcon-spec/v0.1`.
**Estado**: **in-progress** (4 obligaciones cerradas, 15 pendientes, enforcement activo).

### Ciclo 1 — Contratos y versionado (2 semanas)
**Objetivo**: wire DTOs con `schema_version`; clientes tipados generados.
**Entregables**:
- `halcon-plan-builder` crate.
- `halcon-tordo-client` crate con `schemathesis` contract tests.
- `halcon-ledger-client` crate con outbox.
- `halcon-cenzontle-mcp-client` cliente MCP tipado.
- CI contract test matrix.

**Criterio de salida**: contract tests pasan; property tests P-H3, P-H7 pasan.

### Ciclo 2 — Eliminación de bypass (3 semanas)
**Objetivo**: production binary sin routing/LLM/retry local.
**Entregables**:
- Feature `dev-providers` con `default=false`.
- 9 adapters + router + paloma_adapter + executor bajo `dev-providers`.
- `halcon-api /tasks` eliminado.
- `check_boundaries.sh --strict` bloqueante.

**Criterio de salida**: `strings halcon-release | grep api.anthropic.com → empty`; `nm halcon | grep IntelligentRouter → empty`.

### Ciclo 3 — Durabilidad, replay, audit (4 semanas)
**Objetivo**: Tordo integrado; paloma-ledger wired.
**Entregables**:
- `halcon-tordo-client` en production path (flag runtime).
- Chaos tests SIGKILL + replay.
- paloma-ledger service (prereq Paloma).
- `halcon-ledger-client` outbox.
- `StepKind::AgentRound` en Tordo (prereq).

**Criterio de salida**: SIGKILL worker mid-step → replay → artifacts idénticos.

### Ciclo 4 — Verificación formal (3 semanas)
**Objetivo**: evidence suite por propiedad.
**Entregables**:
- TLC runs sobre TLA+ specs.
- Alloy runs sobre ownership model.
- `proptest` suite en 4 crates nuevos.
- Fuzz corpus 24h sin crashes.
- Trace validation job.

**Criterio de salida**: cada P-H1..P-H12 con `status ∈ {proved, model_checked, property_tested, contract_tested, runtime_asserted}`.

### Ciclo 5 — Validación de producto (3 semanas)
**Objetivo**: benchmark público vs peers.
**Entregables**:
- 100-prompt suite comparativa vs {Claude Code, Cline, Cursor, Aider}.
- SLO dashboard.
- Producto docs actualizado con promesas §11.1.

**Criterio de salida**: top-quartile en 3/4 métricas clave.

### Ciclo 6 — Hardening y publication (3 semanas)
**Objetivo**: spec pública + enterprise readiness.
**Entregables**:
- Hard removal de legacy paths.
- SOC 2 readiness.
- Pen test.
- Spec peer-reviewed externamente.

**Criterio de salida**: pen test clean; external reviewer firma.

---

## 10. Enforcement en CI/CD

### 10.1 `scripts/check_boundaries.sh`

7 checks implementados. Modo default = warnings; modo `--strict` falla build.

| Check | Cubre | Estado |
|-------|-------|--------|
| [1] LLM SDK directos | Ω-02, P-GATEWAY | warn |
| [2] Crates internos Paloma | Ω-01, Ω-16 | warn |
| [3] Símbolos Paloma in-proc | Ω-01, P-UNIQ-OWNER | warn |
| [4] schema_version en wire DTOs | Ω-05 | info |
| [5] Feature flag integrity | Ω-19 (I-H13) | **enforcing** |
| [6] `halcon-api /tasks` duplicate | Ω-08 | warn |
| [7] URLs LLM en binary release | Ω-02 | warn (requiere build release) |

### 10.2 `.github/workflows/ci.yml`

Job `boundary-check` añadido como required status check. Depende de nada (corre en paralelo con fmt/clippy). No bloquea hoy (warn mode) pero se promueve a strict en Ciclo 2.

### 10.3 `cargo deny` policy (pendiente Ciclo 1)

Deny-list para:
- LLM SDKs no permitidos en production.
- Licencias incompatibles.
- RUSTSEC advisories open.

### 10.4 Contract tests (pendiente Ciclo 1)

- `schemathesis` contra staging Paloma/Tordo/Cenzontle.
- MCP conformance suite.
- Rust `proptest` de wire DTOs.

---

## 11. Promesa de producto honesta

### 11.1 Qué puede prometer HOY (post obligaciones done)

- CLI reasoning agent con sandbox tools typed + permission pipeline 3-gate.
- MCP host + server con OAuth 2.1 PKCE.
- Session local persistente con replay de conversación.
- Multi-provider via Cenzontle gateway (un único credential set).
- Audit HMAC local con hash-chain verificable.
- Empty-response tipado (post Ω-07).
- Planning adaptativo por tier (post Ω-08).

### 11.2 Qué NO debe prometer aún

- Durabilidad de sesiones con restart safety (requiere Ciclo 3).
- Multi-tenant enforcement (requiere Ciclo 3 + SSO key mgmt).
- Cost accuracy enterprise (requiere Ciclo 4 + reconciliation).
- Zero silent failure end-to-end (requiere Ciclo 2: gateway typed errors).
- SLA de latencia (sin benchmarks públicos).
- Compliance audit trail (requiere ledger service operativo).

### 11.3 Qué debe demostrar antes de prometerlo

| Promesa | Prueba requerida |
|---------|-----------------|
| "Durable sessions" | Chaos test SIGKILL + replay passes 1000 seeds |
| "Multi-tenant" | Property test isolation + pen test |
| "Cost ±5%" | Reconciliation job CI 4 semanas vs ledger |
| "Zero silent fail" | Contract test suite + staging 2 semanas |
| "Enterprise audit" | SOC 2 readiness + external auditor |

### 11.4 Capacidades competitivas tras Ciclo 5

- **Durable session resume** — ningún CLI competidor lo tiene.
- **Content-isolated routing** — único diferenciador compliance.
- **Formally verified budget lifecycle** — único diferenciador rigurosidad.
- **MCP-native + OAuth PKCE + permission pipeline 3-gate**.

---

## 12. Matriz de cumplimiento

| Criterio | Estado actual | Estado objetivo | Condición de cumplimiento |
|----------|--------------|----------------|--------------------------|
| C-H1 Gateway uniqueness | incumplido | cumplido | `strings halcon-release | grep api.anthropic.com → empty` |
| C-H2 No routing local | incumplido | cumplido | `nm halcon | grep IntelligentRouter → empty` |
| C-H3 No budget local | incumplido | cumplido | `cargo tree | grep paloma-budget → empty` |
| C-H4 No execution durable local | incumplido | cumplido | chaos test SIGKILL → resume pass |
| C-H5 No retry local | incumplido | cumplido | AST grep `retry` en production → 0 |
| C-H6 Versioned contracts | incumplido | cumplido | 100% wire DTOs con `schema_version` |
| C-H7 Plan signed | incumplido | cumplido | proptest 10k iteraciones pass |
| C-H8 Content isolation | contradicho por código | cumplido | typestate compile check |
| C-H9 No silent failure | **done** | cumplido | test `p0_empty_stream_terminates_cleanly` passes ✅ |
| C-H10 Audit replication | parcial | cumplido | SLO `|events_local| = |events_ledger|` > 99.9% |
| C-H11 Trace continuity | incumplido | cumplido | 100% sessions con DAG conexo en OTel |
| C-H12 Replay via Tordo | incumplido | cumplido | SIGKILL test pass |
| C-H13 Capability negotiation | incumplido | cumplido | contract test MCP pass |
| C-H14 CI boundary | **in-progress** (warn) | cumplido strict | `scripts/check_boundaries.sh --strict` passes en main |
| C-H15 No api/tasks | incumplido | cumplido | route inventory ∩ = ∅ |
| C-H16 paloma-boundary only | incumplido | cumplido | `cargo tree | grep paloma-types → empty` |
| C-H17 Feature flag integrity | **done** | cumplido | CI check §[5] passes ✅ |
| C-H18 Planning timeout adaptativo | **done** | cumplido | 4 tests adaptive_timeout pass ✅ |
| C-H19 LoopGuard patience | **done** | cumplido | test `hard_min_two_rounds` pass ✅ |
| C-H20 Formal verification | incumplido | cumplido | 0 counterexamples TLA+/Alloy |
| C-H21 Competitive benchmark | no medido | cumplido | public report top-quartile |

**Score actual: 4/21 cumplidos, 1 in-progress, 16 pendientes ≈ 19%.**

---

## 13. Glosario

| Término | Definición |
|---------|-----------|
| **Agent plane** | Plano conceptual owned por Halcon: reasoning, planning, tool sandbox, UX. |
| **Control plane** | Plano owned por Paloma: routing, budget, scoring, plan signing. |
| **Data plane** | Plano owned por Cenzontle: LLM gateway, MCP server, SSE parsing. |
| **Execution plane** | Plano owned por Tordo: durable job execution, retry, replay. |
| **Audit plane** | Plano owned por paloma-ledger: immutable audit SSOT. |
| **Capability** | Funcionalidad atómica con owner único (ej. `routing.decide`, `budget.reserve`). |
| **Frontier** | Término del ecosistema para sistemas con rigor formal, content isolation, INV verificados. |
| **INV-5** | Budget reserve→commit/release lifecycle (owned por Paloma). |
| **INV-6** | Content isolation del router (owned por Paloma). |
| **INV-9** | Plan signature integrity (HMAC/Ed25519, owned por Paloma). |
| **MCP** | Model Context Protocol — JSON-RPC stdio transport para tools. |
| **SSOT** | Single Source Of Truth. |
| **Ω-NN** | Obligación de corrección numerada (§5). |
| **I-HNN** | Invariante de Halcon numerado (§6). |
| **P-...** | Propiedad matemática (ver documento de corrección principal §6). |

---

## 14. ADRs relacionados

| ADR | Título | Status |
|-----|--------|--------|
| ADR-HALCON-001 | Halcon capabilities scope | **Draft** — sign-off pendiente |
| ADR-HALCON-002 | Boundary contracts and versioning | Planned (Ciclo 1) |
| ADR-HALCON-003 | No local retry; no local budget; no local routing | Planned (Ciclo 2) |
| ADR-HALCON-004 | Durable execution via Tordo | Planned (Ciclo 3) |
| ADR-HALCON-005 | Audit replication to paloma-ledger | Planned (Ciclo 3) |

---

## Historia de versiones

| Versión | Fecha | Cambios |
|---------|-------|---------|
| 0.1.0 | 2026-04-17 | Documento inicial. 4 obligaciones done (Ω-07, Ω-08, Ω-17, Ω-19); enforcement en warn mode (Ω-04). |

---

**Cualquier modificación de este documento requiere:**
1. PR con diff explícito.
2. Sign-off de ≥2 principal architects.
3. Actualización correspondiente en `scripts/check_boundaries.sh` si cambian obligaciones o invariantes.
4. Bump de versión (`0.1.0 → 0.2.0`).
