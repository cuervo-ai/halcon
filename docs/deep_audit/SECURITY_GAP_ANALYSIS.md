# HALCON Security Gap Analysis

**Agent:** 6 — Security and Permission Analyzer
**Date:** 2026-03-12
**Branch:** `feature/sota-intent-architecture`
**Scope:** Sandbox model, permission enforcement, RBAC, tool trust, and attack surface review

---

## 1. Sandbox Model

### How It Works

The codebase implements a **dual-layer sandbox architecture** split across two separate crates.

**Layer 1 — `halcon-sandbox` crate** (`crates/halcon-sandbox/src/`)

`SandboxedExecutor` (`executor.rs`) provides OS-level process isolation:

- **macOS**: Wraps commands via `sandbox-exec -p <profile>`. The Seatbelt profile used denies `network*` by default, and optionally denies file-writes to `/etc` and `/var`. The allow-network branch produces the profile `"(version 1)\n(allow default)\n(deny network*)\n"` which permits everything else.
- **Linux**: Uses `unshare --net --` (rootless network namespace isolation). Does **not** isolate the user namespace, filesystem, or PID namespace.
- **Fallback**: Policy denylist only — no OS-level isolation.

`SandboxPolicy` (`policy.rs`) performs pre-spawn string-matching checks:
- Privilege escalation: looks for `"sudo "`, `"sudo\t"`, `" su "`, `"doas "`, `"pkexec "` as substrings
- Network denial: substring match for `"curl "`, `"wget "`, `"nc "`, `"netcat "`, `"ssh "`, `"scp "`, `"rsync "`
- Directory escape: presence of `"../../"`, `"/etc/"`, or `"/var/"` combined with write indicators
- Dangerous patterns: ~17 substring literals (e.g., `"rm -rf /"`, `"dd if="`, `"mkfs."`)
- Extra denylist: user-configured additional patterns
- Command length cap: 4096 chars by default; 2048 in `strict()` mode

Resource limits:
- Execution timeout (default 30s)
- Max output bytes (default 256 KB, head+tail truncated at 60%/30%)
- Working directory restriction passed to `current_dir()`

**Layer 2 — `halcon-tools/bash.rs`**

`BashTool` is the actual agent-facing tool. It performs:
1. A regex-based blacklist check using patterns from `halcon_core::security::CATASTROPHIC_PATTERNS` (18 patterns)
2. `apply_rlimits()` via `pre_exec()` on Unix (CPU time, memory limits)
3. Execution via `tokio::process::Command::new("bash")` directly — **this path does NOT invoke `SandboxedExecutor`**

The two sandboxing paths are architecturally separate and **are not connected**:
- `halcon-sandbox` is a standalone crate with its own executor
- `BashTool` in `halcon-tools` calls `sandbox::apply_rlimits()` but not `SandboxedExecutor`

### Sandbox Gaps

**GAP-S1: BashTool does not use SandboxedExecutor**
- File: `crates/halcon-tools/src/bash.rs`, line 172 — `Command::new("bash")` runs directly
- The OS-level `sandbox-exec` / `unshare` isolation in `halcon-sandbox` is available but unused by the primary bash execution path
- A command that passes the `CATASTROPHIC_PATTERNS` regex check executes with no OS-level isolation

**GAP-S2: macOS Seatbelt profile is overly permissive**
- File: `crates/halcon-sandbox/src/executor.rs`, lines 221-225
- The profile `"(allow default)\n(deny network*)"` only adds network denial on top of a full-allow baseline
- File-write restrictions are limited to `/etc` and `/var` — directories like `/usr/local`, `/tmp`, `/home`, and the user's project root remain fully writable

**GAP-S3: Linux sandbox is network-only**
- File: `crates/halcon-sandbox/src/executor.rs`, lines 239-251
- `unshare --net` only isolates the network namespace
- Filesystem, PID, mount, and user namespaces are not isolated
- A compromised command can still read/write the entire filesystem

**GAP-S4: Privilege escalation detection is substring-based and bypassable**
- File: `crates/halcon-sandbox/src/policy.rs`, lines 117-132
- Detects `"sudo "` as a substring; bypassed by: `$(which sudo) ls`, `eval 'sudo ls'`, `s\
udo ls` (shell line continuation), `SUDO_COMMAND=; sudo ls`
- No detection of `su -c`, `newgrp`, `runuser`, `nsenter`

**GAP-S5: Network denial in SandboxPolicy is substring-based**
- File: `crates/halcon-sandbox/src/policy.rs`, lines 135-148
- Bypassed by: `/usr/bin/curl`, `python3 -c "import urllib.request..."`, `node -e "require('https')..."`, Rust binaries calling network APIs

