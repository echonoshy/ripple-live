#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MODEL_ROOT="${MODEL_ROOT:-$REPO_ROOT/.cache/models}"
source "$SCRIPT_DIR/proxy-and-mirror.sh"
disable_proxy_and_enable_mirrors

MODELSCOPE="$REPO_ROOT/.venv-qwen-vllm/bin/modelscope"
if [[ ! -x "$MODELSCOPE" ]]; then
  echo "Run deploy/agent-stack/install.sh first." >&2
  exit 1
fi

mkdir -p "$MODEL_ROOT"

"$MODELSCOPE" download \
  Qwen/Qwen3-ASR-0.6B \
  --local_dir "$MODEL_ROOT/Qwen3-ASR-0.6B"
"$MODELSCOPE" download \
  Qwen/Qwen3-VL-8B-Instruct \
  --local_dir "$MODEL_ROOT/Qwen3-VL-8B-Instruct"
"$MODELSCOPE" download \
  Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice \
  --local_dir "$MODEL_ROOT/Qwen3-TTS-12Hz-1.7B-CustomVoice"

echo "Models downloaded from ModelScope to $MODEL_ROOT"
