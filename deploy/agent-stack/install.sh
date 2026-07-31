#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
source "$SCRIPT_DIR/proxy-and-mirror.sh"
disable_proxy_and_enable_mirrors

UV_BIN="${UV_BIN:-$(command -v uv)}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust cargo is required to build the Agent Gateway." >&2
  exit 1
fi
cargo build \
  --manifest-path "$REPO_ROOT/services/agent-gateway/Cargo.toml" \
  --release \
  --locked

if [[ ! -x "$REPO_ROOT/.venv-qwen-vllm/bin/python" ]]; then
  "$UV_BIN" venv "$REPO_ROOT/.venv-qwen-vllm" --python 3.12
fi
"$UV_BIN" pip install --python "$REPO_ROOT/.venv-qwen-vllm/bin/python" \
  "qwen-asr[vllm]" \
  "qwen-vl-utils==0.0.14" \
  httpx \
  modelscope \
  numpy \
  websockets

if [[ ! -x "$REPO_ROOT/.venv-vllm-omni/bin/python" ]]; then
  "$UV_BIN" venv "$REPO_ROOT/.venv-vllm-omni" --python 3.12 --seed
fi
"$UV_BIN" pip install --python "$REPO_ROOT/.venv-vllm-omni/bin/python" \
  --torch-backend=cu128 \
  "vllm==0.16.0" \
  "vllm-omni==0.16.0" \
  "prometheus-fastapi-instrumentator==8.1.0"

echo "Rust gateway and uv-managed model environments installed."
