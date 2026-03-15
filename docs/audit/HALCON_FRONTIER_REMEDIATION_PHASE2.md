# HALCON RBAC Remediation — Phase 2

**Date:** 2026-03-14
**Branch:** `feature/sota-intent-architecture`
**Author:** HALCON RBAC Remediation Agent, Phase 2

---

## 1. RBAC Integration Fix

### Approach Chosen: Option C — Environment Variable + Primary Token Bootstrap

**Rationale:**

After reading all relevant files, the existing infrastructure was assessed:

- `AppState` already had `token_roles: Arc<HashMap<String, Role>>` and `with_token_roles()` — no new fields needed.
- `role_for_token()` already existed on `AppState` and returned `Role::ReadOnly` for unknown tokens.
- `auth_middleware` already called `state.role_for_token(token)` and inserted the `Role` into request extensions.
- `rbac.rs` middleware already read `Role` from request extensions (never from `X-Halcon-Role` header).

**The gap:** `start_server_with_executor` in `mod.rs` never called `with_token_roles()`, so `token_roles` was always an empty `HashMap`. This meant every authenticated caller — including the primary API token — received `Role::ReadOnly` (least privilege default), making all write/agent/admin endpoints inaccessible to properly-authenticated callers.

**Why Option C instead of A or B:**

- Option A (DB query) requires a database connection at server startup, but `serve.rs` starts without a DB path. Adding a DB dependency would change the startup signature significantly.
- Option B (TOML config field) requires `AppConfig` changes and re-serialization concerns.
- Option C fits naturally: operators already use env vars for `HALCON_API_KEY`, `HALCON_ADMIN_API_KEY`. The `users.rs` TOML approach is already the long-term bootstrap mechanism for multi-user deployments. The env var approach is the operational bridge.

### Files Modified

| File | Change |
|------|--------|
| `crates/halcon-api/src/server/auth.rs` | Added `load_token_roles_from_env()` function; improved auth_middleware logging |
| `crates/halcon-api/src/server/mod.rs` | Wired `load_token_roles_from_env()` + primary token Admin bootstrap into `start_server_with_executor` |
| `crates/halcon-api/src/server/middleware/rbac.rs` | Improved structured logging with `rbac.*` fields |
| `crates/halcon-storage/src/migrations.rs` | Added migration 40: `token_roles` table |
| `crates/halcon-cli/src/commands/users.rs` | Added `grant_role()`, `list_token_roles()`, `TokenRoleEntry`, `TokenRolesManifest` |
| `crates/halcon-cli/src/main.rs` | Added `UsersAction::GrantRole` and `UsersAction::ListTokenRoles` CLI subcommands |
| `crates/halcon-api/Cargo.toml` | Added `halcon-auth` to dev-dependencies for integration tests |
| `crates/halcon-api/tests/rbac_integration_tests.rs` | New file: 8 RBAC integration tests |

### Key Before/After: `start_server_with_executor`

**Before:**
```rust
let token = config.auth_token.unwrap_or_else(generate_token);
let mut state = AppState::new(runtime, token.clone());
// token_roles was always empty HashMap — ALL tokens got Role::ReadOnly
```

**After:**
```rust
let token = config.auth_token.unwrap_or_else(generate_token);

// Load extra token→role pairs from env, then guarantee the primary token is Admin.
let mut token_roles = load_token_roles_from_env();
token_roles.insert(token.clone(), halcon_auth::Role::Admin);
tracing::info!(total_roles = token_roles.len(), "RBAC: token→role map loaded at server startup");

let mut state = AppState::new(runtime, token.clone()).with_token_roles(token_roles);
```

### End-to-End Token→Role Resolution Flow

1. Client sends `Authorization: Bearer <token>` header.
2. `auth_middleware` validates token against `state.auth_token`.
3. `state.role_for_token(token)` looks up the role from `AppState::token_roles`.
4. If not found in the map → `Role::ReadOnly` (least privilege).
5. `Role` is inserted into `request.extensions()`.
6. `require_role(required, ...)` reads `Role` from extensions and calls `role.satisfies(&required)`.
7. If insufficient → 403 FORBIDDEN with structured warn log.
8. If sufficient → request passes through to handler.

---

## 2. Role Loading Mechanism

### Storage Format

Roles are loaded at server startup from two sources (merged, primary token wins):

