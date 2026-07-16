#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

if [ ! -d "$PROJECT_ROOT/demo" ]; then
  echo "demo/ is missing. Generate the demo fixtures before starting the screenshot demo." >&2
  exit 1
fi

if [ ! -f "$PROJECT_ROOT/web/dist/index.html" ]; then
  echo "==> Building frontend"
  (cd "$PROJECT_ROOT/web" && npm run build)
fi

mkdir -p "$PROJECT_ROOT/data/demo-runtime"

DEMO_ROOT="$PROJECT_ROOT/demo/filesystem"
if [ ! -d "$DEMO_ROOT" ]; then
  DEMO_ROOT="$PROJECT_ROOT/demo"
fi

COMMON_FOLDERS_JSON="$(
  DEMO_ROOT="$DEMO_ROOT" python3 - <<'PY'
import json
import os
from pathlib import Path

root = Path(os.environ["DEMO_ROOT"])
shares = {
    path.name: str(path)
    for path in sorted(root.iterdir())
    if path.is_dir()
}
print(json.dumps(shares))
PY
)"
COMMON_FOLDER_NAMES="$(
  COMMON_FOLDERS_JSON="$COMMON_FOLDERS_JSON" python3 - <<'PY'
import json
import os

print(",".join(json.loads(os.environ["COMMON_FOLDERS_JSON"]).keys()))
PY
)"

export BIND_ADDR="${BIND_ADDR:-127.0.0.1:8080}"
export BASE_URL="${BASE_URL:-http://127.0.0.1:8080}"
export DATA_DIR="${DATA_DIR:-$PROJECT_ROOT/data/demo-runtime}"
export DB_URL="${DB_URL:-sqlite://$PROJECT_ROOT/data/demo-runtime/nasfiles-demo.db?mode=rwc}"
export COMMON_FOLDERS="${COMMON_FOLDERS:-$COMMON_FOLDERS_JSON}"
export AUTH_MODE="${AUTH_MODE:-local}"
export NASFILES_DEV="${NASFILES_DEV:-0}"
export NASFILES_ALLOW_INSECURE_LOCAL_HTTP="${NASFILES_ALLOW_INSECURE_LOCAL_HTTP:-1}"
export SESSION_SECRET="${SESSION_SECRET:-00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000}"
export SETUP_ADMIN_USER="${SETUP_ADMIN_USER:-admin}"
export SETUP_ADMIN_PASSWORD="${SETUP_ADMIN_PASSWORD:-12345678}"
export SETUP_ADMIN_DISPLAY_NAME="${SETUP_ADMIN_DISPLAY_NAME:-Tobias S. Keller}"
export NASFILES_DEMO_ALLOW_WEAK_ADMIN_PASSWORD="${NASFILES_DEMO_ALLOW_WEAK_ADMIN_PASSWORD:-1}"
export DISABLE_PASSKEYS="${DISABLE_PASSKEYS:-true}"
export DEV_USER_ID="${DEV_USER_ID:-demo-user}"
export DEV_USER_NAME="${DEV_USER_NAME:-demo}"
export DEV_USER_DISPLAY="${DEV_USER_DISPLAY:-Demo User}"
export SSO_DEFAULT_COMMON_FOLDERS="${SSO_DEFAULT_COMMON_FOLDERS:-$COMMON_FOLDER_NAMES}"
export SSO_ADMIN_GROUPS="${SSO_ADMIN_GROUPS:-STAFF}"
export NASFILES_DEMO_TRANSFER_DELAY_MS="${NASFILES_DEMO_TRANSFER_DELAY_MS:-600}"
export RUST_LOG="${RUST_LOG:-info}"

echo "==> Starting NASDrive screenshot demo"
echo "    URL:    $BASE_URL"
echo "    Folder: $DEMO_ROOT"
echo "    Shares: $SSO_DEFAULT_COMMON_FOLDERS"
echo "    Transfer delay: ${NASFILES_DEMO_TRANSFER_DELAY_MS}ms per copied chunk"
echo "    Login:  $SETUP_ADMIN_USER / $SETUP_ADMIN_PASSWORD"
echo ""

cd "$PROJECT_ROOT"
exec cargo run --bin nasfiles
