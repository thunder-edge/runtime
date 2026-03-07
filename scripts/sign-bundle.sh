#!/usr/bin/env bash

# Sign a bundle with Ed25519 private key and optionally print deploy headers.
#
# Usage:
#   ./scripts/sign-bundle.sh --bundle ./hello.eszip --private-key ./bundle-signing-private.pem
#   ./scripts/sign-bundle.sh --bundle ./hello.eszip --private-key ./bundle-signing-private.pem --output ./hello.eszip.sig --print-header
#
# Requires OpenSSL 3.x with Ed25519 support.

set -euo pipefail

BUNDLE_PATH=""
PRIVATE_KEY_PATH=""
OUTPUT_PATH=""
PRINT_HEADER=false

usage() {
  cat <<'EOF'
Usage:
  ./scripts/sign-bundle.sh --bundle <path> --private-key <path> [options]

Required:
  --bundle <path>        Path to bundle file (.eszip or .pkg)
  --private-key <path>   Path to Ed25519 private key (PEM)

Options:
  --output <path>        Output signature file path (default: <bundle>.sig)
  --print-header         Print x-bundle-signature-ed25519 header line
  -h, --help             Show this help

Examples:
  ./scripts/sign-bundle.sh --bundle ./hello.eszip --private-key ./bundle-signing-private.pem
  ./scripts/sign-bundle.sh --bundle ./hello.eszip --private-key ./bundle-signing-private.pem --print-header
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle)
      BUNDLE_PATH="${2:-}"
      shift 2
      ;;
    --private-key)
      PRIVATE_KEY_PATH="${2:-}"
      shift 2
      ;;
    --output)
      OUTPUT_PATH="${2:-}"
      shift 2
      ;;
    --print-header)
      PRINT_HEADER=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Error: unknown argument '$1'" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$BUNDLE_PATH" || -z "$PRIVATE_KEY_PATH" ]]; then
  echo "Error: --bundle and --private-key are required" >&2
  usage
  exit 1
fi

if [[ ! -f "$BUNDLE_PATH" ]]; then
  echo "Error: bundle file not found: $BUNDLE_PATH" >&2
  exit 1
fi

if [[ ! -f "$PRIVATE_KEY_PATH" ]]; then
  echo "Error: private key file not found: $PRIVATE_KEY_PATH" >&2
  exit 1
fi

if [[ -z "$OUTPUT_PATH" ]]; then
  OUTPUT_PATH="${BUNDLE_PATH}.sig"
fi

if ! command -v openssl >/dev/null 2>&1; then
  echo "Error: openssl is required but not found in PATH" >&2
  exit 1
fi

# Verify OpenSSL supports Ed25519.
if ! openssl list -public-key-algorithms 2>/dev/null | grep -qi "ED25519"; then
  echo "Error: OpenSSL build does not report Ed25519 support" >&2
  echo "Hint: install OpenSSL 3.x" >&2
  exit 1
fi

openssl pkeyutl -sign \
  -inkey "$PRIVATE_KEY_PATH" \
  -rawin \
  -in "$BUNDLE_PATH" \
  -out "$OUTPUT_PATH"

SIG_B64="$(base64 < "$OUTPUT_PATH" | tr -d '\n')"

echo "Signed bundle: $BUNDLE_PATH"
echo "Signature file: $OUTPUT_PATH"
echo "Signature (base64): $SIG_B64"

if [[ "$PRINT_HEADER" == true ]]; then
  echo "Header: x-bundle-signature-ed25519: $SIG_B64"
fi
