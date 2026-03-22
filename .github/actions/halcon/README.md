# Halcon AI Agent — GitHub Action

Run the Halcon autonomous coding agent as a step in your GitHub Actions workflow.

## Quick start

```yaml
- uses: cuervo-ai/halcon-cli/.github/actions/halcon@main
  with:
    prompt: "Fix any failing tests and open a PR"
  env:
    ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
```

## Inputs

| Input | Required | Default | Description |
|-------|----------|---------|-------------|
| `prompt` | yes | — | Task for the agent to execute |
| `model` | no | `claude-sonnet-4-6` | Model ID |
| `max-turns` | no | `20` | Maximum agent loop turns |
| `output-format` | no | `json` | Output format (`json`, `human`, `plain`) |
| `working-directory` | no | `.` | Working directory for the agent |

## Outputs

| Output | Description |
|--------|-------------|
| `result` | Agent's final text response |
| `session-id` | Session ID for audit trail |
| `cost-usd` | Estimated API cost in USD |

## Provider selection

The action auto-selects a provider based on environment variables:

```yaml
# Direct Anthropic API (default)
env:
  ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}

# AWS Bedrock
env:
  CLAUDE_CODE_USE_BEDROCK: "1"
  AWS_ACCESS_KEY_ID: ${{ secrets.AWS_ACCESS_KEY_ID }}
  AWS_SECRET_ACCESS_KEY: ${{ secrets.AWS_SECRET_ACCESS_KEY }}
  AWS_REGION: us-east-1

# Google Vertex AI
env:
  CLAUDE_CODE_USE_VERTEX: "1"
  ANTHROPIC_VERTEX_PROJECT_ID: my-gcp-project
  GOOGLE_APPLICATION_CREDENTIALS: /path/to/sa.json

# Azure AI Foundry
env:
  CLAUDE_CODE_USE_AZURE: "1"
  AZURE_AI_ENDPOINT: https://myresource.services.ai.azure.com
  AZURE_API_KEY: ${{ secrets.AZURE_API_KEY }}
```

## Full example with output capture

```yaml
jobs:
  ai-review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Run Halcon agent
        id: halcon
        uses: cuervo-ai/halcon-cli/.github/actions/halcon@main
        with:
          prompt: "Review the diff in this PR and report any security issues"
          max-turns: "10"
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}

      - name: Post review comment
        uses: actions/github-script@v7
        with:
          script: |
            github.rest.issues.createComment({
              issue_number: context.issue.number,
              owner: context.repo.owner,
              repo: context.repo.repo,
              body: `**Halcon AI Review** (session: ${{ steps.halcon.outputs.session-id }})\n\n${{ steps.halcon.outputs.result }}`
            })
```

## NDJSON output

When `output-format: json` (the default), every agent event is emitted as a
JSON object on its own line.  You can process the output with `jq`:

```bash
# Show all tool calls
grep '"type":"tool_call"' /tmp/halcon-run.jsonl | jq .

# Get total cost
grep '"type":"session_end"' /tmp/halcon-run.jsonl | jq '.cost_usd'
```
