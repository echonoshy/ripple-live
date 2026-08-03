# Ripple Agent Stack

This deployment is a fully self-hosted cascaded speech Agent:

- Qwen3-ASR-0.6B transcribes a completed VAD turn.
- Qwen3-VL-8B-Instruct receives the transcript and up to three recent camera
  frames and returns regular text or structured tool calls.
- The Rust gateway executes only registered tools and loops tool results back
  into the model.
- Qwen3-TTS-12Hz-1.7B-CustomVoice runs behind vLLM-Omni's two-stage batched
  serving pipeline with the Chinese-native Serena voice, explicit natural
  speaking-style instructions, and 24 kHz output.
- SQLite keeps sessions, turns, tool events, and explicit long-term memories.

## Prerequisites

- Linux with a current Rust toolchain, Python 3.12, and `uv`
- NVIDIA driver and three GPUs with roughly 18 GB, 38 GB, and 16 GB free
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
`ripple-agent-tts-omni`, and `ripple-agent-gateway` services so the processes
survive the launching shell. The first start can take about one to two minutes
while vLLM compiles kernels and captures CUDA graphs; later starts reuse the
compile cache.

The only TTS unit is `ripple-agent-tts-omni`, serving the 1.7B CustomVoice
checkpoint through vLLM-Omni 0.24. The deployment uses the official CUDA 12.9
vLLM wheel against the server's CUDA 12.8 toolkit, and no longer installs or
starts the serial Transformers wrapper or the smaller 0.6B TTS model.

After all four health checks report ready, run the real model smoke test. It
sends an image through the multimodal request, requires the VL model to issue a
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

Every response event carries a `response_id`. The Android client discards
audio from an older response after barge-in, while the Gateway aborts the
upstream request. The smoke test covers tool use, TTS-to-ASR loopback,
streaming first-audio timing, cancellation, and response isolation.

Services bind as follows:

| Port | Bind address | Purpose |
| --- | --- | --- |
| 8700 | `0.0.0.0` | Public Agent WebSocket and health endpoint |
| 8711 | `127.0.0.1` | ASR OpenAI-compatible API |
| 8712 | `127.0.0.1` | Qwen3-VL OpenAI-compatible API |
| 8723 | `127.0.0.1` | vLLM-Omni Qwen3-TTS speech API |

Each inference service uses exactly one physical GPU. Qwen3-TTS uses a
two-stage Talker/Code2Wav pipeline in `qwen3-tts-batch.yaml`. The Talker keeps
continuous batching enabled, while CustomVoice Code2Wav uses one sequence for
the best first-audio latency. The codec emits an initial five-frame chunk,
then uses 25-frame chunks with 72 frames of decoder context. This seeds the
client playback buffer before the first steady-state chunk and keeps adjacent
streaming chunks timbrally consistent. ASR and Qwen3-VL retain vLLM continuous
batching.

Run a repeatable TTS concurrency benchmark with:

```bash
uv run --with httpx deploy/agent-stack/tts-benchmark.py --stream --concurrency 4
```

On the RTX 5880 validation, the 1.7B model delivered 197 ms median first-audio
latency at four-way concurrency, a 0.24 real-time factor, and 14.22 generated
audio-seconds/second. The larger initial chunk trades about 90 ms of server
TTFA for enough audio to prevent an immediate playback underrun. End-to-end
Agent first audio also includes VL generation and sentence accumulation; the
smoke test measured 0.93 seconds.

## Tool extension

Tool JSON schemas and implementations live in
`services/agent-gateway/src/tools.rs`. A tool must be explicitly listed by
`schemas` and dispatched by `ToolExecutor`; arbitrary model output is never
executed as code.

The built-in tools are:

- `web_search` (DuckDuckGo Instant Answer summaries and related sources)
- `get_current_time`
- `calculate`
- `remember`
- `recall`

`web_search` uses DuckDuckGo's keyless Instant Answer endpoint. It returns a
compact answer, source snippets, and URLs, but it is not a complete general web
search API, so some current-events or long-tail queries can return no results.
Set `RIPPLE_SEARCH_PROXY` when the Gateway host needs a dedicated outbound HTTP
proxy. The search client has a 12-second timeout and never treats an empty
result as evidence.

## Context extension

The initial context manager is intentionally behind `ContextStore`. It
persists a complete event log, recent conversational turns, and explicit
memories in `.cache/agent-gateway/context.sqlite3`. A later Redis/PostgreSQL or
vector-memory implementation can replace this class without changing the
Android protocol or model adapters.

The Android client creates a new session UUID whenever a voice or video call is
started. Server-side events record the session lifecycle and each response's
input commit, transcript, context load, Agent rounds, TTS segments, completion,
cancellation, or failure. Inspect the latest flow with:

```bash
sqlite3 .cache/agent-gateway/context.sqlite3 \
  "SELECT datetime(created_at, 'unixepoch', 'localtime'), session_id, kind, payload FROM events ORDER BY id DESC LIMIT 100;"
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
