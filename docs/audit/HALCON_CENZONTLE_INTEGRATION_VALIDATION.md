# HALCON Cenzontle Integration — Architecture Validation Report

**Phase**: Integration Wiring Validation (Phase 2)
**Date**: 2026-03-14
**Branch**: `feature/sota-intent-architecture`
**Analyst**: HALCON Architecture Validation Agent
**Prior Report**: `docs/audit/HALCON_CENZONTLE_INTEGRATION_REVIEW.md`

---

## Executive Summary

The SSO + Cenzontle integration compiles and the CLI commands route correctly. However, the provider is **not fully wired into the runtime**. Two critical functions exist but are never called from any execution path, creating a broken runtime chain that causes every Cenzontle API request to fail with a model error. The integration is **functionally incomplete** — the authentication layer is correct, but the provider-runtime bridge has two missing call sites.

---

## 1. Current Architecture Flow

### 1.1 Intended Flow

```
halcon login cenzontle
  └─ commands::sso::login()
       └─ PKCE flow → Zuclubit SSO /oauth/authorize
       └─ accept_callback() → state validation → code extraction
       └─ POST /oauth/token → access_token + refresh_token
       └─ store_tokens() → OS keychain
       └─ show_available_models() → prints model list to terminal

halcon chat (subsequent session)
  └─ commands::chat::run()
       └─ [MISSING] sso::refresh_if_needed()          ← GAP-2
       └─ build_registry()
            └─ reads keychain: cenzontle:access_token
            └─ CenzonzleProvider::new(token, base_url, Vec::new())  ← empty models
            └─ registry.register(provider)
       └─ ensure_local_fallback()                       ← called ✅
       └─ [MISSING] ensure_cenzontle_models()           ← GAP-1
       └─ precheck_providers(registry, "cenzontle", model)
            └─ p.is_available() → GET /v1/auth/me      ← called ✅
            └─ p.validate_model(model)
                 └─ supported_models() → []             ← always empty
                 └─ returns Err(ModelNotFound)
            └─ best_model fallback → supported_models().first() → None
                 └─ unwrap_or_else(|| model.to_string()) ← original model preserved
            └─ returns Ok(("cenzontle", original_model_string))
       └─ invoke() called with model = original_model_string
            └─ POST /v1/llm/chat {model: "claude-sonnet-4-6", ...}
                 └─ Cenzontle API: 400/422 (unknown model ID)  ← RUNTIME FAILURE
```

### 1.2 Actual Observed Flow (Post-Trace)

**Step 1: Login** — `halcon login cenzontle`
- Routes to `commands::sso::login()` ✅
- PKCE S256 flow executes correctly ✅
- Tokens stored in OS keychain under `halcon-cli` service ✅
- `show_available_models()` is called and prints the model list ✅
- **No model IDs are persisted anywhere** — only displayed to terminal

**Step 2: Chat startup** — `halcon chat "write a function"`
- `build_registry()` reads `cenzontle:access_token` from keychain ✅
- `CenzonzleProvider::new(token, base_url, Vec::new())` registered with **empty model list** ❌
- `ensure_local_fallback()` called ✅
- `ensure_cenzontle_models()` **NOT called** ❌ — function exists, dead code
- `precheck_providers()` runs `p.is_available()` → network probe succeeds if token valid ✅
- `p.validate_model(requested_model)` → **always fails** (empty list) ❌
- Fallback: `supported_models().first()` → `None` → `model.to_string()` (global default preserved)
- Returns `Ok(("cenzontle", "claude-sonnet-4-6"))` — Cenzontle selected but wrong model

**Step 3: Invoke** — actual API call
- `invoke()` called with `model = "claude-sonnet-4-6"` (Anthropic ID, not a Cenzontle model ID)
- `POST /v1/llm/chat {"model": "claude-sonnet-4-6", ...}`
- Cenzontle API returns **400 or 422** — model ID not recognized ❌

---

## 2. Provider Wiring Analysis

### 2.1 Registration Chain

