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
#
# Dev deliberately exposes *three* roots, because a single root cannot exercise
# large parts of the app: a cross-root move is copy-then-delete rather than a
# rename, the cross-root drop opens a Copy/Move choice a same-root drop never
# shows, and a read-only share is the only way to see the "you cannot write to
# that share" refusals. They are plain sibling directories — no extra mounts or
# filesystems needed.
mkdir -p "$PROJECT_ROOT/data"
SAMPLE_DIR="$PROJECT_ROOT/data/sample-files"
ARCHIVE_DIR="$PROJECT_ROOT/data/sample-archive"
READONLY_DIR="$PROJECT_ROOT/data/sample-readonly"

# Each root is seeded on its own, so an existing checkout that only has
# sample-files still gets the two newer roots on the next run.
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

if [ ! -d "$ARCHIVE_DIR" ]; then
    echo "==> Creating second writable share in $ARCHIVE_DIR"
    mkdir -p "$ARCHIVE_DIR/2024" "$ARCHIVE_DIR/2025" "$ARCHIVE_DIR/Inbox"
    echo "Archived meeting notes from 2024." > "$ARCHIVE_DIR/2024/notes.txt"
    printf 'year,revenue\n2025,42\n' > "$ARCHIVE_DIR/2025/summary.csv"
fi

if [ ! -d "$READONLY_DIR" ]; then
    echo "==> Creating read-only share in $READONLY_DIR"
    mkdir -p "$READONLY_DIR/Handbook"
    echo "# Read-only share" > "$READONLY_DIR/Handbook/policy.md"
    echo "Files here can be read and copied out, but not written to." >> "$READONLY_DIR/Handbook/policy.md"
fi

# Default common folders — three roots unless overridden.
export COMMON_FOLDERS="${COMMON_FOLDERS:-{\"Files\":\"$SAMPLE_DIR\",\"Archive\":\"$ARCHIVE_DIR\",\"ReadOnly\":\"$READONLY_DIR\"}}"
# Files and Archive are read+write+share; ReadOnly is read-only on purpose.
export SSO_DEFAULT_COMMON_FOLDERS="${SSO_DEFAULT_COMMON_FOLDERS:-Files,Archive}"
export SSO_DEFAULT_FOLDERS_READ="${SSO_DEFAULT_FOLDERS_READ:-ReadOnly}"
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
