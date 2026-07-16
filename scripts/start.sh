#!/usr/bin/env bash
set -euo pipefail

# Start the NASDrive server with sensible dev defaults.
# The binary must already be built (run ./scripts/build.sh first).
#
# Override any env var before calling this script:
#   BIND_ADDR=0.0.0.0:3000 ./scripts/start.sh

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Prefer release binary, fall back to debug
if [ -x "$PROJECT_ROOT/target/release/nasfiles" ]; then
    BINARY="$PROJECT_ROOT/target/release/nasfiles"
elif [ -x "$PROJECT_ROOT/target/debug/nasfiles" ]; then
    BINARY="$PROJECT_ROOT/target/debug/nasfiles"
else
    echo "Error: no built binary found. Run ./scripts/build.sh first."
    exit 1
fi

echo "==> Using binary: $BINARY"

# ── Dev defaults (override via env) ──────────────────────────────
export BIND_ADDR="${BIND_ADDR:-0.0.0.0:8080}"
export BASE_URL="${BASE_URL:-http://localhost:8080}"

# SQLite database (dev default — auto-created)
export DB_URL="${DB_URL:-sqlite://$PROJECT_ROOT/data/nasfiles.db?mode=rwc}"

# Create data directory if needed
mkdir -p "$PROJECT_ROOT/data"

# Enable dev mode (auth bypass) unless explicitly configured
export NASFILES_DEV="${NASFILES_DEV:-1}"

# Dev user — used when DEV_MODE=true and no SSO is configured
export DEV_USER_ID="${DEV_USER_ID:-dev-user}"
export DEV_USER_NAME="${DEV_USER_NAME:-developer}"
export DEV_USER_DISPLAY="${DEV_USER_DISPLAY:-Developer}"

# Common folders — empty by default, set your own:
#   export COMMON_FOLDERS='{"Media":"/path/to/media","Documents":"/path/to/docs"}'
export COMMON_FOLDERS="${COMMON_FOLDERS:-{}}"

# Logging
export RUST_LOG="${RUST_LOG:-info}"

echo "==> Starting NASDrive on $BIND_ADDR"
echo "    Base URL:  $BASE_URL"
echo "    Database:  $DB_URL"
echo "    Dev mode:  $NASFILES_DEV"
echo ""

exec "$BINARY"
