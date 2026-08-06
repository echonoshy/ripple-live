#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_DIR="$SCRIPT_DIR/run"

for name in asr agent tts gateway; do
  unit="ripple-agent-$name.service"
  if systemctl --user is-active --quiet "$unit" 2>/dev/null; then
    pid="$(systemctl --user show "$unit" --property=MainPID --value)"
    echo "$name: running (systemd user service, pid $pid)"
    continue
  fi
  pid_file="$RUN_DIR/$name.pid"
  if [[ -f "$pid_file" ]] && kill -0 "$(cat "$pid_file")" 2>/dev/null; then
    echo "$name: running (pid $(cat "$pid_file"))"
  else
    echo "$name: stopped"
  fi
done

echo
if command -v "${RIPPLE_FFMPEG_BIN:-ffmpeg}" >/dev/null 2>&1; then
  echo "ffmpeg: available"
else
  echo "ffmpeg: unavailable"
fi

echo
if curl --silent --fail --max-time 2 "http://127.0.0.1:8700/health" >/dev/null; then
  echo "gateway liveness: ok"
else
  echo "gateway liveness: unavailable"
fi
if curl --silent --fail --max-time 2 "http://127.0.0.1:8700/ready" >/dev/null; then
  echo "gateway readiness: ok"
else
  echo "gateway readiness: unavailable"
fi

echo
for endpoint in \
  "asr http://127.0.0.1:8711/health" \
  "agent http://127.0.0.1:8712/health" \
  "tts http://127.0.0.1:8723/health" \
  "gateway http://127.0.0.1:8700/health"; do
  read -r name url <<<"$endpoint"
  if curl --silent --fail --max-time 2 "$url" >/dev/null; then
    echo "$name health: ok"
  else
    echo "$name health: unavailable"
  fi
done
