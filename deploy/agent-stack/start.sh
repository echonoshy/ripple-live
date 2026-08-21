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
ASR_RUNTIME="${ASR_RUNTIME:-$REPO_ROOT/.venv-qwen3-asr-1.7b}"
AGENT_GPU="${AGENT_GPU:-4,5}"
AGENT_MODEL="${AGENT_MODEL:-Qwen3.5-35B-A3B}"
AGENT_RUNTIME="${AGENT_RUNTIME:-$REPO_ROOT/.venv-qwen3.5-35b-a3b}"
AGENT_TOOL_CALL_PARSER="${AGENT_TOOL_CALL_PARSER:-qwen3_coder}"
AGENT_GPU_MEMORY_UTILIZATION="${AGENT_GPU_MEMORY_UTILIZATION:-0.85}"
AGENT_MAX_NUM_SEQS="${AGENT_MAX_NUM_SEQS:-192}"
if [[ "$AGENT_RUNTIME" != /* ]]; then
  AGENT_RUNTIME="$REPO_ROOT/$AGENT_RUNTIME"
fi
TTS_GPU="${TTS_GPU:-7}"
TTS_MODEL="${TTS_MODEL:-Qwen3-TTS-12Hz-1.7B-CustomVoice}"
TTS_RUNTIME="${TTS_RUNTIME:-$REPO_ROOT/.venv-qwen3-tts-12hz-1.7b-customvoice}"
TTS_CUDA_HOME="${TTS_CUDA_HOME:-/usr/local/cuda-12.8}"
TTS_SITE="$TTS_RUNTIME/lib/python3.12/site-packages"
TTS_LIBRARY_PATH="$TTS_CUDA_HOME/targets/x86_64-linux/lib:$TTS_SITE/torch/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
GATEWAY_DATA_DIR="${RIPPLE_DATA_DIR:-$REPO_ROOT/runtime-data/agent-gateway}"
DATABASE_URL="${RIPPLE_DATABASE_URL:-postgres://lake@127.0.0.1:5432/ripple_live}"
POSTGRES_ROOT="${RIPPLE_POSTGRES_ROOT:-$REPO_ROOT/runtime-data/postgres}"
SEARCH_PROXY="${RIPPLE_SEARCH_PROXY:-${https_proxy:-${HTTPS_PROXY:-}}}"
TOOL_CACHE_DB="${RIPPLE_TOOL_CACHE_DB:-$GATEWAY_DATA_DIR/tool-cache.sqlite3}"
if [[ "$GATEWAY_DATA_DIR" != /* ]]; then
  GATEWAY_DATA_DIR="$REPO_ROOT/$GATEWAY_DATA_DIR"
fi
if [[ "$TOOL_CACHE_DB" != /* ]]; then
  TOOL_CACHE_DB="$REPO_ROOT/$TOOL_CACHE_DB"
fi
if [[ "$POSTGRES_ROOT" != /* ]]; then
  POSTGRES_ROOT="$REPO_ROOT/$POSTGRES_ROOT"
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
  payload="$(printf '{"model":"%s","input":"语音服务预热完成。","voice":"serena","language":"Chinese","response_format":"pcm","stream":true,"stream_format":"audio"}' "$TTS_MODEL")"

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

POSTGRES_BIN="$POSTGRES_ROOT/runtime/usr/lib/postgresql/16/bin/postgres"
POSTGRES_DATA="$POSTGRES_ROOT/data"
POSTGRES_SOCKET="$POSTGRES_ROOT/socket"
if [[ ! -x "$POSTGRES_BIN" || ! -d "$POSTGRES_DATA" ]]; then
  echo "PostgreSQL runtime is not installed under $POSTGRES_ROOT" >&2
  exit 1
fi
mkdir -p "$POSTGRES_SOCKET"
start_process postgres \
  "$POSTGRES_BIN" \
  -D "$POSTGRES_DATA" \
  -h 127.0.0.1 \
  -p 5432 \
  -k "$POSTGRES_SOCKET"

start_process asr env CUDA_VISIBLE_DEVICES="$ASR_GPU" \
  "$ASR_RUNTIME/bin/qwen-asr-serve" \
  "$MODEL_ROOT/Qwen3-ASR-1.7B" \
  --host 127.0.0.1 \
  --port 8711 \
  --served-model-name Qwen3-ASR-1.7B \
  --gpu-memory-utilization 0.35 \
  --max-model-len 8192

start_process agent env \
  CUDA_VISIBLE_DEVICES="$AGENT_GPU" \
  PATH="$AGENT_RUNTIME/bin:$PATH" \
  "$AGENT_RUNTIME/bin/vllm" serve \
  "$MODEL_ROOT/$AGENT_MODEL" \
  --host 127.0.0.1 \
  --port 8712 \
  --served-model-name "$AGENT_MODEL" \
  --tensor-parallel-size 2 \
  --gpu-memory-utilization "$AGENT_GPU_MEMORY_UTILIZATION" \
  --max-num-seqs "$AGENT_MAX_NUM_SEQS" \
  --max-model-len 32768 \
  --reasoning-parser qwen3 \
  --enable-auto-tool-choice \
  --tool-call-parser "$AGENT_TOOL_CALL_PARSER" \
  --limit-mm-per-prompt '{"image":3,"video":0}'

start_process tts env \
  CUDA_VISIBLE_DEVICES="$TTS_GPU" \
  LD_LIBRARY_PATH="$TTS_LIBRARY_PATH" \
  PATH="$TTS_RUNTIME/bin:$PATH" \
  "$TTS_RUNTIME/bin/vllm-omni" serve \
  "$MODEL_ROOT/$TTS_MODEL" \
  --omni \
  --host 127.0.0.1 \
  --port 8723 \
  --served-model-name "$TTS_MODEL" \
  --deploy-config "$SCRIPT_DIR/qwen3-tts-batch.yaml" \
  --trust-remote-code \
  --no-enable-log-requests

warm_tts

start_process gateway env \
  RIPPLE_DATA_DIR="$GATEWAY_DATA_DIR" \
  RIPPLE_DATABASE_URL="$DATABASE_URL" \
  RIPPLE_DATABASE_MAX_CONNECTIONS="${RIPPLE_DATABASE_MAX_CONNECTIONS:-16}" \
  RIPPLE_HOST="${RIPPLE_HOST:-0.0.0.0}" \
  RIPPLE_PORT="${RIPPLE_PORT:-8700}" \
  RIPPLE_ASR_MODEL="${RIPPLE_ASR_MODEL:-Qwen3-ASR-1.7B}" \
  RIPPLE_AGENT_URL="${RIPPLE_AGENT_URL:-http://127.0.0.1:8712/v1/responses}" \
  RIPPLE_AGENT_MODEL="${RIPPLE_AGENT_MODEL:-$AGENT_MODEL}" \
  RIPPLE_TTS_URL="${RIPPLE_TTS_URL:-http://127.0.0.1:8723/v1/audio/speech}" \
  RIPPLE_TTS_MODEL="${RIPPLE_TTS_MODEL:-$TTS_MODEL}" \
  RIPPLE_TTS_VOICE="${RIPPLE_TTS_VOICE:-serena}" \
  RIPPLE_TTS_LANGUAGE="${RIPPLE_TTS_LANGUAGE:-Chinese}" \
  RIPPLE_TTS_INSTRUCTIONS="${RIPPLE_TTS_INSTRUCTIONS:-自然、温暖、亲切的中文女声。语速偏快但吐字清晰，语气平静连贯，停顿简洁自然，保持稳定一致的音色，像真人助手交流，避免播音腔和夸张情绪。}" \
  RIPPLE_AUDIO_CHUNK_MS="${RIPPLE_AUDIO_CHUNK_MS:-100}" \
  RIPPLE_SEARCH_PROXY="$SEARCH_PROXY" \
  RIPPLE_TAVILY_API_KEY="${RIPPLE_TAVILY_API_KEY:-}" \
  RIPPLE_TAVILY_API_URL="${RIPPLE_TAVILY_API_URL:-https://api.tavily.com}" \
  RIPPLE_QWEATHER_API_HOST="${RIPPLE_QWEATHER_API_HOST:-}" \
  RIPPLE_QWEATHER_API_KEY="${RIPPLE_QWEATHER_API_KEY:-}" \
  RIPPLE_QWEATHER_PROJECT_ID="${RIPPLE_QWEATHER_PROJECT_ID:-}" \
  RIPPLE_QWEATHER_CREDENTIAL_ID="${RIPPLE_QWEATHER_CREDENTIAL_ID:-}" \
  RIPPLE_QWEATHER_PRIVATE_KEY_PATH="${RIPPLE_QWEATHER_PRIVATE_KEY_PATH:-}" \
  RIPPLE_TOOL_CACHE_DB="$TOOL_CACHE_DB" \
  HTTPS_PROXY="$SEARCH_PROXY" \
  HTTP_PROXY="$SEARCH_PROXY" \
  NO_PROXY="127.0.0.1,localhost" \
  "$REPO_ROOT/services/agent-gateway/target/release/ripple-agent-gateway"
