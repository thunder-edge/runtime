#!/usr/bin/env bash

# Sign and deploy/update a bundle in one command.
#
# Usage:
#   ./scripts/deploy-signed-bundle.sh \
#     --bundle ./hello.eszip \
#     --function hello \
#     --private-key ./bundle-signing-private.pem \
#     --api-key admin-secret
#
# Supports POST (create) and PUT (update).

set -euo pipefail

BUNDLE_PATH=""
FUNCTION_NAME=""
PRIVATE_KEY_PATH=""
API_KEY=""
SERVER_URL="http://127.0.0.1:9000"
METHOD="POST"
TMP_SIG=""
KEEP_SIG=false
OUTPUT_SIG=""

usage() {
  cat <<'EOF'
Usage:
  ./scripts/deploy-signed-bundle.sh --bundle <path> --function <name> --private-key <path> [options]

Required:
  --bundle <path>        Path to bundle file (.eszip or .pkg)
  --function <name>      Function name for x-function-name and route
  --private-key <path>   Path to Ed25519 private key (PEM)

Options:
  --api-key <value>      Admin API key (X-API-Key)
  --server-url <url>     Admin base URL (default: http://127.0.0.1:9000)
  --method <POST|PUT>    HTTP method (default: POST)
  --output-sig <path>    Save signature file to path (default: temporary file)
  --keep-sig             Keep temporary signature file
  -h, --help             Show this help

Examples:
  ./scripts/deploy-signed-bundle.sh \
    --bundle ./hello.eszip \
    --function hello \
    --private-key ./bundle-signing-private.pem \
    --api-key admin-secret

  ./scripts/deploy-signed-bundle.sh \
    --bundle ./hello.eszip \
    --function hello \
    --private-key ./bundle-signing-private.pem \
    --method PUT
EOF
}

cleanup() {
  if [[ -n "$TMP_SIG" && -f "$TMP_SIG" && "$KEEP_SIG" != true ]]; then
    rm -f "$TMP_SIG"
  fi
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle)
      BUNDLE_PATH="${2:-}"
      shift 2
      ;;
    --function)
      FUNCTION_NAME="${2:-}"
      shift 2
      ;;
    --private-key)
      PRIVATE_KEY_PATH="${2:-}"
      shift 2
      ;;
    --api-key)
      API_KEY="${2:-}"
      shift 2
      ;;
    --server-url)
      SERVER_URL="${2:-}"
      shift 2
      ;;
    --method)
      METHOD="${2:-}"
      shift 2
      ;;
    --output-sig)
      OUTPUT_SIG="${2:-}"
      shift 2
      ;;
    --keep-sig)
      KEEP_SIG=true
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

if [[ -z "$BUNDLE_PATH" || -z "$FUNCTION_NAME" || -z "$PRIVATE_KEY_PATH" ]]; then
  echo "Error: --bundle, --function, and --private-key are required" >&2
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

METHOD="$(echo "$METHOD" | tr '[:lower:]' '[:upper:]')"
if [[ "$METHOD" != "POST" && "$METHOD" != "PUT" ]]; then
  echo "Error: --method must be POST or PUT" >&2
  exit 1
fi

if ! command -v openssl >/dev/null 2>&1; then
  echo "Error: openssl is required but not found in PATH" >&2
  exit 1
fi

if [[ -n "$OUTPUT_SIG" ]]; then
  SIG_PATH="$OUTPUT_SIG"
else
  TMP_SIG="$(mktemp -t edge-bundle-signature.XXXXXX)"
  SIG_PATH="$TMP_SIG"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"$SCRIPT_DIR/sign-bundle.sh" \
  --bundle "$BUNDLE_PATH" \
  --private-key "$PRIVATE_KEY_PATH" \
  --output "$SIG_PATH" >/dev/null

SIG_B64="$(base64 < "$SIG_PATH" | tr -d '\n')"

if [[ "$METHOD" == "POST" ]]; then
  ENDPOINT="$SERVER_URL/_internal/functions"
else
  ENDPOINT="$SERVER_URL/_internal/functions/$FUNCTION_NAME"
fi

echo "Deploying signed bundle:"
echo "  method:   $METHOD"
echo "  endpoint: $ENDPOINT"
echo "  function: $FUNCTION_NAME"
echo "  bundle:   $BUNDLE_PATH"

curl_args=(
  -sS
  -w "\n%{http_code}"
  -X "$METHOD"
  "$ENDPOINT"
  -H "content-type: application/octet-stream"
  -H "x-function-name: $FUNCTION_NAME"
  -H "x-bundle-signature-ed25519: $SIG_B64"
  --data-binary "@$BUNDLE_PATH"
)

if [[ -n "$API_KEY" ]]; then
  curl_args+=( -H "X-API-Key: $API_KEY" )
fi

response="$(curl "${curl_args[@]}")"
status="$(echo "$response" | tail -n1)"
body="$(echo "$response" | sed '$d')"

echo ""
echo "Response status: $status"
echo "$body"

if [[ "$status" != "200" && "$status" != "201" ]]; then
  exit 1
fi

if [[ "$KEEP_SIG" == true ]]; then
  echo "Signature file kept at: $SIG_PATH"
fi
