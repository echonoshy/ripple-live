# Ripple Live Android

Tauri 2 + Rust Android client for the MiniCPM-o 4.5 realtime service.

The app supports:

- full-duplex voice conversation;
- camera + voice conversation;
- 16 kHz mono float32 microphone streaming;
- 24 kHz float32 response playback;
- a configurable plain-WebSocket server address.

The default server is `140.143.229.103:8600`. The two API endpoints are:

```text
ws://140.143.229.103:8600/v1/realtime?mode=audio
ws://140.143.229.103:8600/v1/realtime?mode=video
```

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

This initial deployment intentionally permits cleartext `ws://`. Audio, camera
frames, prompts, and model responses are not encrypted, and the public service
does not include authentication. Move to `wss://` and add access control before
using it with sensitive content or exposing it broadly.
