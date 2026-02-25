# 🚀 Métodos de Instalación de Halcon CLI

Guía de referencia rápida de todos los métodos de instalación disponibles.

---

## 📦 Método 1: Script de Instalación (Recomendado)

**⏱️ Tiempo: ~10 segundos**

### Linux / macOS

```bash
curl -sSfL https://cli.cuervo.cloud/install.sh | sh
```

**Lo que hace:**
- ✅ Detecta automáticamente OS, arquitectura y libc
- ✅ Descarga el binario correcto desde GitHub Releases
- ✅ Verifica checksum SHA256
- ✅ Instala en `~/.local/bin/halcon`
- ✅ Configura PATH automáticamente

### Windows (PowerShell)

```powershell
iwr -useb https://cli.cuervo.cloud/install.ps1 | iex
```

**Lo que hace:**
- ✅ Detecta arquitectura (x64/ARM64)
- ✅ Descarga el ZIP correcto
- ✅ Verifica checksum SHA256
- ✅ Instala en `%LOCALAPPDATA%\halcon\bin\halcon.exe`
- ✅ Configura PATH en variables de entorno de usuario

### Instalar versión específica

```bash
# Unix — versión específica
curl -sSfL https://cli.cuervo.cloud/install.sh | sh -s -- --version v0.3.0

# Windows — versión específica
& ([scriptblock]::Create((iwr -useb https://cli.cuervo.cloud/install.ps1))) -Version v0.3.0
```

### Personalizar directorio de instalación

```bash
# Unix
export HALCON_INSTALL_DIR="$HOME/bin"
curl -sSfL https://cli.cuervo.cloud/install.sh | sh

# Windows
$env:InstallDir = "C:\Tools\halcon\bin"
iwr -useb https://cli.cuervo.cloud/install.ps1 | iex
```

---

## 📦 Método 2: cargo install

**⏱️ Tiempo: ~2-5 minutos**

### Requisitos Previos

```bash
# Instalar Rust (si no lo tienes)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Instalación

```bash
# Desde el repositorio Git (recomendado)
cargo install --git https://github.com/cuervo-ai/halcon-cli halcon-cli

