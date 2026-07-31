# Ripple Live Android

Tauri 2 + React client for the self-hosted Ripple multimodal Agent.

The app supports:

- VAD-delimited voice turns with 16 kHz mono float32 transport;
- camera frames sampled at one frame per second during a video turn;
- server-side structured tool calls and visible tool status;
- response interruption when the user starts speaking;
- 24 kHz float32 response playback;
- a new session ID for every voice or video call so separate calls never share
  conversation history;
- a configurable plain-WebSocket server address.

The default endpoint is:

```text
ws://YOUR_SERVER_IP:8700/v1/agent/realtime
```

Every time the user starts a call, the client creates a fresh UUID and sends it
as the WebSocket `session_id`. Reconnecting by starting another call therefore
creates an empty conversation instead of restoring the previous call.

## Build

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

## Security note

This initial deployment permits cleartext `ws://`. Audio, camera frames,
transcripts, tool arguments, and model responses are not encrypted. Move to
`wss://` and add access control before using sensitive content or exposing the
service broadly.
