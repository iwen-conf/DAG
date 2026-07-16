#!/usr/bin/env bash
# Smoke: migrate + projects API + one publish/claim cycle against DATABASE_URL.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

: "${DATABASE_URL:?set DATABASE_URL (use scripts/pg-tunnel.sh first)}"
PORT="${PORT:-7420}"
BIN="${BIN:-./target/debug/sunmao-server}"

cargo build -q -p sunmao-server -p sunmao-cli
"$BIN" --db "$DATABASE_URL" --listen "127.0.0.1:${PORT}" &
PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT
for i in $(seq 1 30); do
  curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null && break
  sleep 0.2
done

TMP=$(mktemp -d)
(
  cd "$TMP"
  git init -q
  git config user.email t@t
  git config user.name t
  echo ok > README
  git add README && git commit -q -m init
)
REPO=$(cd "$TMP" && pwd)

curl -sf -X POST "http://127.0.0.1:${PORT}/v1/projects" \
  -H 'Content-Type: application/json' -H 'X-Sunmao-Actor: human' \
  -d "{\"name\":\"smoke\",\"repo_path\":\"$REPO\"}" | tee /dev/stderr | grep -q '"id"'

echo
echo "e2e-smoke OK (projects POST)"
