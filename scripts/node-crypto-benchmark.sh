#!/bin/bash

# node:crypto focused benchmark runner
# Usage: ./scripts/node-crypto-benchmark.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

echo "[node-crypto-benchmark] Running node:crypto focused benchmark..."
START_TIME=$(date +%s)

cargo test -p functions --test node_crypto_streams_async_hooks crypto_microbenchmark_reports_metrics -- --nocapture

END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

echo "[node-crypto-benchmark] Completed in ${ELAPSED}s"
echo "[node-crypto-benchmark] This benchmark reports:"
echo "  - createHash('sha256') throughput and latency"
echo "  - createHmac('sha256') throughput and latency"
echo "  - randomBytes(32) throughput and latency"
