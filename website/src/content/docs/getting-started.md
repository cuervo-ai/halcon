---
title: "Getting Started with Halcon CLI"
description: "Install and run your first AI agent session in under 2 minutes"
order: 1
category: "Getting Started"
version: "0.3.0"
---

# Getting Started with Halcon CLI

Get from zero to your first AI agent session in under 2 minutes.

## Prerequisites

- macOS 12+, Linux (glibc 2.17+), or Windows 10+
- At least one API key: Anthropic, OpenAI, Gemini, or DeepSeek — OR — a running Ollama instance for local/air-gap use, OR a Cenzontle SSO account

## Step 1: Install

**macOS / Linux (recommended)**

```bash
curl -fsSL https://halcon.cuervo.cloud/install.sh | sh
```

**Homebrew**

```bash
brew tap cuervo-ai/tap && brew install halcon
```

**Windows**

```powershell
iwr -useb https://halcon.cuervo.cloud/install.ps1 | iex
```

## Step 2: Verify

```bash
halcon --version
# halcon 0.3.0 (877118b7 2026-03-14, aarch64-apple-darwin)
```

## Step 3: Configure a provider

```bash
# Anthropic (recommended)
halcon auth set-key anthropic

# Or: Cenzontle SSO (enterprise)
halcon login
```

## Step 4: Run diagnostics

```bash
halcon doctor
```

## Step 5: Start your first session

```bash
halcon chat
```

Type any development task at the `>` prompt. Use `/help` for slash commands.

## Next steps

- [Provider configuration](/docs/providers)
- [Cenzontle SSO](/docs/cenzontle-sso)
- [Command reference](/docs/commands)
