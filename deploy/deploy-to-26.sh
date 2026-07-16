#!/usr/bin/env bash
# Deploy sunmao-server to nulltech@192.168.1.26:30022
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOST="${DEPLOY_HOST:-nulltech@192.168.1.26}"
PORT="${DEPLOY_SSH_PORT:-30022}"
REMOTE_DIR="${REMOTE_DIR:-/home/nulltech/sunmao}"

echo "== rsync → ${HOST}:${REMOTE_DIR} =="
ssh -p "$PORT" "$HOST" "mkdir -p '$REMOTE_DIR'"
rsync -az --delete \
  -e "ssh -p $PORT" \
  --exclude target \
  --exclude .git \
  --exclude .ai-code-index \
  --exclude '**/target' \
  "$ROOT/" "$HOST:$REMOTE_DIR/"

echo "== ensure sunmao database =="
ssh -p "$PORT" "$HOST" 'docker exec csxs-postgres psql -U novel -d novel -tc "SELECT 1 FROM pg_database WHERE datname='\''sunmao'\''" | grep -q 1 \
  || docker exec csxs-postgres psql -U novel -d novel -c "CREATE DATABASE sunmao OWNER novel;"'

echo "== docker compose build & up =="
ssh -p "$PORT" "$HOST" "cd '$REMOTE_DIR' && docker compose -f deploy/docker-compose.yml up -d --build"

echo "== health =="
sleep 2
ssh -p "$PORT" "$HOST" 'curl -sf http://127.0.0.1:7420/health && echo && curl -sf -o /dev/null -w "ui=%{http_code}\n" http://127.0.0.1:7420/ui/'
echo "UI: http://192.168.1.26:7420/ui/"
echo "API: http://192.168.1.26:7420/v1/projects  (header X-Sunmao-Actor: human)"
echo "DONE"