| Component | File | Status |
|-----------|------|--------|
| `CenzonzleProvider` struct | `cenzontle/mod.rs` | ✅ Correct |
| `CenzonzleProvider` re-export | `halcon-providers/src/lib.rs` | ✅ Correct |
| `CenzonzleProvider` import | `provider_factory.rs:6` | ✅ Correct |
| Token resolution (env → keychain) | `provider_factory.rs:222-228` | ✅ Correct |
| Provider registration | `provider_factory.rs:235-237` | ✅ Correct — but empty models |
| Model population call | `chat.rs` (anywhere) | **MISSING** ❌ |
| `ensure_cenzontle_models()` definition | `provider_factory.rs:253-275` | ✅ Exists — never called |

### 2.2 Function Call Audit

| Function | Defined | Called From | Status |
|----------|---------|-------------|--------|
| `sso::login()` | `sso.rs:47` | `main.rs:955,967,979` | ✅ Called |
| `sso::logout()` | `sso.rs:62` | `main.rs:959,973` | ✅ Called |
| `sso::status()` | `sso.rs:81` | `main.rs:964` | ✅ Called |
| `sso::refresh_if_needed()` | `sso.rs:126` | *nowhere* | **DEAD CODE** ❌ |
| `CenzonzleProvider::new()` | `cenzontle/mod.rs:91` | `provider_factory.rs:235` | ✅ Called |
| `CenzonzleProvider::from_token()` | `cenzontle/mod.rs:120` | `ensure_cenzontle_models()` only | **EFFECTIVELY DEAD** ❌ |
| `ensure_cenzontle_models()` | `provider_factory.rs:253` | *nowhere* | **DEAD CODE** ❌ |
| `ensure_local_fallback()` | `provider_factory.rs:281` | `chat.rs:133` | ✅ Called |
| `CenzonzleProvider::invoke()` | `cenzontle/mod.rs:256` | (via runtime if selected) | ✅ Reachable |
| `CenzonzleProvider::is_available()` | `cenzontle/mod.rs:349` | via `precheck_providers` | ✅ Called |

**Critical observation**: `ensure_cenzontle_models()` re-registers the provider with a fully-populated model list. Its absence means `supported_models()` returns `[]` for the entire session lifetime.

### 2.3 `validate_model()` Behavior With Empty List

```rust
// ModelProvider default implementation (halcon-core/src/traits/provider.rs:37-46)
fn validate_model(&self, model: &str) -> crate::error::Result<()> {
    if self.supported_models().iter().any(|m| m.id == model) {
        Ok(())
    } else {
        Err(HalconError::ModelNotFound { ... })
    }
}
```

When `supported_models()` returns `[]`, this returns `Err` for **every model string**, including:
- `claude-sonnet-4-6` (global default)
- `claude-opus-4-6`
- Any Cenzontle-native model ID

The `precheck_providers_with_explicit()` function (line 337-382) handles this by falling back to:
```rust
supported.first().map(|m| m.id.clone()).unwrap_or_else(|| model.to_string())
```

With an empty list, `supported.first()` is `None`, so the **original model string is preserved unchanged**. This means Cenzontle is selected as the active provider but receives an Anthropic model ID it does not know.

---

## 3. Authentication Flow Verification

### 3.1 Login → Token → Provider Chain

```
sso::login()                    PKCE flow
  store_tokens(access, refresh, expires_in)
    keystore.set_secret("cenzontle:access_token", ...)   → OS keychain

chat::run()
  build_registry()
    std::env::var("CENZONTLE_ACCESS_TOKEN")              → env var (CI/CD)
    keystore.get_secret("cenzontle:access_token")        → OS keychain ← correct
    CenzonzleProvider::new(token, ...)                   → token is in struct
      self.access_token = token

CenzonzleProvider::invoke()
  .bearer_auth(&self.access_token)                       → token used ✅
```

**Token is correctly propagated from keychain → provider instance → HTTP request.** The access token flows end-to-end without gaps.

### 3.2 Token Refresh Gap

`refresh_if_needed()` implements this logic:
- Read `cenzontle:expires_at` from keychain
- If `expires_at < now + 300` (expiring in < 5 min): call `POST /oauth/token` with `grant_type=refresh_token`
- Store new tokens in keychain

**This function is never called from any path.** The consequence:

