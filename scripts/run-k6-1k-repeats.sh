#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME_DIR="$(dirname "$SCRIPT_DIR")"
RUNS="${RUNS:-10}"
BASE_URL="${BASE_URL:-http://127.0.0.1:8080}"
PATHNAME="${PATHNAME:-/hello}"
OUTPUT_DIR="${OUTPUT_DIR:-/tmp/thunder-k6-1k-$(date -u +%Y%m%dT%H%M%SZ)}"

if ! command -v k6 >/dev/null 2>&1; then
    echo "k6 is required; install it before running this benchmark" >&2
    exit 1
fi

if ! curl --fail-with-body --silent --show-error \
    "${BASE_URL}${PATHNAME}" >/dev/null; then
    echo "Benchmark preflight failed: ${BASE_URL}${PATHNAME} must return 200" >&2
    exit 1
fi

mkdir -p "$OUTPUT_DIR"
{
    echo "started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "runs=$RUNS"
    echo "base_url=$BASE_URL"
    echo "pathname=$PATHNAME"
    echo "k6_version=$(k6 version)"
    echo "git_revision=$(git -C "$RUNTIME_DIR" rev-parse HEAD)"
    echo "rustc_version=$(rustc --version)"
} >"$OUTPUT_DIR/metadata.txt"

run=1
while [ "$run" -le "$RUNS" ]; do
    summary_path="$OUTPUT_DIR/run-$(printf '%02d' "$run").json"
    echo "Starting run $run/$RUNS; summary: $summary_path"
    k6 run "$SCRIPT_DIR/k6_1k_rps.js" \
        -e BASE_URL="$BASE_URL" \
        -e PATHNAME="$PATHNAME" \
        --summary-export="$summary_path"
    run=$((run + 1))
done

echo "Completed $RUNS valid runs. Results: $OUTPUT_DIR"
