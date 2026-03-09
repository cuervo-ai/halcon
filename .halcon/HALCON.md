# HALCON — cuervo-cli
<!-- Generado por `halcon /init` · 2026-03-07 -->

> **◈ Salud del proyecto: 84/100** — BUENA  
> **◈ Agente listo: 35/100** — LIMITADO  
> **◈ Entorno compatible: 90/100** — ÓPTIMO

## Proyecto
- **Nombre**: `cuervo-cli`
- **Tipo**: Rust Workspace
- **Licencia**: Apache-2.0
- **Dependencias**: 0

## Arquitectura
- **Estilo**: monorepo
- **Crates/Paquetes**: 19
- **Complejidad estimada**: Alta (70/100)

## Infraestructura
- ✓ **CI/CD**: GitHub Actions
- ✓ **Tests**: Detectados (~80% cobertura estimada)
- ✓ **Security Policy**: SECURITY.md presente
- ✓ **Dependency Audit**: Configurado (deny.toml / .snyk)

## Repositorio
- **Rama activa**: `feature/sota-intent-architecture`
- **Remote origin**: https://github.com/cuervo-ai/cuervo-cli.git
- **Último commit**: d4e2b88 — fix(ci): guard ratatui and set_tui_channel behind #[cfg(feature = \"tui\")]
- **Estado**: 55 archivos modificados
- **Total commits**: 52
- **Velocidad**: 12.1 commits/semana
- **Bus factor**: 1 contribuidores (⚐ riesgo)
- **Top contribuidores**: Oscar Valois (52)

## Stack Técnico
- tokio async
- axum web
- reqwest HTTP
- rusqlite / SQLite
- serde
- clap CLI
- ratatui TUI
- crossterm terminal
- tracing observability
- thiserror
- anyhow

## Entorno de Ejecución
- **OS**: macos aarch64
- **CPU**: 10 cores
- **RAM**: 24.0 GB
- **Disco libre**: 24.5 GB
- ✓ **GPU**: Disponible

## Herramientas del Sistema
- **git**: 2.39.5
- **rustc**: 1.90.0
- **cargo**: 1.90.0
- **node**: v24.9.0
- **python**: 3.9.6
- **docker**: 28.5.1,
- **make**: GNU Make 3.81
- **Infra tools**: terraform

## Contexto IDE
- ✓ **LSP / Dev Gateway**: Conectado (puerto 5758)

## Archivos de Contexto AI Detectados
> Este proyecto tiene instrucciones para múltiples asistentes AI.

- `HALCON.md` — Halcon (generated)

## Capacidades del Agente
- **MCP Servers**: 1 activos — filesystem
- ✓ **Plugins**: 7 cargados
- **Subsistemas activos**: Plugins
- **Tools disponibles**: 8 herramientas

## Workspace / Paquetes
```
crates/halcon-agent-core/  # SOTA Goal-Driven Execution Model — redesigned agent core
crates/halcon-api/  # Control plane API types and server for Halcon runtime
crates/halcon-auth/  # Authentication: device flow, keychain, JWT validation
crates/halcon-cli/  # AI-powered CLI for software development
crates/halcon-client/  # Async typed client SDK for the Halcon control plane API
crates/halcon-context/  # Context engine: instruction files, repo map, context assembl
crates/halcon-core/  # Core domain types, traits, and events for Halcon CLI. Zero I
crates/halcon-desktop/  # Native desktop control plane for the Halcon multi-agent runt
crates/halcon-files/  # File intelligence: format detection, text extraction, metada
crates/halcon-integrations/
crates/halcon-mcp/  # MCP runtime: host, stdio transport, tool bridge
crates/halcon-multimodal/  # Multimodal subsystem for HALCÓN CLI: image, audio, video ana
crates/halcon-providers/  # Model provider adapters: Anthropic, Ollama, OpenAI
crates/halcon-runtime/  # Universal multi-agent orchestration runtime for Halcon CLI
crates/halcon-sandbox/  # Sandboxed execution for bash and shell tools — seccomp/names
crates/halcon-search/  # Native search engine: crawling, indexing, and retrieval
crates/halcon-security/  # Cross-cutting security: PII detection, permissions, sanitiza
crates/halcon-storage/  # Persistence layer: SQLite, migrations, audit trail
crates/halcon-tools/  # Tool implementations: file ops, bash, git, search
```

## Estructura
```
cuervo-cli/
├── config/
├── crates/
├── docs/
├── homebrew/
├── img/
├── packaging/
├── paper_downloader/
├── scripts/
├── security/
├── src/
├── tests/
├── website/
└── workers/
```

