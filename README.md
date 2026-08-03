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

## Skills and CLI tools

The gateway discovers external tools from `skills/*/SKILL.md` and
`skills/*/tools.json` at startup. Skill metadata is exposed to the model while
the command is executed as a JSON-in/JSON-out child process with a clean
environment, timeout, output limit, and cancellation on turn interruption.

`skills/system-info` is a minimal read-only example. Add another directory
with the same layout to register a tool without changing gateway source code.

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
ws://YOUR_SERVER_IP:8700/v1/agent/realtime
```

Build the web client:

```bash
cd apps/android
npm ci
npm run build
```

Follow `apps/android/README.md` for Android SDK setup and APK builds.

## Accounts and invitations

Ripple Live requires an account before opening a realtime conversation. Set one
or more comma-separated invitation codes before starting the gateway:

```bash
RIPPLE_INVITE_CODES=first-private-code,second-private-code
RIPPLE_INVITE_MAX_USES=10
RIPPLE_INVITE_TTL_HOURS=168
```

Each code has a redemption limit and an expiration time measured from when the
gateway first stores it. Restarting the gateway does not extend that expiration.
The first registration uses an email address, password, and invitation code.
After that, the user signs in with email and password. The client stores a
revocable access token locally. `RIPPLE_AUTH_TOKEN_TTL_HOURS` defaults to 720
hours.

Authenticated text clients can use `POST /v1/responses` with a string `input`
and optional `conversation` ID. The response implements the non-streaming text
subset of the Responses API shape. Image and file inputs, SSE streaming, and
token usage accounting are not implemented yet. Realtime audio and video
continue to use the WebSocket route.

## Security

The first deployment keeps the existing unauthenticated `ws://` transport.
Audio, frames, transcripts, tool arguments, and responses are visible in
transit. Before exposing this beyond a trusted test network, terminate TLS at a
reverse proxy and add session authentication and per-tool authorization.
