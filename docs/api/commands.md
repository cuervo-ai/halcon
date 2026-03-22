---
title: "Halcon CLI Command Reference"
version: "0.3.0"
generated: "2026-03-14"
format: "machine-readable YAML frontmatter + markdown body"
---

# Command Reference — Halcon CLI v0.3.0

> Machine-readable command reference. Each command section includes YAML metadata for tooling integration.

---

## `halcon chat`

```yaml
command: chat
description: "Start an interactive chat / REPL session"
aliases: []
default: true
flags:
  - name: --provider / -p
    type: string
    env: HALCON_PROVIDER
    description: "Provider to use (anthropic, openai, ollama, gemini, deepseek, bedrock, vertex, cenzontle, claude_code)"
  - name: --model / -m
    type: string
    env: HALCON_MODEL
    description: "Model identifier or alias (haiku, sonnet, opus, or full name)"
  - name: --no-banner
    type: bool
    description: "Suppress startup banner"
  - name: --agents
    type: path[]
    description: "Additional agent definition files to load (session scope)"
  - name: --yes
    type: bool
    description: "Auto-approve all destructive tool calls"
```

```bash
halcon chat
halcon chat --provider anthropic --model claude-opus-4-6
halcon chat --no-banner
halcon chat --agents .halcon/agents/reviewer.md
```

---

## `halcon status`

```yaml
command: status
description: "Show current status: provider, model, session info, configured providers"
aliases: []
flags: []
output:
  - "Provider and model currently active"
  - "List of all configured providers with key/auth status"
  - "Security settings (PII detection, audit trail)"
```

```bash
halcon status
```

---

## `halcon doctor`

```yaml
command: doctor
description: "Run full runtime health diagnostics"
aliases: []
flags: []
output:
  - "Configuration validation"
  - "Provider health with success rate, latency, call count"
  - "Health scores per provider"
  - "Cache stats"
  - "Metrics summary (total invocations, cost, tokens)"
  - "Tool call metrics"
  - "Orchestrator status"
  - "Replay and checkpoint data"
  - "Model rankings (balanced score)"
```

```bash
halcon doctor
```

---

## `halcon auth`

```yaml
command: auth
description: "Manage API keys and authentication"
subcommands:
  status:
    description: "Show status of all configured API keys"
  set-key:
    description: "Set an API key for a provider (stored in system keychain)"
    args: [provider]
  remove-key:
    description: "Remove an API key"
    args: [provider]
  sso-login:
    description: "Initiate SSO login for a provider"
    args: [provider]
  logout:
    description: "Log out and revoke tokens"
    args: [provider]
```

```bash
halcon auth status
halcon auth set-key anthropic
halcon auth remove-key openai
halcon auth sso-login cenzontle
halcon auth logout cenzontle
```

---

## `halcon login`

```yaml
command: login
description: "Shortcut for: halcon auth sso-login cenzontle"
aliases: []
flags: []
```

```bash
halcon login
```

---

## `halcon agents`

```yaml
command: agents
description: "Manage declarative sub-agent configurations"
subcommands:
  list:
    description: "List all registered agents (session + project + user scopes)"
    flags:
      - name: --verbose
        type: bool
        description: "Show full agent details"
  validate:
    description: "Validate agent configuration files"
    args: ["[paths...]  (default: all scopes)"]
```

```bash
halcon agents list
halcon agents list --verbose
halcon agents validate
halcon agents validate .halcon/agents/reviewer.md
```

---

## `halcon tools`

```yaml
command: tools
description: "Tool diagnostics and management"
subcommands:
  list:
    description: "List all available tools with permission tier"
    flags:
      - name: --filter
        type: string
        description: "Filter by name substring"
  health:
    description: "Run tool connectivity checks"
```

```bash
halcon tools list
halcon tools list --filter git
halcon tools health
```

---

## `halcon mcp`

```yaml
command: mcp
description: "Manage MCP (Model Context Protocol) server connections"
subcommands:
  list:
    description: "List configured MCP servers"
  add:
    description: "Add an MCP server"
    args: [name]
    flags:
      - name: --url
        type: string
        description: "HTTP URL for HTTP transport"
      - name: --command
        type: string
        description: "Command for stdio transport"
      - name: --args
        type: string[]
        description: "Arguments for stdio command"
  remove:
    description: "Remove an MCP server"
    args: [name]
  get:
    description: "Show details for an MCP server"
    args: [name]
  auth:
    description: "Authenticate with an MCP server (OAuth)"
    args: [name]
  serve:
    description: "Start Halcon as an MCP server"
    flags:
      - name: --transport
        type: enum[stdio, http]
        default: stdio
      - name: --port
        type: int
        default: 7777
```

```bash
halcon mcp list
halcon mcp add filesystem --url https://mcp.example.com
halcon mcp add local --command "npx @modelcontextprotocol/server-filesystem" --args "/path"
halcon mcp remove filesystem
halcon mcp auth filesystem
halcon mcp serve
halcon mcp serve --transport http --port 7777
```

