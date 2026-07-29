#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
LOCAL_COMPOSE="${ROOT_DIR}/.cache/tools/docker-compose"

if docker compose version >/dev/null 2>&1; then
  exec docker compose "$@"
fi

if [[ -x "${LOCAL_COMPOSE}" ]]; then
  exec "${LOCAL_COMPOSE}" "$@"
fi

echo "Docker Compose is unavailable. Run ${SCRIPT_DIR}/setup.sh first." >&2
exit 1
