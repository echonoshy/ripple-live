#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
RUN_DIR="${SCRIPT_DIR}/run"
PUBLIC_PORT="${GATEWAY_HOST_PORT:-8600}"
INTERNAL_PORT="${GATEWAY_INTERNAL_PORT:-18007}"
BACKEND_PORT="${BACKEND_PORT:-22500}"
WORKER_PORT="${WORKER_PORT:-22400}"

show_process() {
  local name="$1"
  local pid_file="${RUN_DIR}/${name}.pid"
  local unit="minicpm-o45-${name}.service"
  if systemctl --user is-active --quiet "${unit}" 2>/dev/null; then
    local systemd_pid
    systemd_pid="$(systemctl --user show "${unit}" --property=MainPID --value)"
    echo "${name}: running (systemd user service, PID ${systemd_pid})"
    return
  fi
  if [[ -f "${pid_file}" ]]; then
    local pid
    pid="$(<"${pid_file}")"
    if [[ "${pid}" =~ ^[0-9]+$ ]] && kill -0 "${pid}" 2>/dev/null; then
      echo "${name}: running (PID ${pid})"
      return
    fi
  fi
  echo "${name}: stopped"
}

show_url() {
  local label="$1"
  local url="$2"
  if response="$(curl --fail --silent --show-error --max-time 2 "${url}" 2>/dev/null)"; then
    echo "${label}: healthy ${response}"
  else
    echo "${label}: unavailable"
  fi
}

show_process gateway
show_process backend
show_process worker
show_url gateway-internal "http://127.0.0.1:${INTERNAL_PORT}/health"
show_url backend "http://127.0.0.1:${BACKEND_PORT}/health"
show_url worker "http://127.0.0.1:${WORKER_PORT}/health"
show_url gateway-public "http://127.0.0.1:${PUBLIC_PORT}/health"
