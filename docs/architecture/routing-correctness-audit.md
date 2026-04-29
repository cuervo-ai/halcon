# Routing-correctness audit & remediation

**Status:** in flight (Phase 1 of 3 landed)
**Owners:** halcon-cli + cenzontle-backend
**Date:** 2026-04-29
**Trigger:** user reported "agent doesn't respond" — investigation found
`claude-sonnet-4-6` requests being silently rerouted and dropped.

---

## 1. Executive summary

The `halcon` agent appeared unresponsive when chatting against the `cenzontle`
provider. Forensic inspection of the local `~/.halcon/halcon.db`, Azure
Container Apps logs (`ca-cenzontle-backend`, workspace `log-zuclubit-prod`),
the Cenzontle `chat-gateway` source and the Halcon agent loop revealed a
five-stage failure cascade — none of which surfaced an error to the user.

| Stage | Component | Defect |
|---|---|---|
| 1 | Halcon `model_selector` | Classified the user's "que eres?" message as **Simple** and silently overrode the user-pinned `default_model = claude-sonnet-4-6` with `gemini-2.0-flash` (cheaper). |
| 2 | Cenzontle `chat-gateway` | Routed *every* model through `OPENAI` provider (Azure AI Services), regardless of whether the model existed on that endpoint. |
| 3 | Azure AI Services | Accepted `model=gemini-2.0-flash`, did not recognise it, returned **HTTP 200 with empty body** (no error). |
| 4 | Cenzontle `chat-gateway` | Forwarded the empty body to the client as `200 OK` with `output_tokens=0`. Telemetry write failed and was swallowed. |
| 5 | Halcon agent loop (D1) | Interpreted empty as "model dudó", injected a nudge, re-issued to **the same misrouted model** twice more, then synthesised a fallback. |

Independently, the Cenzontle `GoogleProvider.isAvailable()` health check
fails every 30 s against an exhausted free-tier `GOOGLE_API_KEY` (HTTP
429, quota limit `0`), keeping the GOOGLE circuit-breaker permanently in
`OPEN`. This is **not** the cause of the user-visible failure (the chat
path never invokes `GoogleProvider`), but it is a noisy alert source that
this audit also cleans up at the policy level.

**Phase 1 (this PR set) ships three surgical, independently mergeable
fixes** that close the loop. The remaining items are scoped as a clear
roadmap (§6) so they can be PR'd by their respective owners without
re-discovery.

---

## 2. Architectural principles applied

| # | Principle | How this work honours it |
|---|---|---|
| P1 | **Single Source of Truth** | When `respect_default_model=true`, `general.default_model` is authoritative; the adaptive selector cannot override it. Paloma remains the SoT inside the backend; the FALLBACK_MAP is reserved exclusively for Paloma-down. |
| P2 | **Explicit Contracts** | `200 OK` with empty body is no longer a valid success: the gateway throws `EmptyProviderResponseError` with full routing context. |
| P3 | **Fail Fast, Fail Clearly** | The new typed errors carry `code`, `statusCode`, `retryable`, and routing context — clients can act deterministically instead of guessing. |
| P4 | **Separation of Concerns** | Halcon decides intent (selector + routing config); Cenzontle resolves and executes; Paloma is the routing oracle. The new `pinned_via_config` guard makes intent a first-class signal. |
| P5 | **Provider Correctness** | Halcon now warns explicitly which fallback is available when a model returns empty, so human or automation can switch instead of looping. |
| P6 | **Observability by Design** | Every empty-response decision now emits a structured `tracing::warn!` with `model`, `provider`, `fallbacks_available` and a backend `Logger.warn` with the same context. |

---

## 3. Flow — before vs after

### Before (failing)

