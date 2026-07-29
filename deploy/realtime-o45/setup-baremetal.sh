#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
SOURCE_DIR="${ROOT_DIR}/.cache/services/MiniCPM-o-Demo"
VENV_DIR="${ROOT_DIR}/.venv-realtime-o45"
UV_BIN="${UV_BIN:-${HOME}/.local/bin/uv}"

"${SCRIPT_DIR}/setup.sh"

if [[ ! -x "${UV_BIN}" ]]; then
  echo "uv was not found at ${UV_BIN}" >&2
  echo "Install uv or set UV_BIN to its absolute path." >&2
  exit 1
fi

if [[ ! -x "${VENV_DIR}/bin/python" ]]; then
  "${UV_BIN}" venv --python 3.12 "${VENV_DIR}"
fi

echo "Installing the official PyTorch 2.8 CUDA 12.8 runtime..."
"${UV_BIN}" pip install \
  --python "${VENV_DIR}/bin/python" \
  --index-url https://download.pytorch.org/whl/cu128 \
  torch==2.8.0 torchaudio==2.8.0

requirements_without_utils="$(mktemp)"
trap 'rm -f -- "${requirements_without_utils}"' EXIT
grep -vE '^[[:space:]]*minicpmo-utils(\[all\])?>=' \
  "${SOURCE_DIR}/requirements.txt" > "${requirements_without_utils}"

echo "Installing the MiniCPM-o realtime dependencies..."
"${UV_BIN}" pip install \
  --python "${VENV_DIR}/bin/python" \
  "minicpmo-utils[all]>=1.0.5"
"${UV_BIN}" pip install \
  --python "${VENV_DIR}/bin/python" \
  --requirement "${requirements_without_utils}"

"${VENV_DIR}/bin/python" - <<'PY'
import torch
import torchaudio
import transformers

print(f"torch={torch.__version__}")
print(f"torchaudio={torchaudio.__version__}")
print(f"transformers={transformers.__version__}")
print(f"cuda_available={torch.cuda.is_available()}")
PY

echo "Bare-metal runtime installed at ${VENV_DIR}"
