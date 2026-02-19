#!/usr/bin/env bash
# wasm-rebuild.sh — Rebuild and sync the Momoto UI Core WASM binary.
#
# Usage:
#   ./scripts/wasm-rebuild.sh          # Rebuild and copy to website/
#   ./scripts/wasm-rebuild.sh --verify # Only verify checksums, no rebuild
#
# Requirements:
#   - wasm-pack (https://rustwasm.github.io/wasm-pack/)
#   - Rust toolchain with wasm32-unknown-unknown target
#   - vendor/momoto-ui submodule initialized

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

WASM_CRATE="$REPO_ROOT/vendor/momoto-ui/momoto/crates/momoto-ui-core"
WASM_OUT="$REPO_ROOT/website/src/lib/momoto"
BUILD_TMP="$(mktemp -d)"

trap 'rm -rf "$BUILD_TMP"' EXIT

# ── Color helpers ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()    { echo -e "${BLUE}[wasm]${NC} $*"; }
success() { echo -e "${GREEN}[wasm]${NC} ✅ $*"; }
warn()    { echo -e "${YELLOW}[wasm]${NC} ⚠️  $*"; }
error()   { echo -e "${RED}[wasm]${NC} ❌ $*" >&2; }

# ── Preflight checks ─────────────────────────────────────────────────────────
if [ ! -f "$WASM_CRATE/Cargo.toml" ]; then
    error "momoto-ui submodule not initialized."
    echo ""
    echo "  Initialize with:"
    echo "    git submodule update --init --recursive"
    echo ""
    echo "  Or add the submodule if missing:"
    echo "    git submodule add https://github.com/cuervo-ai/momoto-ui.git vendor/momoto-ui"
    exit 1
fi

if ! command -v wasm-pack &>/dev/null; then
    error "wasm-pack not found."
    echo ""
    echo "  Install with:"
    echo "    curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh"
    exit 1
fi

if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
    info "Adding wasm32-unknown-unknown target..."
    rustup target add wasm32-unknown-unknown
fi

# ── Verify-only mode ─────────────────────────────────────────────────────────
if [[ "${1:-}" == "--verify" ]]; then
    info "Verifying WASM checksums (verify-only mode)..."

    if [ ! -f "$WASM_OUT/momoto_ui_core_bg.wasm" ]; then
        error "No committed WASM found at $WASM_OUT/"
        exit 1
    fi

    info "Building fresh WASM for comparison..."
    (cd "$WASM_CRATE" && wasm-pack build --target web --release --out-dir "$BUILD_TMP" -q)

    COMMITTED_HASH=$(sha256sum "$WASM_OUT/momoto_ui_core_bg.wasm" | cut -d' ' -f1)
    FRESH_HASH=$(sha256sum "$BUILD_TMP/momoto_ui_core_bg.wasm" | cut -d' ' -f1)

    if [ "$COMMITTED_HASH" = "$FRESH_HASH" ]; then
        success "WASM is synchronized (sha256: $COMMITTED_HASH)"
        exit 0
    else
        error "WASM is OUT OF SYNC!"
        echo "  Committed: $COMMITTED_HASH"
        echo "  Fresh:     $FRESH_HASH"
        echo ""
        echo "  Run: ./scripts/wasm-rebuild.sh"
        exit 1
    fi
fi

# ── Full rebuild ─────────────────────────────────────────────────────────────
info "Building Momoto UI Core WASM..."
info "  Source: $WASM_CRATE"
info "  Output: $WASM_OUT"
echo ""

(cd "$WASM_CRATE" && wasm-pack build --target web --release --out-dir "$BUILD_TMP")

info "Copying artifacts..."
cp "$BUILD_TMP/momoto_ui_core_bg.wasm"     "$WASM_OUT/momoto_ui_core_bg.wasm"
cp "$BUILD_TMP/momoto_ui_core_bg.wasm.d.ts" "$WASM_OUT/momoto_ui_core_bg.wasm.d.ts"
cp "$BUILD_TMP/momoto_ui_core.js"          "$WASM_OUT/momoto_ui_core.js"
cp "$BUILD_TMP/momoto_ui_core.d.ts"        "$WASM_OUT/momoto_ui_core.d.ts"

WASM_SIZE=$(du -sh "$WASM_OUT/momoto_ui_core_bg.wasm" | cut -f1)
WASM_HASH=$(sha256sum "$WASM_OUT/momoto_ui_core_bg.wasm" | cut -d' ' -f1)

echo ""
success "WASM rebuilt and synchronized"
echo ""
echo "  Size:   $WASM_SIZE"
echo "  SHA256: $WASM_HASH"
echo ""
echo "  Next steps:"
echo "    git add website/src/lib/momoto/"
echo "    git commit -m 'chore(wasm): rebuild momoto-ui-core WASM'"