**GAP-S6: Directory escape detection only flags writes to `/etc/` and `/var/`**
- File: `crates/halcon-sandbox/src/policy.rs`, lines 184-199
- Writes to `/proc/`, `/sys/`, `/boot/`, `/root/`, `/home/`, `/opt/` are not flagged
- The working_dir is not enforced as a hard boundary in the policy layer; only the OS sandbox restricts this at runtime (and only on macOS via Seatbelt)

---

## 2. Permission Enforcement

### Architecture

Permission checks follow a four-layer policy chain defined in `crates/halcon-cli/src/repl/security/authorization.rs`:

```
CIDetectionPolicy → NonInteractivePolicy → PermissionLevelPolicy → [PersistentRulesPolicy] → SessionMemoryPolicy
```

- **CIDetectionPolicy** (line 74): Auto-approves ALL tools when CI environment variables are detected (`CI=true`, `GITHUB_ACTIONS`, etc.)
- **NonInteractivePolicy** (line 89): Auto-approves ALL tools when session is non-interactive, with an exception for `always_denied` tools
- **PermissionLevelPolicy** (line 124): Auto-allows `ReadOnly` and `ReadWrite` tools; `Destructive` falls through
- **PersistentRulesPolicy** (line 176): Checks SQLite-persisted rules (optional — injected via `with_persistent_rules()`)
- **SessionMemoryPolicy** (line 147): Checks in-memory `always_allowed` (5-minute TTL) and `always_denied` sets

`PermissionLevel` classification:
- `ReadOnly` — auto-allowed (no prompt)
- `ReadWrite` — auto-allowed (no prompt), as seen in `permissions.rs` test line 388-397
- `Destructive` — requires prompt when interactive, auto-approved in non-interactive/CI

`BashTool.permission_level()` returns `PermissionLevel::Destructive` (`bash.rs`, line 136).

### Permission Gaps

**GAP-P1: TBAC is disabled by default**
- File: `crates/halcon-core/src/types/security.rs` (referenced at `permissions.rs`, line 688)
- `SecurityConfig::tbac_enabled` defaults to `false`
- When disabled, `check_tbac()` returns `NoContext` unconditionally, meaning task-scoped tool allowlists are never enforced
- A sub-agent can call any tool regardless of the task contract unless TBAC is explicitly enabled

**GAP-P2: `ReadWrite` tools are auto-allowed without prompts**
- File: `crates/halcon-cli/src/repl/security/authorization.rs`, lines 126-143
- Tools classified as `ReadWrite` (e.g., `file_edit`) bypass the permission prompt entirely
- This is intentional by design, but no user-facing documentation or warning covers what operations qualify as ReadWrite
- The distinction between `ReadWrite` and `Destructive` is set per-tool at compile time and cannot be reconfigured at runtime

**GAP-P3: CI environment auto-approval has a broad attack surface**
- File: `crates/halcon-cli/src/repl/git_tools/ci_detection.rs`, lines 86-116
- Setting any of 11 environment variables (e.g., `SEMAPHORE=1`, `DRONE=1`) causes all destructive tools to auto-approve
- On a shared developer machine or in a shared CI namespace, an attacker who can set env vars before invoking Halcon can fully bypass all destructive-operation prompts
- The only protection is the `always_denied` set, which is ephemeral (in-memory, cleared on restart)

**GAP-P4: `set_non_interactive()` silently approves all destructive tools**
- File: `crates/halcon-cli/src/repl/security/authorization.rs`, lines 396-399
- `AuthorizationMiddleware::set_non_interactive()` flips `state.interactive = false`, causing `NonInteractivePolicy` to auto-approve everything not in `always_denied`
- This is called from `PermissionChecker::set_non_interactive()` which is exposed as a public method (`permissions.rs`, line 107)
- Any caller with access to the `PermissionChecker` instance can permanently disable interactive checks for the session

**GAP-P5: `always_allowed` TTL is 5 minutes but not enforced during the permission decision in SandboxPolicy**
- File: `crates/halcon-cli/src/repl/security/authorization.rs`, lines 49-82
- Prune happens at the start of `authorize()`, but if `auto_decide()` is called instead, pruning does not occur (line 380-394)
- A tool granted `AllowedAlways` that is not accessed for 5+ minutes will still appear in `always_allowed` until the next `authorize()` call triggers prune

---

## 3. RBAC Model

### What Is Defined

The RBAC model is defined in `crates/halcon-auth/src/rbac.rs`. Four roles exist:

