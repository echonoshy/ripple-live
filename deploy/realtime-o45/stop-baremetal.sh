#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
RUN_DIR="${SCRIPT_DIR}/run"

for unit in minicpm-o45-worker.service minicpm-o45-backend.service minicpm-o45-gateway.service; do
  if systemctl --user is-active --quiet "${unit}" 2>/dev/null; then
    systemctl --user stop "${unit}"
    echo "Stopped ${unit}."
  fi
done

stop_pid_file() {
  local name="$1"
  local pid_file="${RUN_DIR}/${name}.pid"
  [[ -f "${pid_file}" ]] || return 0

  local pid
  pid="$(<"${pid_file}")"
  if [[ "${pid}" =~ ^[0-9]+$ ]] && kill -0 "${pid}" 2>/dev/null; then
    kill "${pid}"
    for _ in {1..20}; do
      kill -0 "${pid}" 2>/dev/null || break
      sleep 0.25
    done
    if kill -0 "${pid}" 2>/dev/null; then
      kill -KILL "${pid}"
    fi
    echo "Stopped ${name} (PID ${pid})."
  fi
  rm -f -- "${pid_file}"
}

stop_pid_file worker
stop_pid_file backend
stop_pid_file gateway
rm -f -- "${RUN_DIR}/registration.json"
