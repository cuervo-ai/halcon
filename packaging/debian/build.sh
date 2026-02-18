#!/usr/bin/env bash
# packaging/debian/build.sh — Build .deb package for Halcon CLI
# Usage: ./packaging/debian/build.sh <version> <binary-path> [arch]
# Example: ./packaging/debian/build.sh 0.2.0 ./target/x86_64-unknown-linux-musl/release/halcon amd64
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

VERSION="${1:?Usage: $0 <version> <binary-path> [arch]}"
BINARY="${2:?Missing binary path}"
ARCH="${3:-amd64}"

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found: $BINARY" >&2
    exit 1
fi

VERSION_BARE="${VERSION#v}"
PKG_NAME="halcon_${VERSION_BARE}_${ARCH}"
BUILD_DIR="$(mktemp -d)"
trap "rm -rf '$BUILD_DIR'" EXIT

PKG_DIR="${BUILD_DIR}/${PKG_NAME}"

echo "==> Building .deb: ${PKG_NAME}.deb"

# Create directory structure
mkdir -p "${PKG_DIR}/DEBIAN"
mkdir -p "${PKG_DIR}/usr/local/bin"
mkdir -p "${PKG_DIR}/usr/share/doc/halcon"
mkdir -p "${PKG_DIR}/usr/share/man/man1"

# Install binary
cp "$BINARY" "${PKG_DIR}/usr/local/bin/halcon"
chmod 0755 "${PKG_DIR}/usr/local/bin/halcon"

# Install docs
cp "${ROOT_DIR}/README.md" "${PKG_DIR}/usr/share/doc/halcon/" 2>/dev/null || true
cp "${ROOT_DIR}/LICENSE" "${PKG_DIR}/usr/share/doc/halcon/copyright" 2>/dev/null || true

# Create control file from template
sed \
    -e "s/{{VERSION}}/${VERSION_BARE}/g" \
    -e "s/{{ARCH}}/${ARCH}/g" \
    "${SCRIPT_DIR}/control.template" > "${PKG_DIR}/DEBIAN/control"

# Create postinst script
cat > "${PKG_DIR}/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e

# Create config directory for all users on first install
if [ "$1" = "configure" ]; then
    mkdir -p /etc/halcon
fi

exit 0
EOF
chmod 0755 "${PKG_DIR}/DEBIAN/postinst"

# Build the .deb
OUTPUT_DIR="${ROOT_DIR}/dist/packages"
mkdir -p "$OUTPUT_DIR"
OUTPUT="${OUTPUT_DIR}/${PKG_NAME}.deb"

dpkg-deb --build --root-owner-group "$PKG_DIR" "$OUTPUT"

echo "==> Built: $OUTPUT"
echo "    Size: $(du -sh "$OUTPUT" | cut -f1)"

# Generate checksum
sha256sum "$OUTPUT" > "${OUTPUT}.sha256"
echo "    SHA256: $(cat "${OUTPUT}.sha256" | cut -d' ' -f1)"
