#!/bin/bash

# Deploy ESZIP bundles and run load tests
# Usage: ./scripts/deploy-and-test-eszip.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

SERVER_URL="http://localhost:9000"
BUNDLE_DIR="$PROJECT_ROOT/bundles/eszip"

echo "🚀 ESZIP Bundle Load Test"
echo "════════════════════════════════════════════════════════════"
echo ""

# Check if bundles exist
if [ ! -d "$BUNDLE_DIR" ]; then
    echo "❌ Bundle directory not found: $BUNDLE_DIR"
    echo "   Please run: ./scripts/bundle-eszip.sh"
    exit 1
fi

# Check if server is running
echo "✅ Checking if server is running on $SERVER_URL..."
if ! curl -s -f "$SERVER_URL/_internal/metrics" > /dev/null 2>&1; then
    echo "❌ Server not responding on $SERVER_URL"
    echo "   Please start the server with: cargo run --release --bin server"
    exit 1
fi

echo "✅ Server is running"
echo ""

# Deploy ESZIP bundles
echo "📦 Deploying ESZIP bundles..."
echo "────────────────────────────────────────────────────────────"

deployed_count=0
failed_count=0

for bundle in "$BUNDLE_DIR"/*.pkg; do
    if [ -f "$bundle" ]; then
        example_name=$(basename "$bundle" .pkg)

        echo -n "📝 $example_name... "

        response=$(curl -s -w "\n%{http_code}" -X POST "$SERVER_URL/_internal/functions" \
            -H "content-type: application/octet-stream" \
            -H "x-function-name: $example_name" \
            --data-binary "@$bundle")

        status=$(echo "$response" | tail -n1)

        if [ "$status" = "201" ] || [ "$status" = "200" ]; then
            echo "✅"
            ((deployed_count++))
        else
            echo "❌ (status: $status)"
            ((failed_count++))
        fi
    fi
done

echo ""
echo "📊 Deployed: $deployed_count | Failed: $failed_count"
echo ""

if [ $deployed_count -eq 0 ]; then
    echo "❌ No bundles deployed. Aborting test."
    exit 1
fi

# Wait for functions to be ready
echo "⏳ Waiting for functions to initialize..."
sleep 3

# Run load test with k6
echo "🔥 Starting load test with k6..."
echo "════════════════════════════════════════════════════════════"
echo ""

if command -v k6 &> /dev/null; then
    k6 run "$SCRIPT_DIR/load-test.js" \
        --vus 10 \
        --duration 60s \
        -e BASE_URL="$SERVER_URL"
else
    echo "⚠️  k6 not found. Please install k6:"
    echo "   macOS: brew install k6"
    echo "   Linux: https://k6.io/docs/getting-started/installation/"
    exit 1
fi

echo ""
echo "════════════════════════════════════════════════════════════"
echo "✅ ESZIP load test completed"
echo ""

# Fetch final metrics
echo "📊 Final Metrics:"
echo "────────────────────────────────────────────────────────────"

if curl -s "$SERVER_URL/_internal/metrics" | grep -q "functions"; then
    curl -s "$SERVER_URL/_internal/metrics" | jq '.functions[] | {name, status, metrics}' 2>/dev/null || \
    curl -s "$SERVER_URL/_internal/metrics" | python3 -m json.tool 2>/dev/null || \
    curl -s "$SERVER_URL/_internal/metrics"
fi

echo ""
