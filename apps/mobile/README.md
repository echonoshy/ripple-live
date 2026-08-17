# Ripple Live Mobile

Shared Tauri 2 + React client for the self-hosted Ripple multimodal Agent.
Android is the active delivery target. The iOS host is retained but frozen.

## Directory layout

```text
src/                         Shared React UI and realtime media logic
public/                      Shared browser assets and AudioWorklets
src-tauri/src/               Shared native Rust entry points
src-tauri/gen/android/       Tauri-generated Android/Gradle host project
src-tauri/gen/apple/         Tauri-generated iOS/Xcode host project
src-tauri/tauri.conf.json    Shared Tauri configuration
src-tauri/tauri.ios.conf.json iOS configuration override
```

The Android and Apple directories intentionally remain below `src-tauri/gen`.
They are platform hosts generated for one shared Tauri application, rather than
independent app implementations.

The app supports:

- VAD produces tentative voice pauses with a one-second local pre-roll and
  16 kHz mono float32 transport; idle audio is not uploaded;
- a continuous model-gated flow where a `complete` semantic decision commits
  immediately, while `continue` remains tentative and resumable until the
  1.5-second fallback expires;
- a spoken stop command silences output and is not added to chat history;
- on-demand camera capture only after the server accepts a video turn;
- server-side structured tool calls and visible tool status;
- response interruption when a new speech turn starts, with correlated
  cancellation across playback, transport, and server generation;
- continuous 24 kHz float32 response playback through an AudioWorklet ring
  buffer, with 450 ms startup buffering and underrun recovery;
- a new conversation for calls started from Home, plus explicit continuation
  of an existing conversation from its history detail screen;
- a configurable plain-WebSocket server address.

The default endpoint is:

```text
ws://YOUR_SERVER_IP:8700/v1/agent/realtime
```

Realtime sessions use protocol v5. The session negotiates audio/video mode and
supports in-call camera transitions. The server-side response gate decides
whether each completed speech turn needs a response.

## Web UI debugging

The browser preview runs the same React UI and media code as the Android
WebView, so most layout and interaction changes can be checked without building
or installing an APK.

From `apps/mobile`, run:

```bash
npm ci
npm run web:dev
```

Then open <http://127.0.0.1:1420>. Use the browser's responsive device toolbar
to select an Android viewport (for example, 360 x 800 or 412 x 915). The preview
connects to the configured Ripple server directly, including its HTTP APIs and
WebSocket realtime endpoint.

To open the preview from another device on the same network, run:

```bash
npm run web:dev:lan
```

Then visit `http://YOUR_COMPUTER_LAN_IP:1420`. A LAN page served over plain HTTP
is useful for visual layout checks, but mobile browsers normally block microphone
and camera access outside a secure context. Use `127.0.0.1`/`localhost` on the
development computer when testing realtime audio or video.

Starting a call from Home omits `conversation_id`, so the server creates a new
conversation. Starting a call from a conversation detail screen sends that
conversation's ID and continues its stored history.

## Android build

Install the local Android SDK/NDK once:

```bash
./scripts/setup-android.sh
```

Configure the current shell:

```bash
export ANDROID_HOME="$PWD/.android-sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export NDK_HOME="$ANDROID_HOME/ndk/27.2.12479018"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"
```

Then build the ARM64 debug APK:

```bash
npm ci
npm run lint
npm run android:build -- --debug --target aarch64
```

The APK is written to:

```text
src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

Install it on a USB-connected Android device with:

```bash
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

Android 7.0 (API 24) or newer is required. The app requests microphone and
camera permission at runtime.

## iOS status

The generated iOS host and shared code are retained for compatibility, but iOS
development and delivery are frozen. Do not extend or modify the iOS-specific
implementation unless the project scope explicitly reactivates it.

## Security note

This initial deployment permits cleartext `ws://`. Audio, camera frames,
transcripts, tool arguments, and model responses are not encrypted. Move to
`wss://` and add access control before using sensitive content or exposing the
service broadly.
