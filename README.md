# Halcon CLI

<div align="center">

**Plataforma de IA Generativa para Desarrollo de Software**

[![Rust](https://img.shields.io/badge/Rust-1.80+-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()
[![Documentation](https://img.shields.io/badge/docs-complete-green.svg)](docs/)

**Unifica modelos propietarios, open source y locales en un solo CLI extensible**

---

### 🚀 Instalación Rápida

<table>
<tr>
<td>

**Linux / macOS**
```bash
curl -fsSL https://raw.githubusercontent.com/cuervo-ai/halcon-cli/main/scripts/install-binary.sh | sh
```

</td>
<td>

**Windows**
```powershell
iwr -useb https://raw.githubusercontent.com/cuervo-ai/halcon-cli/main/scripts/install-binary.ps1 | iex
```

</td>
</tr>
</table>

[📖 Guía de Inicio Rápido](QUICKSTART.md) | [📚 Documentación Completa](INSTALL.md) | [🎯 Ejemplos de Uso](#-uso-rápido)

---

</div>

## 🚀 Visión

Halcon CLI es la primera plataforma de IA para desarrollo que unifica modelos propietarios, open source y locales en un solo CLI extensible, con soporte nativo para self-hosting, fine-tuning integrado, y orquestación multi-agente — diseñada desde cero para equipos enterprise y el mercado latinoamericano.

## ✨ Características Principales

| Característica | Descripción | Estado |
|----------------|-------------|--------|
| **Multi-modelo** | Soporte unificado para Anthropic, OpenAI, Ollama, Gemini, DeepSeek y más | ✅ |
| **Self-hosted** | Ejecución local/on-premise con control total de datos | ✅ |
| **Open Source** | Núcleo completamente abierto y extensible | ✅ |
| **Fine-tuning** | Pipeline integrado para personalización de modelos | 🚧 |
| **Multi-agente** | Orquestación de equipos de agentes especializados | ✅ |
| **Modo Offline** | Funcionalidad completa sin conexión a internet | ✅ |
| **Soporte LATAM** | Interfaz en español/portugués y contexto regional | ✅ |
| **Compliance** | Diseñado para cumplimiento normativo (GDPR, LGPD, etc.) | ✅ |
| **MCP Native** | Integración nativa con Model Context Protocol | ✅ |
| **Memoria Persistente** | Sistema de memoria semántica con búsqueda vectorial | ✅ |
| **Control Plane API** | Servidor HTTP/WebSocket para integración con apps externas | ✅ |
| **Desktop App** | Aplicación nativa egui con chat en tiempo real + streaming WS | ✅ |
| **Claude Code Provider** | Integración con Claude Code CLI como proveedor de modelos | ✅ |
| **Multimodal** | Análisis de imágenes/archivos adjuntos en chat (base64 inline) | ✅ |

## 📦 Instalación

### 🚀 Instalación Rápida (Un Solo Comando)

Instala Halcon CLI en **menos de 10 segundos** con detección automática de tu plataforma:

<table>
<tr>
<td width="50%">

**Linux / macOS**
```bash
curl -fsSL https://raw.githubusercontent.com/cuervo-ai/halcon-cli/main/scripts/install-binary.sh | sh
```

</td>
<td width="50%">

**Windows (PowerShell)**
```powershell
iwr -useb https://raw.githubusercontent.com/cuervo-ai/halcon-cli/main/scripts/install-binary.ps1 | iex
```

</td>
</tr>
</table>

**¿Qué hace el instalador?**
- ✅ **Detecta automáticamente** tu sistema operativo y arquitectura (x86_64, ARM64, etc.)
- ✅ **Descarga el binario** precompilado desde [GitHub Releases](https://github.com/cuervo-ai/halcon-cli/releases/latest)
- ✅ **Verifica integridad** con checksums SHA256
- ✅ **Instala en** `~/.local/bin/halcon` (Unix) o `%USERPROFILE%\.local\bin\halcon.exe` (Windows)
- ✅ **Configura PATH** automáticamente para tu shell (bash, zsh, fish, PowerShell)
- ✅ **Fallback inteligente** a `cargo install` si no hay binario para tu plataforma

### ✅ Verificar Instalación

Después de instalar, verifica que funcione correctamente:

```bash
# Verificar versión
halcon --version
# Salida esperada: halcon 0.3.0 (2e33dd1f, aarch64-apple-darwin)

# Ejecutar diagnósticos
halcon doctor

# Mostrar ayuda
halcon --help
```

Si el comando `halcon` no se encuentra, recarga tu shell:
```bash
# Bash
source ~/.bashrc

# Zsh
source ~/.zshrc

# Fish
source ~/.config/fish/config.fish
```

---

### 📦 Métodos Alternativos de Instalación

<details>
<summary><b>Método 2: Instalación desde Cargo</b> (Compilación desde fuentes, ~2-5 minutos)</summary>

**Requisitos previos:**
- Rust 1.80+ ([instalar rustup](https://rustup.rs/))
- SQLite 3.35+ (generalmente incluido en sistemas modernos)

```bash
# Instalar desde repositorio Git
cargo install --git https://github.com/cuervo-ai/halcon-cli --features tui --locked

# El binario se instalará en ~/.cargo/bin/halcon
```

**Ventajas:**
- Siempre obtienes la última versión
- Compilado específicamente para tu sistema
- Incluye optimizaciones locales

**Desventajas:**
- Requiere tener Rust instalado
- Toma varios minutos compilar
- Requiere espacio en disco para dependencias

</details>

<details>
<summary><b>Método 3: Descarga Manual de Binarios</b></summary>

1. Ve a la página de [Releases](https://github.com/cuervo-ai/halcon-cli/releases/latest)
2. Descarga el archivo para tu plataforma:
   - **Linux x64 (glibc)**: `halcon-x86_64-unknown-linux-gnu.tar.gz`
   - **Linux x64 (musl/Alpine)**: `halcon-x86_64-unknown-linux-musl.tar.gz`
   - **macOS Intel**: `halcon-x86_64-apple-darwin.tar.gz`
   - **macOS Apple Silicon (M1/M2/M3/M4)**: `halcon-aarch64-apple-darwin.tar.gz`
   - **Windows x64**: `halcon-x86_64-pc-windows-msvc.zip`
3. Descarga también el archivo `.sha256` correspondiente
4. Verifica el checksum:
   ```bash
   # Linux/macOS
   sha256sum -c halcon-*.tar.gz.sha256

   # Windows (PowerShell)
   (Get-FileHash halcon-*.zip).Hash -eq (Get-Content halcon-*.zip.sha256).Split()[0]
   ```
5. Extrae el archivo:
   ```bash
   # Linux/macOS
   tar xzf halcon-*.tar.gz

   # Windows
   Expand-Archive halcon-*.zip
   ```
6. Mueve el binario a una ubicación en tu PATH:
   ```bash
   # Linux/macOS
   mv halcon ~/.local/bin/
   chmod +x ~/.local/bin/halcon

   # Windows
   move halcon.exe %USERPROFILE%\.local\bin\
   ```

</details>

<details>
<summary><b>Método 4: Compilación desde Fuentes (Desarrollo)</b></summary>

Para desarrollo activo o contribuciones:

```bash
# Clonar el repositorio
git clone https://github.com/cuervo-ai/halcon-cli.git
cd cuervo-cli

# Compilar en modo debug (más rápido, sin optimizaciones)
cargo build --features tui

# Compilar en modo release (optimizado, más lento)
cargo build --release --features tui

# El binario estará en:
# - Debug: ./target/debug/halcon
# - Release: ./target/release/halcon

# Ejecutar sin instalar
cargo run --features tui -- --help

# Instalar localmente desde el código fuente
cargo install --path crates/halcon-cli --features tui
```

</details>

---

### ⚙️ Configuración Inicial

Después de instalar, configura tus credenciales de API:

```bash
# Método 1: Asistente interactivo (recomendado)
halcon init

# Método 2: Configuración manual por proveedor
halcon auth login anthropic   # Para Claude (Anthropic)
halcon auth login openai      # Para GPT (OpenAI)
halcon auth login deepseek    # Para DeepSeek
halcon auth login ollama      # Para modelos locales (Ollama)

# Verificar configuración
halcon config show
```

**Variables de entorno (alternativa):**
```bash
# Añadir a ~/.bashrc, ~/.zshrc, o equivalente
export ANTHROPIC_API_KEY="sk-ant-..."
export OPENAI_API_KEY="sk-..."
export DEEPSEEK_API_KEY="sk-..."
```

---

### 📚 Documentación Completa

- **[Guía de Instalación Completa](INSTALL.md)** - Troubleshooting, plataformas soportadas, métodos avanzados
- **[Guía de Usuario](docs/USER_GUIDE.md)** - Uso completo del CLI
- **[Guía de Releases](RELEASE.md)** - Para mantenedores y contribuidores

## 🎯 Uso Rápido

### Chat Interactivo (REPL)
```bash
# Iniciar sesión interactiva (modo por defecto)
halcon

# Con prompt inicial
halcon "Ayúdame a escribir una función en Rust"

# Especificar proveedor y modelo
halcon --provider ollama --model llama3.2 "Explica este código"
```

### Comandos Principales
```bash
# Gestión de configuración
halcon config show
halcon config set general.default_model "claude-sonnet-4-6"

# Estado del sistema
halcon status
halcon doctor

# Gestión de sesiones
halcon chat --resume <session-id>
halcon trace export <session-id>

# Memoria semántica
halcon memory search "patrones de diseño"
halcon memory list --type code_snippet

# Inicializar proyecto
halcon init --force
```

### Modo Servidor (Control Plane API)
```bash
# Iniciar servidor HTTP/WebSocket en puerto 9000
halcon serve --port 9000

# Con variables de entorno
export ANTHROPIC_API_KEY="sk-ant-..."
export DEEPSEEK_API_KEY="sk-..."
halcon serve --port 9000

# El servidor expone:
# - REST API:  http://localhost:9000/api/v1/
# - WebSocket: ws://localhost:9000/api/v1/ws
# - Auth token en variable HALCON_API_TOKEN

# Ejemplo: crear sesión de chat via API
curl -X POST http://localhost:9000/api/v1/chat/sessions \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"model":"claude-haiku-4-5-20251001","provider":"anthropic"}'
```

### Comandos REPL (dentro de sesión interactiva)
```
/help                    # Mostrar ayuda categorizada
/model                   # Mostrar modelo actual
/cost                    # Desglose de costos de sesión
/session list            # Listar sesiones recientes
/memory search <query>   # Buscar en memoria
/doctor                  # Ejecutar diagnósticos
/quit                    # Guardar y salir
```

## 🏗️ Arquitectura

### Estructura del Workspace (20 crates)
```
halcon-cli/
├── crates/
│   ├── halcon-cli/          # Binary: REPL, TUI, commands, rendering
│   ├── halcon-core/         # Domain: types, traits, events (zero I/O)
│   ├── halcon-providers/    # Model adapters: Anthropic, OpenAI, DeepSeek, Gemini, Ollama
│   ├── halcon-tools/        # 21+ tool implementations: file ops, bash, git, search
│   ├── halcon-auth/         # Auth: device flow, keychain, JWT, OAuth PKCE
│   ├── halcon-storage/      # Persistence: SQLite, migrations, audit, cache, metrics
│   ├── halcon-security/     # Cross-cutting: PII detection, permissions, sanitizer
│   ├── halcon-context/      # Context engine v2: L0-L4 tiers, pipeline, elider
│   ├── halcon-mcp/          # MCP runtime: host, server, stdio transport
│   ├── halcon-files/        # File intelligence: 12 format handlers
│   ├── halcon-runtime/      # Multi-agent runtime: registry, federation, executor
│   ├── halcon-api/          # Shared API types + axum server
│   ├── halcon-client/       # Async typed SDK (HTTP + WebSocket)
│   ├── halcon-agent-core/   # 10-layer GDEM architecture (127 tests)
│   ├── halcon-sandbox/      # macOS/Linux sandboxing (policy + executor)
│   └── halcon-desktop/      # egui native desktop app
├── docs/                    # Documentation
├── config/                  # Default configurations
└── scripts/                 # Build and test scripts
```

### Proveedores Soportados
| Proveedor | Modelos | Local | Cloud | API Key |
|-----------|---------|-------|-------|---------|
| **Anthropic** | Claude Sonnet 4.6, Haiku 4.5, Opus 4.6 | ❌ | ✅ | ✅ |
| **Ollama** | Llama, Mistral, CodeLlama, etc. | ✅ | ❌ | ❌ |
| **OpenAI** | GPT-4o, GPT-4 Turbo | ❌ | ✅ | ✅ |
| **Gemini** | Gemini Pro, Flash | ❌ | ✅ | ✅ |
| **DeepSeek** | DeepSeek Coder, Chat, Reasoner | ❌ | ✅ | ✅ |
| **Claude Code** | Subprocess via Claude CLI (NDJSON) | ✅ | ✅ | ❌ |
| **OpenAI Compat** | Compatible con APIs OpenAI | ✅/❌ | ✅/❌ | Opcional |
| **Echo** | Debug/testing | ✅ | ❌ | ❌ |
| **Replay** | Reproducción de trazas | ✅ | ❌ | ❌ |

### Herramientas Disponibles (23 herramientas nativas + 33 de plugins)
| Herramienta | Descripción | Permisos |
|-------------|-------------|----------|
| `file_read` | Lectura de archivos | ReadOnly |
| `file_write` | Escritura atómica de archivos | Destructive |
| `file_edit` | Edición atómica de archivos | Destructive |
| `file_delete` | Eliminación de archivos | Destructive |
| `file_inspect` | Inspección de formatos de archivo | ReadOnly |
| `directory_tree` | Exploración de directorios | ReadOnly |
| `grep` | Búsqueda en contenido | ReadOnly |
| `glob` | Búsqueda por patrones | ReadOnly |
| `fuzzy_find` | Búsqueda difusa de archivos | ReadOnly |
| `symbol_search` | Búsqueda de símbolos en código | ReadOnly |
| `bash` | Ejecución de comandos shell | Destructive |
| `git_status` | Estado de repositorio Git | ReadOnly |
| `git_diff` | Diferencias Git | ReadOnly |
| `git_log` | Historial de commits | ReadOnly |
| `git_add` | Staging de archivos | ReadWrite |
| `git_commit` | Creación de commits | Destructive |
| `web_fetch` | HTTP GET/fetch | ReadOnly |
| `web_search` | Búsqueda web (Brave API) | ReadOnly |
| `http_request` | HTTP POST/PUT/DELETE/PATCH | Destructive |
| `task_track` | Seguimiento de tareas | ReadWrite |
| `background_start` | Procesos en segundo plano | Destructive |
| `background_output` | Salida de procesos | ReadOnly |
| `background_kill` | Terminar procesos | Destructive |

### 🔌 Ecosistema de Plugins (7 plugins — 33 herramientas adicionales)

Los plugins extienden HALCON con herramientas especializadas sin modificar el core. Se instalan en `~/.halcon/plugins/` y se registran automáticamente en cada sesión.

| Plugin | Categoría | Herramientas | Descripción |
|--------|-----------|-------------|-------------|
| `halcon-dev-sentinel` | Seguridad | 4 | Análisis de seguridad: secretos, dependencias, SAST, OWASP |
| `halcon-dependency-auditor` | Seguridad | 4 | Auditoría de dependencias Rust/Node.js, licencias, CVE |
| `halcon-ui-inspector` | Frontend | 5 | Inspección de componentes UI, accesibilidad, rendimiento |
| `halcon-perf-analyzer` | Frontend | 5 | Análisis de bundles, lazy loading, recursos bloqueantes, imágenes |
| `halcon-api-sculptor` | Backend | 5 | Análisis de APIs REST, contratos OpenAPI, seguridad de endpoints |
| `halcon-schema-oracle` | Backend | 5 | Análisis de esquemas DB, migraciones, índices, patrones SQL |
| `halcon-otel-tracer` | Arquitectura | 5 | Cobertura de trazado, inventario de métricas, logging estructurado |

**Instalar plugins de ejemplo:**
```bash
# Los plugins se activan copiando el .plugin.toml a ~/.halcon/plugins/
ls ~/.halcon/plugins/*.plugin.toml

# Usar herramientas de plugin directamente (el LLM las invoca automáticamente)
halcon chat "analiza la observabilidad de este proyecto"
# → invoca plugin_halcon_otel_tracer_observability_health_report

halcon chat "revisa el schema de la base de datos"
# → invoca plugin_halcon_schema_oracle_schema_health_report
```

**Formato de manifest de plugin (`~/.halcon/plugins/<id>.plugin.toml`):**
```toml
[meta]
id       = "mi-plugin"
name     = "Mi Plugin"
version  = "1.0.0"
category = "backend"

[meta.transport]
type    = "stdio"
command = "/path/to/plugin.py"
args    = []

[[capabilities]]
name                   = "plugin_mi_plugin_mi_herramienta"
description            = "Descripción de la herramienta para el LLM"
risk_tier              = "low"
idempotent             = true
permission_level       = "read_only"
budget_tokens_per_call = 600

[supervisor_policy]
halt_on_failures           = 3
reward_weight              = 1.0
requires_explicit_approval = false
```

## 🔧 Configuración

### Archivos de Configuración
Halcon CLI utiliza configuración jerárquica:
1. **Comandos CLI** (--model, --provider)
2. **Variables de entorno** (HALCON_MODEL, HALCON_PROVIDER)
3. **Config local** (`./.halcon/config.toml`)
4. **Config global** (`~/.halcon/config.toml`)
5. **Config por defecto** (`config/default.toml`)

### Ejemplo de Configuración
```toml
# ~/.halcon/config.toml
[general]
default_provider = "anthropic"
default_model = "claude-sonnet-4-5-20250929"
max_tokens = 8192
temperature = 0.0

[models.providers.ollama]
enabled = true
api_base = "http://localhost:11434"
default_model = "llama3.2"

[tools]
confirm_destructive = true
timeout_secs = 120
allowed_directories = ["/home/user/projects"]

[security]
pii_detection = true
pii_action = "warn"
audit_enabled = true
```

### Variables de Entorno
```bash
export CUERVO_MODEL="claude-sonnet-4-5-20250929"
export CUERVO_PROVIDER="anthropic"
export CUERVO_LOG="debug"
export ANTHROPIC_API_KEY="sk-ant-..."
```

## 🛡️ Seguridad

### Características de Seguridad
- **Detección de PII**: Identificación automática de información personal
- **Auditoría**: Registro completo de todas las operaciones
- **Aislamiento**: Sandboxing de herramientas potencialmente peligrosas
- **Cifrado**: Almacenamiento seguro de claves API en keychain del sistema
- **Control de acceso**: Permisos granulares por herramienta y directorio

### Configuración de Seguridad
```toml
[security]
pii_detection = true
pii_action = "block"  # warn, block, or redact
audit_enabled = true
audit_retention_days = 90

[tools]
confirm_destructive = true
allowed_directories = ["/safe/path"]
blocked_patterns = [
    "**/.env",
    "**/.env.*",
    "**/credentials.json",
    "**/*.pem",
    "**/*.key",
]
```

## 📚 Sistema de Memoria

### Tipos de Memoria
```rust
enum MemoryEntryType {
    Fact,           // Hechos aprendidos
    SessionSummary, // Resúmenes de sesiones
    Decision,       // Decisiones tomadas
    CodeSnippet,    // Fragmentos de código
    ProjectMeta,    // Metadatos de proyecto
}
```

### Comandos de Memoria
```bash
# Búsqueda semántica
halcon memory search "patrón singleton en Rust"

# Listado filtrado
halcon memory list --type code_snippet --limit 20

# Estadísticas
halcon memory stats

# Mantenimiento
halcon memory prune --force
```

## 🔄 Integración MCP (Model Context Protocol)

Halcon CLI incluye soporte nativo para MCP, permitiendo:

```bash
# Iniciar servidor MCP para integración con IDEs
halcon mcp-server --working-dir ./project

# Los clientes MCP pueden conectarse via stdio
# para acceder a herramientas y contexto
```

### Características MCP
- **Transporte stdio**: Comunicación bidireccional
- **Pool de conexiones**: Múltiples clientes simultáneos
- **Bridge unificado**: Integración con herramientas existentes
- **Contexto compartido**: Memoria y estado disponibles para clientes

## 🧪 Testing y Calidad

### Suite de Tests
```bash
# Tests unitarios
cargo test

# Tests de integración
cargo test --test cli_e2e

# Tests de proveedores (requiere configuración)
./scripts/test_providers.sh

# Tests interactivos
python tests/interactive/run_pty_tests.py
```

### Métricas de Calidad
- **Cobertura de código**: >85% (objetivo)
- **Tests E2E**: Comandos CLI principales
- **Validación de proveedores**: Tests de integración reales
- **Pruebas de seguridad**: Auditoría de herramientas
- **Benchmarks**: Rendimiento y latencia

## 📊 Roadmap

### Fase Actual (Q1 2026) — COMPLETO
- [x] CLI básico con REPL interactivo
- [x] Soporte multi-proveedor (Anthropic, Ollama, OpenAI, DeepSeek, Claude Code)
- [x] Sistema de herramientas nativas (61 herramientas: 23 core + 33 plugins + 5 meta)
- [x] Almacenamiento persistente con SQLite
- [x] Sistema de memoria semántica
- [x] Integración MCP nativa
- [x] **Sistema de plugins V3** — 7 plugins, 33 herramientas especializadas
- [x] **SOTA meta-cognición** — UCB1, ReasoningEngine, LoopCritic, RoundScorer
- [x] **TUI multi-panel** con tema fire/amber adaptativo
- [x] **Multimodal** — análisis de imágenes inline (base64) con MIME detection
- [x] **Control Plane API** — servidor HTTP/WS con sesiones de chat persistentes
- [x] **Desktop App** — interfaz egui nativa con streaming en tiempo real
- [x] **Claude Code Provider** — subprocess NDJSON, modo auto/chat, root detection
- [x] **GDEM Architecture** — 10-layer Goal-Driven Execution Machine (127 tests)
- [x] **Synthesis Hardening** — 5 vulnerabilidades V1-V5 corregidas, XML artifact stripping
- [x] **UTF-8 Safety** — truncación segura por char boundary en segmentos de contexto
- [x] **Sub-agent Orchestration** — orphan modal fix, clean pill labels, spinner sync

### Próximas Fases
- [ ] Fine-tuning integrado (Q2 2026)
- [ ] Plugin sandbox WASM (Extism) para plugins de terceros (Q2 2026)
- [ ] Orquestación multi-agente avanzada (Q3 2026)
- [ ] Marketplace de extensiones (Q4 2026)
- [ ] Halcon Cloud (auto-hosting gestionado) (2027)
- [ ] SDK para desarrolladores (2027)

## 🤝 Contribuir

### Guía de Contribución
1. **Fork** el repositorio
2. **Crea una rama** (`git checkout -b feature/amazing-feature`)
3. **Commit cambios** (`git commit -m 'Add amazing feature'`)
4. **Push a la rama** (`git push origin feature/amazing-feature`)
5. **Abre un Pull Request**

### Estándares de Código
- **Rustfmt**: Formateo automático de código
- **Clippy**: Linting estático
- **Tests**: Nuevas funcionalidades requieren tests
- **Documentación**: Comentarios y docs actualizados

### Estructura de Commits
```
feat: nueva funcionalidad
fix: corrección de bug
docs: documentación
style: formato (sin cambios funcionales)
refactor: refactorización de código
test: tests
chore: mantenimiento
```

## 📄 Licencia

Este proyecto está licenciado bajo la **Apache License 2.0** - ver el archivo [LICENSE](LICENSE) para más detalles.

## 🌐 Recursos

- **Documentación Completa**: [docs/](docs/)
- **Reporte de Investigación**: [docs/01-research/](docs/01-research/)
- **Arquitectura Enterprise**: [docs/08-enterprise-design/](docs/08-enterprise-design/)
- **Sistema de Conocimiento**: [docs/09-knowledge-system/](docs/09-knowledge-system/)
- **Especificaciones UX**: [docs/ux/](docs/ux/)

## 🆘 Soporte

- **Issues**: [GitHub Issues](https://github.com/cuervo-ai/halcon-cli/issues)
- **Discusiones**: [GitHub Discussions](https://github.com/cuervo-ai/halcon-cli/discussions)
- **Documentación**: [docs/](docs/)

---

<div align="center">

**Halcon CLI** - Plataforma de IA Generativa para Desarrollo de Software

*"Unificando el futuro del desarrollo asistido por IA"*

</div>
