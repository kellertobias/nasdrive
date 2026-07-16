#!/usr/bin/env bash
set -euo pipefail

# Build everything: frontend → backend (release)
# Produces a single binary at target/release/nasfiles that embeds the SPA.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "=== Building NASDrive ==="
echo ""

"$SCRIPT_DIR/build-frontend.sh"
echo ""
"$SCRIPT_DIR/build-backend.sh" release

echo ""
echo "=== Done ==="
echo "Binary: target/release/nasfiles"
echo "Run:    ./scripts/start.sh"
