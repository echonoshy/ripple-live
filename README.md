# Ripple Live

Ripple Live is a self-contained Android realtime multimodal assistant built
with Rust, Tauri 2, and MiniCPM-o 4.5.

## Features

- Full-duplex realtime voice conversation
- Camera + voice conversation
- Native Tauri WebSocket transport
- 16 kHz mono float32 microphone input
- 24 kHz float32 streaming speech output

## Repository layout

```text
apps/android/          Ripple Live Android application
deploy/realtime-o45/   MiniCPM-o 4.5 realtime inference service
.cache/models/         Local model weights (ignored by Git)
.cache/services/       Downloaded upstream service source (ignored by Git)
.venv-realtime-o45/    Python runtime (ignored by Git)
```

## Realtime service

The production endpoint configured in the app is:

```text
ws://140.143.229.103:8600/v1/realtime?mode=audio
ws://140.143.229.103:8600/v1/realtime?mode=video
```

Prepare and start the bare-metal PyTorch service on GPU 1:

```bash
./deploy/realtime-o45/setup-baremetal.sh
./deploy/realtime-o45/start-baremetal.sh
./deploy/realtime-o45/status-baremetal.sh
```

Stop it with:

```bash
./deploy/realtime-o45/stop-baremetal.sh
```

## Android

```bash
cd apps/android
npm ci
./scripts/setup-android.sh
```

Follow `apps/android/README.md` for the Android environment variables and APK
build command.

## Local assets

Model weights and toolchains live inside this project so it can run
independently, but they are intentionally excluded from Git. A fresh clone can
recreate the upstream service, Python environment, and Android toolchain with
the included setup scripts. Model weights must be copied or downloaded
separately.

This deployment intentionally uses unauthenticated, unencrypted `ws://`.
Audio, camera frames, prompts, and responses are visible in transit.