| Scenario | Without `refresh_if_needed()` | With `refresh_if_needed()` |
|----------|-------------------------------|---------------------------|
| Token valid at startup | Works | Works |
| Token expiring in 3 min at startup | Works at start, 401 mid-session | Refreshed proactively |
| Token expired at startup | Provider registered but all calls → 401 | Refreshed before registration |

The 401 error produces a user-visible message: *"Cenzontle: access token expired or invalid. Run `halcon login cenzontle` to refresh."* — which is correct guidance, but the expiry is avoidable.

### 3.3 CSRF and PKCE Correctness

| Check | Result |
|-------|--------|
| `state` validated before code exchange | ✅ `sso.rs:475-477` |
| `code_verifier` entropy: 32 random bytes | ✅ |
| `code_challenge` = base64url(SHA256(verifier)) | ✅ RFC 7636 Appendix B passes |
| Token exchange includes `code_verifier` | ✅ `sso.rs:231` |
| Access token not in logs or error messages | ✅ |
| Access token redacted in Debug | ✅ `cenzontle/mod.rs:82` |

Authentication cryptography is correct. No security issues found in the auth layer.

---

## 4. CLI Command Routing Review

### 4.1 Routing Table (from `main.rs:953-979`)

| User Command | Routes To | Correct |
|--------------|-----------|---------|
| `halcon login` | `commands::sso::login()` | ✅ |
| `halcon auth login cenzontle` | `commands::sso::login()` | ✅ |
| `halcon auth login anthropic` | `commands::auth::login("anthropic")` | ✅ |
| `halcon auth logout cenzontle` | `commands::sso::logout()` | ✅ |
| `halcon auth logout anthropic` | `commands::auth::logout("anthropic")` | ✅ |
| `halcon auth status` | `commands::auth::status()` + `commands::sso::status()` | ✅ |
| `halcon auth sso-login` | `commands::sso::login()` (default provider: cenzontle) | ✅ |
| `halcon auth sso-login openai` | `bail!("SSO login not supported for provider 'openai'")` | ✅ |
| `halcon auth sso-logout` | `commands::sso::logout()` (default provider: cenzontle) | ✅ |

All 9 routing paths are correct. No duplicate paths or conflicting routes. The `if p == "cenzontle"` guard pattern correctly separates SSO-based auth from API-key-based auth.

### 4.2 Status Command Coverage

`halcon auth status` calls both:
1. `commands::auth::status()` — shows API key status for all providers
2. `commands::sso::status()` — shows Cenzontle token expiry from keychain

This correctly aggregates both auth styles in a single command.

---

## 5. Runtime Provider Behavior

### 5.1 Provider Selection at Startup

When `HALCON_PROVIDER=cenzontle` (or `-p cenzontle`):

```
precheck_providers("cenzontle", "claude-sonnet-4-6")
  registry.get("cenzontle") → Some(provider)  [registered ✅]
  provider.is_available() → GET /v1/auth/me   [network call — may succeed or fail]
  IF available:
    provider.validate_model("claude-sonnet-4-6")
      supported_models() → []                 [always empty ❌]
      → Err(ModelNotFound)
    fallback: supported_models().first() → None
      → best = "claude-sonnet-4-6" (original unchanged)
    → Ok(("cenzontle", "claude-sonnet-4-6"))  [selected but model unknown to API]
```

### 5.2 When Cenzontle Is a Fallback Provider

If primary provider fails and Cenzontle is tried as fallback (`provider_client.rs:138-158`):
```rust
// fallback path checks model support
if fb_provider.supported_models().iter().any(|m| m.id == request.model) { ... }
// → false (empty list)
else if let Some(default) = fb_provider.supported_models().first() { ... }
// → None (empty list)
// falls through to:
request.clone()  // original request unchanged — same wrong model
```

The fallback path also fails to adapt the model for Cenzontle.

### 5.3 SSE Streaming Reachability

`CenzonzleProvider::invoke()` is reachable — the agent loop calls it through `SpeculativeInvoker → provider.invoke()`. The SSE parsing code (`build_sse_stream`) uses `eventsource_stream` and `OpenAICompatibleProvider::map_sse_chunk()`. This path is **correctly implemented** and would work if the correct model ID were provided.

