# Ripple Live

Ripple Live is a self-hosted multimodal voice Agent for Android. The current
implementation uses a cascaded pipeline so speech recognition, visual
reasoning, tool execution, context storage, and speech synthesis can be
operated and upgraded independently.

## Architecture

```text
Android microphone ──16 kHz PCM──┐
Android camera ──sampled JPEG─────┼── Rust Agent Gateway :8700
                                 │     ├── Qwen3-ASR 0.6B :8711
                                 │     ├── Qwen3-VL 8B :8712
                                 │     ├── allowlisted tools
                                 │     ├── SQLite context/event store
Android speaker ◀─24 kHz PCM─────┘     └── Qwen3-TTS 1.7B + vLLM-Omni :8723
```

The gateway owns conversation turns and the tool loop. Tool calls are parsed
from the model's structured response, executed by the server, recorded, and
returned to the model before the final spoken answer is generated.

## Repository layout

```text
apps/android/                     Tauri 2 + React Android client
services/agent-gateway/           Rust WebSocket protocol, tools, context, adapters
deploy/agent-stack/               Install, download, start, stop, and status
.cache/models/                    Local model weights (ignored by Git)
.venv-qwen-vllm/                  uv-managed ASR/VL environment (ignored)
.venv-vllm-omni/                  uv-managed concurrent TTS environment (ignored)
```

## Install and run the Agent stack

The gateway is built as a Rust release binary. All remaining Python
dependencies are installed into isolated environments with `uv`; nothing is
installed into the global Python. The scripts invoke the local `proxy_off`
shell function, remove all common proxy variables, use the Tsinghua PyPI
mirror, and download model weights from ModelScope.

```bash
./deploy/agent-stack/install.sh
./deploy/agent-stack/download-models.sh
cp deploy/agent-stack/.env.example deploy/agent-stack/.env
./deploy/agent-stack/start.sh
./deploy/agent-stack/status.sh
```

On this host the processes run as transient user-systemd services, so they
remain alive after the setup shell exits. The first vLLM start performs kernel
compilation and can need one to two minutes before all health checks pass.

The default GPU allocation is ASR on GPU 2, Qwen3-VL on GPU 3, and concurrent
Qwen3-TTS 1.7B on GPU 7. Edit `deploy/agent-stack/.env` to change the
allocation.

Stop the stack with:

```bash
./deploy/agent-stack/stop.sh
```

## Android

The default endpoint is:

```text
ws://140.143.229.103:8700/v1/agent/realtime
```

Build the web client:

```bash
cd apps/android
npm ci
npm run build
```

Follow `apps/android/README.md` for Android SDK setup and APK builds.

## Security

The first deployment keeps the existing unauthenticated `ws://` transport.
Audio, frames, transcripts, tool arguments, and responses are visible in
transit. Before exposing this beyond a trusted test network, terminate TLS at a
reverse proxy and add session authentication and per-tool authorization.
