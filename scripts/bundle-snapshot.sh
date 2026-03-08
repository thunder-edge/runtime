#!/bin/bash

# Bundle all TypeScript files in examples directory using snapshot format.
# Usage: ./scripts/bundle-snapshot.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
CLI_BIN="$PROJECT_ROOT/target/debug/thunder"

if [ ! -f "$CLI_BIN" ]; then
	echo "❌ Error: $CLI_BIN not found. Please run 'cargo build' first"
	exit 1
fi

mkdir -p "$PROJECT_ROOT/bundles/snapshot"

echo "📦 Bundling examples with snapshot format (with ESZIP fallback)..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

bundled_count=0
failed_count=0

for file in "$PROJECT_ROOT"/examples/*/*.ts; do
	if [ -f "$file" ]; then
		example_name=$(basename "$(dirname "$file")")
		output="$PROJECT_ROOT/bundles/snapshot/${example_name}.pkg"

		echo -n "🧪 $example_name... "

		if "$CLI_BIN" bundle -e "$file" -o "$output" --format snapshot 2>/dev/null; then
			size=$(du -h "$output" | cut -f1)
			echo "✅ ($size)"
			((bundled_count++))
		else
			echo "❌ Failed"
			((failed_count++))
		fi
	fi
done

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ Snapshot bundles gerados: $bundled_count"
if [ $failed_count -gt 0 ]; then
	echo "❌ Falhas: $failed_count"
	exit 1
fi
echo ""
echo "📂 Output directory: $PROJECT_ROOT/bundles/snapshot"
