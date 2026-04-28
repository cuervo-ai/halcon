# Phase 1 — E2E validation runbook

Companion of `docs/development/phase-1-test-matrix.xlsx`. Open
the Excel file in Numbers / Excel / LibreOffice; this runbook
walks through the same scenarios in plain text so you can
follow either or both.

> **Branch under test**: `phase-1/security-cleanup-ci`
> **Builds in**: `target/release/halcon` (29 MB binary)
> **Status as of generation**: 63/63 phase-1 unit tests green;
> binary builds; boundary check passes. Three items remain
> blocked on operator action — see §6.

---

## 0 · Pre-flight

```bash
cd ~/Documents/Projects/cuervo-ai/active/halcon
git fetch origin
git checkout phase-1/security-cleanup-ci
git log --oneline main..HEAD     # expect 9 commits
```

If you don't yet have the branch locally, this is the moment.

---

## 1 · Automated suite (≈ 5 min)

Already passed during initial validation; re-run before merge:

```bash
# Workspace check — no warnings beyond the pre-existing 6
cargo fmt --all -- --check
cargo check --workspace --no-default-features --features tui \
    --exclude momoto-core --exclude momoto-metrics --exclude momoto-intelligence

# Phase-1 unit tests (63 in total)
cargo test -p halcon-tools     --no-default-features network_policy
cargo test -p halcon-tools     --no-default-features http_request
cargo test -p halcon-tools     --no-default-features web_fetch
cargo test -p halcon-auth      --no-default-features oauth
cargo test -p halcon-providers --no-default-features --features paloma --lib cenzontle::tests::sanitize

# Boundary enforcer
bash scripts/check_boundaries.sh
```

**Expected**: every command exits 0. The boundary script prints
`PASS (0 violaciones estrictas)` plus the pre-existing
`/tasks endpoint` warning carried over from `main`.

---

## 2 · Build & install on your machine

```bash
# Release build (already done, takes ~3 min cold)
cargo build --release -p halcon-cli --no-default-features --features tui

# Sanity
file target/release/halcon
./target/release/halcon --version
./target/release/halcon --help

# Optional install
mkdir -p ~/.local/bin
install -m 0755 target/release/halcon ~/.local/bin/halcon
which halcon
```

**Note**: if you previously had a Halcón binary that was
installed under sudo, files in `~/.halcon/` may be owned by
root. Run `sudo chown -R "$USER" ~/.halcon` once.

---

## 3 · Live SSRF guard validation (P0)

This is the single most important manual test. Goal: prove a
prompt-injected agent **cannot** reach loopback or cloud
metadata.

### 3.1 Without provider auth (one-shot tool invocation)

The CLI exposes tools through the agent loop, so the real test
goes through Cenzontle. If you do not have credentials set up
for Cenzontle, skip this section and use 3.2 (unit-test path).

```bash
halcon chat
# In the prompt, type something like:
> Use the http_request tool to GET http://127.0.0.1:8080/secret
```

**Expected**: the agent's tool call returns
`http_request error: address 127.0.0.1 is blocked (loopback (127.0.0.0/8))`,
or equivalent for whatever URL you used. The agent never opens
a socket.

Repeat for:

| URL | Expected reject reason |
|---|---|
| `http://10.0.0.1/admin` | `private (RFC1918)` |
| `http://169.254.169.254/latest/meta-data/iam/` | hostname / link-local |
| `http://metadata.google.internal/` | hostname |
| `http://[::1]/` | `loopback (::1)` |
| `http://[fe80::1]/` | `link-local (fe80::/10)` |

### 3.2 Without a provider — direct tool unit invocation

If you cannot run the agent loop (no Cenzontle login), prove
the guard works through unit tests:

```bash
cargo test -p halcon-tools --no-default-features network_policy -- --nocapture
```

Each test name corresponds 1-to-1 with one row of the
**P1-B_SSRF** sheet. All 17 must pass.

### 3.3 Negative — public URL must still work

```bash
halcon chat
> Use web_fetch on https://example.com/
```

**Expected**: tool returns the HTML body, not blocked.

