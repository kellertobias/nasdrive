#!/usr/bin/env bash
set -euo pipefail

# Build the NASDrive frontend (Vite + React)
# Output: web/dist/

cd "$(dirname "$0")/../web"

echo "==> Installing frontend dependencies..."
npm ci --prefer-offline 2>/dev/null || npm install

echo "==> Building frontend..."
npx vite build

echo "==> Frontend built → web/dist/"
ls -lh dist/assets/ 2>/dev/null || true