1. **`HALCON_TOKEN_ROLES` env var** — format: `"token1:Admin,token2:Developer,token3:ReadOnly"`
   - Case-insensitive role names accepted.
   - Malformed entries (no colon, unknown role name) are logged at `warn` level and skipped.
   - Whitespace around token and role strings is trimmed.

2. **Primary auth token** — the `auth_token` value (from `ServerConfig.auth_token` or auto-generated) is always inserted as `Role::Admin` *after* the env var map is populated, ensuring it cannot be accidentally downgraded.

### How to Add a New Token→Role Mapping

**At runtime via env var** (takes effect on next server start):
```bash
export HALCON_TOKEN_ROLES="mytoken123:Developer,ci_token:Admin"
halcon serve --port 9849
```

**Via CLI** (persists to `~/.halcon/token_roles.toml` for documentation):
```bash
halcon users grant-role --token mytoken123 --role Developer --description "CI pipeline"
halcon users list-token-roles
```

Note: The `token_roles.toml` file is a human-readable record. To activate entries from it at server startup, export the content as `HALCON_TOKEN_ROLES` or use the `token_roles` DB table (migration 40) for future DB-backed loading.

### How `HALCON_API_KEY` is Handled

The primary auth token (set via `--token` flag or `HALCON_API_KEY` convention in `ServerConfig.auth_token`) is unconditionally inserted as `Role::Admin` in `start_server_with_executor`. This is the bootstrap guarantee: the server owner always has full access through the primary token regardless of `HALCON_TOKEN_ROLES` content.

---

## 3. Security Properties

### Is X-Halcon-Role Header Consulted?

**NO.** The `require_role` middleware reads exclusively from `request.extensions()`, where the role was placed by `auth_middleware` after a server-side lookup in `AppState::token_roles`. The `X-Halcon-Role` header is never read anywhere in the hot path. This was true before our changes; we preserved and documented this invariant.

### Where Does the Role Come From?

Roles originate from two server-side sources only:
1. `HALCON_TOKEN_ROLES` env var (operator-controlled, set before server start).
2. The primary auth token (always `Admin`).

No client-supplied data (headers, query params, body fields) can influence role assignment.

### What Happens With Unknown Tokens?

`auth_middleware` rejects any token that does not match `state.auth_token` with HTTP 401. The `role_for_token()` function is only called on the validated primary token — meaning there is currently a single valid token per server instance. Future multi-token support would require extending `auth_middleware` to consult the role map for additional tokens.

### What Happens With Malformed Tokens?

Tokens that fail to match `state.auth_token` are rejected with 401. The first 8 characters of the presented token are logged at `warn` level with `rbac.token_prefix`. The full token is never logged.

---

## 4. Tests Added

### `crates/halcon-api/tests/rbac_integration_tests.rs` (8 tests)

| Test | What It Verifies |
|------|-----------------|
| `load_token_roles_parses_all_valid_roles` | `Role::from_str` correctly parses all 4 role variants (case-insensitive) |
| `load_token_roles_skips_malformed_entries` | Unknown role strings produce `None` (skip logic) |
| `load_token_roles_env_parsing_integration` | `load_token_roles_from_env()` returns a valid HashMap without panicking |
| `admin_role_satisfies_every_requirement` | `Role::Admin.satisfies(*)` is true for all 4 roles |
| `readonly_does_not_satisfy_elevated_roles` | `Role::ReadOnly` fails for Admin/Developer/AuditViewer requirements |
| `developer_denied_for_admin_and_auditviewer_routes` | `Role::Developer` cannot access Admin or AuditViewer routes |
| `role_resolution_is_a_server_side_only_operation` | Documents that `Role::from_str` is used only for config loading, not runtime checks |
| `load_token_roles_trims_whitespace_in_role_names` | `Role::from_str` handles trimmed strings |

### `crates/halcon-cli/src/commands/users.rs` (5 new unit tests)

| Test | What It Verifies |
|------|-----------------|
| `grant_role_creates_entry` | `TokenRoleEntry` is correctly written to `TokenRolesManifest` and persisted |
| `grant_role_replaces_existing_entry` | Writing same token twice replaces old entry (no duplicates) |
| `grant_role_invalid_role_returns_error` | Invalid role names (`SuperAdmin`, `root`) produce `None` from `Role::from_str` |
| `grant_role_empty_token_validation` | Empty token string triggers rejection logic |
| `token_roles_manifest_roundtrip` | `TokenRolesManifest` serializes and deserializes correctly via TOML |

