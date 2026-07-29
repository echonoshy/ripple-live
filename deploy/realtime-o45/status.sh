#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"

export MODEL_HOST_PATH="${MODEL_HOST_PATH:-${ROOT_DIR}/.cache/models/MiniCPM-o-4_5}"
export REALTIME_SOURCE_DIR="${REALTIME_SOURCE_DIR:-${ROOT_DIR}/.cache/services/MiniCPM-o-Demo}"
export GATEWAY_HOST_PORT="${GATEWAY_HOST_PORT:-8600}"
export GPU_DEVICE_ID="${GPU_DEVICE_ID:-1}"

"${SCRIPT_DIR}/compose.sh" --file "${SCRIPT_DIR}/docker-compose.yml" ps
curl --fail --silent --show-error \
  "http://127.0.0.1:${GATEWAY_HOST_PORT}/docs" \
  >/dev/null
echo "Gateway is reachable at http://127.0.0.1:${GATEWAY_HOST_PORT}"
