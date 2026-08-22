# Ripple AI Passport Firmware

English | [简体中文](README.zh_CN.md)

Standalone ESP-IDF firmware that turns a FoloToy AI Passport into a focused
push-to-talk client for the Ripple Agent Gateway. Hold `OK`, speak, release the
button, and listen to the streamed response. There is no login screen, message
history, replay control, or local conversation management.

This branch contains only the device firmware. The Ripple Live mobile app,
Gateway implementation, deployment stack, and tools are intentionally outside
this repository. A compatible Gateway is an external runtime dependency.

## FoloToy hardware and upstream resources

This firmware targets the **FoloToy AI Passport** and builds on the public
hardware contract and BSP baseline published by FoloToy:

- [FoloToy/ai-passport](https://github.com/FoloToy/ai-passport) — official AI
  Passport development baseline, hardware interfaces, examples, and agent
  development guidance.
- [FoloToy GitHub organization](https://github.com/FoloToy) — official source
  repositories and related projects.
- [FoloToy documentation](https://docs.folotoy.com/) — product documentation,
  drivers, diagnostics, and general device guidance.
- [FoloToy Web Tool](https://tool.folotoy.com/) and
  [official firmware releases](https://github.com/FoloToy/folotoy-bin/releases)
  — useful for logs and restoring official stock firmware.

Ripple AI Passport is a separate application firmware. It does not use the
stock FoloToy MQTT/server protocol, and a stock FoloToy server cannot replace
the Ripple Agent Gateway described below. The FoloToy Web Tool expects a
complete stock image at address `0x0`; do not write this project's application
binary (`build/Ripple-AI-Passport.bin`) to `0x0`.

## Product behavior

- 16 kHz mono microphone input streamed through a bounded upload queue.
- 24 kHz mono response playback with a sample-counted 400 ms jitter buffer.
- Press-to-talk interruption: pressing `OK` during playback cancels the current
  response and starts a new recording.
- Original Ripple pet animations for connecting, listening, thinking, ready,
  setup, and error states.
- Browser-based 2.4 GHz Wi-Fi provisioning with captive-portal detection.
- Persistent local volume control and transient battery/network diagnostics.

### Buttons

| Input | Action |
| --- | --- |
| Hold `OK` | Record speech |
| Release `OK` | Send the turn |
| Press `OK` during playback | Interrupt and start speaking |
| Click `UP` / `DOWN` | Volume +10% / -10% |
| Hold `DOWN` for 1.5 s | Show battery, Wi-Fi RSSI, and Gateway status |
| Hold `UP` for 3 s | Clear Wi-Fi configuration and restart provisioning |

Double-click actions are deliberately unused.

## Requirements

- FoloToy AI Passport with ESP32-C3, 8 MB flash, ST7789P3 display, ES8311
  codec, and the existing three-button ADC ladder.
- ESP-IDF 5.5.x; the validated toolchain is 5.5.3.
- A compatible Ripple Agent Gateway exposing WebSocket protocol v5 at
  `/v1/agent/realtime` and configured with an anonymous device account.
- A 2.4 GHz Wi-Fi network. ESP32-C3 cannot join a 5 GHz-only network.

## Clone this firmware branch

```bash
git clone --branch ai-passport-firmware --single-branch \
  https://github.com/echonoshy/ripple-live.git ripple-ai-passport
cd ripple-ai-passport
```

## Deploy the Ripple Agent Gateway

The Passport is a thin client: speech recognition, Responses API orchestration,
tools, memory, and speech synthesis run on the Gateway. Choose one path:

1. Use an existing compatible Gateway and obtain its `host:port` from the
   operator.
2. Self-host the reference Ripple stack from the `master` branch.
3. Implement another Gateway against [the protocol contract](docs/PROTOCOL.md).

### Self-host the reference stack

The reference stack is in the same Git repository but intentionally not in
this firmware branch:

```bash
git clone --branch master --single-branch \
  https://github.com/echonoshy/ripple-live.git ripple-live-server
cd ripple-live-server
cp deploy/agent-stack/.env.example deploy/agent-stack/.env
```

Read and adapt `deploy/agent-stack/README.md` and `.env` before installation.
The validated deployment is a Linux/NVIDIA GPU stack using Rust, Python 3.12,
`uv`, PostgreSQL, Qwen3-ASR, a Responses API model, and Qwen3-TTS. The reference
Agent uses tensor parallelism across two GPUs, with separate ASR and TTS GPUs;
adjust all GPU indices, model paths, CUDA versions, and credentials for the
target host.

```bash
./deploy/agent-stack/install.sh
./deploy/agent-stack/download-models.sh
./deploy/agent-stack/start.sh
./deploy/agent-stack/status.sh
curl --fail http://127.0.0.1:8700/health
curl --fail http://127.0.0.1:8700/ready
```

The current device firmware intentionally sends no login token. Create a real
Gateway user, then configure that user's ID as the development-only anonymous
device account:

```bash
curl --fail --request POST http://127.0.0.1:8700/v1/auth/register \
  --header 'Content-Type: application/json' \
  --data '{
    "email":"passport-device@example.com",
    "password":"replace-with-a-private-password",
    "invitation_code":"the-code-from-RIPPLE_INVITE_CODES"
  }'
```

Copy `user.id` from the JSON response into
`RIPPLE_ANONYMOUS_REALTIME_USER_ID` in `deploy/agent-stack/.env`, restart the
Gateway stack, and confirm `/ready` again. This tokenless account is for local
product validation; use authenticated device identity for a production
deployment.

## Build and flash

Install [ESP-IDF 5.5.3](https://docs.espressif.com/projects/esp-idf/en/v5.5.3/esp32c3/get-started/index.html)
using Espressif's instructions, then build from the repository root:

```bash
source /path/to/esp-idf-v5.5.3/export.sh
idf.py set-target esp32c3
idf.py build
idf.py -p /dev/cu.usbmodemXXXX flash monitor
```

Typical ports are `/dev/cu.usbmodem*` on macOS and `/dev/ttyACM*` on Linux.
The project uses native USB Serial/JTAG; if no port appears, try another USB
data cable and consult the FoloToy driver/diagnostic documentation above.

Use `flash`, not `erase-flash`, for ordinary updates. It preserves Wi-Fi and
volume data in both NVS copies. Configuration is mirrored into a dedicated
`ripple_backup` partition and restored if either copy needs recovery. Use
`idf.py erase-flash` only when a full factory reset is intended.

The current image is about 2.18 MB in a 7 MB factory partition, with a separate
64 KB backup NVS partition. The ESP32-C3
has no PSRAM; changes to image assets, task stacks, LVGL buffers, or audio
queues must be checked against internal RAM.

## First-time setup

1. Power on the Passport.
2. Join the open access point `Ripple-Passport-XXXX` from a phone or computer.
3. Open `http://192.168.4.1/`.
4. Enter a 2.4 GHz Wi-Fi SSID, password, and Gateway host/port.
5. Save. The device restarts and shows `HOLD OK TO TALK` after the Gateway
   session becomes ready.

The validation default is `140.143.229.103:8700`. The firmware currently uses
plain `ws://`; use it only in a trusted validation environment. TLS/WSS and
device identity are separate production-hardening work.

## Repository layout

```text
assets/pet-gifs/       Canonical, immutable pet animation sources
components/bsp/        Display, buttons, codec, battery, and shared-I2C BSP
docs/                  User, protocol, and hardware/development documentation
main/                  Product UI, controls, Wi-Fi, realtime, and pet runtime
main/pet_assets/       Generated LVGL I8 animation frames
tools/                 Deterministic asset conversion tool
partitions.csv         8 MB flash layout
sdkconfig.defaults     Reproducible ESP32-C3/LVGL defaults
```

## Documentation

- [Chinese user guide](docs/USER_GUIDE.zh_CN.md)
- [Realtime protocol contract](docs/PROTOCOL.md)
- [Hardware and development guide](docs/DEVELOPMENT.md)
- [Official FoloToy AI Passport baseline](https://github.com/FoloToy/ai-passport)

## Asset regeneration

The committed C frames are build-ready. Regeneration is needed only when the
canonical GIFs intentionally change:

```bash
python -m pip install pillow pypng lz4
# pngquant must also be available on PATH
python tools/generate_pet_assets.py
```

The converter validates source dimensions, frame counts, and frame durations,
then proportionally resizes the complete 384×416 canvas to 144×156 without
cropping or redrawing the character.

## Validation checklist

- Startup reaches `session ready` without a watchdog reset or reboot loop.
- The pet is complete, uncropped, and animates correctly for each state.
- PTT records, commits, plays a complete reply, and interrupts playback.
- Normal replies log `playback started with 400 ms buffered`; no repeated
  underruns occur on a stable network.
- Volume changes survive a restart.
- The status shortcut reports plausible battery and RSSI values.
- Ten consecutive turns do not continuously reduce minimum free heap.
- Slow Wi-Fi, an unresponsive Gateway handshake, and a stalled response time
  out and recover without a reboot.
- Two rapid interruptions cannot let an old `response_id` affect the new turn.

See the user guide for detailed troubleshooting and acceptance steps.
