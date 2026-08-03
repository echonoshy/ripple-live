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
cargo build \
  --manifest-path "$REPO_ROOT/tools/ripple-tool/Cargo.toml" \
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

if [[ ! -x "$REPO_ROOT/.venv-vllm-omni-024/bin/python" ]]; then
  "$UV_BIN" venv "$REPO_ROOT/.venv-vllm-omni-024" --python 3.12 --seed
fi
"$UV_BIN" pip install --python "$REPO_ROOT/.venv-vllm-omni-024/bin/python" \
  --torch-backend=cu128 \
  "vllm-omni==0.24.0" \
  "prometheus-fastapi-instrumentator==8.1.0"
# The default 0.24 wheel targets CUDA 13. Use the official CUDA 12.9 wheel,
# which is compatible with the server's CUDA 12.8 driver/toolkit stack.
"$UV_BIN" pip install --python "$REPO_ROOT/.venv-vllm-omni-024/bin/python" \
  --no-deps \
  --reinstall \
  "https://github.com/vllm-project/vllm/releases/download/v0.24.0/vllm-0.24.0%2Bcu129-cp38-abi3-manylinux_2_28_x86_64.whl"

echo "Rust gateway, read-only tool CLI, and uv-managed model environments installed."