If a public URL is blocked, the guard is over-rejecting and
must be fixed before merge.

---

## 4 · OAuth state validation (P0)

### 4.1 Unit test (always works)

```bash
cargo test -p halcon-auth --no-default-features oauth -- --nocapture
```

Specifically verify these names appear in the output:

- `exchange_code_success`
- `exchange_code_rejects_state_mismatch`
- `exchange_code_rejects_empty_expected_state`

### 4.2 Live tampering test (interactive)

Only if you have a Cenzontle login configured.

1. Run `halcon auth login cenzontle`.
2. Wait for the browser tab to open (`/oauth/authorize?...`).
3. Before clicking **Authorize**, **edit the `state` query
   parameter** in the browser URL bar (e.g. change one char) and
   reload.
4. Authorize.
5. The browser redirects to `http://localhost:NNNN/callback?code=…&state=TAMPERED`.

**Expected**: the CLI prints
`State mismatch in OAuth callback (CSRF protection triggered).`
and exits with non-zero. No token is written to the keystore.

### 4.3 Happy path

Repeat without tampering. Browser flow completes; the CLI
prints the model list (proof token was written + reused).

---

## 5 · Log sanitization (P1)

Hard to trigger without a misbehaving Cenzontle. Two paths:

### 5.1 Unit test

```bash
cargo test -p halcon-providers --no-default-features --features paloma \
    --lib cenzontle::tests::sanitize -- --nocapture
```

The 6 tests prove every redaction shape, the truncation
threshold, and UTF-8 boundary safety.

### 5.2 Live (advanced — only if you can mock Cenzontle)

Point `CENZONTLE_BASE_URL` to a local mock server that returns
a 500 with `{"error":"Bearer abc.GhIjKl.123 expired"}` and run a
chat. The CLI's error output should show `[REDACTED]`, not the
token. Log entries from `tracing` should also be free of the
token.

---

## 6 · Items waiting on operator action

Three rows in the matrix are marked **BLOCKED** until you do:

1. **CI green run** — set the secret:
   ```bash
   gh secret set PALOMA_HTTPS_TOKEN --repo cuervo-ai/halcon \
       --body "github_pat_..."
   ```
   See `docs/development/CI_SECRETS.md` for token scopes.

2. **Live SSRF block** — operator validation (§3.1).

3. **Live OAuth happy path** — operator validation (§4.3).

---

## 7 · How to record results

The Excel matrix has a `Status` column on every test sheet.
Convention:

- **PASS** — verified, fill in the cell with `PASS`
- **FAIL** — fill in with `FAIL` plus a note explaining
- **BLOCKED** — waiting on something out of scope; explain in
  `Notes`
- **N/A** — not applicable to this build / target

The `Initial Results` sheet is the agent's first pass; the
operator's job is to re-run §1 + manually walk §3, §4, §5
and update the per-area sheets to mark all rows PASS or note
exceptions.

---

## 8 · If anything fails

1. **Test failure that wasn't there on `main`** — flag it on
   the PR; the matrix sheet's `Status=FAIL` cell with a link to
   the diff is enough triage information.
2. **Build failure** — confirm you're on the right branch and
   reran `cargo clean -p halcon-tools` before re-checking. The
   SSRF guard imports a new module, so a stale incremental
   cache can mask the change.
3. **Boundary-check regression** — script output includes the
   exact line. Compare against `main`'s baseline to confirm
   it's actually new and not a pre-existing warning.

---

## 9 · Pre-merge checklist

- [ ] Section 1 — automated suite green
- [ ] Section 2 — release binary built and `--help` works
- [ ] Section 3 — at least 5 SSRF rows manually validated
- [ ] Section 4 — OAuth unit tests + at least one live flow
- [ ] Section 5 — sanitization unit tests
- [ ] Section 6 — `PALOMA_HTTPS_TOKEN` secret configured on the
  repo so CI is unblocked from the moment the PR is opened
- [ ] Excel `Initial Results` sheet reviewed; any `FAIL` rows
  resolved before merge
- [ ] Phase-2 decisions sheet logged into the team tracker
  (D-1..D-8) so they are not lost
