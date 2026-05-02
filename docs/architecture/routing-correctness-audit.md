# Routing-correctness audit & remediation

**Status:** Phase 1 + Phase 2 (R1) landed
**Owners:** halcon-cli + cenzontle-backend
**Date:** 2026-04-29 (Phase 1) · 2026-05-01 (Phase 2 / R1)
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
| R1 | ✅ True empty-response failover (mutate `effective_provider/model` and continue loop with next fallback) — **landed Phase 2 (2026-05-01)**. See §10. | halcon-cli | n/a | n/a |
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
| Fallback does not retry the same model blindly | ✅ **Phase 2 (R1):** the agent loop now sets a transient `failover_pinned_model` + `failover_pinned_provider` consumed by `round_setup`, walks `agent.routing.fallback_models` skipping exhausted entries, and surfaces a visible diagnostic when no candidate remains. Same-model retry is preserved as a config-gated transient buffer (`same_model_empty_retries`, default `0`). |
| Telemetry reflects real provider/model/deployment | ✅ FIX 3 emits a `Logger.warn` with `requested → effective` mapping at the detection point, regardless of whether the underlying telemetry write succeeds. |
| Tests cover happy path + errors + degradation | ✅ Phase 1 ships unit tests for the config back-compat and roundtrip; Phase 2 adds 7 failover-selection unit tests (`failover_picks_first_unseen_fallback`, `failover_skips_current_model_even_if_listed_first`, `failover_skips_exhausted_models`, `failover_returns_none_when_all_exhausted`, `failover_returns_none_for_empty_fallback_list`, `failover_pin_take_semantics`, `same_model_streak_gates_transient_absorption`) plus 3 `RoutingConfig` serde tests, and updates the legacy `p0_empty_stream_terminates_cleanly` / `zero_token_output_completion_no_stuck_states` to lock in the new visible-diagnostic contract. |

Legend: ✅ closed by Phase 1 / Phase 2 · 🔶 partially closed; remainder tracked as roadmap item.

---

## 10. Phase 2 (R1) — automated empty-response failover (2026-05-01)

### What changed

| Path | Change |
|---|---|
| `crates/halcon-core/src/types/config.rs` | `RoutingConfig::failover_on_empty: bool` (default `true`) — master switch. `RoutingConfig::same_model_empty_retries: u8` (default `0`) — number of same-model retries before failover triggers, preserving the legacy nudge path as a transient absorption buffer. |
| `crates/halcon-cli/src/repl/agent/loop_state.rs` | New `LoopState` fields: `failover_pinned_model: Option<String>`, `failover_pinned_provider: Option<String>` (single-round pin, consumed via `take()`), `exhausted_models: Vec<String>` (de-duped, ordered), `same_model_empty_streak: u8` (reset on success or on failover). |
| `crates/halcon-cli/src/repl/agent/round_setup.rs` | At the top of model resolution, if a failover pin is active, the pin is consumed and applied **before** Paloma and the adaptive selector. The provider name is resolved via the registry; if not found, falls back to the current effective provider so `validate_model` fails loudly downstream rather than silently misrouting. |
| `crates/halcon-cli/src/repl/agent/mod.rs` | The `EmptyResponse` branch is restructured into three deterministic paths: (1) **transient absorption** when `same_model_empty_streak ≤ same_model_empty_retries`; (2) **failover** when `failover_on_empty=true` and a fresh fallback exists, walking `fallback_models` skipping `exhausted_models` and the current model, with the registry-walk locating the owning provider via `supported_models()`; (3) **terminal cascade** when no candidate remains, emitting both `render_sink.warning(...)` and `render_sink.stream_text(...)` so the TUI conversation pane shows a visible message — closes the "Agent completed without assistant text" symptom. The `ToolUse(out)` success path resets `same_model_empty_streak`. |

### Decision notes

**A. `failover_on_empty` defaults to `true`.** Unlike `respect_default_model` (opt-in) which changes routing *intent*, `failover_on_empty` only changes the *recovery* path — the new behaviour strictly improves on the prior silent break. New installs are protected by default.

**B. `same_model_empty_retries` defaults to `0`.** The audit revealed structural empties dominate. Burning two same-model retries against a misrouted upstream is observability noise, not recovery. Operators who measure genuine transient empties on their network can bump this to absorb them.

**C. Provider resolution via `supported_models()` walk.** The fallback list is just model IDs; the owning provider has to be recovered. Walking the `ProviderRegistry` and matching against each provider's `supported_models()` is O(providers × models) but providers are O(10) and models O(100) — negligible. If no match: keep the current provider so `round_setup`'s `validate_model` produces a loud, actionable error rather than a silent misroute.

**D. Visible terminal diagnostic.** Both `render_sink.warning(...)` and `render_sink.stream_text(...)` are called when the cascade exhausts. The `stream_text` path puts the message in the conversation pane (where the user expects assistant output); the `warning` path keeps the structured warning channel for log parsers. Acceptance criterion "no debe mostrarse 'Agent completed' sin mensaje del assistant" is enforced by the updated `p0_empty_stream_terminates_cleanly` test which now requires `full_text.contains("[frontier]")`.

**E. Phase 2 leaves the legacy `state.next_round_restarts` counter intact.** It is consumed by oscillation detectors (`SubsystemHealth::shows_oscillation`, threshold 3); decoupling it from empty-retry semantics is a separate concern. The new `same_model_empty_streak` is the dedicated counter for empty-recovery decisions.

### How to validate (Phase 2)

