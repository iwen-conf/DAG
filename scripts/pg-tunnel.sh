#!/usr/bin/env bash
# Forward remote Postgres to localhost — do NOT install Postgres locally.
# Usage:
#   ./scripts/pg-tunnel.sh              # default host/port below
#   SSH_HOST=user@host REMOTE_PG=15432 LOCAL_PG=15432 ./scripts/pg-tunnel.sh
set -euo pipefail

SSH_HOST="${SSH_HOST:-root@192.168.1.31}"
REMOTE_PG="${REMOTE_PG:-15432}"
LOCAL_PG="${LOCAL_PG:-15432}"

echo "ssh -N -L ${LOCAL_PG}:127.0.0.1:${REMOTE_PG} ${SSH_HOST}"
echo "Then: export DATABASE_URL='postgres://USER:PASS@127.0.0.1:${LOCAL_PG}/sunmao'"
exec ssh -o ExitOnForwardFailure=yes -N -L "${LOCAL_PG}:127.0.0.1:${REMOTE_PG}" "${SSH_HOST}"
