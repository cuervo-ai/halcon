#!/usr/bin/env bash
# packaging/rpm/build.sh — Build .rpm package for Halcon CLI
# Usage: ./packaging/rpm/build.sh <version> <binary-path>
# Example: ./packaging/rpm/build.sh 0.2.0 ./target/x86_64-unknown-linux-musl/release/halcon
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

VERSION="${1:?Usage: $0 <version> <binary-path>}"
BINARY="${2:?Missing binary path}"

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found: $BINARY" >&2
    exit 1
fi

VERSION_BARE="${VERSION#v}"

# Check for rpmbuild
if ! command -v rpmbuild &>/dev/null; then
    echo "ERROR: rpmbuild not found. Install rpm-build package." >&2
    echo "  Ubuntu/Debian: apt-get install rpm"
    echo "  Fedora/RHEL:   dnf install rpm-build"
    exit 1
fi

BUILD_DIR="$(mktemp -d)"
trap "rm -rf '$BUILD_DIR'" EXIT

# Setup RPM build tree
mkdir -p "${BUILD_DIR}"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# Create source tarball (RPM expects a tarball as Source0)
TARBALL_NAME="halcon-${VERSION_BARE}-x86_64-unknown-linux-musl"
TARBALL_DIR="${BUILD_DIR}/SOURCES/${TARBALL_NAME}"
mkdir -p "$TARBALL_DIR"

cp "$BINARY" "${TARBALL_DIR}/halcon"
cp "${ROOT_DIR}/README.md" "${TARBALL_DIR}/" 2>/dev/null || echo "# Halcon CLI" > "${TARBALL_DIR}/README.md"
cp "${ROOT_DIR}/LICENSE" "${TARBALL_DIR}/" 2>/dev/null || echo "Apache-2.0" > "${TARBALL_DIR}/LICENSE"

tar czf "${BUILD_DIR}/SOURCES/${TARBALL_NAME}.tar.gz" -C "${BUILD_DIR}/SOURCES" "${TARBALL_NAME}"

# Generate spec file from template
DATE="$(date -u '+%a %b %d %Y')"
sed \
    -e "s/{{VERSION}}/${VERSION_BARE}/g" \
    -e "s/{{DATE}}/${DATE}/g" \
    "${SCRIPT_DIR}/halcon.spec.template" > "${BUILD_DIR}/SPECS/halcon.spec"

echo "==> Building RPM for Halcon CLI v${VERSION_BARE}"

# Build RPM
rpmbuild \
    --define "_topdir ${BUILD_DIR}" \
    --define "_binary_payload w2.xzdio" \
    -bb "${BUILD_DIR}/SPECS/halcon.spec"

# Copy output
OUTPUT_DIR="${ROOT_DIR}/dist/packages"
mkdir -p "$OUTPUT_DIR"

find "${BUILD_DIR}/RPMS" -name "*.rpm" | while read -r rpm_file; do
    output="${OUTPUT_DIR}/$(basename "$rpm_file")"
    cp "$rpm_file" "$output"
    echo "==> Built: $output"
    echo "    Size: $(du -sh "$output" | cut -f1)"

    # Generate checksum
    sha256sum "$output" > "${output}.sha256"
    echo "    SHA256: $(cat "${output}.sha256" | cut -d' ' -f1)"
done
