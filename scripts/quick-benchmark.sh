#!/bin/bash

# Quick benchmark runner - assumes build and bundles already exist
# For quick re-runs without rebuilding everything
# Usage: ./scripts/quick-benchmark.sh [eszip|snapshot|both]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_header() {
    echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
}

print_section() {
    echo -e "${YELLOW}$1${NC}"
}

print_success() {
    echo -e "${GREEN}✅ $1${NC}"
}

print_error() {
    echo -e "${RED}❌ $1${NC}"
}

# Parse arguments
TEST_TYPE="${1:-both}"

if [[ ! "$TEST_TYPE" =~ ^(eszip|snapshot|both)$ ]]; then
    echo "Usage: $0 [eszip|snapshot|both]"
    exit 1
fi

print_header "⚡ QUICK BENCHMARK RUNNER"
echo ""

# Check if binaries exist
if [ ! -f "$PROJECT_ROOT/target/release/deno-edge-runtime" ]; then
    print_error "Binary not found. Please run: cargo build --release"
    exit 1
fi

# Start server
print_section "Starting server..."
pkill -f "deno-edge-runtime.*start" 2>/dev/null || true
sleep 1

"$PROJECT_ROOT/target/release/deno-edge-runtime" start --host 0.0.0.0 --port 9000 > /tmp/edge-runtime.log 2>&1 &
SERVER_PID=$!

sleep 3

if ! curl -s -f "http://localhost:9000/_internal/metrics" > /dev/null 2>&1; then
    print_error "Server failed to start"
    cat /tmp/edge-runtime.log
    kill $SERVER_PID 2>/dev/null || true
    exit 1
fi

print_success "Server running (PID: $SERVER_PID)"
echo ""

# Cleanup function
cleanup() {
    echo ""
    print_section "Cleaning up..."
    kill $SERVER_PID 2>/dev/null || true
    wait $SERVER_PID 2>/dev/null || true
    print_success "Server stopped"
}

trap cleanup EXIT

START_TIME=$(date +%s)

# Run tests
if [[ "$TEST_TYPE" == "eszip" || "$TEST_TYPE" == "both" ]]; then
    print_section "Running ESZIP benchmark..."
    echo ""
    bash "$SCRIPT_DIR/deploy-and-test-eszip.sh"
    echo ""

    if [[ "$TEST_TYPE" == "both" ]]; then
        print_section "Waiting before next test..."
        curl -s -X DELETE "http://localhost:9000/_internal/functions" 2>/dev/null || true
        sleep 5
    fi
fi

if [[ "$TEST_TYPE" == "snapshot" || "$TEST_TYPE" == "both" ]]; then
    print_section "Running SNAPSHOT benchmark..."
    echo ""
    bash "$SCRIPT_DIR/deploy-and-test-snapshot.sh"
    echo ""
fi

END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

print_header "✨ QUICK BENCHMARK COMPLETED"
echo "⏱️  Time elapsed: ${ELAPSED}s"