| Role | Permissions |
|------|-------------|
| `Admin` | All endpoints, config writes, user management |
| `Developer` | Agent invocation, tasks, tools, metrics |
| `ReadOnly` | GET endpoints only |
| `AuditViewer` | `/audit/*` and `/admin/usage/*` only |

The hierarchy is: `Admin > Developer > ReadOnly/AuditViewer` (the latter two are parallel leaf nodes — `Developer` does not include `AuditViewer`).

### How RBAC Is Enforced (or Not)

The RBAC middleware is defined in `crates/halcon-api/src/server/middleware/rbac.rs` as a standalone `require_role()` async function.

**Critical finding**: `require_role()` is never called in the router.

- File: `crates/halcon-api/src/server/router.rs` — a full-text search finds zero invocations of `require_role`
- The router mounts all API routes behind a single Bearer token auth middleware (`auth_middleware` at line 122-126), which validates a single shared secret token
- Admin endpoints use a second shared secret from `HALCON_ADMIN_API_KEY`
- The `X-Halcon-Role` header checked by `require_role()` is **never enforced** on any route

### RBAC Gaps

**GAP-R1: RBAC is defined but not enforced — all routes are accessible to any valid Bearer token holder**
- File: `crates/halcon-api/src/server/router.rs`, lines 120-126
- Any holder of the single `auth_token` can invoke agent endpoints, submit tasks, read config, and call tools
- The role distinction (`Developer` vs `ReadOnly` vs `AuditViewer`) provides no actual access control
- The `require_role()` function in `rbac.rs` is tested in isolation but is a dead code path in production

**GAP-R2: Bearer token is a single shared secret, not per-user**
- File: `crates/halcon-api/src/server/auth.rs`, lines 22-32
- All authenticated users share the same `state.auth_token`; there is no per-user identity
- Token compromise affects all users; no revocation mechanism for individual users

**GAP-R3: The RBAC middleware trusts a client-supplied header for role claims**
- File: `crates/halcon-api/src/server/middleware/rbac.rs`, lines 37-62
- Even if `require_role()` were wired up, it reads the role from the `X-Halcon-Role` header — a value the client supplies
- There is no signature validation; any client can claim to be `Admin` by sending `X-Halcon-Role: Admin`
- The comment at line 9 says this is a "bootstrap implementation" to be replaced with signed JWT extraction, but no timeline is defined

**GAP-R4: Admin auth uses a single shared API key with no audit trail at the key level**
- File: `crates/halcon-api/src/server/router.rs`, lines 23-48
- `HALCON_ADMIN_API_KEY` is compared via string equality with no rate limiting, brute-force protection, or per-request logging beyond a warn on failure

---

## 4. Tool Trust Model

### How It Works

`ToolTrustScorer` (`crates/halcon-cli/src/repl/security/tool_trust.rs`) tracks per-tool runtime performance:

```
trust_score = 0.60 × success_rate + 0.25 × latency_score + 0.15 × recency_bonus
```

Three decision outcomes:
- `Include` — trust ≥ `deprioritize_threshold` (default: 0.40)
- `Deprioritize` — between `hide_threshold` (0.15) and `deprioritize_threshold`
- `Hide` — success_rate < `hide_threshold` (after ≥ `min_calls_for_filtering` = 3 calls)

Unknown tools (no history) receive trust score `1.0` — **full optimistic trust**.

The trust system is about **operational reliability**, not security. Its purpose is to suppress broken tools from the LLM's tool surface, not to prevent malicious tool use. Key properties:

- Trust is session-local (in-memory HashMap, not persisted)
- Trust is based on exit codes, not semantic analysis of outputs
- `record_success()` is called when exit code is 0, regardless of what the command did

### Tool Trust Gaps

**GAP-T1: Trust score measures operational success, not security safety**
- File: `crates/halcon-cli/src/repl/security/tool_trust.rs`, lines 119-138
- A tool that successfully exfiltrates data or writes to sensitive paths will be recorded as a success and gain trust
- There is no semantic validation of tool outputs against expected behavior

**GAP-T2: Unknown tools receive full trust (optimistic prior)**
- File: `crates/halcon-cli/src/repl/security/tool_trust.rs`, lines 148-151, 160-161
- A newly injected or MCP-sourced tool with no history gets trust `1.0` and `TrustDecision::Include`
- An attacker who registers a malicious MCP tool will have it fully trusted on first use

**GAP-T3: Tool trust is not integrated with the permission gate**
- Trust filtering only affects which tools appear in the LLM's tool surface
- A low-trust or hidden tool can still be explicitly invoked if the LLM constructs the correct JSON structure — the tool executor does not check trust before executing

---

## 5. Path Security

### How It Works

