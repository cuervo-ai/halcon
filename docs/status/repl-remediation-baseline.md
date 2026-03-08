# repl/ Remediation Baseline — 2026-03-08

Captured immediately before FASE 1 edits.

| Métrica | Valor |
|---------|-------|
| Archivos flat en repl/ (maxdepth 1, excl. mod.rs) | 151 |
| Total archivos .rs en repl/ (todos los niveles) | 233 |
| LOC totales en repl/ | 121,495 |
| Build warnings+errors | 834 |
| .unwrap() fuera de tests en repl/ | 1,386 |
| std::sync::Mutex en async | 18+ (model_selector.rs principal) |
| Archivos ORPHAN (no en mod.rs) | 7 |
| Tests pasando (workspace) | 4,320 halcon-cli + 4,472 total + 10 otras suites |
| Tests fallando (pre-existentes, halcon-client) | 2 (client_config_ws_url, _https) |

## Suites de test baseline
```
halcon-cli:     4320 passed, 0 failed, 6 ignored
workspace:      4472 passed, 0 failed, 6 ignored
halcon-client:  7 passed, 2 FAILED (pre-existing, unrelated to repl/)
halcon-mcp:     27 passed, 0 failed
halcon-tools:   281 passed, 0 failed
halcon-context: 34 passed, 0 failed
```

## Pre-existing failures (NOT introduced by remediation)
- `halcon-client::client_tests::client_config_ws_url` — pre-existing
- `halcon-client::client_tests::client_config_ws_url_https` — pre-existing
