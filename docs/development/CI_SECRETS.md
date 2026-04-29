# CI Secrets — Halcón

Halcón depends on `cuervo-ai/paloma` (private repo) as a git
dependency. GitHub Actions runners do not have credentials by
default and so cannot fetch it, which is why every CI run since
the Paloma pin (commit `c374be9`) has been failing at the Clippy /
Check / Test steps.

This document is the runbook for setting up the credentials
required to make CI green again. You only need to do this once
per repo. The action and workflow wiring are already in place.

---

## What you need

| Secret | Type | Required by | Purpose |
|---|---|---|---|
| `PALOMA_HTTPS_TOKEN` | string | clippy, check, test-linux, test-macos | Fetches `cuervo-ai/paloma` via HTTPS during cargo build |
| `CLOUDFLARE_API_TOKEN` | string | deploy-website | Already configured |
| `CLOUDFLARE_ACCOUNT_ID` | string | deploy-website | Already configured |

Only `PALOMA_HTTPS_TOKEN` is new and required for Phase 1.

---

## Option A — Fine-grained PAT (recommended for Phase 1)

1. Create a fine-grained personal access token at
   <https://github.com/settings/personal-access-tokens/new>
   - **Repository access**: only `cuervo-ai/paloma`
   - **Permissions** → **Repository permissions**:
     - Contents: **Read-only**
     - Metadata: **Read-only** (auto-required)
   - **Expiration**: 90 days (rotate quarterly)

2. Copy the token (`github_pat_…`).

3. Add it as a repo secret:
   ```bash
   gh secret set PALOMA_HTTPS_TOKEN --repo cuervo-ai/halcon --body "<paste token>"
   ```
   or via UI: **Settings → Secrets and variables → Actions →
   New repository secret**.

4. Re-run the failing workflow. Expected: green build.

5. Set a calendar reminder for renewal 7 days before expiry.

---

## Option B — GitHub App (recommended once Phase 1 stabilises)

A GitHub App tied to the `cuervo-ai` org grants short-lived
installation tokens, eliminating PAT rotation.

1. Create a GitHub App on the `cuervo-ai` org with these
   repository permissions: **Contents: Read-only**.
2. Install it on `cuervo-ai/paloma` and `cuervo-ai/halcon`.
3. Store the App private key as `PALOMA_APP_PRIVATE_KEY` and the
   App ID as `PALOMA_APP_ID`.
4. Replace the `setup-private-deps` action with one that calls
   `tibdex/github-app-token` to mint a 60-min installation token,
   then performs the same `url.insteadOf` rewrite.

This migration is tracked as a Phase 2 follow-up.

---

## Option C — Deploy key (per-repo SSH)

If the org disallows PATs, generate a read-only SSH deploy key:

```bash
ssh-keygen -t ed25519 -f paloma_deploy -C "halcon-ci" -N ""
```

- Add `paloma_deploy.pub` to **cuervo-ai/paloma → Settings →
  Deploy keys** (read-only, do **not** check "Allow write").
- Add the private key bytes (`paloma_deploy`) as the `PALOMA_SSH_KEY`
  secret.
- Replace the HTTPS rewrite step with `webfactory/ssh-agent@v0.9`
  loading the key, and use a SSH-form rewrite:
  `git config --global url."git@github.com:cuervo-ai/".insteadOf "https://github.com/cuervo-ai/"`.

The composite action `setup-private-deps` accepts a `token` input
today; an `ssh_key` mode is a Phase 2 enhancement.

---

## Local dev

You don't need this secret on a developer laptop — your normal
git credentials (SSH key or `gh auth login`) will pick up the
Paloma fetch automatically. The CI-only friction is the absence
of those credentials on a fresh runner.

---

## Verifying

After setting `PALOMA_HTTPS_TOKEN`, kick a workflow run:

```bash
gh workflow run CI --repo cuervo-ai/halcon --ref main
```

Watch the `clippy` job. The first step `Verify token present`
should print `PALOMA_HTTPS_TOKEN detected (length=…)`. If it
errors with `Missing secret`, the secret is not visible to the
job (check that it's a *repo secret*, not an *environment
secret* scoped to `production`).

---

## Token rotation

When the PAT expires, the same `gh secret set` command overwrites
the value. There is no other change required. If you observe a
sudden CI break with a `401` from `github.com/cuervo-ai/paloma`,
rotate first.