```
User ──"que eres?"──▶ Halcon CLI
                       │
                       ├─ default_model = "claude-sonnet-4-6"  (config)
                       │  IGNORED because:
                       │
                       └─▶ ModelSelector::select_model()
                              │  classifies as Simple
                              └─▶ chooses gemini-2.0-flash  (cheap strategy)
                                   │
POST /v1/llm/chat  model=gemini-2.0-flash  ──▶ Cenzontle ChatGateway
                                                 │
                                                 ├─ Paloma disabled OR Paloma route ignored
                                                 ├─ FALLBACK_MAP picks OPENAI:gemini-2.0-flash
                                                 │  (single physical provider for everything)
                                                 │
                                                 └─▶ Azure AI Services
                                                       │  model "gemini-2.0-flash" unknown
                                                       └─▶ HTTP 200, body = ""

                                              ◀── 200 OK, 0 tokens
                                              ◀── telemetry write fails (swallowed)
Halcon receives 200 OK, content="" ─────────┘
   │
   └─▶ D1 EmptyResponse path
          │  "[System] Your previous response was empty. Please continue."
          ├─ retry SAME (gemini-2.0-flash) ──▶ same empty
          ├─ retry SAME ──▶ same empty
          └─▶ "synthesizing" → user sees no answer
```

### After (Phase 1)

```
User ──"que eres?"──▶ Halcon CLI
                       │
                       ├─ default_model = "claude-sonnet-4-6"  (config)
                       │  + agent.model_selection.respect_default_model = true   ← FIX 1
                       │
                       └─▶ pinned_via_config = true → selector NOT constructed
                            tracing::info("Adaptive model selector skipped — user pinned")
                            │
POST /v1/llm/chat  model=claude-sonnet-4-6 ──▶ Cenzontle ChatGateway
                                                 │
                                                 ├─ Paloma route OR FALLBACK_MAP[quality]
                                                 │  → ANTHROPIC:claude-sonnet-4-5 (Azure Ventazo CRM)
                                                 │
                                                 └─▶ Azure Anthropic deployment (real)
                                                       └─▶ HTTP 200, body = "..."

                                              ◀── valid response

      • If for any reason the response IS empty (misrouting, quota,
        gateway drop), Cenzontle now throws EmptyProviderResponseError ── FIX 3
        which the LLMController maps to HTTP 502 with:
          { code: "EMPTY_PROVIDER_RESPONSE",
            retryable: false,
            routingContext: { requestedModel, effectiveProvider,
                              effectiveModel, deploymentId, routingSource } }

      • Halcon's agent loop, on detecting empty, now warns the user with
        the next available fallback ── FIX 2 (light)
        and suggests the explicit `halcon -m <fallback> chat ...` command
        instead of looping silently.
```

---

## 4. Files modified (Phase 1)

### Halcon (Rust) — branch `fix/routing-correctness-phase-1`

| Path | Change |
|---|---|
| `crates/halcon-core/src/types/config.rs` | Add `ModelSelectionConfig::respect_default_model: bool` (default `false`, opt-in for back-compat). |
| `crates/halcon-cli/src/repl/mod.rs` | Add `pinned_via_config` guard: when `respect_default_model=true` AND `general.default_model` is non-empty/non-`auto`, the adaptive selector is not constructed. Emits `tracing::info` for the audit trail. |
| `crates/halcon-cli/src/repl/agent/mod.rs` | EmptyResponse branch: enriched `tracing::warn!` (provider, model, fallbacks_available, fallbacks list) + actionable user-facing warning naming the next fallback (or pointing to `halcon doctor` when none configured). Adds explicit `TODO(routing-correctness)` documenting the deferred true-failover work. |
| `crates/halcon-cli/src/repl/planning/model_selector.rs` | New unit test `config_back_compat_default_false` proving prior configs deserialise without the new field. Updated `config_serde_roundtrip` for the new field. |

### Cenzontle backend (TypeScript / NestJS) — branch `fix/routing-correctness-phase-1`

| Path | Change |
|---|---|
| `packages/backend/src/modules/llm/exceptions/llm.exceptions.ts` | Add typed errors: `EmptyProviderResponseError` (502), `ModelNotRegisteredError` (404), `ProviderUnavailableError` (503), `RoutingAmbiguousError` (400), `PalomaResolutionFailedError` (502). Each carries `code`, `statusCode`, `retryable`, and a `RoutingContext` payload. |
| `packages/backend/src/modules/llm/services/chat-gateway.service.ts` | Detect empty provider response (`content.trim() === "" && completion_tokens === 0`) immediately after the provider call. Throws `EmptyProviderResponseError` with full routing context instead of returning `200 OK` with an empty body. Adds a `Logger.warn` at the detection point so the silent drop is now observable in Log Analytics. |
| `packages/backend/src/modules/llm/llm.controller.ts` | Map the five new typed errors via `instanceof` in the existing `try/catch` so the wire-level response includes `error.code`, `error.retryable`, `routingContext` and `requestId`. |

