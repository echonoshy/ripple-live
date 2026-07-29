#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
SOURCE_DIR="${REALTIME_SOURCE_DIR:-${ROOT_DIR}/.cache/services/MiniCPM-o-Demo}"
MODEL_DIR="${MODEL_HOST_PATH:-${ROOT_DIR}/.cache/models/MiniCPM-o-4_5}"
VENV_DIR="${REALTIME_VENV_DIR:-${ROOT_DIR}/.venv-realtime-o45}"
PUBLIC_PORT="${GATEWAY_HOST_PORT:-8600}"
INTERNAL_PORT="${GATEWAY_INTERNAL_PORT:-18007}"
BACKEND_PORT="${BACKEND_PORT:-22500}"
WORKER_PORT="${WORKER_PORT:-22400}"
GPU_DEVICE_ID="${GPU_DEVICE_ID:-1}"
RUN_DIR="${SCRIPT_DIR}/run"
LOG_DIR="${SCRIPT_DIR}/logs"
PYTHON="${VENV_DIR}/bin/python"
USE_USER_SYSTEMD=0

if systemctl --user show-environment >/dev/null 2>&1; then
  USE_USER_SYSTEMD=1
fi

if [[ ! -x "${PYTHON}" ]]; then
  echo "Runtime not found. Run ${SCRIPT_DIR}/setup-baremetal.sh first." >&2
  exit 1
fi
if [[ ! -f "${MODEL_DIR}/config.json" ]]; then
  echo "Model not found at ${MODEL_DIR}" >&2
  exit 1
fi
if [[ ! -f "${SOURCE_DIR}/gateway.py" ]]; then
  echo "Realtime source not found at ${SOURCE_DIR}" >&2
  exit 1
fi

mkdir -p "${RUN_DIR}" "${LOG_DIR}"

pid_is_alive() {
  local pid_file="$1"
  [[ -f "${pid_file}" ]] || return 1
  local pid
  pid="$(<"${pid_file}")"
  [[ "${pid}" =~ ^[0-9]+$ ]] && kill -0 "${pid}" 2>/dev/null
}

if systemctl --user is-active --quiet minicpm-o45-gateway.service 2>/dev/null ||
   systemctl --user is-active --quiet minicpm-o45-backend.service 2>/dev/null ||
   systemctl --user is-active --quiet minicpm-o45-worker.service 2>/dev/null ||
   pid_is_alive "${RUN_DIR}/gateway.pid" ||
   pid_is_alive "${RUN_DIR}/backend.pid" ||
   pid_is_alive "${RUN_DIR}/worker.pid"; then
  echo "One or more realtime processes are already running." >&2
  "${SCRIPT_DIR}/status-baremetal.sh" || true
  exit 1
fi

port_is_free() {
  local port="$1"
  ! ss -ltnH "sport = :${port}" | grep -q .
}

for port in "${PUBLIC_PORT}" "${INTERNAL_PORT}" "${BACKEND_PORT}" "${WORKER_PORT}"; do
  if ! port_is_free "${port}"; then
    echo "TCP port ${port} is already in use." >&2
    exit 1
  fi
done

wait_for_url() {
  local url="$1"
  local label="$2"
  local attempts="$3"
  local process_pid="$4"
  for ((attempt = 1; attempt <= attempts; attempt++)); do
    if curl --fail --silent --show-error --max-time 2 "${url}" >/dev/null 2>&1; then
      return 0
    fi
    if ! kill -0 "${process_pid}" 2>/dev/null; then
      echo "${label} exited before becoming healthy." >&2
      return 1
    fi
    sleep 2
  done
  echo "Timed out waiting for ${label} at ${url}." >&2
  return 1
}

cleanup_on_error() {
  local code="$?"
  if [[ "${code}" -ne 0 ]]; then
    "${SCRIPT_DIR}/stop-baremetal.sh" >/dev/null 2>&1 || true
    echo "Startup failed. Inspect ${LOG_DIR}/*.log" >&2
  fi
  exit "${code}"
}
trap cleanup_on_error EXIT

cd "${SOURCE_DIR}"
export PYTHONPATH="${SOURCE_DIR}"
export PYTHONUNBUFFERED=1
export TORCHINDUCTOR_CACHE_DIR="${ROOT_DIR}/.cache/torch-realtime-o45"
mkdir -p "${TORCHINDUCTOR_CACHE_DIR}"

if [[ "${USE_USER_SYSTEMD}" -eq 1 ]]; then
  systemd-run --user \
    --unit=minicpm-o45-gateway \
    --collect \
    --service-type=exec \
    --property="WorkingDirectory=${SOURCE_DIR}" \
    --property="StandardOutput=append:${LOG_DIR}/gateway.log" \
    --property="StandardError=append:${LOG_DIR}/gateway.log" \
    --setenv="PYTHONPATH=${SOURCE_DIR}" \
    --setenv=PYTHONUNBUFFERED=1 \
    "${PYTHON}" gateway.py \
      --host 0.0.0.0 \
      --port "${PUBLIC_PORT}" \
      --internal-port "${INTERNAL_PORT}" \
      --http
  gateway_pid="$(systemctl --user show minicpm-o45-gateway.service --property=MainPID --value)"