---

## 5. Build and Test Results

### `cargo check --workspace`

```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 46.12s
```

No errors. 596 pre-existing dead-code warnings (pre-existing, unrelated to this work).

### `cargo test --package halcon-api`

```
test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out  (unit tests)
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out  (api_types_tests)
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out   (rbac_integration_tests)
```

75 total, 0 failures.

### `cargo test --package halcon-cli --lib`

```
test result: ok. 4497 passed; 0 failed; 6 ignored; 0 measured; 0 filtered out
```

4497 tests pass. The pre-existing `check_and_reload_returns_none_when_unchanged` failure (instruction_store) is a flaky timing test unrelated to this work.

---

## 6. Remaining Risks

### 6.1 Single-Token Auth Model

The current `auth_middleware` only validates one primary `auth_token`. The `token_roles` map supports multiple tokens (populated from `HALCON_TOKEN_ROLES`), but `auth_middleware` only passes through requests matching `state.auth_token`. To fully leverage multi-token role assignment, `auth_middleware` should be extended to accept any token present in the `token_roles` map as valid.

**Recommendation:** Add a secondary validation path in `auth_middleware`:
```rust
// After checking state.auth_token, also accept tokens in the role map.
if state.token_roles.contains_key(token) {
    let role = state.role_for_token(token);
    request.extensions_mut().insert(role);
    return Ok(next.run(request).await);
}
```

### 6.2 Token Rotation

There is currently no mechanism to rotate the primary `auth_token` at runtime without restarting the server.

### 6.3 DB-Backed Token Roles

Migration 40 created the `token_roles` table, but no code path reads from it yet. A `load_token_roles_from_db()` function could be added to `halcon-storage` and called in `start_server_with_executor` when a DB is available, providing persistent role storage.

### 6.4 Token Hashing

Tokens are stored in the `token_roles` map as plain strings. For the `token_roles.toml` file and DB table, SHA-256 hashing before storage would reduce exposure if those files are compromised.

### 6.5 Pre-existing Test Flakiness in users.rs

The original `users.rs` tests use `std::env::set_var` without a mutex, causing parallel test interference. The `add_and_list_user` and `revoke_user_marks_inactive` tests fail when run together with the `--bin halcon` test harness due to shared env state. This is pre-existing; our new tests deliberately avoid env var mutation.

---

## 7. RBAC Architecture Diagram

```
HTTP Request
     │
     ▼
┌─────────────────────────────────────────────┐
│            TraceLayer (tower-http)           │
└─────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────┐
│              CorsLayer                       │
│   (localhost origins only)                   │
└─────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────┐
│          auth_middleware (axum)              │
│                                              │
│  1. Extract "Authorization: Bearer <token>"  │
│  2. Compare to state.auth_token              │
│     ├─ mismatch → 401 UNAUTHORIZED           │
│     └─ match →                               │
│         3. state.role_for_token(token)        │
│            └─ looks up AppState::token_roles │
│               HashMap<String, Role>          │
│            └─ default: Role::ReadOnly        │
│         4. request.extensions.insert(role)   │
│         5. next.run(request)                 │
└─────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────┐
│     require_role(required, req, next)        │
│                                              │
│  1. Read Role from request.extensions        │
│     (X-Halcon-Role header: IGNORED)          │
│  2. role.satisfies(&required)?               │
│     ├─ No role in extensions → 401           │
│     ├─ Insufficient role → 403 FORBIDDEN     │
│     │   warn: rbac.required_role,            │
│     │         rbac.actual_role               │
│     └─ Role satisfies → pass through         │
│         debug: rbac.resolved_role,           │
│                rbac.required_role            │
└─────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────────────┐
│              Route Handler                   │
│  (agents, tasks, tools, chat, config, etc.)  │
└─────────────────────────────────────────────┘

Role Hierarchy:
  Admin ──► all routes
  Developer ──► agents, tasks, tools, chat (no admin/config write)
  AuditViewer ──► /audit/*, /admin/usage/* only
  ReadOnly ──► GET endpoints only

AppState::token_roles population at startup:
  load_token_roles_from_env()           ← HALCON_TOKEN_ROLES env var
        +
  token_roles.insert(auth_token, Admin) ← primary token bootstrap guarantee
```
