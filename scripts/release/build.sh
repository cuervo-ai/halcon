#!/usr/bin/env bash
# scripts/release/build.sh — Local multi-platform build script for Halcón CLI
# Usage: ./dist/build.sh [version] [target...]
# Example: ./dist/build.sh v0.2.0
#          ./dist/build.sh v0.2.0 x86_64-unknown-linux-musl aarch64-apple-darwin
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

VERSION="${1:-$(grep '^version' "$ROOT_DIR/Cargo.toml" | head -1 | cut -d'"' -f2)}"
VERSION="${VERSION#v}"  # strip leading 'v' if present

# Default targets if none specified
DEFAULT_TARGETS=(
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-gnu"
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-pc-windows-msvc"
)

if [ $# -gt 1 ]; then
    shift
    TARGETS=("$@")
else
    TARGETS=("${DEFAULT_TARGETS[@]}")
fi

OUT_DIR="$ROOT_DIR/dist/artifacts/${VERSION}"
mkdir -p "$OUT_DIR"

echo "==> Building Halcón CLI v${VERSION}"
echo "    Targets: ${TARGETS[*]}"
echo "    Output:  ${OUT_DIR}"
echo ""

# Check for cross
if ! command -v cross &>/dev/null; then
    echo "Installing cross-rs..."
    cargo install cross --git https://github.com/cross-rs/cross --locked
fi

build_target() {
    local target="$1"
    local binary_name="halcon"
    local use_cross=false

    # Determine if cross is needed
    local host_target
    host_target="$(rustc -vV | grep host | cut -d' ' -f2)"
    if [[ "$target" != "$host_target" ]]; then
        use_cross=true
    fi

    echo "--> Building for ${target}..."

    cd "$ROOT_DIR"

    # Set build env vars
    export HALCON_GIT_HASH
    HALCON_GIT_HASH="$(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
    export HALCON_BUILD_DATE
    HALCON_BUILD_DATE="$(date -u +%Y-%m-%d)"
    export HALCON_TARGET="$target"

    if [[ "$use_cross" == "true" ]]; then
        cross build --release \
            --target "$target" \
            --no-default-features \
            --features tui \
            --locked \
            -p halcon-cli
    else
        cargo build --release \
            --target "$target" \
            --no-default-features \
            --features tui \
            --locked \
            -p halcon-cli
    fi

    # Determine binary path
    local binary_src="$ROOT_DIR/target/${target}/release/${binary_name}"
    if [[ "$target" == *"windows"* ]]; then
        binary_src="${binary_src}.exe"
        binary_name="halcon.exe"
    fi

    # Create archive
    local archive_base="halcon-${VERSION}-${target}"
    if [[ "$target" == *"windows"* ]]; then
        local archive="${OUT_DIR}/${archive_base}.zip"
        (cd "$(dirname "$binary_src")" && zip "$archive" "$binary_name")
        echo "    Created: ${archive}"
    else
        local archive="${OUT_DIR}/${archive_base}.tar.gz"
        (cd "$(dirname "$binary_src")" && tar czf "$archive" "$binary_name")
        echo "    Created: ${archive}"
    fi

    echo "    Done: ${target}"
}

export -f build_target

# Build all targets (sequentially to avoid disk/CPU contention)
for target in "${TARGETS[@]}"; do
    build_target "$target"
done

echo ""
echo "==> Build complete! Artifacts in: ${OUT_DIR}"
ls -lh "$OUT_DIR/"
