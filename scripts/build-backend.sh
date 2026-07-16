#!/usr/bin/env bash
set -euo pipefail

# Build the NASDrive backend (Rust).
# Embeds the frontend via rust-embed, so you must build the frontend first.
#
# Usage:
#   ./scripts/build-backend.sh          # debug build
#   ./scripts/build-backend.sh release  # optimized release build

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Ensure frontend is built
if [ ! -d "$PROJECT_ROOT/web/dist" ]; then
    echo "==> Frontend not built yet, building first..."
    "$SCRIPT_DIR/build-frontend.sh"
fi

MODE="${1:-debug}"

cd "$PROJECT_ROOT"

if [ "$MODE" = "release" ]; then
    echo "==> Building backend (release)..."
    cargo build --release
    BINARY="target/release/nasfiles"
else
    echo "==> Building backend (debug)..."
    cargo build
    BINARY="target/debug/nasfiles"
fi

echo "==> Backend built → $BINARY"
ls -lh "$BINARY"
