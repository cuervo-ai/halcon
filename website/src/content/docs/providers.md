---
title: "Provider Configuration"
description: "Configure Anthropic, OpenAI, Gemini, Ollama, DeepSeek, Bedrock, Vertex, Cenzontle, and Claude Code providers"
order: 3
category: "Configuration"
version: "0.3.0"
---

# Provider Configuration

Halcon supports 9 AI providers. Select with `--provider` flag or set default in `~/.halcon/config.toml`.

## Anthropic (default)

Models: `claude-sonnet-4-6`, `claude-opus-4-6`, `claude-haiku-4-5-20251001`

```bash
halcon auth set-key anthropic
# or: export ANTHROPIC_API_KEY="sk-ant-..."
halcon chat --provider anthropic --model claude-opus-4-6
```

## OpenAI

Models: `gpt-4o`, `gpt-4o-mini`, `o1`, `o3-mini`

```bash
export OPENAI_API_KEY="sk-..."
halcon chat --provider openai --model gpt-4o
```

## Gemini

Models: `gemini-2.0-flash`, `gemini-1.5-pro`

```bash
export GEMINI_API_KEY="AIza..."
halcon chat --provider gemini --model gemini-2.0-flash
```

## Ollama (local / air-gap)

No API key required. Requires Ollama running locally.

```bash
ollama pull llama3.2
halcon chat --provider ollama --model llama3.2
```

## DeepSeek

Models: `deepseek-chat`, `deepseek-coder`, `deepseek-reasoner`

```bash
export DEEPSEEK_API_KEY="sk-..."
halcon chat --provider deepseek --model deepseek-reasoner
```

## Cenzontle SSO

Enterprise identity via Zuclubit OAuth 2.1. See [Cenzontle SSO guide](/docs/cenzontle-sso).

```bash
halcon login
halcon chat --provider cenzontle
```

## Default config

```toml
# ~/.halcon/config.toml
[agent]
provider = "anthropic"
model    = "claude-sonnet-4-6"
```

## Model aliases

| Alias | Resolves to |
|-------|-------------|
| `haiku` | `claude-haiku-4-5-20251001` |
| `sonnet` | `claude-sonnet-4-6` |
| `opus` | `claude-opus-4-6` |

## Provider health

```bash
halcon doctor      # full health report
halcon metrics show  # usage baselines and cost
```