### 5.4 `is_available()` Network Call at Startup

Every `halcon chat` invocation calls `GET /v1/auth/me` (with 5s timeout) as part of `precheck_providers()`. This adds up to 5s latency to CLI startup when the Cenzontle token is set but the endpoint is slow. With a healthy connection this is typically <100ms, but it is an unconditional network call.

---

## 6. Error Handling Analysis

### 6.1 Error Message Quality

| Error Condition | Message | Guidance |
|----------------|---------|----------|
| 401 Unauthorized | "access token expired or invalid. Run `halcon login cenzontle` to refresh." | ✅ Actionable |
| 403 Forbidden | "insufficient permissions for this model." | ✅ |
| Connection refused | "Cannot connect to {base_url}: {e}" | ✅ |
| Request timeout | "Cenzontle request timed out after {N}s" | ✅ |
| All retries exhausted | "all retry attempts exhausted" | Weak — no guidance |
| Empty model list (current bug) | No error — wrong model sent silently | ❌ Silent failure |
| SSO callback state mismatch | "State mismatch in OAuth callback (CSRF protection)" | ✅ |
| SSO callback `error` param | "SSO authorization error: {error}" | ✅ |
| Token exchange failure | "Token exchange failed (HTTP {status}): {body}" | ✅ |

### 6.2 Silent Failure Paths

1. **Empty model list with wrong model**: `precheck_providers` returns `Ok` but sends wrong model → API error only appears at response time, not at startup
2. **`store_tokens()` keychain failure**: `let _ = keystore.set_secret(...)` — no log, no user notification
3. **`HALCON_SSO_CLIENT_SECRET` empty string**: triggers `client_credentials` grant with empty secret → 401 from SSO server with no fallback to PKCE

---

## 7. Performance Assessment

### 7.1 Startup Latency

| Operation | Called At | Max Latency |
|-----------|-----------|-------------|
| Keychain read (token resolution) | `build_registry()` | ~1ms (OS keychain) |
| `GET /v1/auth/me` (is_available) | `precheck_providers()` | 5s timeout |
| `GET /v1/llm/models` (model discovery) | `ensure_cenzontle_models()` — NOT called | N/A |
| `ensure_local_fallback()` (Ollama probe) | `chat.rs:133` | ~2s timeout |

The `is_available()` call runs on every chat startup. When Cenzontle token is present, this is an unconditional network call to `GET /v1/auth/me`. If `ensure_cenzontle_models()` were called, it would add a second network call to `GET /v1/llm/models` at startup.

Both calls should be bounded (5s and 10s respectively) and run sequentially. Acceptable for startup.

### 7.2 No Blocking I/O in Hot Path

- Token is loaded into `self.access_token` at construction — no keychain reads during `invoke()`
- No synchronous blocking calls in the `invoke()` path
- Retry backoff uses `tokio::time::sleep` — non-blocking ✅

### 7.3 Memory Profile

- Models loaded once per session into `Vec<ModelInfo>` — O(n) where n = account models (typically <50)
- No caching or deduplication needed — provider lifetime = session lifetime

---

## 8. Integration Gaps

### GAP-1 — CRITICAL: `ensure_cenzontle_models()` Never Called

**File**: `crates/halcon-cli/src/commands/chat.rs`
**Location**: After line 133 (`ensure_local_fallback().await`)
**Effect**: `CenzonzleProvider.models` is always `[]` for the entire session. `validate_model()` fails for every model ID. Runtime sends wrong model to API. Every Cenzontle request fails.

**Trace**:
```
chat.rs:130  build_registry()               → registers Cenzontle with Vec::new()
chat.rs:133  ensure_local_fallback().await  → ✅ called
             ensure_cenzontle_models()       → ❌ MISSING CALL SITE
chat.rs:137  precheck_providers()           → empty model list → wrong model selected
```

---

### GAP-2 — HIGH: `refresh_if_needed()` Never Called

**File**: `crates/halcon-cli/src/commands/chat.rs`
**Location**: Before `build_registry()` (line 130)
**Effect**: Access tokens (15min lifetime) expire mid-session without proactive refresh. User gets 401 errors during long sessions. Must re-run `halcon login cenzontle` manually.

