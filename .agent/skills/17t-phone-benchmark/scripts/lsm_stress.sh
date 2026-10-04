#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

echo "=== Xiaomi 17T LSM Storage & Concurrency Stress Test ==="
"$WORKSPACE_ROOT/target/release/turing-server" --bench 20000 8
