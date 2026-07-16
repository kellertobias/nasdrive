#!/usr/bin/env bash
set -euo pipefail

# Start a development environment with hot-reloading for both frontend and backend.
#
# - Backend: cargo watch (rebuilds on Rust changes)
# - Frontend: vite dev server with proxy to backend
#
# Prerequisites: cargo-watch (`cargo install cargo-watch`)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Dev defaults ──────────────────────────────────────────────────
export BIND_ADDR="${BIND_ADDR:-0.0.0.0:8080}"
export BASE_URL="${BASE_URL:-http://localhost:5173}"
export DB_URL="${DB_URL:-sqlite://$PROJECT_ROOT/data/nasfiles.db?mode=rwc}"
export NASFILES_DEV="${NASFILES_DEV:-1}"
export DEV_USER_ID="${DEV_USER_ID:-dev-user}"
export DEV_USER_NAME="${DEV_USER_NAME:-developer}"
export DEV_USER_DISPLAY="${DEV_USER_DISPLAY:-Developer}"
export RUST_LOG="${RUST_LOG:-info}"

# Create data directory and sample files for development
mkdir -p "$PROJECT_ROOT/data"
SAMPLE_DIR="$PROJECT_ROOT/data/sample-files"
if [ ! -d "$SAMPLE_DIR" ]; then
    echo "==> Creating sample files in $SAMPLE_DIR"
    mkdir -p "$SAMPLE_DIR/Documents" "$SAMPLE_DIR/Photos" "$SAMPLE_DIR/Projects" "$SAMPLE_DIR/Media"
    echo "# Welcome to NASDrive" > "$SAMPLE_DIR/Documents/README.md"
    echo "This is a sample text file for testing." > "$SAMPLE_DIR/Documents/notes.txt"
    echo '{"key": "value", "nested": {"a": 1}}' > "$SAMPLE_DIR/Documents/config.json"
    echo "fn main() { println!(\"Hello from NASDrive!\"); }" > "$SAMPLE_DIR/Projects/main.rs"
    echo "console.log('hello world');" > "$SAMPLE_DIR/Projects/index.js"
    
    if [ -d "$PROJECT_ROOT/test-data" ]; then
        cp -r "$PROJECT_ROOT/test-data/"* "$SAMPLE_DIR/"
    fi
fi

# Default common folders — points to sample data unless overridden
export COMMON_FOLDERS="${COMMON_FOLDERS:-{\"Files\":\"$SAMPLE_DIR\"}}"
export SSO_DEFAULT_COMMON_FOLDERS="${SSO_DEFAULT_COMMON_FOLDERS:-Files}"
export SSO_ADMIN_GROUPS="${SSO_ADMIN_GROUPS:-STAFF}"

# Check if cargo-watch is installed
if ! command -v cargo-watch &> /dev/null; then
    echo "cargo-watch not found, install with: cargo install cargo-watch"
    echo "Falling back to manual cargo run..."
    echo ""
fi

cleanup() {
    echo ""
    echo "==> Shutting down..."
    kill 0 2>/dev/null || true
}
trap cleanup EXIT

echo "==> Starting dev environment"
echo "    Backend:  http://localhost:${BIND_ADDR##*:}"
echo "    Frontend: http://localhost:5173 (Vite proxy → backend)"
echo ""

# Start backend
if command -v cargo-watch &> /dev/null; then
    (cd "$PROJECT_ROOT" && cargo watch -x 'run --bin nasfiles' -w crates/) &
else
    (cd "$PROJECT_ROOT" && cargo run --bin nasfiles) &
fi

# Wait a moment for backend to start
sleep 2

# Start Vite dev server
(cd "$PROJECT_ROOT/web" && npm run dev) &

wait