```bash
# Ensure the CI tests still cover the failover surface.
cargo test -p halcon-cli --lib --no-default-features --features tui \
    repl::agent::loop_state::tests::failover
cargo test -p halcon-core --lib types::config::tests::routing_config

# Smoke-test the cascade-exhausted contract end-to-end.
cargo test -p halcon-cli --lib --no-default-features --features tui \
    p0_empty_stream_terminates_cleanly zero_token_output_completion
```

### Hot-fixes shipped alongside R1 (2026-05-01)

While verifying R1 in production, three independent client-side defects
surfaced and were resolved in the same Phase 2 build:

1. **`Phase 30 fallback_adapted_model` clobbering the failover pin.**
   `round_setup.rs` had a Phase 30 block that always copied
   `state.fallback_adapted_model` over `selected_model` after the model
   resolution branches. Once the first failover latched
   `fallback_adapted_model` to the FIRST fallback, every subsequent pin
   was overwritten — the loop nudged `exhausted_models` correctly but
   kept retrying the SAME model. Fix: Phase 30 is now gated by
   `failover_pin_consumed`; when a pin was consumed this round, the
   latch is *updated* to the pin's value instead of read.

2. **Cenzontle gateway rejects `max_completion_tokens`.** Models with
   `supports_reasoning=true` (claude-opus-4-6, deepseek-r1-reasoning)
   get the modern OpenAI rename via the shared `openai_compat::build_request`,
   but the Cenzontle backend uses strict-whitelist validation that does
   not include the new field. The request fails with `HTTP 400 — property
   max_completion_tokens should not exist`. Fix: a compatibility shim in
   `cenzontle/mod.rs::invoke()` maps `max_completion_tokens` →
   `max_tokens` before dispatch (`if let Some(mct) = chat_request.max_completion_tokens.take()`). Locked by 4 unit tests in
   `cenzontle::tests::cenzontle_shim_*`.

3. **Pre-existing release-build breakage (`audit_sink`/`tenant_id`,
   `theme.rs::Context`).** The default release feature set
   (`color-science, tui, paloma`) failed to compile because
   `mod.rs:352-353` destructured `audit_sink: _audit_sink` while the
   `cfg(feature = "paloma")` blocks referenced the unbinded name, and
   `commands/theme.rs` used `Option::context()` without importing the
   `anyhow::Context` trait. Both fixed in this PR.

### Outstanding backend issue (R4, deferred to cenzontle-backend)

After Phase 2 client-side hot-fixes, R1 mechanically failover-walks
through all configured models and produces a visible diagnostic when
the cascade exhausts — the original "Agent completed silently" symptom
is fully resolved at the CLI layer.

However, **production traffic against `cenzontle.api.zuclubit.com/v1/llm/chat`
returns SSE streams that contain only the `type:stage`/`cognitive_state`
telemetry envelope (no `delta.content` or `delta.tool_calls`) when the
request body exceeds approximately 30 KB**. The 35 KB body that halcon
sends after answering "Yes" to the `Load HALCON.md instructions?` prompt
hits this threshold consistently across every model registered for the
provider (claude-sonnet-4-6, gpt-4o-mini, claude-opus-4-6,
deepseek-v3-2-coding, mistral-small-latest). Direct curl with a 600-byte
body succeeds for the same models. Trace evidence:

```
TRACE Cenzontle: outbound body  body_bytes=35492 n_tools=0 n_messages=2
WARN  D1: empty response detected  latency_ms=1929 input_tokens=0
```

This is **not** the routing bug Phase 1/2 closed; it is an upstream
limitation of the Cenzontle gateway's deployment (likely a header or
upstream Azure body-size cap on the streaming proxy). Tracked as **R4**.
Until the backend is patched, operators have three workarounds:

- Decline `Load HALCON.md instructions?` at session start (smaller prompt).
- Run with `halcon -p claude_code` for direct chat — Claude Code CLI is
  local and handles the full prompt without the gateway layer.
- Run `halcon` from a directory without an `HALCON.md` file.

### Operator validation checklist (post-Phase 2)

After installing the Phase 2 binary the operator can confirm the CLI
side is healthy independent of the backend status:

```bash
# Verify shim + R1 + visible terminal diagnostic in a single run.
halcon -m claude-sonnet-4-6 chat "responde solo OK" --output-format json 2>&1 \
  | grep -E "Cenzontle compat|empty response detected|R1: failover pin applied"
```

Expected (regardless of whether the response itself is empty):
- One `Cenzontle compat: mapped max_completion_tokens → max_tokens` line
  for every reasoning-model failover step.
- One `D1: empty response detected` warning per round when the backend
  returns an empty stream.
- One `R1: failover pin applied` line per round AFTER the first empty.
- The `exhausted` array grows monotonically until either a non-empty
  response arrives OR the visible terminal diagnostic fires.

Production smoke (operator):

1. Confirm `failover_on_empty = true` (the default) is in effect — no config change required for new installs.
2. With a `fallback_models` list configured (e.g. `["claude-haiku-4-5-20251001", "claude-sonnet-4-6", "gpt-4o-mini"]`), force an empty by setting `default_model` to a known-misrouted id (e.g. `gemini-2.0-flash` on a deployment with no Gemini provider). Observe the TUI surfaces `[frontier] gemini-2.0-flash returned empty — failing over to claude-haiku-4-5-20251001 (provider=…)` and the chat receives a real response from the next viable model.
3. With an empty `fallback_models = []`, the same scenario produces the visible terminal diagnostic in the conversation pane (instead of a silent "Agent completed").