`resolve_and_validate()` in `crates/halcon-tools/src/path_security.rs`:
1. Resolves relative paths against working directory
2. Normalizes `..` and `.` components without filesystem access (no `canonicalize()`)
3. Checks against blocked glob patterns (filename and full path)
4. Enforces that the path starts with `working_dir` or is in `allowed_dirs`

Symlink attacks are not addressed: `normalize_path()` resolves path components syntactically but does not follow symlinks, so a symlink within the working directory pointing outside it is not detected.

### Path Security Gaps

**GAP-PATH1: Symlink escape not detected**
- File: `crates/halcon-tools/src/path_security.rs`, lines 158-177
- `normalize_path()` resolves `..` without calling `std::fs::canonicalize()`
- A symlink at `<working_dir>/link -> /etc` passes validation because the path `<working_dir>/link/passwd` starts with `working_dir`
- Exploitation requires the ability to create symlinks inside the working directory

**GAP-PATH2: Default blocked patterns are not enforced unless callers pass them**
- The blocked patterns (`".env"`, `"*.pem"`, etc.) are only applied if the calling tool passes them in `blocked_patterns: &[String]`
- There is no default blocked list enforced at the `resolve_and_validate` level; each tool must opt in
- Tools that call `resolve_and_validate(..., &[], &[])` with empty patterns get no file-type protection

**GAP-PATH3: `bash` tool does not use `path_security`**
- The bash tool (`bash.rs`) has no path-based restrictions at all
- The blacklist checks commands by string pattern, not by resolved path
- A command like `cat $(readlink -f /symlink/to/etc/shadow)` bypasses all path checks

---

## 6. Hardcoded Credentials

No production secrets or API keys were found hardcoded in Rust source files. All credentials are read from environment variables or generated at runtime.

Test-only patterns found (not a security risk in themselves, but worth noting):

- `crates/halcon-providers/src/anthropic/mod.rs`, line 916: `"sk-ant-oat01-test-token-abc123"` — used in a `#[test]` function to verify Bearer header construction. Not a real credential.
- `crates/halcon-auth/src/oauth.rs`, line 283: `"sk-test-token-abc123"` — in a mock OAuth server response inside `#[test]` block.
- `crates/halcon-cli/src/config_loader.rs`, line 396: `"secret123"` — in a `#[test]` function for env-var expansion.

Config files use environment variable references only:
- `config/default.toml`, line 14: `api_key_env = "ANTHROPIC_API_KEY"` — references env var, does not embed a value.

**No hardcoded production credentials found.**

---

## 7. Security Gaps Summary

| ID | Severity | Component | Description |
|----|----------|-----------|-------------|
| GAP-R1 | **Critical** | RBAC / API | `require_role()` is defined but never wired into the API router — RBAC provides no access control |
| GAP-R3 | **Critical** | RBAC | Role claim (`X-Halcon-Role`) is client-supplied with no signature validation |
| GAP-S1 | **High** | Sandbox | `BashTool` does not use `SandboxedExecutor` — OS-level isolation is unused on the primary execution path |
| GAP-P3 | **High** | Permissions | Setting any of 11 CI env vars (e.g., `SEMAPHORE=1`) bypasses all destructive tool prompts |
| GAP-P1 | **High** | TBAC | Task-based authorization (TBAC) is disabled by default; sub-agents have unrestricted tool access |
| GAP-T2 | **High** | Tool Trust | New/unknown tools receive full trust — malicious MCP tools are immediately trusted |
| GAP-S2 | **Medium** | Sandbox / macOS | Seatbelt profile is overly permissive (`allow default`) — only denies network and two directories |
| GAP-S3 | **Medium** | Sandbox / Linux | `unshare --net` provides no filesystem isolation |
| GAP-S4 | **Medium** | Sandbox | Privilege escalation detection bypassed by indirect invocation |
| GAP-R2 | **Medium** | Auth | Single shared Bearer token — all users are indistinguishable; no per-user revocation |
| GAP-PATH1 | **Medium** | Path Security | Symlink escape not detected by `normalize_path()` |
| GAP-S5 | **Medium** | Sandbox | Network denial bypassed by non-detected network-capable tools |
| GAP-PATH2 | **Low** | Path Security | Blocked file patterns only enforced when callers opt in |
| GAP-T1 | **Low** | Tool Trust | Trust score measures operational success, not security safety |
| GAP-T3 | **Low** | Tool Trust | Trust filtering does not prevent direct tool invocation by LLM |
| GAP-P2 | **Low** | Permissions | `ReadWrite` tools are silently auto-allowed |
| GAP-P4 | **Low** | Permissions | `set_non_interactive()` can be called by any code with PermissionChecker access |
| GAP-PATH3 | **Low** | Path Security | `bash` tool has no path-based restrictions |
| GAP-S6 | **Low** | Sandbox | Directory escape detection misses many sensitive paths |

