# Ripple AI Passport Firmware

English | [简体中文](README.zh_CN.md)

Standalone ESP-IDF firmware that turns a FoloToy AI Passport into a focused
push-to-talk client for the Ripple Agent Gateway. Hold `OK`, speak, release the
button, and listen to the streamed response. There is no login screen, message
history, replay control, or local conversation management.

This branch contains only the device firmware. The Ripple Live mobile app,
Gateway implementation, deployment stack, and tools are intentionally outside
this repository. A compatible Gateway is an external runtime dependency.

## Product behavior

- 16 kHz mono microphone input streamed over WebSocket.
- 24 kHz mono response playback with a 400 ms jitter buffer.
- Press-to-talk interruption: pressing `OK` during playback cancels the current
  response and starts a new recording.
- Original Ripple pet animations for connecting, listening, thinking, ready,
  setup, and error states.
- Browser-based 2.4 GHz Wi-Fi provisioning.
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

## Build and flash

```bash
source /path/to/esp-idf-v5.5.3/export.sh
idf.py set-target esp32c3
idf.py build
idf.py -p /dev/cu.usbmodemXXXX flash monitor
```

Use `flash`, not `erase-flash`, for ordinary updates. It preserves Wi-Fi and
volume data in NVS. Use `idf.py erase-flash` only when a full factory reset is
intended.

The current image is about 2.18 MB in a 7 MB factory partition. The ESP32-C3
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

See the user guide for detailed troubleshooting and acceptance steps.
