#!/usr/bin/env bash
# scripts/release/package.sh — Final packaging: tar.gz for Unix, zip for Windows
# Includes: binary + README + LICENSE
# Usage: ./dist/package.sh <version>
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

VERSION="${1:?Usage: $0 <version>}"
VERSION="${VERSION#v}"

ARTIFACTS_DIR="${SCRIPT_DIR}/artifacts/${VERSION}"

if [ ! -d "$ARTIFACTS_DIR" ]; then
    echo "ERROR: Artifacts directory not found: $ARTIFACTS_DIR" >&2
    echo "Run dist/build.sh first." >&2
    exit 1
fi

echo "==> Packaging Cuervo CLI v${VERSION}"

# Create a staging area for each target
find "$ARTIFACTS_DIR" -maxdepth 1 \( -name "*.tar.gz" -o -name "*.zip" \) | sort | while read -r archive; do
    # Skip already-packaged files
    if [[ "$archive" == *"-full."* ]]; then
        continue
    fi

    basename_no_ext="${archive%.tar.gz}"
    basename_no_ext="${basename_no_ext%.zip}"
    target_name="$(basename "$basename_no_ext")"

    echo "--> Packaging: ${target_name}"

    # Create staging directory
    stage_dir="$(mktemp -d)"
    trap "rm -rf '$stage_dir'" EXIT

    pkg_dir="${stage_dir}/${target_name}"
    mkdir -p "$pkg_dir"

    # Extract binary
    if [[ "$archive" == *.tar.gz ]]; then
        tar xzf "$archive" -C "$pkg_dir"
    else
        unzip -q "$archive" -d "$pkg_dir"
    fi

    # Add docs
    cp "${ROOT_DIR}/README.md" "$pkg_dir/" 2>/dev/null || true
    cp "${ROOT_DIR}/LICENSE" "$pkg_dir/" 2>/dev/null || true

    # Create final archive with docs included
    if [[ "$archive" == *.tar.gz ]]; then
        final_archive="${ARTIFACTS_DIR}/${target_name}.tar.gz"
        # Replace existing archive with the one including docs
        tar czf "$final_archive" -C "$stage_dir" "$target_name"
        echo "    Updated: ${final_archive}"
    else
        final_archive="${ARTIFACTS_DIR}/${target_name}.zip"
        (cd "$stage_dir" && zip -r "$final_archive" "$target_name")
        echo "    Updated: ${final_archive}"
    fi

    trap - EXIT
    rm -rf "$stage_dir"
done

echo ""
echo "==> Package complete. Contents:"
ls -lh "$ARTIFACTS_DIR/"
