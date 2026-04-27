# ADR-HALCON-001: Halcon capabilities scope

| Metadato | Valor |
|----------|-------|
| Status | **Draft** — sign-off pendiente de 3 principal architects |
| Date | 2026-04-17 |
| Deciders | TBD |
| Supersedes | N/A |
| Superseded by | N/A |
| Related | `docs/architecture/halcon-v3-correction.md` (spec principal) |

---

## Context

Halcon es el CLI agent del ecosistema CUERVO. El ecosistema fue diseñado con 4 sistemas especializados:

- **Halcon** — CLI agent planner.
- **Paloma** — router control plane (Thompson Sampling, budget, plan signing).
- **Cenzontle** — AI backend + MCP gateway (única superficie LLM).
- **Tordo** — execution fabric durable (Postgres queue, replay, DLQ).

La auditoría de código (ver `halcon-v3-correction.md` §2) demuestra que **Halcon duplica capabilities que pertenecen a los otros 3 sistemas**:

1. Instancia `paloma_pipeline::Pipeline` in-process (debe ser HTTP).
2. Contiene 9 adapters HTTP directos a LLMs (Cenzontle es gateway único).
3. No conoce Tordo (`grep -r tordo crates → 0`); ejecuta DAGs localmente.
4. Expone `halcon-api /tasks` paralelo a `tordo-api /v1/jobs`.
5. Mantiene su propio audit sink HMAC desconectado de cualquier ledger central.

Esto produce drift multi-tenant, duplicación de fixes, degradaciones silenciosas y hace imposible declarar el sistema frontier-grade con rigor.

---

## Decision

### Principio D-1: Halcon es **agent plane** exclusivo

Halcon es dueño único de:
- `agent.plan` — generar `ExecutionPlan` desde prompt del usuario.
- `agent.sandbox` — ejecución local de tools con permission pipeline.
- `session.conversation` — estado de conversación local SQLite.
- `tui.render` — UX interactiva.
- `plan.build_and_sign` — construir plans firmados HMAC para Tordo.
- `tool.execute_local` — ejecución de tools client-side.

### Principio D-2: Halcon **NO es** control/data/execution plane

Halcon **NO posee y NO implementa localmente**:

| Capability | Owner canónico | Halcon consume vía |
|-----------|----------------|-------------------|
| `routing.decide` | Paloma | HTTP (indirecto vía Cenzontle) |
| `budget.reserve / commit / release` | Paloma | HTTP (indirecto vía Cenzontle) |
| `inference.llm` | Cenzontle | MCP JSON-RPC |
| `capability.negotiate` | Cenzontle | MCP `get_model_capabilities` |
| `execution.durable` | Tordo | HTTP `POST /v1/jobs` |
| `execution.replay` | Tordo | HTTP `POST /v1/jobs/:id/replay` |
| `audit.canonical` | paloma-ledger | HTTP `POST /v1/audit/ingest` |
| `retry.policy` | Tordo + Cenzontle (jerarquía) | Delegado; Halcon no retries |

### Principio D-3: Halcon **NO habla con Paloma directamente** (excepto read-only audit)

Halcon NO tiene conexión HTTP directa a Paloma `/v1/route` ni `/v1/outcome`. Paloma se accede vía Cenzontle por diseño del ecosistema:
- Cenzontle internamente llama Paloma (`cenzontle/packages/backend/src/modules/paloma/paloma-client.service.ts`).
- Halcon sólo ve la respuesta tipada del MCP de Cenzontle.

**Excepción controlada**: `GET /v1/ledger/session/:id/usage` para display UX (read-only, cached 60s, no decision).

### Principio D-4: Production default **sólo** `cenzontle-mcp`

El binary release default de Halcon compila **sin** adapters LLM directos. Los 9 adapters existentes (anthropic/openai/deepseek/gemini/bedrock/vertex/azure_foundry/ollama/claude_code) migran a feature `dev-providers` (OFF por default).

Esto preserva capacidad de desarrollo offline sin backend; production multi-tenant usa Cenzontle exclusivamente.

### Principio D-5: Ejecución durable **delegada** a Tordo

Cuando una sesión requiere durabilidad (restart safety, replay, long-running), Halcon:
1. Construye `ExecutionPlan` (firmado HMAC).
2. Envía a `tordo-api POST /v1/jobs`.
3. Consume eventos via SSE `/v1/jobs/:id/events`.
4. No persiste ExecutionState local.

Sesiones triviales cortas pueden seguir en el executor local **bajo feature flag runtime** (`execution_backend=local|tordo`) durante la transición Ciclo 3; tras Ciclo 6 el local execution queda fuera del binary production.

### Principio D-6: Contratos versionados obligatorios

Todo wire type que cruza frontera de Halcon lleva:
- `schema_version: String` (SemVer).
- `#[serde(deny_unknown_fields)]`.
- Deserialización rechaza versión no soportada con `ErrorClass::Permanent(SchemaVersionUnsupported)`.

### Principio D-7: Enforcement automatizado

La violación de cualquier obligación (Ω-01..Ω-20 en spec principal) DEBE ser detectable por:
- `scripts/check_boundaries.sh` (CI bash + grep).
- `cargo deny` (licencias + SDKs prohibidos).
- `schemathesis` contract tests.
- Binary inspection (`strings`, `nm`) en release.

Ningún CI debe bypassarse por "urgencia". Si una obligación bloquea trabajo crítico, se escala a decisión de arquitectura formal y se actualiza el ADR — no se ignora.

---