---

## 8. Potential Attack Vectors

### AV-1: RBAC Bypass via Any Valid Token
**Path:** Obtain any valid Bearer token → send requests to agent invocation or config-write endpoints → full access regardless of intended role.
**Root cause:** GAP-R1 — `require_role()` is never called.

### AV-2: CI Environment Variable Injection → Full Tool Approval Bypass
**Path:** Set `SEMAPHORE=1` (or any other CI var from the list) before invoking Halcon → `CIDetectionPolicy` auto-approves all destructive tools → arbitrary command execution without user prompts.
**Root cause:** GAP-P3.
**Scenario:** A malicious build script, compromised CI configuration, or a developer running Halcon in a container that already has `GITHUB_ACTIONS` set.

### AV-3: Malicious MCP Tool Registration
**Path:** Register an MCP tool with a legitimate-sounding name → it receives full trust on first call (GAP-T2) → the tool executes malicious actions → recorded as `success` and gains permanent session trust.
**Root cause:** GAP-T1 + GAP-T2.

### AV-4: Symlink Escape from Working Directory
**Path:** Create a symlink inside the working directory pointing to `/etc` → call `file_read` or `file_write` with the symlink path → `normalize_path()` validates it as inside working_dir → actual file operation hits the symlink target outside the boundary.
**Root cause:** GAP-PATH1.

### AV-5: Indirect Privilege Escalation via BashTool
**Path:** Execute a command that does not contain the literal strings `"sudo "` or `"doas "` but achieves equivalent effect. Examples:
- `$(python3 -c "import subprocess; subprocess.call(['sudo', 'ls'])")` — `sudo` is not a literal prefix
- `env -i PATH=/evil bash -c "ls"` — PATH manipulation
- Shell aliases pre-configured in `.bashrc`
**Root cause:** GAP-S1 (no OS sandbox) + GAP-S4 (string-based detection only).

### AV-6: Role Claim Forgery (Future Risk)
**Path:** If `require_role()` is ever wired into the router, a client sends `X-Halcon-Role: Admin` → the middleware accepts it without signature validation → full admin access.
**Root cause:** GAP-R3.

### AV-7: TBAC Disabled → Sub-Agent Tool Abuse
**Path:** A sub-agent spawned with a narrow task context (e.g., "read only") can invoke destructive tools because TBAC is disabled by default → the task contract is not enforced.
**Root cause:** GAP-P1.

---

## 9. File Reference Index

| File | Key Security Finding |
|------|---------------------|
| `crates/halcon-sandbox/src/executor.rs` | macOS Seatbelt profile (L221-235), Linux unshare (L239-251) |
| `crates/halcon-sandbox/src/policy.rs` | Privilege escalation detection (L117-132), network denial (L135-148) |
| `crates/halcon-tools/src/bash.rs` | BashTool does NOT use SandboxedExecutor (L172); uses CATASTROPHIC_PATTERNS (L25-33) |
| `crates/halcon-core/src/security.rs` | 18 CATASTROPHIC_PATTERNS (L31-52), 12 DANGEROUS_COMMAND_PATTERNS (L58-119) |
| `crates/halcon-cli/src/repl/security/permissions.rs` | PermissionChecker, TBAC (L202-237), `set_non_interactive()` (L107-109) |
| `crates/halcon-cli/src/repl/security/authorization.rs` | Policy chain (L239-251), CI bypass (L241), TTL (L68-82) |
| `crates/halcon-cli/src/repl/git_tools/ci_detection.rs` | CI env var list (L94-106), auto-approval (L152-153) |
| `crates/halcon-cli/src/repl/security/tool_trust.rs` | Optimistic prior for unknown tools (L148-151) |
| `crates/halcon-cli/src/repl/security/output_risk.rs` | Output risk scoring (used but not blocking by default) |
| `crates/halcon-auth/src/rbac.rs` | Role definitions and hierarchy (L1-74) |
| `crates/halcon-api/src/server/middleware/rbac.rs` | `require_role()` — defined but never called from router (L30-63) |
| `crates/halcon-api/src/server/router.rs` | Single shared Bearer token auth (L120-126), no `require_role` wiring |
| `crates/halcon-api/src/server/auth.rs` | Token comparison (L22-32), `generate_token()` is cryptographically sound (L39-52) |
| `crates/halcon-tools/src/path_security.rs` | `normalize_path()` — no symlink resolution (L158-177) |
