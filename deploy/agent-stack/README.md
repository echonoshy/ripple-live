# Ripple Agent Stack

This deployment is a fully self-hosted cascaded speech Agent:

- Qwen3-ASR-1.7B transcribes a completed VAD turn.
- Qwen3.5-35B-A3B receives the transcript and up to three recent camera
  frames and returns regular text or structured tool calls.
- The Rust gateway executes only registered tools and loops tool results back
  into the model.
- Qwen3-TTS-12Hz-1.7B-CustomVoice runs behind vLLM-Omni's two-stage batched
  serving pipeline with the Chinese-native Serena voice, explicit natural
  speaking-style instructions, and 24 kHz output.
- PostgreSQL keeps accounts, sessions, turns, tool events, projects, memories,
  and retrieval indexes; binary assets remain in the configured data directory.

## Prerequisites

- Linux with a current Rust toolchain, Python 3.12, and `uv`
- NVIDIA driver and GPUs with capacity for ASR, two-way Agent tensor parallelism,
  and TTS
- A shell function named `proxy_off` (the helper also unsets proxy variables as
  a defensive fallback)

## One-time setup

```bash
./deploy/agent-stack/install.sh
./deploy/agent-stack/download-models.sh
cp deploy/agent-stack/.env.example deploy/agent-stack/.env
```

Package downloads use `https://pypi.tuna.tsinghua.edu.cn/simple`. Model
downloads use the official ModelScope repositories. Each script calls
`proxy_off` before network access.

## Lifecycle

```bash
./deploy/agent-stack/start.sh
./deploy/agent-stack/status.sh
tail -f deploy/agent-stack/logs/*.log
./deploy/agent-stack/stop.sh
```

When a user systemd session is available, the lifecycle scripts create
transient `ripple-agent-asr`, `ripple-agent-agent`,
`ripple-agent-tts`, and `ripple-agent-gateway` services so the processes
survive the launching shell. The first start can take about one to two minutes
while vLLM compiles kernels and captures CUDA graphs; later starts reuse the
compile cache.

The TTS unit is `ripple-agent-tts`, serving the Qwen3-TTS-12Hz-1.7B-CustomVoice
checkpoint through vLLM-Omni 0.24. The deployment uses the official CUDA 12.9
vLLM wheel against the server's CUDA 12.8 toolkit, and no longer installs or
starts the serial Transformers wrapper or the smaller 0.6B TTS model.

After all four health checks report ready, run the real model smoke test. It
sends an image through the multimodal request, requires the Agent model to issue a
structured `calculate` call, and verifies that the final response contains
playable audio. It then resamples that speech and feeds it through Qwen3-ASR as
an audio loopback test:

```bash
uv run --with httpx --with numpy --with websockets \
  deploy/agent-stack/smoke-test.py
```

The Gateway requests raw 24 kHz PCM from vLLM-Omni with HTTP streaming and
forwards playable 100 ms Float32 chunks immediately. It does not wait for a
complete WAV file. `RIPPLE_AUDIO_CHUNK_MS` controls the downstream packet
size; keep the default at 100 ms unless the client or network requires a
different trade-off.

Every response event carries a `response_id`. The mobile client discards
audio from an older response after barge-in, while the Gateway aborts the
upstream request. The smoke test covers tool use, TTS-to-ASR loopback,
streaming first-audio timing, cancellation, and response isolation.

Services bind as follows:

| Port | Bind address | Purpose |
| --- | --- | --- |
| 8700 | `0.0.0.0` | Public Agent WebSocket and health endpoint |
| 8711 | `127.0.0.1` | ASR OpenAI-compatible API |
| 8712 | `127.0.0.1` | Qwen3.5-35B-A3B Responses API |
| 8723 | `127.0.0.1` | vLLM-Omni Qwen3-TTS speech API |

ASR and TTS each use one physical GPU; the main Agent uses two-way tensor
parallelism. Qwen3-TTS uses a
two-stage Talker/Code2Wav pipeline in `qwen3-tts-batch.yaml`. The Talker keeps
continuous batching enabled, while CustomVoice Code2Wav uses one sequence for
the best first-audio latency. The codec emits an initial five-frame chunk,
then uses 25-frame chunks with 72 frames of decoder context. This seeds the
client playback buffer before the first steady-state chunk and keeps adjacent
streaming chunks timbrally consistent. ASR and Qwen3.5 retain vLLM continuous
batching.

Run a repeatable TTS concurrency benchmark with:

```bash
uv run --with httpx deploy/agent-stack/tts-benchmark.py --stream --concurrency 4
```

On the RTX 5880 validation, the 1.7B model delivered 197 ms median first-audio
latency at four-way concurrency, a 0.24 real-time factor, and 14.22 generated
audio-seconds/second. The larger initial chunk trades about 90 ms of server
TTFA for enough audio to prevent an immediate playback underrun. End-to-end
Agent first audio also includes multimodal generation and sentence accumulation; the
smoke test measured 0.93 seconds.

## Tool extension

Tool JSON schemas and implementations live in
`services/agent-gateway/src/tools.rs`. A tool must be explicitly listed by
`schemas` and dispatched by `ToolExecutor`; arbitrary model output is never
executed as code.

The native Gateway tools are:

- `get_current_time`
- `calculate`
- `remember`
- `recall`
- `create_todo`
- `list_todos`

The external read-only tools are loaded from Skills and executed through the
standalone Rust `ripple-tool` CLI:

- `web_search` uses Tavily Basic Search and returns bounded source snippets.
- `web_fetch` uses Tavily Extract for one public HTTP/HTTPS URL.
- `weather_lookup` uses QWeather GeoAPI and Weather API.
- `system_info` returns allowlisted read-only host information.

Configure `RIPPLE_TAVILY_API_KEY` plus the QWeather API host and either an API
key or the JWT project ID, credential ID, and Ed25519 private-key path in `.env`. The Gateway passes only
the manifest allowlisted variables to each short-lived CLI process. Set
`RIPPLE_SEARCH_PROXY` when the Gateway host needs a dedicated outbound proxy.
Tool results use a stable JSON envelope, share a small local cache, and never treat
an empty or failed result as evidence.

## Context extension

`ContextStore` persists the event log, recent conversational turns, projects,
profiles, and explicit memories in PostgreSQL. Search combines PostgreSQL text
matching with pgvector retrieval; binary assets remain under
`RIPPLE_DATA_DIR/assets`.

Calls started from Home create a new conversation, while calls started from a
conversation detail screen continue that conversation. Server-side events
record the session lifecycle and each response's
input commit, transcript, context load, Agent rounds, TTS segments, completion,
cancellation, or failure. Inspect the latest flow with:

```bash
psql "$RIPPLE_DATABASE_URL" -c \
  "SELECT to_timestamp(created_at), session_id, kind, payload FROM events ORDER BY id DESC LIMIT 100;"
```

Gateway logs carry the same `session_id` and, for per-turn work, `response_id`:

```bash
tail -f deploy/agent-stack/logs/gateway.log
```

## Development without models

Build and run the gateway with deterministic local adapters:

```bash
cargo build --manifest-path services/agent-gateway/Cargo.toml
RIPPLE_ASR_BACKEND=mock \
RIPPLE_AGENT_BACKEND=mock \
RIPPLE_TTS_BACKEND=mock \
services/agent-gateway/target/debug/ripple-agent-gateway
```

This mode validates capture, protocol, interruption, UI, persistence, and
audio transport. It does not provide real recognition or synthesized speech.
