#!/usr/bin/env bash
# scripts/release/sign.sh — Cosign keyless signing for Cuervo CLI artifacts
# Uses GitHub OIDC in CI, or local identity in dev
# Usage: ./dist/sign.sh <artifacts-dir>
set -euo pipefail

ARTIFACTS_DIR="${1:?Usage: $0 <artifacts-dir>}"

if [ ! -d "$ARTIFACTS_DIR" ]; then
    echo "ERROR: Directory not found: $ARTIFACTS_DIR" >&2
    exit 1
fi

# Check for cosign
if ! command -v cosign &>/dev/null; then
    echo "Installing cosign..."
    if [[ "$(uname)" == "Darwin" ]]; then
        brew install sigstore/tap/cosign
    else
        # Install cosign binary for Linux
        COSIGN_VERSION="v2.4.1"
        curl -sSfL "https://github.com/sigstore/cosign/releases/download/${COSIGN_VERSION}/cosign-linux-amd64" \
            -o /usr/local/bin/cosign
        chmod +x /usr/local/bin/cosign
    fi
fi

echo "==> Signing artifacts in: ${ARTIFACTS_DIR}"

# Sign all archives (tar.gz and zip)
find "$ARTIFACTS_DIR" -maxdepth 1 \( -name "*.tar.gz" -o -name "*.zip" \) | sort | while read -r artifact; do
    if [[ "$artifact" == *.sig ]]; then
        continue
    fi

    echo "--> Signing: $(basename "$artifact")"

    # Keyless signing — uses OIDC identity
    # In GitHub Actions: SIGSTORE_ID_TOKEN is provided automatically
    # In local dev: will open browser for identity verification
    cosign sign-blob \
        --yes \
        --output-signature "${artifact}.sig" \
        --output-certificate "${artifact}.pem" \
        "$artifact"

    echo "    Signature: ${artifact}.sig"
    echo "    Certificate: ${artifact}.pem"
done

echo ""
echo "==> All artifacts signed."
echo ""
echo "Verify with:"
echo "  cosign verify-blob \\"
echo "    --signature <file>.sig \\"
echo "    --certificate <file>.pem \\"
echo "    --certificate-identity-regexp 'https://github.com/cuervo-ai/cuervo-cli' \\"
echo "    --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \\"
echo "    <file>"