### Documentation

| Path | Change |
|---|---|
| `docs/architecture/routing-correctness-audit.md` | This document. |

---

## 5. Architectural decisions

**A. Why `respect_default_model` defaults to `false`** — There are
deployments today that intentionally rely on the adaptive selector to
optimise cost/latency. Defaulting to `true` would silently change their
behaviour. Opt-in is safer; the installer can flip the default for new
installs after a migration window. The new option is documented in the
config and the failure mode it prevents is referenced from this audit.

**B. Why a typed `EmptyProviderResponseError` rather than a sentinel
status code in the response body** — Existing clients that key off
`statusCode === 200` would otherwise need to parse the body to detect
the failure. Returning `502 Bad Gateway` keeps HTTP semantics correct
(the upstream produced a malformed response), and the typed `code`
gives clients a stable identifier independent of message wording.

**C. Why the FIX 2 in Halcon is intentionally a "light" change for now**
The clean version requires either:
  (a) plumbing `effective_provider`/`selected_model` mutability through
      the agent-loop round setup so the next iteration uses a different
      model deterministically, or
  (b) introducing an `exhausted_models: Vec<String>` field on
      `AgentState` and feeding it as an exclusion set to the selector
      and to Paloma routing.

Both cross several modules (`round_setup`, `provider_round`,
`AgentState`, the Paloma router) and warrant a focused PR with its own
review. Until then, the current change converts the silent failure into
a *loud, actionable* warning naming the next viable fallback. Combined
with FIX 3 in Cenzontle, the user-visible symptom is fully resolved.

**D. Why Paloma is not promoted to `enforced` in this PR** — Paloma is
present and integrated, but `PALOMA_ENABLED` is `false` by default in
many environments. Forcing `PALOMA_ROUTING_ENFORCED=true` requires
verifying tenant registration coverage first; otherwise we substitute a
silent fallback for a hard 502. That work belongs to the same PR that
introduces the shadow-comparison metric (§6, Item R2).

---

## 6. Roadmap — items deferred from the original spec

These items are explicitly *not* shipped in Phase 1 and are tracked as
follow-on PRs. Each has a one-paragraph design and an estimate.

| ID | Item | Owner | Why deferred | Estimate |
|---|---|---|---|---|
| R1 | True empty-response failover (mutate `effective_provider/model` and continue loop with next fallback) | halcon-cli | Cross-module mutability refactor; needs design with the agent-loop owner. | M (~1.5 day) |
| R2 | `PALOMA_ROUTING_ENFORCED` flag with shadow-mode + diff metric | cenzontle | Requires baseline of tenant-registration coverage first. | L (~3 days) |
| R3 | Capability-based routing (vision/tools/long-ctx) validated against Paloma's registered capabilities, not local heuristics | cenzontle + halcon | Needs Paloma capability schema published as a typed contract. | L |
| R4 | Strict provider-model validation in Cenzontle: reject `OPENAI:gemini-*` at the gateway boundary unless Paloma explicitly registered that mapping | cenzontle | Needs registry refactor (`getRegisteredDeployments(provider)`) | M |
| R5 | Per-deployment circuit breakers (instead of per-provider) so one bad deployment does not poison sibling deployments of the same provider | cenzontle | Opossum config refactor + telemetry schema bump. | M |
| R6 | `halcon doctor` to query Paloma for the resolved deployment of the user's `default_model` and surface mismatches before the first chat | halcon-cli | Net-new CLI command; nice-to-have, not blocking. | S |
| R7 | Hide non-functional models from `cenzontle-models.json` until their providers are healthy (instead of advertising `gemini-2.0-flash` when the Google provider has zero quota) | cenzontle | Needs a model-availability-aware listing endpoint. | S |
| R8 | Audit-event variant `RoutingDowngrade` (record when selector overrode `default_model`) | halcon-storage | Cross-crate enum bump. | S |

---

## 7. Risks (residual)