**Trace**:
```
chat.rs:130  build_registry()  → reads stale/near-expiry token from keychain
             sso::refresh_if_needed()  → ❌ MISSING CALL SITE
```

---

### GAP-3 — MEDIUM: Silent Keychain Write Failure

**File**: `crates/halcon-cli/src/commands/sso.rs:487-498`
**Pattern**: `let _ = keystore.set_secret(KEY_ACCESS_TOKEN, access_token);`
**Effect**: If OS keychain is locked (headless session, locked screen), tokens are not persisted. No warning is shown. User is told "Cenzontle session stored in OS keychain" even when it silently failed.

---

### GAP-4 — MEDIUM: Empty `HALCON_SSO_CLIENT_SECRET` Bypasses PKCE

**File**: `crates/halcon-cli/src/commands/sso.rs:54-56`
**Code**: `if let Ok(secret) = std::env::var("HALCON_SSO_CLIENT_SECRET")`
**Effect**: Setting the env var to empty string (`HALCON_SSO_CLIENT_SECRET=`) triggers client_credentials grant with no secret. SSO returns 401. PKCE fallback is never attempted.

---

### GAP-5 — LOW: Struct Name Typo in Public API

**File**: `crates/halcon-providers/src/cenzontle/mod.rs:62`
**Issue**: `pub struct CenzonzleProvider` (double-z). Public re-exported type.
**Effect**: Confusing for downstream callers. Must be fixed before merging to `main`.

---

### GAP-6 — LOW: Missing `tokenizer_hint()` Override

**File**: `crates/halcon-providers/src/cenzontle/mod.rs`
**Issue**: `tokenizer_hint()` not overridden — returns `TokenizerHint::Unknown` (default).
**Effect**: Token estimation uses conservative ~4.0 chars/token. Cenzontle models are typically built on GPT-4/Claude families with known tokenizers. Estimation may be 20-30% off for context window calculations.

---

## 9. Required Fixes

### Fix 1 — CRITICAL: Wire `ensure_cenzontle_models()` into `chat.rs`

**File**: `crates/halcon-cli/src/commands/chat.rs`
**After line 133** (`provider_factory::ensure_local_fallback(&mut registry).await;`):

```rust
// Populate Cenzontle model list if the provider was registered.
// This call is async-safe and is a no-op when Cenzontle is not registered.
provider_factory::ensure_cenzontle_models(&mut registry).await;
```

This single line call:
1. Checks if Cenzontle is registered (no-op if not)
2. Calls `GET /v1/llm/models` with the stored token
3. Re-registers the provider with the full model list
4. `validate_model()` now works for all Cenzontle model IDs
5. `precheck_providers()` selects the correct model

Also remove the `#[allow(dead_code)]` annotation from `ensure_cenzontle_models()` in `provider_factory.rs`.

---

### Fix 2 — HIGH: Wire `refresh_if_needed()` before `build_registry()`

**File**: `crates/halcon-cli/src/commands/chat.rs`
**Before line 130** (`let mut registry = provider_factory::build_registry(&config);`):

```rust
// Proactively refresh Cenzontle SSO token if near-expiry (< 5 min remaining).
// Must run before build_registry() so the fresh token is read from keychain.
let _ = super::sso::refresh_if_needed().await;
```

Also remove the `#[allow(dead_code)]` annotation from `refresh_if_needed()` in `sso.rs`.

---

### Fix 3 — MEDIUM: Log Keychain Write Failures

**File**: `crates/halcon-cli/src/commands/sso.rs:485-498`

Replace silent `let _ = ...` with logged errors:

```rust
fn store_tokens(access_token: &str, refresh_token: Option<&str>, expires_in: u64) {
    let keystore = KeyStore::new(SERVICE_NAME);

    if let Err(e) = keystore.set_secret(KEY_ACCESS_TOKEN, access_token) {
        tracing::warn!(error = %e, "Failed to store Cenzontle access token — token will not persist across sessions");
    }
    if let Some(rt) = refresh_token {
        if let Err(e) = keystore.set_secret(KEY_REFRESH_TOKEN, rt) {
            tracing::warn!(error = %e, "Failed to store Cenzontle refresh token");
        }
    }
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + expires_in;
    if let Err(e) = keystore.set_secret(KEY_EXPIRES_AT, &expires_at.to_string()) {
        tracing::warn!(error = %e, "Failed to store Cenzontle token expiry");
    }
}
```

