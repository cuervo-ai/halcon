#!/usr/bin/env bash
# Halcón CLI — Installation Script
# Usage: ./scripts/install.sh
# Installs halcon to ~/.local/bin/ (no sudo required)
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

INSTALL_DIR="${HALCON_INSTALL_DIR:-$HOME/.local/bin}"
REQUIRED_MSRV="1.80.0"
REPO_URL="https://github.com/cuervo-ai/halcon-cli"

info()  { echo -e "${BLUE}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
fail()  { echo -e "${RED}[FAIL]${NC}  $*"; exit 1; }

echo -e "${BOLD}${BLUE}"
echo "  ╔═════════════════════════════════════════════╗"
echo "  ║         Halcón CLI — Installation           ║"
echo "  ╚═════════════════════════════════════════════╝"
echo -e "${NC}"

# ── Step 1: Verify Rust ──────────────────────────────────────────────────────
info "[1/6] Checking Rust installation..."

if ! command -v rustc &>/dev/null; then
    warn "Rust not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
fi

RUST_VERSION=$(rustc --version | cut -d' ' -f2)
if [ "$(printf '%s\n' "$REQUIRED_MSRV" "$RUST_VERSION" | sort -V | head -n1)" = "$REQUIRED_MSRV" ]; then
    ok "Rust $RUST_VERSION (>= $REQUIRED_MSRV)"
else
    warn "Rust $RUST_VERSION < $REQUIRED_MSRV required. Updating..."
    rustup update stable
    RUST_VERSION=$(rustc --version | cut -d' ' -f2)
    ok "Rust updated to $RUST_VERSION"
fi

# ── Step 2: Get source code ──────────────────────────────────────────────────
info "[2/6] Preparing source code..."

# If we're already in the repo, use it; otherwise clone
if [ -f "Cargo.toml" ] && grep -q 'name = "halcon"' Cargo.toml 2>/dev/null; then
    ok "Already in halcon-cli repository"
    REPO_DIR="$(pwd)"
elif [ -f "../Cargo.toml" ] && grep -q 'name = "halcon"' ../Cargo.toml 2>/dev/null; then
    ok "Found halcon-cli repository in parent directory"
    REPO_DIR="$(cd .. && pwd)"
elif [ -d "halcon-cli" ]; then
    info "Updating existing clone..."
    cd halcon-cli
    git pull origin main
    REPO_DIR="$(pwd)"
else
    info "Cloning from $REPO_URL..."
    git clone "$REPO_URL"
    cd halcon-cli
    REPO_DIR="$(pwd)"
fi

# ── Step 3: Build ─────────────────────────────────────────────────────────────
info "[3/6] Building release binary (this may take several minutes)..."

cd "$REPO_DIR"
cargo build --release --no-default-features 2>&1 | tail -5

BINARY="$REPO_DIR/target/release/halcon"
if [ ! -f "$BINARY" ]; then
    fail "Build failed — binary not found at $BINARY"
fi

BINARY_SIZE=$(du -h "$BINARY" | cut -f1)
ok "Build complete — $BINARY_SIZE"

# ── Step 4: Install ──────────────────────────────────────────────────────────
info "[4/6] Installing to $INSTALL_DIR..."

mkdir -p "$INSTALL_DIR"
cp "$BINARY" "$INSTALL_DIR/halcon"
chmod +x "$INSTALL_DIR/halcon"
ok "Installed to $INSTALL_DIR/halcon"

# Ensure install dir is in PATH
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    warn "$INSTALL_DIR is not in your PATH."
    echo ""
    echo "  Add this to your shell profile (~/.zshrc or ~/.bashrc):"
    echo ""
    echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
    echo ""
fi

# ── Step 5: Configuration ────────────────────────────────────────────────────
info "[5/6] Checking configuration..."

CONFIG_DIR="$HOME/.halcon"
CONFIG_FILE="$CONFIG_DIR/config.toml"

mkdir -p "$CONFIG_DIR"

if [ -f "$CONFIG_FILE" ]; then
    ok "Configuration exists at $CONFIG_FILE"
else
    info "Creating default configuration..."
    cat > "$CONFIG_FILE" << 'TOML'
[general]
default_provider = "deepseek"
default_model = "deepseek-chat"
max_tokens = 8192
temperature = 0.0

[models.providers.deepseek]
enabled = true
api_base = "https://api.deepseek.com"
default_model = "deepseek-chat"

[models.providers.ollama]
enabled = true
api_base = "http://localhost:11434"
default_model = "deepseek-coder-v2:latest"

[models.providers.openai]
enabled = true
api_base = "https://api.openai.com/v1"
default_model = "gpt-4o-mini"

[tools]
confirm_destructive = true
timeout_secs = 120

[security]
pii_detection = true
audit_enabled = true
TOML
    ok "Default configuration created at $CONFIG_FILE"
fi

# ── Step 6: Verify ───────────────────────────────────────────────────────────
info "[6/6] Verifying installation..."

HALCON_BIN="$INSTALL_DIR/halcon"
if [ -x "$HALCON_BIN" ]; then
    VERSION=$("$HALCON_BIN" --version 2>&1 || true)
    ok "halcon --version: $VERSION"
else
    fail "Binary not executable at $HALCON_BIN"
fi

# Quick smoke test
if "$HALCON_BIN" --help >/dev/null 2>&1; then
    ok "halcon --help: exit 0"
else
    warn "halcon --help returned non-zero (may need API key configuration)"
fi

echo ""
echo -e "${BOLD}${GREEN}Installation complete!${NC}"
echo ""
echo "  Next steps:"
echo "  1. Set API keys (choose one or more):"
echo "     export DEEPSEEK_API_KEY=sk-..."
echo "     export OPENAI_API_KEY=sk-..."
echo "     export ANTHROPIC_API_KEY=sk-ant-..."
echo ""
echo "  2. Start chatting:"
echo "     halcon chat \"Hello, Halcón!\""
echo ""
echo "  3. Interactive REPL:"
echo "     halcon"
echo ""
echo "  4. Run E2E tests:"
echo "     ./scripts/test_e2e.sh"
echo ""
echo -e "  Documentation: ${BLUE}$REPO_URL${NC}"
