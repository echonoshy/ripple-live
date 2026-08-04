#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_DIR="$SCRIPT_DIR/run"

for name in gateway tts agent asr; do
  unit="ripple-agent-$name.service"
  if systemctl --user is-active --quiet "$unit" 2>/dev/null; then
    systemctl --user stop "$unit"
    echo "stopped $name (systemd user service)"
  fi
  pid_file="$RUN_DIR/$name.pid"
  [[ -f "$pid_file" ]] || continue
  pid="$(cat "$pid_file")"
  if kill -0 "$pid" 2>/dev/null &&
     ! systemctl --user is-active --quiet "$unit" 2>/dev/null; then
    kill "$pid"
    echo "stopped $name (pid $pid)"
  fi
  rm -f "$pid_file"
done