---

### Fix 4 — MEDIUM: Guard Empty `HALCON_SSO_CLIENT_SECRET`

**File**: `crates/halcon-cli/src/commands/sso.rs:54-56`

```rust
// Before:
if let Ok(secret) = std::env::var("HALCON_SSO_CLIENT_SECRET") {
    return login_client_credentials(&sso_url, &cenzontle_url, &secret).await;
}

// After:
if let Ok(secret) = std::env::var("HALCON_SSO_CLIENT_SECRET") {
    if !secret.is_empty() {
        return login_client_credentials(&sso_url, &cenzontle_url, &secret).await;
    }
}
```

---

### Fix 5 — LOW: Rename `CenzonzleProvider` → `CenzontleProvider`

**Files to update**:
- `crates/halcon-providers/src/cenzontle/mod.rs` — struct definition + all internal uses
- `crates/halcon-providers/src/lib.rs` — re-export
- `crates/halcon-cli/src/commands/provider_factory.rs` — import + usage

This is a public API rename; do before merging to `main`.

---

## 10. Final Integration Status

### Status Matrix

| Component | Implemented | Wired | Functional |
|-----------|-------------|-------|------------|
| SSO PKCE Login flow | ✅ | ✅ | ✅ |
| Token storage (keychain) | ✅ | ✅ | ✅ (silent failure risk) |
| Token-to-provider handoff | ✅ | ✅ | ✅ |
| Provider registration | ✅ | ✅ | ✅ |
| Model discovery | ✅ | **❌** | **❌** |
| Model list population | ✅ | **❌** | **❌** |
| Chat invocation | ✅ | ✅ | **❌** (wrong model ID) |
| SSE streaming | ✅ | ✅ | ✅ (if model ID correct) |
| Token refresh (proactive) | ✅ | **❌** | **❌** |
| CLI routing | ✅ | ✅ | ✅ |
| CSRF protection | ✅ | ✅ | ✅ |
| Air-gap exclusion | ✅ | ✅ | ✅ |
| CI/CD bypass | ✅ | ✅ | ✅ (empty-secret risk) |
| Error messages (401/403) | ✅ | ✅ | ✅ |

### Verdict

| Category | Result |
|----------|--------|
| Authentication layer | **PASS** — PKCE, CSRF, keychain all correct |
| CLI command routing | **PASS** — all 9 paths route correctly |
| Provider registration | **PASS** — registered when token available |
| Model discovery | **FAIL** — `ensure_cenzontle_models()` never called |
| Runtime invocation | **FAIL** — wrong model ID sent to API |
| Token refresh | **FAIL** — `refresh_if_needed()` never called |
| Error handling | **PARTIAL** — actionable messages for known errors, silent for model mismatch |
| Architecture compliance | **PASS** — trait conformance, dep boundaries respected |
| Performance | **PASS** — no blocking calls in hot path |
| Code quality | **PARTIAL** — typo in public type name, dead code in two key functions |

### Path to Production-Ready

**Minimum required** (enables basic end-to-end functionality):
1. Add `provider_factory::ensure_cenzontle_models(&mut registry).await;` to `chat.rs` — **1 line**
2. Add `super::sso::refresh_if_needed().await;` before `build_registry()` — **1 line**

With these two lines added, the integration becomes functionally correct:
- Cenzontle provider will carry real model IDs after startup
- `validate_model()` will succeed for valid Cenzontle models
- `precheck_providers()` will select the correct model
- Token will be refreshed proactively before expiry
- API requests will use the correct model ID

**Recommended before GA**:
3. Fix `store_tokens()` keychain warning
4. Fix empty `HALCON_SSO_CLIENT_SECRET` guard
5. Rename `CenzonzleProvider` → `CenzontleProvider`
6. Add `tokenizer_hint()` override (optional — estimation quality)
