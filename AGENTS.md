# Ripple AI Passport Development Rules

## Scope

This branch is a standalone ESP-IDF firmware project for FoloToy AI Passport.
Do not add the Ripple Live mobile app, Agent Gateway implementation, deployment
stack, account UI, history, replay, meetings, memory, projects, or tool systems.
The Gateway is an external service governed by `docs/PROTOCOL.md`.

## Product contract

- Keep one primary interaction: hold `OK` to talk and release to send.
- Responses API is the only permitted model orchestration protocol.
- Preserve the current button mapping documented in the README unless the user
  explicitly requests a product change.
- Keep the UI minimal: near-black canvas, warm-white text, gray secondary text,
  and one electric-purple accent.
- Do not add a menu or bottom navigation.

## Pet asset contract

The pet is immutable. Do not redraw, replace, crop, recolor, regenerate with an
image model, change frame order/timing, or alter its appearance. Only its
surrounding container, spacing, and proportional display size may change.
Canonical sources are in `assets/pet-gifs/`; generated LVGL frames are in
`main/pet_assets/`.

## Architecture

- `main/`: product state, controls, provisioning, realtime protocol, UI, assets.
- `components/bsp/`: reusable board access only.
- `components/bsp/include/bsp_pins.h`: single source of truth for pins,
  addresses, ADC windows, and display parameters.
- `docs/PROTOCOL.md`: external Gateway contract.
- `docs/DEVELOPMENT.md`: hardware and concurrency constraints.

Button callbacks and Wi-Fi callbacks must remain non-blocking. Put NVS, I2C,
audio, restart, and other slow operations in worker tasks. LVGL access must stay
in its task or use the existing UI event API. Do not create a second ADC1 unit
or I2C0 bus.

## Build and validation

Use ESP-IDF 5.5.x, validated with 5.5.3:

```bash
idf.py set-target esp32c3
idf.py build size
idf.py -p PORT flash monitor
```

Never claim hardware validation from a build alone. For device changes, report
startup, display, buttons, audio, networking, heap, and any unverified physical
checks separately. Ordinary flashing must preserve NVS; do not erase flash
unless explicitly requested.

The ESP32-C3 has no PSRAM. Evaluate image data, LVGL buffers, queue depth, task
stacks, and temporary audio allocations against internal RAM. Avoid large LVGL
software shadows because they can trigger the watchdog.

## Style and commits

Use four-space C indentation, K&R braces, `snake_case`, `s_` file-local state,
and `BSP_*` hardware constants. Prefer focused Conventional Commit subjects.
Do not edit generated dependencies under `managed_components/`.
