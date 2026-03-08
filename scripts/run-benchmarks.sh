#!/bin/bash

# Master benchmark script - Build, bundle, deploy and test ESZIP format
# Usage: ./scripts/run-benchmarks.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

print_header() {
    echo -e "${BLUE}════════════════════════════════════════════════════════════$(tput sgr0)"
    echo -e "${BLUE}$1$(tput sgr0)"
    echo -e "${BLUE}════════════════════════════════════════════════════════════$(tput sgr0)"
}

print_section() {
    echo -e "${YELLOW}$1$(tput sgr0)"
}

print_success() {
    echo -e "${GREEN}✅ $1$(tput sgr0)"
}

print_error() {
    echo -e "${RED}❌ $1$(tput sgr0)"
}

# Start time
START_TIME=$(date +%s)

print_header "🚀 DENO EDGE RUNTIME - COMPREHENSIVE BENCHMARKS"
echo ""

# Step 1: Build the project
print_section "Step 1/4: Building the project..."
cd "$PROJECT_ROOT"

if cargo build --release 2>&1 | tail -20; then
    print_success "Project built successfully"
else
    print_error "Build failed"
    exit 1
fi

echo ""

# Step 2: Bundle ESZIP
print_section "Step 2/4: Bundling examples (ESZIP format)..."
if bash "$SCRIPT_DIR/bundle-eszip.sh"; then
    print_success "ESZIP bundles created"
else
    print_error "ESZIP bundling failed"
    exit 1
fi

echo ""

# Step 3: Start server
print_section "Step 3/4: Starting thunder server..."
SERVER_PID=""

# Kill any existing server
pkill -f "thunder.*start" 2>/dev/null || true
sleep 1

# Start server in background
"$PROJECT_ROOT/target/release/thunder" start --host 0.0.0.0 --port 9000 > /tmp/thunder.log 2>&1 &
SERVER_PID=$!

echo "Server PID: $SERVER_PID"

# Wait for server to be ready
echo "Waiting for server to start..."
sleep 3

# Check if server is running
if ! curl -s -f "http://localhost:9000/_internal/metrics" > /dev/null 2>&1; then
    print_error "Server failed to start"
    cat /tmp/thunder.log
    exit 1
fi

print_success "Server started and ready"
echo ""

# Cleanup function
cleanup() {
    echo ""
    print_section "Cleaning up..."
    if [ -n "$SERVER_PID" ]; then
        kill $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true
    fi
    print_success "Server stopped"
}

trap cleanup EXIT

# Step 4: Run ESZIP benchmark
print_section "Step 4/4: Running ESZIP benchmark..."
echo ""

if bash "$SCRIPT_DIR/deploy-and-test-eszip.sh"; then
    print_success "ESZIP benchmark completed"
else
    print_error "ESZIP benchmark failed"
    exit 1
fi

if [[ "${RUN_EXTREME_CONTEXT_BENCH:-0}" == "1" ]]; then
    echo ""
    print_section "Optional: Running extreme context+isolate comparative benchmark..."
    if bash "$SCRIPT_DIR/benchmark-context-isolate-extreme.sh"; then
        print_success "Extreme context+isolate benchmark completed"
    else
        print_error "Extreme context+isolate benchmark failed"
        exit 1
    fi
fi

# Calculate elapsed time
END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

print_header "✨ BENCHMARKS COMPLETED SUCCESSFULLY"
echo ""
echo "📈 Summary:"
echo "   Total time: ${ELAPSED}s"
echo "   ESZIP bundles: $PROJECT_ROOT/bundles/eszip"
echo "   Metrics log: /tmp/thunder.log"
echo "   Extreme context+isolate bench: RUN_EXTREME_CONTEXT_BENCH=1 ./scripts/run-benchmarks.sh"
echo ""

print_success "All benchmarks completed!"