---

## `halcon audit`

```yaml
command: audit
description: "Compliance and audit export (SOC 2)"
subcommands:
  list:
    description: "List audit sessions"
  export:
    description: "Export audit trail for a session"
    flags:
      - name: --session
        type: string
        description: "Session ID (default: most recent)"
      - name: --format
        type: enum[jsonl, csv, pdf]
        default: jsonl
      - name: --output
        type: path
        description: "Output file path"
  verify:
    description: "Verify HMAC chain integrity for a session"
    args: [session_id]
    exit_codes:
      0: "Chain intact"
      1: "Tampered row detected"
```

```bash
halcon audit list
halcon audit export --format jsonl
halcon audit export --session abc123 --format pdf --output report.pdf
halcon audit verify abc123
```

---

## `halcon memory`

```yaml
command: memory
description: "Manage persistent semantic memory"
subcommands:
  list:
    description: "List memory entries"
  add:
    description: "Add a memory entry"
    args: [content]
  remove:
    description: "Remove a memory entry"
    args: [id]
  search:
    description: "Semantic search over memory"
    args: [query]
```

```bash
halcon memory list
halcon memory add "prefer async/await over callbacks in JS"
halcon memory search "TypeScript preferences"
```

---

## `halcon metrics`

```yaml
command: metrics
description: "Metrics and baseline analysis"
subcommands:
  show:
    description: "Show metrics baseline report"
    flags:
      - name: --output-format
        type: enum[human, json, junit, plain]
        env: HALCON_OUTPUT_FORMAT
        default: human
  export:
    description: "Export baselines to JSON file"
  prune:
    description: "Remove old baseline data"
  decide:
    description: "Generate integration decision based on baselines"
```

```bash
halcon metrics show
halcon metrics show --output-format json
halcon metrics export
halcon metrics decide
```

---

## `halcon schedule`

```yaml
command: schedule
description: "Manage cron-based scheduled agent tasks"
subcommands:
  list:
    description: "List scheduled tasks"
  add:
    description: "Add a scheduled task"
    flags:
      - name: --cron
        type: string
        description: "Cron expression (e.g., '0 9 * * 1-5')"
      - name: --agent
        type: string
        description: "Agent name to run"
      - name: --task
        type: string
        description: "Task description"
  remove:
    description: "Remove a scheduled task"
    args: [id]
```

```bash
halcon schedule list
halcon schedule add --cron "0 9 * * 1-5" --agent daily-standup --task "Summarize yesterday's commits"
halcon schedule remove task-id
```

---

## `halcon serve`

```yaml
command: serve
description: "Start the Halcon control plane API server (REST + WebSocket)"
flags:
  - name: --port
    type: int
    default: 9000
  - name: --host
    type: string
    default: "127.0.0.1"
  - name: --token
    type: string
    env: HALCON_API_TOKEN
    description: "Bearer token for API authentication"
```

```bash
halcon serve
halcon serve --port 9000 --host 0.0.0.0
HALCON_API_TOKEN=secret halcon serve
```

---

## `halcon users`

```yaml
command: users
description: "Manage user accounts and role assignments (RBAC)"
subcommands:
  list:
    description: "List all users"
  add:
    description: "Add a user"
    args: [email]
    flags:
      - name: --role
        type: enum[viewer, editor, admin, operator]
  remove:
    description: "Remove a user"
    args: [email]
```

```bash
halcon users list
halcon users add alice@company.com --role editor
halcon users remove alice@company.com
```

---

## `halcon trace`

```yaml
command: trace
description: "Export or inspect a session trace"
subcommands:
  export:
    description: "Export trace to file"
    args: [session_id]
    flags:
      - name: --format
        type: enum[json, jsonl]
  show:
    description: "Display trace summary"
    args: [session_id]
```

---

## `halcon update`

```yaml
command: update
description: "Update Halcon CLI to the latest version"
flags:
  - name: --check
    type: bool
    description: "Check for updates without installing"
  - name: --channel
    type: enum[stable, beta]
    default: stable
```

```bash
halcon update
halcon update --check
halcon update --channel beta
```

---

## Global flags

These flags apply to all commands:

```yaml
global_flags:
  - name: --model / -m
    type: string
    env: HALCON_MODEL
  - name: --provider / -p
    type: string
    env: HALCON_PROVIDER
  - name: --verbose / -v
    type: bool
    description: "Set log level to debug"
  - name: --log-level
    type: enum[trace, debug, info, warn, error]
    env: HALCON_LOG
    default: warn
  - name: --trace-json
    type: bool
    description: "Emit traces as JSON lines to stderr"
  - name: --config
    type: path
    env: HALCON_CONFIG
    description: "Configuration file path"
  - name: --no-banner
    type: bool
    description: "Suppress startup banner"
```
