#!/usr/bin/env bash
# scripts/release/checksums.sh — Generate and sign SHA-256 checksums
# Usage: ./dist/checksums.sh <artifacts-dir> [version]
set -euo pipefail

ARTIFACTS_DIR="${1:?Usage: $0 <artifacts-dir> [version]}"
VERSION="${2:-}"

if [ ! -d "$ARTIFACTS_DIR" ]; then
    echo "ERROR: Directory not found: $ARTIFACTS_DIR" >&2
    exit 1
fi

CHECKSUMS_FILE="${ARTIFACTS_DIR}/checksums.txt"

echo "==> Generating SHA-256 checksums in: ${ARTIFACTS_DIR}"

# Clear existing checksums file
> "$CHECKSUMS_FILE"

# Generate checksums for all archives
find "$ARTIFACTS_DIR" -maxdepth 1 \( -name "*.tar.gz" -o -name "*.zip" \) | sort | while read -r artifact; do
    if [[ "$artifact" == *.sig ]] || [[ "$artifact" == *.pem ]]; then
        continue
    fi

    filename="$(basename "$artifact")"
    echo "--> Checksum: ${filename}"

    if command -v sha256sum &>/dev/null; then
        sha256sum "$artifact" | awk "{ print \$1 \"  ${filename}\" }" >> "$CHECKSUMS_FILE"
    elif command -v shasum &>/dev/null; then
        shasum -a 256 "$artifact" | awk "{ print \$1 \"  ${filename}\" }" >> "$CHECKSUMS_FILE"
    else
        echo "ERROR: No sha256sum or shasum found" >&2
        exit 1
    fi
done

echo ""
echo "==> Checksums file: ${CHECKSUMS_FILE}"
cat "$CHECKSUMS_FILE"

# Sign the checksums file with cosign if available
if command -v cosign &>/dev/null; then
    echo ""
    echo "==> Signing checksums file..."
    cosign sign-blob \
        --yes \
        --output-signature "${CHECKSUMS_FILE}.sig" \
        --output-certificate "${CHECKSUMS_FILE}.pem" \
        "$CHECKSUMS_FILE"
    echo "    Signature: ${CHECKSUMS_FILE}.sig"
    echo "    Certificate: ${CHECKSUMS_FILE}.pem"
fi

echo ""
echo "==> Done."
