#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

echo "=== Xiaomi 17T Full Multi-Subagent Benchmark ==="
"$WORKSPACE_ROOT/target/release/nimble-shell" sysinfo
echo ""
"$WORKSPACE_ROOT/target/release/nimble-shell" bench --all --agents 4 --duration 2
