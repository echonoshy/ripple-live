#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MODEL_ROOT="${MODEL_ROOT:-$REPO_ROOT/.cache/models}"
RUN_DIR="$SCRIPT_DIR/run"
LOG_DIR="$SCRIPT_DIR/logs"
USE_USER_SYSTEMD=0

if systemctl --user show-environment >/dev/null 2>&1; then
  USE_USER_SYSTEMD=1
fi

if [[ -f "$SCRIPT_DIR/.env" ]]; then
  set -a
  source "$SCRIPT_DIR/.env"
  set +a
fi

ASR_GPU="${ASR_GPU:-2}"
AGENT_GPU="${AGENT_GPU:-3}"
TTS_OMNI_GPU="${TTS_OMNI_GPU:-7}"
TTS_OMNI_MODEL="${TTS_OMNI_MODEL:-Qwen3-TTS-12Hz-1.7B-CustomVoice}"
GATEWAY_DATA_DIR="${RIPPLE_DATA_DIR:-$REPO_ROOT/.cache/agent-gateway}"
if [[ "$GATEWAY_DATA_DIR" != /* ]]; then
  GATEWAY_DATA_DIR="$REPO_ROOT/$GATEWAY_DATA_DIR"
fi
mkdir -p "$RUN_DIR" "$LOG_DIR"

start_process() {
  local name="$1"
  shift
  local unit="ripple-agent-$name.service"
  if systemctl --user is-active --quiet "$unit" 2>/dev/null; then
    echo "$name is already running"
    return
  fi
  if [[ -f "$RUN_DIR/$name.pid" ]] &&
     kill -0 "$(cat "$RUN_DIR/$name.pid")" 2>/dev/null; then
    echo "$name is already running"
    return
  fi
  rm -f "$RUN_DIR/$name.pid"
  touch "$LOG_DIR/$name.log"
  if [[ "$USE_USER_SYSTEMD" -eq 1 ]]; then
    systemd-run --user \
      --unit="ripple-agent-$name" \
      --collect \
      --service-type=exec \
      --property="WorkingDirectory=$REPO_ROOT" \
      --property="StandardOutput=append:$LOG_DIR/$name.log" \
      --property="StandardError=append:$LOG_DIR/$name.log" \
      --setenv=PYTHONUNBUFFERED=1 \
      "$@" >/dev/null
    local pid
    pid="$(systemctl --user show "$unit" --property=MainPID --value)"
    echo "$pid" >"$RUN_DIR/$name.pid"
    echo "started $name (systemd user service, pid $pid)"
  else
    PYTHONUNBUFFERED=1 nohup "$@" >"$LOG_DIR/$name.log" 2>&1 &
    echo "$!" >"$RUN_DIR/$name.pid"
    echo "started $name (pid $!)"
  fi
}

warm_tts() {
  local health_url="http://127.0.0.1:8723/health"
  local speech_url="http://127.0.0.1:8723/v1/audio/speech"
  local payload
  payload="$(printf '{"model":"%s","input":"语音服务预热完成。","voice":"serena","language":"Chinese","response_format":"pcm","stream":true,"stream_format":"audio"}' "$TTS_OMNI_MODEL")"

  for _ in $(seq 1 180); do
    if curl --fail --silent --max-time 2 "$health_url" >/dev/null; then
      if curl --fail --silent --show-error --max-time 60 \
        --header "Content-Type: application/json" \
        --data-binary "$payload" \
        "$speech_url" >/dev/null; then
        echo "tts warm-up complete"
      else
        echo "warning: tts warm-up request failed" >&2
      fi
      return
    fi
    sleep 1
  done

  echo "warning: tts did not become healthy in time; skipping warm-up" >&2
}

start_process asr env CUDA_VISIBLE_DEVICES="$ASR_GPU" \
  "$REPO_ROOT/.venv-qwen-vllm/bin/qwen-asr-serve" \
  "$MODEL_ROOT/Qwen3-ASR-0.6B" \
  --host 127.0.0.1 \
  --port 8711 \
  --served-model-name Qwen3-ASR-0.6B \
  --gpu-memory-utilization 0.35 \
  --max-model-len 8192

start_process agent env CUDA_VISIBLE_DEVICES="$AGENT_GPU" \
  "$REPO_ROOT/.venv-qwen-vllm/bin/vllm" serve \
  "$MODEL_ROOT/Qwen3-VL-8B-Instruct" \
  --host 127.0.0.1 \
  --port 8712 \
  --served-model-name Qwen3-VL-8B-Instruct \
  --gpu-memory-utilization 0.8 \
  --max-model-len 32768 \
  --enable-auto-tool-choice \
  --tool-call-parser hermes \
  --limit-mm-per-prompt '{"image":3,"video":0}'

start_process tts-omni env \
  CUDA_VISIBLE_DEVICES="$TTS_OMNI_GPU" \
  "$REPO_ROOT/.venv-vllm-omni/bin/vllm-omni" serve \
  "$MODEL_ROOT/$TTS_OMNI_MODEL" \
  --omni \
  --host 127.0.0.1 \
  --port 8723 \
  --served-model-name "$TTS_OMNI_MODEL" \
  --stage-configs-path "$SCRIPT_DIR/qwen3-tts-batch.yaml" \
  --trust-remote-code \
  --disable-log-requests

warm_tts

start_process gateway env \
  RIPPLE_DATA_DIR="$GATEWAY_DATA_DIR" \
  RIPPLE_HOST="${RIPPLE_HOST:-0.0.0.0}" \
  RIPPLE_PORT="${RIPPLE_PORT:-8700}" \
  RIPPLE_TTS_URL="${RIPPLE_TTS_URL:-http://127.0.0.1:8723/v1/audio/speech}" \
  RIPPLE_TTS_MODEL="${RIPPLE_TTS_MODEL:-$TTS_OMNI_MODEL}" \
  RIPPLE_TTS_VOICE="${RIPPLE_TTS_VOICE:-serena}" \
  RIPPLE_TTS_LANGUAGE="${RIPPLE_TTS_LANGUAGE:-Chinese}" \
  RIPPLE_TTS_INSTRUCTIONS="${RIPPLE_TTS_INSTRUCTIONS:-自然、温暖、亲切的中文女声。语速适中，语气平静，停顿自然，像真人助手交流，避免播音腔和夸张情绪。}" \
  RIPPLE_AUDIO_CHUNK_MS="${RIPPLE_AUDIO_CHUNK_MS:-100}" \
  "$REPO_ROOT/services/agent-gateway/target/release/ripple-agent-gateway"