# Con todas las features
cargo install --git https://github.com/cuervo-ai/halcon-cli halcon-cli --features tui --locked
```

**Ventajas:**
- Siempre obtiene la última versión
- Compilado específicamente para tu sistema
- No requiere binario precompilado

**Desventajas:**
- Requiere Rust instalado
- Toma varios minutos compilar
- Requiere espacio para dependencias

---

## 📦 Método 3: cargo-binstall

**⏱️ Tiempo: ~15 segundos**

### Requisitos Previos

```bash
# Instalar cargo-binstall
cargo install cargo-binstall
```

### Instalación

```bash
cargo binstall halcon-cli
```

**Ventajas:**
- Más rápido que `cargo install`
- Descarga binarios precompilados
- Integrado con cargo

---

## 📦 Método 4: Descarga Manual

**⏱️ Tiempo: ~2 minutos**

### Pasos

1. **Descarga** desde [GitHub Releases](https://github.com/cuervo-ai/halcon-cli/releases/latest)

2. **Selecciona tu plataforma:**
   - `halcon-x86_64-unknown-linux-gnu.tar.gz` (Linux x64 glibc)
   - `halcon-x86_64-unknown-linux-musl.tar.gz` (Linux x64 musl/Alpine)
   - `halcon-aarch64-unknown-linux-gnu.tar.gz` (Linux ARM64)
   - `halcon-x86_64-apple-darwin.tar.gz` (macOS Intel)
   - `halcon-aarch64-apple-darwin.tar.gz` (macOS M1/M2/M3/M4)
   - `halcon-x86_64-pc-windows-msvc.zip` (Windows x64)

3. **Verifica checksum:**
   ```bash
   # Linux/macOS
   curl -sSfL https://releases.cli.cuervo.cloud/latest/checksums.txt | \
     grep halcon-*.tar.gz | sha256sum -c

   # Windows (PowerShell)
   $hash = (Get-FileHash halcon-*.zip -Algorithm SHA256).Hash.ToLower()
   $expected = (Invoke-RestMethod https://releases.cli.cuervo.cloud/latest/checksums.txt) |
     Select-String "halcon-.*\.zip" | ForEach-Object { ($_ -split '\s+')[0] }
   $hash -eq $expected
   ```

4. **Extrae:**
   ```bash
   # Linux/macOS
   tar xzf halcon-*.tar.gz

   # Windows
   Expand-Archive halcon-*.zip
   ```

5. **Instala:**
   ```bash
   # Linux/macOS
   chmod +x halcon
   mv halcon ~/.local/bin/

   # Windows
   move halcon.exe %LOCALAPPDATA%\halcon\bin\
   ```

---

## 📦 Método 5: Desde Código Fuente

**⏱️ Tiempo: ~5-10 minutos**

### Para Desarrollo

```bash
# Clonar repositorio
git clone https://github.com/cuervo-ai/halcon-cli.git
cd halcon-cli

# Compilar (debug - rápido)
cargo build --features tui

# Compilar (release - optimizado)
cargo build --release --features tui

# Ejecutar sin instalar
cargo run --features tui -- --help

# Instalar desde código local
cargo install --path crates/halcon-cli --features tui
```

---

## 📦 Instalación en Entornos Especiales

### Docker

```dockerfile
FROM ubuntu:22.04

RUN apt-get update && apt-get install -y curl ca-certificates && \
    curl -sSfL https://cli.cuervo.cloud/install.sh | sh

ENV PATH="/root/.local/bin:${PATH}"

RUN halcon --version
```

### GitHub Actions

```yaml
steps:
  - name: Install Halcon CLI
    run: |
      curl -sSfL https://cli.cuervo.cloud/install.sh | sh
      echo "$HOME/.local/bin" >> $GITHUB_PATH

  - name: Verify
    run: halcon --version
```

### GitLab CI

```yaml
install_halcon:
  script:
    - curl -sSfL https://cli.cuervo.cloud/install.sh | sh
    - export PATH="$HOME/.local/bin:$PATH"
    - halcon --version
```

---

## ✅ Verificación Post-Instalación

```bash
# Verificar versión
halcon --version

# Ejecutar diagnósticos
halcon doctor

# Mostrar ayuda
halcon --help
```

---

## 🔄 Actualización

### Script de instalación (sobrescribe binario existente)

```bash
# Linux/macOS
curl -sSfL https://cli.cuervo.cloud/install.sh | sh

# Windows
iwr -useb https://cli.cuervo.cloud/install.ps1 | iex
```

### cargo install

```bash
cargo install --git https://github.com/cuervo-ai/halcon-cli halcon-cli --force
```

---

## 🗑️ Desinstalación

### Binario

```bash
# Linux/macOS
rm ~/.local/bin/halcon
rm -rf ~/.halcon  # Opcional: elimina config y datos

# Windows (PowerShell)
Remove-Item "$env:LOCALAPPDATA\halcon\bin\halcon.exe"
Remove-Item -Recurse "$env:LOCALAPPDATA\halcon"  # Opcional
```

### cargo install

```bash
cargo uninstall halcon-cli
rm -rf ~/.halcon  # Opcional
```

---

## 🆘 Troubleshooting

### "Command not found: halcon"

```bash
# Verificar que existe
ls ~/.local/bin/halcon

# Añadir a PATH (bash/zsh)
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

### "Permission denied"

```bash
chmod +x ~/.local/bin/halcon
```

### Checksum verification fails

```bash
# Re-descargar (script siempre obtiene la versión más reciente)
curl -sSfL https://cli.cuervo.cloud/install.sh | sh

# O instalar desde código fuente
cargo install --git https://github.com/cuervo-ai/halcon-cli halcon-cli
```

---

## 📊 Comparación de Métodos

| Método | Tiempo | Requisitos | Ventaja Principal |
|--------|--------|------------|-------------------|
| **Script de instalación** | ~10s | curl/wget | ✅ Más rápido |
| **cargo-binstall** | ~15s | Rust + cargo-binstall | Integrado con cargo |
| **cargo install** | ~2-5min | Rust | Siempre actualizado |
| **Manual** | ~2min | Ninguno | Control total |
| **Desde código** | ~5-10min | Rust + Git | Desarrollo |

---

## 🌍 Plataformas Soportadas

| Plataforma | Arquitectura | Estado |
|-----------|--------------|--------|
| Linux (Ubuntu, Debian, Fedora) | x86_64 (glibc) | ✅ Tier 1 |
| Linux (Alpine) | x86_64 (musl) | ✅ Tier 1 |
| Linux | ARM64 / aarch64 | ✅ Tier 1 |
| macOS | Intel (x86_64) | ✅ Tier 1 |
| macOS | Apple Silicon (M1/M2/M3/M4) | ✅ Tier 1 |
| Windows | x64 | ✅ Tier 1 |

---

**Última actualización:** 2026-02-23

**¿Problemas?** Abre un [issue](https://github.com/cuervo-ai/halcon-cli/issues) o visita la [documentación](https://halcon.cuervo.cloud/docs).
