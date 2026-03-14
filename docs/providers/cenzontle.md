# Cenzontle SSO via Zuclubit

Cenzontle is the Halcon enterprise identity layer. It authenticates through **Zuclubit**, Cuervo AI's internal OAuth 2.1 identity provider, and issues short-lived access tokens that Halcon manages and auto-refreshes.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Enterprise Network                          │
│                                                                 │
│  ┌──────────────┐    OAuth 2.1     ┌─────────────────────────┐ │
│  │  Halcon CLI  │◄────────────────►│   Zuclubit IdP          │ │
│  │              │    PKCE flow     │                         │ │
│  │  ~/.halcon/  │                  │  • JWT issuance         │ │
│  │  tokens/     │                  │  • Token refresh        │ │
│  │  cenzontle   │                  │  • RBAC enforcement     │ │
│  └──────┬───────┘                  └─────────────────────────┘ │
│         │                                                       │
│         │  Bearer token                                         │
│         ▼                                                       │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │               Cenzontle Gateway                          │  │
│  │                                                          │  │
│  │  • Routes to Anthropic Claude models                     │  │
│  │  • Enforces usage quotas per user/team                   │  │
│  │  • Audit logging for all LLM calls                       │  │
│  │  • Data residency controls                               │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Token Flow (OAuth 2.1 PKCE)

```
1. halcon login
   └─► Generate code_verifier (256 random bytes, base64url)
   └─► Derive code_challenge = BASE64URL(SHA256(code_verifier))

2. Browser opens:
   https://auth.zuclubit.io/authorize
     ?client_id=halcon-cli
     &redirect_uri=http://localhost:9876/callback
     &response_type=code
     &code_challenge=<challenge>
     &code_challenge_method=S256
     &scope=openid+profile+cenzontle:api

3. User authenticates in browser (SAML/LDAP/social, depending on org config)

4. Zuclubit redirects to http://localhost:9876/callback?code=<auth_code>
   └─► Halcon loopback server captures the code

5. Token exchange:
   POST https://auth.zuclubit.io/token
     code=<auth_code>
     code_verifier=<verifier>
     grant_type=authorization_code

6. Response:
   {
     "access_token":  "eyJ...",    # short-lived (1h)
     "refresh_token": "rt_...",    # long-lived (30d)
     "expires_in":    3600
   }

7. Tokens stored in system keychain:
   Service: "halcon-cenzontle"
   Account: "<user@org.com>"

8. Auto-refresh: Halcon refreshes 5 minutes before expiry
```

---

## Setup

### Login

```bash
halcon login
# or the long form:
halcon auth sso-login cenzontle
```

This opens your browser. After authenticating, the terminal shows:

```
Logged in to Cenzontle as alice@company.com
Token expires: 2026-03-14 10:00 UTC (auto-refresh enabled)
```

### Verify

```bash
halcon auth status
# Output includes:
#   cenzontle: logged in as alice@company.com (expires in 58m, auto-refresh on)
```

### Use

```bash
# Explicit provider selection
halcon chat --provider cenzontle --model claude-sonnet-4-6

# Set as default in config
# ~/.halcon/config.toml
[agent]
provider = "cenzontle"
model    = "claude-sonnet-4-6"
```

### Logout

```bash
halcon auth logout cenzontle
# Revokes token at Zuclubit and removes from keychain
```

---

## CI / Non-interactive environments

For CI pipelines and servers where browser-based login is not possible, use a service account token:

```bash
# Set the token directly (no browser flow)
export CENZONTLE_ACCESS_TOKEN="eyJ..."

# Halcon detects this env var and uses it as the Bearer token
halcon chat --provider cenzontle
```

Obtain service account tokens from the Cenzontle admin console or via the Zuclubit management API.

---

## Configuration

```toml
# ~/.halcon/config.toml

[providers.cenzontle]
base_url     = "https://api.cenzontle.cuervo.cloud"  # default
token_url    = "https://auth.zuclubit.io/token"       # default
client_id    = "halcon-cli"                            # default
# No client_secret needed (PKCE public client)
```

Environment variable overrides:

| Variable | Description |
|----------|-------------|
| `CENZONTLE_ACCESS_TOKEN` | Direct bearer token (skips PKCE flow) |
| `CENZONTLE_BASE_URL` | Override gateway URL |
| `ZUCLUBIT_TOKEN_URL` | Override identity provider URL |

---

## Troubleshooting

### `cenzontle: not logged in`

Run `halcon login` to initiate the SSO flow.

### `Token expired` / `401 Unauthorized`

Auto-refresh should handle this transparently. If it fails:

```bash
halcon auth logout cenzontle
halcon login
```

### `localhost:9876 connection refused`

The loopback callback server failed to bind port 9876. Check:
- No other process is using port 9876: `lsof -i :9876`
- Firewall is not blocking localhost connections

### `Browser did not open`

Manually open the URL printed to the terminal, or set `BROWSER=` env var to your browser binary.

### Org SSO not working

Contact your Cenzontle admin to verify:
1. Your email is in the allowed domain list
2. Your account has the `halcon:api` scope enabled
3. The Halcon client ID (`halcon-cli`) is registered in Zuclubit

---

## Security notes

- Halcon uses OAuth 2.1 **PKCE** (Proof Key for Code Exchange) — no client secret stored on disk
- Tokens are stored in the **system keychain** (macOS Keychain, Linux Secret Service, Windows Credential Manager)
- The loopback redirect URI (`localhost:9876`) is a one-time server that closes after receiving the code
- Refresh tokens are single-use and rotated on each refresh