## Inteligencia de Lenguajes
- **Lenguaje primario**: Rust
- **Lenguajes secundarios**: Shell, TypeScript, JavaScript, PowerShell
- ◈ **Repositorio poliglota**: múltiples lenguajes de producción
- **Distribución**: Rust (727), Shell (25), TypeScript (15), JavaScript (11), PowerShell (3), Python (2), CSS (1), HTML (1)
- **Monorepo**: Cargo workspaces · 20 sub-proyectos
  - Sub-proyectos: crates/halcon-api, crates/halcon-sandbox, crates/halcon-integrations, crates/halcon-search, crates/halcon-files, crates/halcon-multimodal
- **Escala**: Medium (501–5 000 archivos) · 1208 archivos escaneados
- **LOC estimadas**: ~144960 líneas

## Dashboard de Calidad (10 Métricas)
| Métrica | Puntuación | Nivel |
|---|---|---|
| Salud del Proyecto | 84/100 | ◈ Alto |
| Listo para Agente | 35/100 | ⚐ Bajo |
| Compatibilidad Entorno | 90/100 | ◈ Alto |
| Calidad de Arquitectura | 75/100 | ◇ Medio |
| Escalabilidad | 45/100 | ⚐ Bajo |
| Mantenibilidad | 100/100 | ◈ Alto |
| Deuda Técnica | 18/100 | ◈ Bajo |
| Developer Experience | 80/100 | ◈ Alto |
| Preparación IA | 60/100 | ◇ Medio |
| Madurez Distribuida | 25/100 | ⚐ Bajo |

## Matriz de Capacidades
| Capacidad | Detectada | Estado | Riesgo |
|---|---|---|---|
| Tests | ✓ | Cobertura alta | Bajo |
| CI/CD | ✓ | GitHub Actions | Bajo |
| Containers | ✗ | Sin containerización | Medio |
| Security Policy | ✓ | SECURITY.md presente | Bajo |
| Dep Auditing | ✓ | deny.toml / .snyk | Bajo |
| Observability | ✗ | Sin observability | Bajo |
| Message Broker | ✗ | No detectado | Bajo |
| Service Mesh | ✗ | Sin mesh | Bajo |

## Configuración de Agente Sugerida
> **Análisis**: large monorepo with many sub-projects, polyglot repository

```bash
halcon chat --full --expert
```

- **Modelo sugerido**: premium (Opus / GPT-4o — mejor para proyectos complejos)
- **Estrategia de planning**: adaptive
- ◈ **Reasoning profundo**: Recomendado para esta arquitectura

## Riesgos Detectados
- ⚐ Bus factor of 1 — single point of failure

## Recomendaciones
1. Encourage more contributors to reduce knowledge concentration

## Oportunidades de Optimización
1. Activar ReasoningEngine con `halcon chat --full` para tareas complejas
2. Activar Multi-Agent Orchestration con `--full` para paralelismo
3. Integrar con IDE: instalar extensión HALCON para VSCode/Cursor para LSP

## Instrucciones para el Agente

Eres **HALCON**, un asistente de ingeniería autónomo para el proyecto `cuervo-cli`.

### Identidad
- Responde siempre en el idioma del usuario (ES ↔ EN)
- Sé conciso y orientado a la acción — sin relleno
- Usa las convenciones y estilo del proyecto existente

### Flujo de trabajo
- Lee los archivos relevantes ANTES de modificarlos
- Prefiere editar archivos existentes sobre crear nuevos
- Ejecuta las pruebas después de cambios significativos
- Usa `git status`/`git diff` para entender el árbol de trabajo

### Comandos clave
```bash
cargo build --release -p cuervo-cli  # Release
cargo test -p cuervo-cli --lib        # Tests unitarios
cargo clippy --workspace -- -D warnings    # Linting
cargo fmt --all                            # Formateo
cargo deny check                           # Auditoría deps
```

### Convenciones Rust
- Errors: `thiserror` en libs, `anyhow` en binarios
- Tests: inline `#[cfg(test)]` en cada módulo
- No usar `unwrap()` en código de producción
- `#[allow(...)]` solo con justificación explícita

### Prioridades
1. Seguridad y correctitud
2. Rendimiento y eficiencia
3. Legibilidad y mantenibilidad
4. Tests de regresión para cada fix

---
*Generado por `halcon /init` · 2026-03-07*  
*Análisis: 2.8s · 335 archivos · 28 herramientas*
*Detección recursos: 2272ms · Entorno: 2272ms · IDE: 0ms · HICON: 546ms*
