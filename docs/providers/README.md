# Provider Reference

Halcon supports 9 AI providers. Each provider is independently configured and can be selected per-session (`--provider`), per-command (`HALCON_PROVIDER`), or set as default in `~/.halcon/config.toml`.

---

## Provider Comparison Matrix

| Provider | Best for | Max context | Tool use | Streaming | Cost tier |
|----------|---------|-------------|----------|-----------|-----------|
| **Anthropic** | Code, reasoning, long context | 200K tokens | Full | Yes | Medium |
| **OpenAI** | General, function calling | 128K tokens | Full | Yes | Medium |
| **Gemini** | Multimodal, search-grounded | 1M tokens | Full | Yes | Low |
| **Ollama** | Air-gap, privacy, local | Model-dependent | Full | Yes | Free |
| **DeepSeek** | Code generation, math | 64K tokens | Full | Yes | Very low |
| **AWS Bedrock** | Enterprise, compliance | Model-dependent | Full | Yes | Pay-per-use |
| **Vertex AI** | GCP workloads | Model-dependent | Full | Yes | Pay-per-use |
| **Claude Code** | IDE-native, autonomous | Model-dependent | Full | Yes | Subprocess |
| **Cenzontle** | Enterprise SSO | Same as Anthropic | Full | Yes | Enterprise |

---

## Anthropic

**Models**: `claude-sonnet-4-6` (default), `claude-opus-4-6`, `claude-haiku-4-5-20251001`

```bash
# Set key
halcon auth set-key anthropic
export ANTHROPIC_API_KEY="sk-ant-..."

# Use
halcon chat --provider anthropic --model claude-opus-4-6
```

Config:

```toml
[agent]
provider = "anthropic"
model    = "claude-sonnet-4-6"
```

---

## OpenAI

**Models**: `gpt-4o`, `gpt-4o-mini`, `o1`, `o3-mini`

```bash
halcon auth set-key openai
export OPENAI_API_KEY="sk-..."

halcon chat --provider openai --model gpt-4o
```

---

## Gemini

**Models**: `gemini-2.0-flash`, `gemini-1.5-pro`, `gemini-pro`

```bash
halcon auth set-key gemini
export GEMINI_API_KEY="AIza..."

halcon chat --provider gemini --model gemini-2.0-flash
```

---

## Ollama (local / air-gap)

No API key required. Requires [Ollama](https://ollama.com/) running locally.

```bash
# Pull a model
ollama pull llama3.2
ollama pull deepseek-coder-v2

# Use with Halcon
halcon chat --provider ollama --model llama3.2
```

Config:

```toml
[providers.ollama]
base_url = "http://localhost:11434"  # default
```

**Air-gap deployment**: No internet access needed. Works entirely offline.

---

## DeepSeek

**Models**: `deepseek-chat`, `deepseek-coder`, `deepseek-reasoner`

```bash
export DEEPSEEK_API_KEY="sk-..."
halcon chat --provider deepseek --model deepseek-reasoner
```

DeepSeek Reasoner includes extended chain-of-thought that Halcon can display in the TUI thinking panel.

---

## AWS Bedrock

Requires an AWS account with Bedrock access enabled for the relevant models.

```toml
[providers.bedrock]
region     = "us-east-1"
# Uses default AWS credential chain (env vars, ~/.aws/credentials, IAM role)
```

```bash
# With IAM role (recommended for production)
halcon chat --provider bedrock --model anthropic.claude-3-5-sonnet-20241022-v2:0

# With explicit credentials
AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... halcon chat --provider bedrock
```

---

## Vertex AI

Requires a GCP project with Vertex AI enabled.

```toml
[providers.vertex]
project_id = "my-gcp-project"
region     = "us-central1"
# Uses application default credentials (gcloud auth application-default login)
```

---

## Claude Code (subprocess)

Routes through the `claude` CLI process using NDJSON streaming. No additional API key needed if `claude` is already authenticated.

```bash
# Check claude is available
which claude

# Use Claude Code provider
halcon chat --provider claude_code --model claude-opus-4-6
```

Notes:
- Detects when running as root: automatically downgrades to Chat mode (blocks `--dangerously-skip-permissions`)
- Guards against nested session conflicts
- Suitable for IDE-native deployments

---

## Cenzontle SSO

See the full [Cenzontle SSO guide](cenzontle.md).

```bash
halcon login
halcon chat --provider cenzontle
```

---

## Model aliases

When specifying a model, these short aliases are supported:

| Alias | Resolves to |
|-------|-------------|
| `haiku` | `claude-haiku-4-5-20251001` |
| `sonnet` | `claude-sonnet-4-6` |
| `opus` | `claude-opus-4-6` |

---

## Provider health

Halcon tracks provider health (success rate, latency, cost) across all sessions and uses it for model selection:

```bash
halcon doctor         # full health report
halcon metrics show   # usage baselines
```

The Balanced score used for auto-selection:

```
score = success_rate × (1 / latency_weight) × (1 / cost_weight)
```

Circuit breakers automatically exclude degraded providers until health recovers.