else
  nohup "${PYTHON}" gateway.py \
    --host 0.0.0.0 \
    --port "${PUBLIC_PORT}" \
    --internal-port "${INTERNAL_PORT}" \
    --http \
    >"${LOG_DIR}/gateway.log" 2>&1 &
  gateway_pid="$!"
fi
echo "${gateway_pid}" > "${RUN_DIR}/gateway.pid"
wait_for_url "http://127.0.0.1:${INTERNAL_PORT}/health" "gateway" 30 "${gateway_pid}"

if [[ "${USE_USER_SYSTEMD}" -eq 1 ]]; then
  systemd-run --user \
    --unit=minicpm-o45-backend \
    --collect \
    --service-type=exec \
    --property="WorkingDirectory=${SOURCE_DIR}" \
    --property="StandardOutput=append:${LOG_DIR}/backend.log" \
    --property="StandardError=append:${LOG_DIR}/backend.log" \
    --setenv="PYTHONPATH=${SOURCE_DIR}" \
    --setenv=PYTHONUNBUFFERED=1 \
    --setenv="TORCHINDUCTOR_CACHE_DIR=${TORCHINDUCTOR_CACHE_DIR}" \
    --setenv="CUDA_VISIBLE_DEVICES=${GPU_DEVICE_ID}" \
    "${PYTHON}" -m py_backend.server \
      --host 127.0.0.1 \
      --port "${BACKEND_PORT}" \
      --gpu-id 0 \
      --model-path "${MODEL_DIR}"
  backend_pid="$(systemctl --user show minicpm-o45-backend.service --property=MainPID --value)"
else
  CUDA_VISIBLE_DEVICES="${GPU_DEVICE_ID}" nohup "${PYTHON}" -m py_backend.server \
    --host 127.0.0.1 \
    --port "${BACKEND_PORT}" \
    --gpu-id 0 \
    --model-path "${MODEL_DIR}" \
    >"${LOG_DIR}/backend.log" 2>&1 &
  backend_pid="$!"
fi
echo "${backend_pid}" > "${RUN_DIR}/backend.pid"
wait_for_url "http://127.0.0.1:${BACKEND_PORT}/health" "model backend" 180 "${backend_pid}"

if [[ "${USE_USER_SYSTEMD}" -eq 1 ]]; then
  systemd-run --user \
    --unit=minicpm-o45-worker \
    --collect \
    --service-type=exec \
    --property="WorkingDirectory=${SOURCE_DIR}" \
    --property="StandardOutput=append:${LOG_DIR}/worker.log" \
    --property="StandardError=append:${LOG_DIR}/worker.log" \
    --setenv="PYTHONPATH=${SOURCE_DIR}" \
    --setenv=PYTHONUNBUFFERED=1 \
    --setenv="CUDA_VISIBLE_DEVICES=${GPU_DEVICE_ID}" \
    "${PYTHON}" worker.py \
      --host 127.0.0.1 \
      --port "${WORKER_PORT}" \
      --gpu-id 0 \
      --model-path "${MODEL_DIR}" \
      --backend-server-url "http://127.0.0.1:${BACKEND_PORT}"
  worker_pid="$(systemctl --user show minicpm-o45-worker.service --property=MainPID --value)"
else
  CUDA_VISIBLE_DEVICES="${GPU_DEVICE_ID}" nohup "${PYTHON}" worker.py \
    --host 127.0.0.1 \
    --port "${WORKER_PORT}" \
    --gpu-id 0 \
    --model-path "${MODEL_DIR}" \
    --backend-server-url "http://127.0.0.1:${BACKEND_PORT}" \
    >"${LOG_DIR}/worker.log" 2>&1 &
  worker_pid="$!"
fi
echo "${worker_pid}" > "${RUN_DIR}/worker.pid"
wait_for_url "http://127.0.0.1:${WORKER_PORT}/health" "worker" 45 "${worker_pid}"

curl --fail --silent --show-error \
  --request PUT \
  --header "Content-Type: application/json" \
  --data "{\"endpoint\":\"127.0.0.1:${WORKER_PORT}\",\"gpu_group\":\"gpu-${GPU_DEVICE_ID}\",\"labels\":{\"deployment\":\"baremetal\"}}" \
  "http://127.0.0.1:${INTERNAL_PORT}/internal/workers/worker-gpu-${GPU_DEVICE_ID}" \
  > "${RUN_DIR}/registration.json"

trap - EXIT
echo "MiniCPM-o realtime service is ready."
echo "Audio: ws://140.143.229.103:${PUBLIC_PORT}/v1/realtime?mode=audio"
echo "Video: ws://140.143.229.103:${PUBLIC_PORT}/v1/realtime?mode=video"
