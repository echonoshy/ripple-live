# Ripple Live Mobile

Shared Tauri 2 + React client for the self-hosted Ripple multimodal Agent on
Android and iOS.

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
- a continuous model-gated flow where the server commits complete semantic
  decisions immediately and uses a 1.5-second fallback for other pauses;
- a spoken stop command silences output and is not added to chat history;
- on-demand camera capture only after the server accepts a video turn;
- server-side structured tool calls and visible tool status;
- response interruption only after the model accepts a new turn, so unrelated
  speech does not stop playback;
- continuous 24 kHz float32 response playback through an AudioWorklet ring
  buffer, with 450 ms startup buffering and underrun recovery;
- a new session ID for every voice or video call so separate calls never share
  conversation history;
- a configurable plain-WebSocket server address.

The default endpoint is:

```text
ws://YOUR_SERVER_IP:8700/v1/agent/realtime
```

Realtime sessions use protocol v4. Wake words and manual wake state are not part
of the client protocol; the server model decides whether each speech turn needs
a response.

Every time the user starts a call, the client creates a fresh UUID and sends it
as the WebSocket `session_id`. Reconnecting by starting another call therefore
creates an empty conversation instead of restoring the previous call.

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

## iOS build

iOS development requires macOS with Xcode and the Rust iOS targets installed.
From this directory, initialize or refresh the generated host and build with:

```bash
npm ci
npm run ios:init
npm run ios:build
```

Use `npm run ios:dev` to launch through Xcode during development. The signing
team and iOS-specific bundle identifier are defined in
`src-tauri/tauri.ios.conf.json`.

## Security note

This initial deployment permits cleartext `ws://`. Audio, camera frames,
transcripts, tool arguments, and model responses are not encrypted. Move to
`wss://` and add access control before using sensitive content or exposing the
service broadly.
