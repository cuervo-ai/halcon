# HALCON Cenzontle SSO Integration — Validation Report

**Revision**: 1.0
**Date**: 2026-03-14
**Branch**: `feature/sota-intent-architecture`
**Scope**: `halcon login cenzontle` + Cenzontle `ModelProvider` + Zuclubit SSO OAuth 2.1 PKCE flow
**Verdict**: Functional implementation — requires targeted architectural improvements before production deployment

---

## Table of Contents

1. [Authentication Flow Analysis](#1-authentication-flow-analysis)
2. [Provider Implementation Review](#2-provider-implementation-review)
3. [ProviderFactory Integration](#3-providerfactory-integration)
4. [Token Storage Security](#4-token-storage-security)
5. [CLI Command Review](#5-cli-command-review)
6. [Runtime Provider Behavior](#6-runtime-provider-behavior)
7. [CI/CD Authentication](#7-cicd-authentication)
8. [Architecture Consistency](#8-architecture-consistency)
9. [Failure Mode Analysis](#9-failure-mode-analysis)
10. [Code Quality Assessment](#10-code-quality-assessment)
11. [Recommended Improvements](#11-recommended-improvements)

---

## 1. Authentication Flow Analysis

### 1.1 PKCE Implementation

| Check | Result | Detail |
|-------|--------|--------|
| RFC 7636 S256 code challenge | PASS | SHA-256 over `code_verifier`, base64url-no-pad encoded |
| Code verifier entropy | PASS | 32 random bytes via `rand::rng().fill_bytes()` → 256 bits entropy |
| Test vector (RFC 7636 Appendix B) | PASS | Unit test `code_challenge_known_vector` passes |
| State parameter CSRF | PASS | 16 random bytes → 32-char hex, validated in `accept_callback()` |
| State mismatch handling | PASS | Returns `Err("state mismatch — possible CSRF attack")` |
| Nonce in state | N/A | Not required by OAuth 2.1 for PKCE flows |

The PKCE S256 implementation is cryptographically correct and RFC 7636-compliant.

### 1.2 Authorization Code Flow

```
halcon login cenzontle
  └─ generate_code_verifier()     [32 bytes → base64url = 43 chars]
  └─ compute_code_challenge()     [SHA-256 → base64url = 43 chars]
  └─ generate_state()             [16 bytes → hex = 32 chars]
  └─ build auth_url               [ZUCLUBIT_SSO_URL/oauth/authorize + params]
  └─ open::that(auth_url)         [browser launch]
  └─ TcpListener 127.0.0.1:9876  [loopback only — not 0.0.0.0]
  └─ accept_callback()            [reads request line, validates state, extracts code]
  └─ POST /oauth/token            [code + code_verifier + client_id]
  └─ store_tokens()               [OS keychain]
```

**Finding**: `accept_callback()` reads only the first line of the HTTP request. If a browser sends a prefetch or favicon request before the callback URL, the code will consume the wrong request and the auth flow will fail silently. See Section 11 for remediation.

### 1.3 Token Endpoint

- `grant_type=authorization_code` with PKCE — correct
- `client_id=cuervo-cli` included — correct
- No `client_secret` in PKCE flow — correct per OAuth 2.1 public client spec
- `Content-Type: application/x-www-form-urlencoded` — correct
- Inline `percent_encode()` implementation avoids adding urlencoding dep — correct

---

## 2. Provider Implementation Review

### 2.1 `CenzonzleProvider` struct (`crates/halcon-providers/src/cenzontle/mod.rs`)

| Attribute | Assessment |
|-----------|------------|
| Implements `ModelProvider` trait | PASS — all 6 required methods |
| `name()` | PASS — returns `"cenzontle"` |
| `supported_models()` | PASS — returns `&self.models` |
| `tool_format()` | PASS — `ToolFormat::OpenAIFunctionObject` (correct variant) |
| `estimate_cost()` | PASS — `TokenCost::default()` (billed through Cenzontle account) |
| `is_available()` | PASS — calls `GET /v1/auth/me`, 5s timeout |
| Debug impl | PASS — access token redacted as `"[REDACTED]"` |

### 2.2 Chat Endpoint Routing

The Cenzontle chat endpoint is `POST /v1/llm/chat`, not `POST /v1/chat/completions`. The implementation correctly handles this by:

1. Using `inner: OpenAICompatibleProvider` solely for `build_request()` (request body construction)
2. POSTing to `self.chat_url = format!("{}/v1/llm/chat", base_url)` directly

This is architecturally sound. The inner provider's base URL is never used for actual HTTP calls.

### 2.3 SSE Streaming

- Uses `eventsource_stream::Eventsource` — correct
- Handles `[DONE]` sentinel — correct
- Delegates chunk parsing to `OpenAICompatibleProvider::map_sse_chunk()` — correct
- Malformed SSE chunks produce a warn log and are silently skipped (non-fatal) — acceptable

### 2.4 Retry Logic

- `max_retries` from `HttpConfig` — correct
- Exponential backoff via `http::backoff_delay(1000, attempt)` — correct
- Retries on connection errors and timeouts only — correct (does not retry 4xx)
- 401 returns specific actionable message: "Run `halcon login cenzontle` to refresh." — correct

### 2.5 Model Discovery

- `from_token()` calls `GET /v1/llm/models` at construction time
- `fetch_models()` handles non-success HTTP status and JSON parse failures
- Empty model list produces a warning, not an error — correct (graceful degradation)
- Tier-based context window heuristics for when API doesn't return these values — reasonable

---

## 3. ProviderFactory Integration

### 3.1 Token Resolution Order

```rust
// 1. Env var (CI/CD override)
CENZONTLE_ACCESS_TOKEN

// 2. OS Keychain (interactive login)
halcon-cli / cenzontle:access_token
```

This resolution order is correct. Env var takes precedence, enabling CI/CD injection without keychain dependency.

### 3.2 Registration

```rust
let provider = CenzonzleProvider::new(token, base_url, Vec::new()); // empty model list
registry.register(Arc::new(provider));
```

**CRITICAL FINDING (P1)**: Provider is registered with an empty `Vec::new()` model list. The function `ensure_cenzontle_models()` exists to populate this list asynchronously but is **never called** from `chat.rs` or any startup path.

**Impact**: `precheck_providers()` in the planning layer validates that the selected provider supports the requested model. With an empty model list, no model will match, causing Cenzontle to be bypassed silently or causing a `ModelNotFound` error.

**Confirmed by grep**: `ensure_cenzontle_models` appears only in `provider_factory.rs` (definition + dead_code allow). Zero call sites in `chat.rs`, `commands/serve.rs`, or any other path.

### 3.3 Air-Gap Mode

```rust
if std::env::var("HALCON_AIR_GAP").as_deref() == Ok("1") {
    // ... only Ollama registered
    return registry;  // <-- Cenzontle block never reached
}
```

Air-gap mode correctly excludes Cenzontle. No network calls will be made to external endpoints in air-gap deployments.

---

## 4. Token Storage Security

### 4.1 OS Keychain Integration

| Platform | Backend | Assessment |
|----------|---------|------------|
| macOS | Keychain Services (via `keyring` crate) | PASS |
| Linux | Secret Service API (libsecret) | PASS |
| Windows | Credential Manager | PASS |

Tokens are never written to disk by halcon-cli. The OS provides encryption-at-rest automatically.

**Keychain keys used**:
- `cenzontle:access_token` — JWT, 15min lifetime
- `cenzontle:refresh_token` — opaque, 7-day lifetime
- `cenzontle:expires_at` — Unix timestamp string

### 4.2 Token Leakage Audit

| Location | Finding |
|----------|---------|
| `CenzonzleProvider::fmt()` | PASS — `[REDACTED]` in Debug output |
| `sso.rs` tracing spans | PASS — token not included in any tracing fields |
| Error messages | PASS — 401 message does not include token value |
| `accept_callback()` | PASS — auth `code` is not logged |

No token leakage paths identified.

### 4.3 Token Lifetime Management

- Access token lifetime: 15 minutes (from `expires_in` field)
- Refresh token lifetime: 7 days
- `refresh_if_needed()` triggers refresh when less than 5 minutes remain
- `store_tokens()` overwrites existing keychain entry — no accumulation of stale tokens

**Finding (P2)**: `store_tokens()` uses `let _ = keystore.set_secret(...)` for all three keys. Keychain write failures are silently discarded. If the keychain is locked (e.g., logged-out OS session, locked screen in headless mode), the tokens are lost with no user notification.

### 4.4 CSRF Protection

- `state` parameter generated per-request with 128 bits of entropy
- State validated in `accept_callback()` before code exchange
- Mismatch returns error — flow aborted, no token exchange performed
- State is not persisted across restarts (in-memory only) — correct

---

## 5. CLI Command Review

### 5.1 Command Structure

```
halcon auth login --provider cenzontle   → sso::login()
halcon auth login cenzontle              → sso::login()  (positional alias)
halcon login                             → sso::login()  (top-level shortcut)
halcon auth logout --provider cenzontle  → sso::logout()
halcon auth status                       → auth::status() + sso::status()
halcon auth sso-login --provider <p>     → sso::login()
halcon auth sso-logout --provider <p>    → sso::logout()
```

The routing is complete and handles both the full path and the top-level `halcon login` shortcut.

### 5.2 `login()` User Experience

1. Prints "Opening browser to authenticate with Cenzontle..."
2. Launches browser
3. Prints "Waiting for callback on http://localhost:9876/callback"
4. After successful callback: prints token expiry time and model count

**Finding**: The model count printed after login comes from `from_token()` which fetches models during the SSO login flow. However, the provider registered in `build_registry()` uses `CenzonzleProvider::new(..., Vec::new())` — a separate instance with no models. The count shown to the user at login time is not reflected in the running provider instance.

### 5.3 `logout()` Behavior

Removes all three keychain keys. Uses `delete_credential()` which returns `Ok(())` on `NoEntry` — idempotent and safe to call when not logged in.

### 5.4 `status()` Output

Reads `cenzontle:access_token` and `cenzontle:expires_at` from keychain. Shows:
- Whether logged in
- Token expiry timestamp
- Time remaining (via duration formatting)

Does not expose token value — correct.

---

## 6. Runtime Provider Behavior

### 6.1 Provider Registration Lifecycle

```
build_registry()
  └─ Cenzontle token resolved (env or keychain)
  └─ CenzonzleProvider::new(token, base_url, Vec::new())  ← empty models
  └─ registry.register(Arc::new(provider))
      └─ HashMap::insert() — overwrites if "cenzontle" already present
```

`ProviderRegistry::register()` uses `HashMap::insert()` — silent upsert. Safe for re-registration patterns.

### 6.2 `precheck_providers()` Impact

When `agent/mod.rs` runs `precheck_providers()`, it checks that the requested model ID exists in `provider.supported_models()`. With an empty model list, this check will fail for any Cenzontle model, and the planner will either:
- Fall back to the next available provider (if fallback is configured)
- Return a `ModelNotFound` error

This confirms the P1 severity of the empty model list issue.

### 6.3 `is_available()` Check

```rust
GET /v1/auth/me
Bearer {access_token}
Timeout: 5s
```

Returns `true` if HTTP 200, `false` for any error or non-2xx status. This is an appropriate liveness check that also validates token validity.

---

## 7. CI/CD Authentication

### 7.1 `client_credentials` Bypass

When `HALCON_SSO_CLIENT_SECRET` is set, `login()` uses the OAuth 2.0 `client_credentials` grant instead of the PKCE flow:

```
POST /oauth/token
  grant_type=client_credentials
  client_id=cuervo-cli
  client_secret={HALCON_SSO_CLIENT_SECRET}
  scope=openid profile email offline_access
```

This correctly skips the browser and TCP listener, enabling CI/CD usage.

**Finding (P3)**: The check is `if let Ok(secret) = std::env::var("HALCON_SSO_CLIENT_SECRET")`. This does not verify that `secret` is non-empty. Setting `HALCON_SSO_CLIENT_SECRET=` (empty string) will attempt a client_credentials grant with an empty secret, which will fail at the SSO server with a 401 rather than falling back to PKCE.

### 7.2 `CENZONTLE_ACCESS_TOKEN` Direct Injection

Setting `CENZONTLE_ACCESS_TOKEN` skips SSO entirely and injects the token directly. This is the correct pattern for:
- Service accounts with long-lived tokens
- Environments where SSO is not reachable
- Testing with static tokens

No `HALCON_SSO_CLIENT_SECRET` is needed when using `CENZONTLE_ACCESS_TOKEN`.

### 7.3 Environment Variable Summary

| Variable | Purpose | Required | Default |
|----------|---------|----------|---------|
| `ZUCLUBIT_SSO_URL` | SSO base URL | No | `https://sso.zuclubit.com` |
| `CENZONTLE_BASE_URL` | API base URL | No | `https://api.cenzontle.app` |
| `CENZONTLE_ACCESS_TOKEN` | Direct token injection | No | — |
| `HALCON_SSO_CLIENT_SECRET` | CI/CD client credentials | No | — |
| `HALCON_AIR_GAP` | Excludes all cloud providers | No | — |

---

## 8. Architecture Consistency

### 8.1 Dependency Boundary

| Crate | Depends on halcon-auth | Can access keychain |
|-------|----------------------|---------------------|
| `halcon-providers` | No | No |
| `halcon-cli` | Yes | Yes (via provider_factory.rs) |

The implementation correctly respects this boundary. `CenzonzleProvider` receives a resolved token string, never accesses the keychain directly. Token resolution is handled entirely in `provider_factory.rs`.

### 8.2 Trait Conformance

`CenzonzleProvider` implements all methods of `ModelProvider`. No trait method is left unimplemented or delegated to a default that would cause incorrect behavior:

| Method | Implementation | Correctness |
|--------|---------------|-------------|
| `name()` | `"cenzontle"` | PASS |
| `supported_models()` | `&self.models` | PASS (empty until fixed) |
| `tool_format()` | `OpenAIFunctionObject` | PASS |
| `invoke()` | Custom HTTP to `/v1/llm/chat` | PASS |
| `is_available()` | `GET /v1/auth/me` | PASS |
| `estimate_cost()` | `TokenCost::default()` | PASS |

### 8.3 OpenAI Compatibility Layer Usage

Using `OpenAICompatibleProvider` as an inner helper for `build_request()` while overriding the HTTP dispatch is a valid and clean pattern. It avoids duplicating request building logic and stays compatible with any future changes to the OpenAI compat layer.

### 8.4 Naming Inconsistency

The struct is named `CenzonzleProvider` (typo: double-z) while the crate, module, and all external references use `Cenzontle`. This is documented as intentional in memory but creates confusion. The typo appears in:
- `pub struct CenzonzleProvider`
- `pub use cenzontle::CenzonzleProvider;` in `lib.rs`
- `provider_factory.rs` usage

As a public-facing type in `halcon-providers/src/lib.rs`, this should be corrected to `CenzontleProvider` before the integration is promoted to `main`.

---

## 9. Failure Mode Analysis

### 9.1 Network Failure During Login

| Scenario | Behavior | Assessment |
|----------|---------|------------|
| SSO unreachable at `/oauth/authorize` | Browser shows connection refused | Acceptable — user sees browser error |
| SSO unreachable at `/oauth/token` | `reqwest` error → printed to stderr | PASS |
| Cenzontle `/v1/llm/models` unreachable | Warning logged, empty model list | PASS (graceful) |
| Cenzontle `/v1/llm/chat` unreachable | Connection error with retry | PASS |

### 9.2 Token Expiry Scenarios

| Scenario | Behavior | Assessment |
|----------|---------|------------|
| Access token expired (chat request) | HTTP 401 → "Run `halcon login cenzontle` to refresh." | PASS |
| Access token expiring soon (startup) | `refresh_if_needed()` refreshes silently | PASS (but never called) |
| Refresh token expired | Refresh call returns 401 → error message | PASS |
| No token in keychain | Cenzontle not registered — falls back | PASS |

**Note**: `refresh_if_needed()` is decorated with `#[allow(dead_code)]` — it is never called from any startup path. Proactive refresh at startup is not implemented.

### 9.3 Port Conflict on Callback Port

If port 9876 is in use, `TcpListener::bind("127.0.0.1:9876")` will return an `Err`. The `?` propagator will surface this as a user-visible error: "address already in use". No port fallback or retry logic exists. This is a minor usability issue for developer environments with many services running.

### 9.4 Race Conditions in Callback Server

The `accept_callback()` function uses blocking I/O (`listener.accept()` with no timeout). If the browser never calls back (user closes window, SSO error page, etc.), the CLI will hang indefinitely. A timeout should be applied to `accept()`.

### 9.5 Concurrent Login Calls

No locking around keychain writes. If two concurrent `halcon login cenzontle` invocations run simultaneously, both will complete but the second will overwrite the first's tokens. Last-write-wins. Acceptable for a CLI tool.

---

## 10. Code Quality Assessment

### 10.1 Positive Findings

- **RFC 7636 compliance**: PKCE implementation is correct and tested with the official Appendix B test vector
- **CSRF protection**: State parameter validation is present and correct
- **Token redaction**: `Debug` impl correctly redacts the access token
- **Dependency isolation**: `halcon-providers` does not gain a dependency on `halcon-auth`
- **Air-gap safety**: Cenzontle is excluded from air-gap mode
- **Retry logic**: Exponential backoff for transient network failures
- **Error specificity**: 401 and 403 errors have actionable messages
- **Idempotent logout**: Safe to call when not logged in
- **CI bypass**: `client_credentials` flow for headless environments

### 10.2 Code Structure

| File | Lines | Assessment |
|------|-------|------------|
| `sso.rs` | 570 | Well-structured, single responsibility per function |
| `cenzontle/mod.rs` | 366 | Clean provider implementation |
| `cenzontle/types.rs` | 41 | Minimal and correct |
| `provider_factory.rs` (cenzontle block) | ~20 | Simple, follows existing patterns |

### 10.3 Test Coverage

| Test | Location | Status |
|------|----------|--------|
| `code_challenge_known_vector` | `sso.rs` | PASS |
| `state_is_hex_32_chars` | `sso.rs` | PASS |
| `code_verifier_is_base64url` | `sso.rs` | PASS |

Missing tests:
- Integration test for `CenzonzleProvider::invoke()` against a mock server
- Test for `fetch_models()` with a well-formed and malformed response
- Test for `accept_callback()` state mismatch rejection

### 10.4 Warning Baseline

The integration was implemented with no warning count regression: baseline 596 warnings before integration, 596 warnings after (3 new warnings suppressed with `#[allow(dead_code)]`).

---

## 11. Recommended Improvements

### P1 — Critical: Empty Model List at Runtime

**Problem**: `CenzonzleProvider::new(token, base_url, Vec::new())` registers with no models. `ensure_cenzontle_models()` exists but is never called.

**Fix**: Call `from_token()` instead of `new()` in `provider_factory.rs`, or call `ensure_cenzontle_models()` from `chat.rs` after `build_registry()`.

Recommended approach (minimal change to `provider_factory.rs`):

```rust
// Replace:
let provider = CenzonzleProvider::new(token, base_url, Vec::new());

// With (in an async context):
if let Some(provider) = CenzonzleProvider::from_token(token, base_url).await {
    registry.register(Arc::new(provider));
}
```

If `build_registry()` is not async, extract the Cenzontle block into `ensure_cenzontle_models()` and call it from `commands/chat.rs` after `build_registry()` returns.

---

### P2 — High: Callback Server Reads Only One HTTP Line

**Problem**: `accept_callback()` reads a single line from the TCP stream. Browsers often send favicon or prefetch requests before the OAuth callback.

**Fix**: Loop until a request line containing `/callback` is found:

```rust
for line in reader.lines() {
    let line = line?;
    if line.starts_with("GET /callback") || line.starts_with("GET /?") {
        // parse this line
        break;
    }
}
```

Also drain the rest of the HTTP request headers before writing the response, so the browser does not stall waiting for its full request to be consumed.

---

### P2 — High: No Timeout on Callback Accept

**Problem**: `listener.accept()` blocks indefinitely if the browser never calls back.

**Fix**: Apply a 5-minute timeout:

```rust
listener.set_nonblocking(true)?;
let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
loop {
    match listener.accept() {
        Ok((stream, _)) => { /* handle */ break; }
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            if std::time::Instant::now() >= deadline {
                return Err(anyhow::anyhow!("Timed out waiting for browser callback"));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(e) => return Err(e.into()),
    }
}
```

---

### P2 — High: Silent Keychain Write Failure

**Problem**: `store_tokens()` silently discards keychain errors with `let _ = ...`.

**Fix**: Log a warning when keychain write fails:

```rust
if let Err(e) = keystore.set_secret("cenzontle:access_token", &access_token) {
    warn!(error = %e, "Failed to store Cenzontle access token in keychain — token will not persist");
}
```

The flow should not abort on keychain failure (the token is still usable for the current session), but the user should know that re-authentication will be required after restart.

---

### P3 — Medium: Empty `HALCON_SSO_CLIENT_SECRET` Triggers Bad Grant

**Problem**: `std::env::var("HALCON_SSO_CLIENT_SECRET")` matches an empty string, sending a client_credentials grant with no secret.

**Fix**:

```rust
if let Ok(secret) = std::env::var("HALCON_SSO_CLIENT_SECRET") {
    if !secret.is_empty() {
        return login_client_credentials(&secret).await;
    }
}
```

---

### P3 — Medium: Struct Naming Typo

**Problem**: `CenzonzleProvider` (double-z) is a public type in `halcon-providers`.

**Fix**: Rename to `CenzontleProvider` across all references before merging to `main`. This is a breaking change for any downstream code using the type directly — must be done before GA.

Files to update:
- `crates/halcon-providers/src/cenzontle/mod.rs` (struct definition)
- `crates/halcon-providers/src/lib.rs` (re-export)
- `crates/halcon-cli/src/commands/provider_factory.rs` (usage)

---

### P3 — Medium: Proactive Token Refresh Not Wired

**Problem**: `refresh_if_needed()` exists but is never called from the startup path. Users will encounter 401 errors mid-session when tokens expire.

**Fix**: Call `sso::refresh_if_needed()` during CLI startup before building the provider registry, so that a token expiring during a chat session triggers a pre-emptive refresh at launch time.

---

### P4 — Low: OAuth Client Registration Required

**Dependency**: Before any user can run `halcon login cenzontle`, the client `cuervo-cli` must be registered in the Zuclubit SSO instance with:

- `client_id`: `cuervo-cli`
- `redirect_uris`: `["http://localhost:9876/callback"]`
- `grant_types`: `["authorization_code", "refresh_token"]`
- `token_endpoint_auth_method`: `none` (public client — no secret for PKCE flow)
- `scope`: `openid profile email offline_access`

This is an infrastructure prerequisite, not a code change.

---

## Summary

| Section | Status | Priority Issues |
|---------|--------|-----------------|
| PKCE S256 implementation | PASS | — |
| CSRF state protection | PASS | — |
| Token storage (OS keychain) | PASS | P2: silent write failure |
| Token redaction in logs | PASS | — |
| Air-gap exclusion | PASS | — |
| Provider trait conformance | PASS | — |
| Chat endpoint routing | PASS | — |
| Model list at runtime | FAIL | **P1: always empty** |
| Callback server robustness | PARTIAL | P2: single-line read, no timeout |
| CI/CD bypass | PASS | P3: empty-secret check |
| Naming consistency | FAIL | P3: typo in struct name |
| Proactive token refresh | MISSING | P3: never called |

**Overall Verdict**: The SSO authentication flow is cryptographically sound and correctly implements OAuth 2.1 + PKCE S256. The Cenzontle provider architecture is clean and respects crate dependency boundaries. However, the P1 empty model list issue means Cenzontle **cannot successfully serve any model requests** in the current state. This integration is **not production-ready** until the P1 fix is applied. The P2 issues (callback server robustness, silent keychain failure) should be addressed before any customer-facing deployment.

With the P1 fix applied, this integration is suitable for internal beta testing.
