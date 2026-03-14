---
title: "Cenzontle SSO via Zuclubit"
description: "Enterprise Single Sign-On for Halcon CLI using Zuclubit OAuth 2.1 PKCE"
order: 4
category: "Enterprise"
version: "0.3.0"
---

# Cenzontle SSO via Zuclubit

Cenzontle is the Halcon enterprise identity layer. It authenticates through **Zuclubit**, an OAuth 2.1 identity provider, and issues short-lived access tokens that Halcon manages and auto-refreshes.

## Quick Setup

```bash
# 1. Login (opens browser)
halcon login

# 2. Verify
halcon auth status

# 3. Use
halcon chat --provider cenzontle
```

## How it works

Halcon uses **OAuth 2.1 PKCE** (Proof Key for Code Exchange):

1. `halcon login` generates a random `code_verifier` and derives `code_challenge = BASE64URL(SHA256(verifier))`
2. Your browser opens the Zuclubit authorization page
3. You authenticate (SAML/LDAP/social depending on your org config)
4. Zuclubit redirects to `localhost:9876/callback` with an authorization code
5. Halcon exchanges the code + verifier for tokens
6. Tokens are stored in your system keychain and auto-refreshed

## Token management

| Token | Lifetime | Storage |
|-------|----------|---------|
| Access token | 1 hour | System keychain |
| Refresh token | 30 days | System keychain |

Halcon auto-refreshes access tokens 5 minutes before expiry. No manual intervention required.

## CI / Non-interactive environments

```bash
export CENZONTLE_ACCESS_TOKEN="eyJ..."
halcon chat --provider cenzontle
```

Obtain service account tokens from the Cenzontle admin console.

## Logout

```bash
halcon auth logout cenzontle
# Revokes token at Zuclubit and removes from keychain
```

## Troubleshooting

**`cenzontle: not logged in`** — Run `halcon login`

**`401 Unauthorized`** — Token may have expired during an extended offline period. Run `halcon auth logout cenzontle && halcon login`

**Port 9876 in use** — Check `lsof -i :9876` and stop the conflicting process

**Browser did not open** — Manually open the URL printed to the terminal

## Security

- PKCE public client — no client secret stored anywhere
- Tokens in system keychain, never in plaintext files
- Loopback redirect URI is a one-time server that closes after use
- Refresh tokens are single-use and rotated on each refresh
