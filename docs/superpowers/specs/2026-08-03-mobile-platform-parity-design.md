# Ripple Live Mobile Platform Parity Design

## Goal

Ship an iPhone build that replaces the current Ripple Live app, uses the supplied Ripple mark on both phone home screens, and retains the Android real-time audio/video experience.

## Design

Android and iOS already run the React application under `apps/android/src` and the Tauri runtime under `apps/android/src-tauri`. Preserve that shared architecture. Track the supplied PNG as the canonical source, derive each Android launcher and Xcode AppIcon raster from it, and add the two iOS privacy declarations needed by the existing `LiveMedia` calls to `navigator.mediaDevices.getUserMedia`.

## Constraints

- Keep bundle identifier `cn.minicpm.ripplelive.debug`, which is the existing Ripple Live app on the paired phone, to overwrite it.
- Do not change the realtime service, its endpoint, credentials, or signing identity.
- Do not fork a separate Swift UI; audio, video, camera switching, mute, real-time text/audio, settings, and WebSocket behavior remain in the shared code path.

## Verification

Use a dependency-free Node test for icon sizes and iOS privacy keys, then run the frontend checks, Xcode archive/export, signature verification, paired-device install, launch, permissions, and endpoint checks separately.
