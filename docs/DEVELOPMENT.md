# Hardware and Development Guide

This guide describes the hardware facts and runtime boundaries of the Ripple
AI Passport firmware. `components/bsp/include/bsp_pins.h` remains the source of
truth when this document and code disagree.

## Hardware contract

| Function | Device and connection | Runtime boundary |
| --- | --- | --- |
| MCU | ESP32-C3, 160 MHz, 8 MB flash, no PSRAM | All dynamic buffers use internal RAM |
| Display | ST7789P3, 240×320 RGB565, SPI2 at 40 MHz | No LCD readback, touch, or TE interface |
| Backlight | GPIO21, LEDC 5 kHz | UART0 TX conflicts with this pin; use USB Serial/JTAG |
| Buttons | `UP`, `DOWN`, `OK` on GPIO0 / ADC1_CH0 | One ADC unit and resistor ladder; simultaneous keys are not reliable |
| Codec | ES8311 at I2C address `0x18` | PCM read/write blocks and must stay outside callbacks/UI |
| Battery | CW2017 at I2C address `0x63` | Optional; return values may be unavailable or uncalibrated |
| Shared I2C | SDA GPIO10, SCL GPIO7 | Codec and gauge must reuse the BSP-owned I2C0 bus |
| I2S | MCLK 6, BCLK 5, WS 3, DOUT 2, DIN 4 | Full duplex I2S0, one active codec format at a time |
| USB console | Native USB Serial/JTAG on GPIO18/19 | Used for flashing and logs |

The three nominal button voltages are approximately 0 mV (`UP`), 300 mV
(`DOWN`), and 595 mV (`OK`). The release state is pulled near 3.3 V. Thresholds
are defined only in `bsp_pins.h`.

## Software layout

```text
app_main
  ├─ BSP init: I2C -> display/LVGL -> buttons -> audio -> battery
  ├─ control task: volume, device status, Wi-Fi reset
  ├─ Wi-Fi station or provisioning AP
  └─ realtime client
       ├─ WebSocket event task
       ├─ recording task
       └─ playback task
```

Application behavior belongs in `main/`. Reusable board access belongs in
`components/bsp/`. Pins, addresses, panel settings, and ADC windows must not be
duplicated outside `bsp_pins.h`.

## Concurrency rules

- Button callbacks only enqueue actions. They must not write NVS, access I2C,
  restart the MCU, record audio, or block.
- `bsp_audio_read` and `bsp_audio_write` are blocking calls. Recording and
  playback share a mutex because changing the ES8311 format closes and reopens
  the codec.
- LVGL is accessed by its task. Other tasks use `passport_ui_set` or
  `passport_ui_notice`, which enqueue bounded UI events.
- Wi-Fi callbacks only update event bits and schedule reconnection/UI state.
- Cancelling a response must stop accepting audio and free queued PCM blocks.
- Do not create another ADC1 oneshot unit or I2C0 master bus.

## Audio pipeline

Input:

```text
ES8311 -> PCM16 16 kHz mono -> 640 samples / 40 ms
       -> Float32 little-endian -> base64 -> WebSocket
```

Output:

```text
WebSocket -> Float32 little-endian 24 kHz mono -> PCM16
          -> 8-block queue -> 4-block (~400 ms) prebuffer -> ES8311
```

The Gateway normally emits 100 ms blocks. Queue capacity is 800 ms; the target
prebuffer is 400 ms. When `response.done` arrives for a short response, any
available audio may start without reaching four blocks.

The playback mutex and queue allocations must be reconsidered before changing
sample rates or block sizes. The board has no PSRAM.

## UI and pet assets

The UI uses a near-black background, warm-white text, low-contrast gray, and a
single electric-purple accent. Large software shadows are prohibited: a
174-pixel LVGL software shadow previously starved the single-core idle task and
triggered the watchdog. Use borders or pre-rendered assets instead.

The pet is a fixed product asset. Do not redraw, crop, recolor, reorder frames,
or change frame timing. The complete 384×416 source canvas is proportionally
resized to 144×156 and converted to LVGL I8.

State mapping:

| UI state | Asset |
| --- | --- |
| Booting, connecting, listening | `waiting` |
| Thinking | `running` |
| Ready, speaking | `idle` |
| Setup | `waving`, once |
| Error | `failed`, once |

To regenerate, first run an ESP-IDF build so the LVGL converter exists under
`managed_components/`, then install Pillow, pypng, lz4, and pngquant and run:

```bash
python tools/generate_pet_assets.py
```

The script deletes and regenerates only `main/pet_assets/passport_pet_*.c` and
validates dimensions, counts, and durations.

## Persistence

NVS uses two namespaces:

| Namespace | Data | Reset behavior |
| --- | --- | --- |
| `ripple` | SSID, password, Gateway host/port | Cleared by holding `UP` |
| `ripple_ui` | Volume | Preserved when Wi-Fi is reset |

Ordinary `idf.py flash` preserves both. `idf.py erase-flash` removes both.

## Build

The supported build is ESP-IDF 5.5.x:

```bash
source /path/to/esp-idf-v5.5.3/export.sh
idf.py set-target esp32c3
idf.py build size
```

Expected constraints:

- Custom 8 MB partition table.
- Factory application partition: 7 MB.
- USB Serial/JTAG console, not UART0.
- LVGL RGB565 with Montserrat 14 and 20.
- Image around 2.18 MB and DRAM below 50% in the validated configuration.

Do not edit `managed_components/`; update component manifests instead. Generated
directories are ignored by Git.

## Flash and monitor

```bash
idf.py -p /dev/cu.usbmodemXXXX flash monitor
```

Healthy startup contains, in order:

```text
显示就绪 240x320
LVGL 就绪
按键就绪
ES8311 就绪
hardware ready
wifi:connected
websocket connected
session ready
```

## Change-specific verification

### Display or assets

- Inspect orientation, full-frame pet bounds, transparency, animation timing,
  and watchdog output for at least ten seconds.

### Buttons or controls

- Check click and long-press separation for every key.
- Ensure a 3-second `UP` hold is required before Wi-Fi reset.
- Reboot after changing volume and verify persistence.

### Audio or networking

- Complete ten PTT turns, including two playback interruptions.
- Confirm `playback started with 400 ms buffered` on normal replies.
- Confirm no repeated `playback underrun`, queue overflow, reconnect loop, or
  steady reduction of minimum free heap.

### BSP changes

- Record the physical board revision and observed values.
- Validate optional-device failure behavior.
- Recheck ADC thresholds, I2C addresses, I2S clocking, and internal RAM usage as
  applicable.
