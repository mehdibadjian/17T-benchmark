#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

echo "=== Xiaomi 17T Multi-Core Scaling & Thermal Check ==="
"$WORKSPACE_ROOT/target/release/nimble-shell" bench --scaling --duration 2
