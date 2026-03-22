# Getting Started with Halcon CLI

> Get from zero to your first AI agent session in under 2 minutes.

---

## Prerequisites

- macOS 12+, Linux (glibc 2.17+), or Windows 10+
- At least one of: an Anthropic / OpenAI / Gemini API key, a running Ollama instance, or a Cenzontle SSO account

---

## 1. Install

### macOS / Linux (one-liner)

```bash
curl -fsSL https://halcon.cuervo.cloud/install.sh | sh
```

The installer downloads the appropriate pre-built binary, places it at `~/.local/bin/halcon`, and adds the directory to your `PATH` (in `.bashrc` / `.zshrc`).

### Homebrew (macOS)

```bash
brew tap cuervo-ai/tap
brew install halcon
```

### Windows

```powershell
iwr -useb https://halcon.cuervo.cloud/install.ps1 | iex
```

### From source

```bash
# Requires Rust 1.80+
cargo install --git https://github.com/cuervo-ai/halcon-cli halcon-cli
```

---

## 2. Verify installation

```bash
halcon --version
# halcon 0.3.0 (877118b7 2026-03-14, aarch64-apple-darwin)

halcon doctor
# Should show all providers and health checks
```

---

## 3. Configure a provider

### Option A — Anthropic (recommended)

```bash
halcon auth set-key anthropic
# Prompts for your key, stores in system keychain
```

Or via environment variable:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Option B — OpenAI

```bash
halcon auth set-key openai
# or: export OPENAI_API_KEY="sk-..."
```

### Option C — Ollama (local / air-gap)

```bash
# Start Ollama first (no key needed)
ollama pull llama3.2
halcon chat --provider ollama --model llama3.2
```

### Option D — Cenzontle SSO (enterprise)

```bash
halcon login
# Opens browser for Zuclubit OAuth 2.1 PKCE flow
# Token is stored in system keychain and auto-refreshed
```

---

## 4. First session

```bash
halcon chat
```

You should see the Halcon banner, the active provider/model, and a `>` prompt. Type any task:

```
> List all TODO comments in the current directory

> Write a function to parse CSV files in Python

> What does this codebase do? Give me a 3-sentence summary
```

Type `/help` for slash commands, `/exit` to quit.

---

## 5. Initialize a project

```bash
# In your project root
halcon init
```

This creates `.halcon/` with:

```
.halcon/
  config.toml       # project-level overrides
  agents/           # sub-agent definitions (optional)
  skills/           # skill libraries (optional)
  MEMORY.md         # persistent project memory
```

---

## 6. TUI mode

For a full terminal UI with activity timeline, working memory panel, and conversational overlay:

```bash
halcon --tui
# or press 't' during a regular chat session
```

---

## 7. Create your first sub-agent

Create `.halcon/agents/code-reviewer.md`:

```markdown
---
name: code-reviewer
description: Thorough code reviewer. Call after making changes.
tools: [file_read, grep, glob, git_diff]
model: haiku
max_turns: 15
---

You are an expert code reviewer. Review the provided code for:
1. Security vulnerabilities (injection, auth bypass, data leaks)
2. Performance issues (N+1 queries, unnecessary allocations)
3. Maintainability (magic numbers, unclear naming, dead code)

Always cite file:line references. Be concise.
```

```bash
# Verify it validates
halcon agents validate

# List all registered agents
halcon agents list
```

---

## 8. MCP integration (optional)

Add external tools via MCP:

```bash
# File system access
halcon mcp add filesystem \
  --command "npx @modelcontextprotocol/server-filesystem" \
  --args "/home/user/documents"

# GitHub integration
halcon mcp add github \
  --url https://api.githubcopilot.com/mcp/v1

# Verify
halcon mcp list
```

---

## Next steps

- [Provider configuration](../providers/README.md)
- [Cenzontle SSO setup](../providers/cenzontle.md)
- [Command reference](../api/commands.md)
- [Security & compliance](../security/)
- [Architecture overview](../../README.md#architecture)
