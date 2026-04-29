# Security Policy

## Supported Versions

Only the latest minor on `main` receives security fixes during
the pre-1.0 phase. There is no LTS branch yet.

| Version | Supported          |
| ------- | ------------------ |
| 0.3.x   | :white_check_mark: |
| < 0.3   | :x:                |

## Reporting a Vulnerability

**Please do NOT report security vulnerabilities through public GitHub issues.**

Instead, please report them via email to: **security@cuervo.ai**

You should receive a response within 48 hours. If for some reason you do not, please follow up via email to ensure we received your original message.

Please include the following information in your report:

- Type of issue (e.g., buffer overflow, SQL injection, cross-site scripting, path traversal, etc.)
- Full paths of source file(s) related to the manifestation of the issue
- The location of the affected source code (tag/branch/commit or direct URL)
- Any special configuration required to reproduce the issue
- Step-by-step instructions to reproduce the issue
- Proof-of-concept or exploit code (if possible)
- Impact of the issue, including how an attacker might exploit it

## Security Model

### Tool Permission Levels

Cuervo CLI implements a three-tier permission model for all tools:

| Level | Description | User Consent |
|-------|-------------|--------------|
| **ReadOnly** | Cannot modify filesystem or system state | Not required |
| **ReadWrite** | Can create or modify files | Required on first use |
| **Destructive** | Can delete files, execute commands, or cause irreversible changes | Always required |

### File Operation Safety

- **Atomic writes**: All file write/edit operations use temp file + fsync + rename to prevent corruption
- **Symlink protection**: File operations refuse to follow or write through symlinks
- **Path traversal prevention**: All file paths are validated against allowed directories
- **Blocked patterns**: Sensitive files (`.env`, `*.key`, `*.pem`, `credentials.json`) are blocked by default
- **Size limits**: File writes are limited to 10 MB to prevent resource exhaustion

### Sandbox

- Bash commands run with configurable rlimits (CPU time, file size)
- Output is truncated to prevent memory exhaustion
- Destructive commands always require user confirmation

### Network Policy (SSRF Defense)

All outbound HTTP tools (`http_request`, `web_fetch`, `http_probe`)
validate URLs through `halcon_tools::network_policy::NetworkPolicy`
before issuing the request. Blocked classes:

- Loopback (`127.0.0.0/8`, `::1`)
- RFC 1918 private (`10/8`, `172.16/12`, `192.168/16`)
- Link-local (`169.254.0.0/16`, `fe80::/10`)
- Unique-local IPv6 (`fc00::/7`)
- CGNAT (`100.64.0.0/10`)
- Cloud metadata services (AWS / GCP / Azure IMDS) by FQDN

This protects against prompt-injection attacks that try to drive
the agent into the operator's internal network.

### OAuth Flow Defenses

- PKCE S256 for all authorization-code exchanges
- `state` parameter validated in **constant time** (`subtle::ConstantTimeEq`)
  on every callback to prevent CSRF
- Tokens are persisted to OS keystores (macOS Keychain, Linux
  Secret Service / XDG `0600`, Windows Credential Manager) — never
  to plain disk

### Data Protection

- API keys are stored in the system keychain (macOS Keychain, Linux Secret Service)
- PII detection can warn, block, or redact sensitive information
- Full audit trail of all operations is maintained in local SQLite

## Disclosure Policy

We follow coordinated disclosure. We will:

1. Confirm the vulnerability within 48 hours
2. Provide an estimated timeline for a fix
3. Notify you when the fix is released
4. Credit you in the release notes (unless you prefer to remain anonymous)
