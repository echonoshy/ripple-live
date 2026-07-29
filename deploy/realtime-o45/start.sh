#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"

export MODEL_HOST_PATH="${MODEL_HOST_PATH:-${ROOT_DIR}/.cache/models/MiniCPM-o-4_5}"
export REALTIME_SOURCE_DIR="${REALTIME_SOURCE_DIR:-${ROOT_DIR}/.cache/services/MiniCPM-o-Demo}"
export GATEWAY_HOST_PORT="${GATEWAY_HOST_PORT:-8600}"
export GPU_DEVICE_ID="${GPU_DEVICE_ID:-1}"

if [[ ! -f "${MODEL_HOST_PATH}/config.json" ]]; then
  echo "Model not found at ${MODEL_HOST_PATH}" >&2
  exit 1
fi

if [[ ! -f "${REALTIME_SOURCE_DIR}/gateway.py" ]]; then
  echo "Realtime source not found. Run ${SCRIPT_DIR}/setup.sh first." >&2
  exit 1
fi

if ! docker info >/dev/null 2>&1; then
  echo "Docker is not accessible to the current user." >&2
  echo "Run this script as a Docker-enabled user or ask an administrator to add the user to the docker group." >&2
  exit 1
fi

mkdir -p "${SCRIPT_DIR}/data"
exec "${SCRIPT_DIR}/compose.sh" \
  --file "${SCRIPT_DIR}/docker-compose.yml" \
  up --detach --build