## Consequences

### Positivas

1. **Ownership único por capability** → elimina drift estructural que causó los incidentes observados (empty stream, planning timeout, tool failures en cascada).
2. **Content-isolation real (INV-6)** → routing nunca ve prompts; habilita compliance multi-tenant.
3. **Single choke point para LLM** → un solo credential set, un solo pricing SSOT, un solo pen test surface.
4. **Durabilidad real** → sesiones sobreviven restart; replay determinista.
5. **Audit compliant** → hash-chain centralizado en paloma-ledger consultable por auditor externo.
6. **Observabilidad end-to-end** → W3C TraceContext propagado obligatoriamente.
7. **Competitividad** → features únicas (durable CLI, formal budget) posibles sólo con diseño correcto.

### Negativas / tradeoffs aceptados

1. **Latencia añadida** — round-trip HTTP a Cenzontle MCP añade ~2-5ms local. Aceptable.
2. **Dependencia de gateway** — si Cenzontle no está disponible, Halcon production no funciona. Mitigación: `dev-providers` feature para desarrollo offline.
3. **Refactor no trivial** — Ciclos 2-3 eliminan ~30 % del código actual de `halcon-providers` y `halcon-runtime`. Mitigable con feature flags + shadow mode.
4. **Pricing/budget ahora centralizado** — Halcon no puede decidir budget offline. Rationale: era incorrecto antes (no-SSOT); ahora es honesto.

### Riesgos

| Riesgo | Mitigación |
|--------|-----------|
| Paloma-ledger service no existe aún | Ciclo 3 prereq; hasta entonces Halcon mantiene audit local + outbox buffer |
| `StepKind::AgentRound` no existe en Tordo aún | Ciclo 3 prereq; coordinación con equipo Tordo |
| Cenzontle MCP `capability.negotiate` no existe aún | Ciclo 2 prereq; coordinación con equipo Cenzontle |
| Tests legacy asumen comportamiento monolítico | Actualización en Ciclos 2-3 (algunos ya actualizados en ciclo inicial) |

---

## Alternatives considered

### A1. Mantener Halcon monolítico pero añadir "shadow mode" de Paloma/Tordo

- **Pro**: menos trabajo de refactor.
- **Contra**: viola P-UNIQ-OWNER permanentemente; shadow se convierte en dead code o en vector de drift; no es frontier-grade.
- **Rechazada**.

### A2. Mover Paloma/Tordo in-process (todo en Halcon)

- **Pro**: simplicidad conceptual.
- **Contra**: viola toda la arquitectura del ecosistema; Paloma y Tordo tienen ADRs que los prohíben; rompe multi-tenant.
- **Rechazada**.

### A3. Halcon duplica todo pero "eventually consistent" con los otros sistemas

- **Pro**: autonomía operacional.
- **Contra**: "eventually" en finance y audit es anti-patrón compliance; drift inevitable; no verificable formalmente.
- **Rechazada**.

### A4. La decisión actual: thin planner con delegación estricta

- **Pro**: alineada con ecosistema CUERVO; cada sistema cumple su rol diseñado; enforcement posible; frontier-grade alcanzable.
- **Contra**: requiere coordinación cross-repo + refactor.
- **Aceptada**.

---

## Implementation

### Ciclo 0 (actual)

| Tarea | Estado | Evidencia |
|-------|--------|-----------|
| Este ADR | **Draft** | `docs/architecture/adr/ADR-HALCON-001-capabilities-scope.md` |
| Spec principal | **Draft** | `docs/architecture/halcon-v3-correction.md` |
| CI boundary check | **Active (warn)** | `scripts/check_boundaries.sh` + `.github/workflows/ci.yml: boundary-check` |
| Ω-07 empty stream | **Done** | `p0_empty_stream_terminates_cleanly` test passes |
| Ω-08 planning adaptativo | **Done** | `adaptive_timeout_tests::*` (4 tests) |
| Ω-17 audit sink activo | **Done** | `audit_events.db` > 0 rows tras sesión |
| Ω-19 feature flag integrity | **Done** | `scripts/check_boundaries.sh §[5]` enforcing |

### Ciclos 1-6 (pendiente)

Ver `halcon-v3-correction.md §9` para plan detallado.

---

## Compliance and verification

Este ADR es **normativo**. Cumplimiento se verifica por:

1. **Presencia de la obligación documentada en spec** (§5 de halcon-v3-correction.md).
2. **Check automatizado** en `scripts/check_boundaries.sh` (cuando aplique).
3. **Test o property test** que valida el invariante concreto.
4. **Sign-off de arquitecto** en PR que cierra la obligación.

Violaciones detectadas en main que se hayan merged sin sign-off se escalan a revisión de arquitectura y se abre hotfix PR.

---

## Sign-off

| Architect | Role | Date | Signature |
|-----------|------|------|-----------|
| TBD | Principal Architect | | |
| TBD | Staff+ Platform Engineer | | |
| TBD | Reliability Engineer | | |

**Hasta obtener 3 firmas, este ADR permanece en status Draft.**

---

## References

- `docs/architecture/halcon-v3-correction.md` — spec principal de corrección.
- Paloma ADR-002 — router content isolation (INV-6).
- Tordo ADR-008 — boundary guardrails (modelo que replicamos).
- Tordo ADR-012 — plan integrity + HMAC signing (INV-9).

## Revision history

| Version | Date | Changes |
|---------|------|---------|
| 0.1.0 | 2026-04-17 | Documento inicial. Draft para sign-off. |