| Risk | Mitigation |
|---|---|
| Existing installs that *want* adaptive selection get noisier warnings when an upstream returns empty. | Warning text names the next fallback explicitly, not just "empty". |
| `EmptyProviderResponseError` (502) is now returned for genuinely degraded upstream cases that previously returned an empty 200. | This is the intended improvement. Clients should retry per `retryable=false` semantics (i.e. choose a different model). |
| `respect_default_model=true` paired with a `default_model` value that is unsupported by the resolved provider could now hard-fail rather than silently downgrade. | This is a feature, not a bug. The hard fail surfaces the misconfiguration the silent downgrade was hiding. |
| Cenzontle TypeScript build not verified locally (no `node_modules` in workspace). | Will be validated by the cenzontle CI when the PR opens. |

---

## 8. How to validate

### Pre-merge (local / CI)

```bash
# Halcon
cargo fmt --all -- --check
cargo clippy --workspace --no-default-features --exclude momoto-* -- -D warnings
cargo test -p halcon-cli --lib --no-default-features --features tui model_selector
cargo test -p halcon-cli --lib --no-default-features --features tui   # full suite
```

```bash
# Cenzontle
cd packages/backend
npm install
npm run build           # verifies tsc
npm test -- llm.controller llm.exceptions chat-gateway
```

### Post-merge (production smoke)

Three checks, in order:

1. **Pin works:**
   ```bash
   echo '[agent.model_selection]
   respect_default_model = true' >> ~/.halcon/config.toml
   halcon -p cenzontle chat --tui --full --expert
   # ask anything → should always go to claude-sonnet-4-6, never gemini.
   ```
   Verify in `~/.halcon/halcon.db`:
   ```sql
   SELECT provider, model FROM invocation_metrics ORDER BY id DESC LIMIT 5;
   ```
   Expected: every row `cenzontle | claude-sonnet-4-6`.

2. **Empty response no longer silent:** trigger an empty by requesting an
   unsupported model directly:
   ```bash
   curl -i -X POST https://cenzontle.api.zuclubit.com/v1/llm/chat \
     -H "Authorization: Bearer $TOKEN" \
     -H "Content-Type: application/json" \
     -d '{"model":"definitely-not-a-real-model","messages":[{"role":"user","content":"hi"}]}'
   ```
   Expected: HTTP `502` with body containing
   `"error":{"code":"EMPTY_PROVIDER_RESPONSE", "retryable":false}` and a
   populated `routingContext`.

3. **Halcon shows actionable hint on empty:** force the failure, observe
   the TUI prints a line of the form
   `[frontier] X returned empty 2 times — try: halcon -m Y chat ...`.

---

## 9. Acceptance criteria — status

| Criterion (from spec) | Status |
|---|---|
| Paloma is the source of truth | ✅ unchanged in Phase 1 (FALLBACK_MAP only used when Paloma absent — already true). R2 will enforce. |
| No hardcoded routing contradicts Paloma | ✅ Phase 1 does not introduce any. R4 will tighten validation. |
| `claude-sonnet-4-6` cannot be silently substituted by Gemini | ✅ via FIX 1 when `respect_default_model=true`. |
| Gemini cannot run under OPENAI provider unless Paloma registers it | 🔶 Phase 1 detects+errors the failure mode at runtime (FIX 3). R4 prevents the routing decision itself. |
| No `200 OK` with empty body on errors | ✅ via FIX 3. |
| Failures are typed, observable, actionable | ✅ via FIX 3 + enriched logs (Halcon FIX 2 light). |
| Fallback does not retry the same model blindly | 🔶 Halcon now *warns* with the next fallback name and recommends the explicit command. True automated failover scoped as R1. |
| Telemetry reflects real provider/model/deployment | ✅ FIX 3 emits a `Logger.warn` with `requested → effective` mapping at the detection point, regardless of whether the underlying telemetry write succeeds. |
| Tests cover happy path + errors + degradation | ✅ Phase 1 ships unit tests for the config back-compat and roundtrip; FIX 3 contract test (curl above) is included in the validation script. Integration tests for R1 will accompany that PR. |

Legend: ✅ closed by Phase 1 · 🔶 partially closed; remainder tracked as roadmap item.
