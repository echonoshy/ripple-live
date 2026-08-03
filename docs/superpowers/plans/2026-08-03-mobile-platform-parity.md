# Ripple Live Mobile Platform Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace both phone launch icons with the supplied Ripple mark and package an iPhone build whose shared real-time features can request required native permissions.

**Architecture:** The existing React/Tauri application remains shared. A tracked icon source and macOS `sips` generator update Android and Xcode resources. A Node test validates resource dimensions and iOS privacy configuration before build.

**Tech Stack:** React, TypeScript, Tauri 2, Rust, Node test runner, macOS `sips`, Xcode.

## Global Constraints

- Use the supplied icon without redrawing or adding text.
- Keep bundle identifier `cn.minicpm.live`.
- Do not alter remote services, credentials, or signing identities.

---

### Task 1: Establish a failing mobile-package contract

**Files:**
- Create: `apps/android/tests/mobile-package.test.mjs`
- Modify: `apps/android/package.json`

- [ ] Write a Node `node:test` contract that reads PNG IHDR dimensions, requires `src-tauri/icons/ripple-live-source.png`, validates `icon.png` at 512x512, validates Android xxxhdpi launcher and round assets at 192x192, iterates every AppIcon `Contents.json` entry to validate `size * scale`, and requires `NSCameraUsageDescription` plus `NSMicrophoneUsageDescription` in the generated iOS `Info.plist`.
- [ ] Run `cd apps/android && node --test tests/mobile-package.test.mjs`; confirm it fails because the source icon and the two privacy keys are absent.
- [ ] Add `"test:mobile": "node --test tests/mobile-package.test.mjs"` to `apps/android/package.json`.

### Task 2: Implement canonical icon generation

**Files:**
- Create: `apps/android/src-tauri/icons/ripple-live-source.png`
- Create: `apps/android/scripts/generate-mobile-icons.sh`
- Modify: `apps/android/src-tauri/icons/*`
- Modify: `apps/android/src-tauri/gen/android/app/src/main/res/mipmap-*/ic_launcher*.png`
- Modify: `apps/android/src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset/*.png`

- [ ] Copy `/var/folders/jc/nb_k97t15h36xbd2l_yfynyc0000gn/T/codex-clipboard-ddae48b9-c49d-4aa6-aea0-5cbc5b442362.png` unchanged as the canonical source.
- [ ] Write a shell generator with `set -euo pipefail` and `/usr/bin/sips -z <pixels> <pixels> "$SOURCE" --out <target>` for shared Tauri PNGs, Android mdpi through xxxhdpi launcher/round/foreground assets, and each Xcode manifest entry; generate `icon.icns` using `iconutil`.
- [ ] Run `cd apps/android && bash scripts/generate-mobile-icons.sh`.
- [ ] Run `npm run test:mobile`; confirm only the absent privacy-key checks fail.

### Task 3: Enable iOS media permissions and pass checks

**Files:**
- Modify: `apps/android/src-tauri/gen/apple/ripple-live_iOS/Info.plist`

- [ ] Insert `<key>NSCameraUsageDescription</key><string>Ripple Live 使用相机进行实时视频通话。</string>` and `<key>NSMicrophoneUsageDescription</key><string>Ripple Live 使用麦克风进行实时语音通话。</string>` before `UILaunchStoryboardName`.
- [ ] Run `cd apps/android && npm run test:mobile`; expect zero failures.
- [ ] Run `npm run lint && npm run build`; expect exit code 0.
- [ ] Commit the source, generator, resource assets, test, package script, and Info.plist with message `feat(mobile): align Ripple icons and iOS permissions`.

### Task 4: Build and overwrite-install the iPhone app

**Files:**
- Create: `outputs/Ripple-Live-0.1.1.ipa`

- [ ] Run `cd apps/android && npm ci`, then create a development-signed iOS archive/export through Tauri/Xcode using bundle id `cn.minicpm.live`.
- [ ] Inspect IPA contents and run `codesign --verify --deep --strict` on the extracted app; use `plutil -p` to verify the bundle identifier and both privacy keys.
- [ ] Install over device `F163C439-44CB-5B60-88E8-65B46FF8097F` and launch `cn.minicpm.live` through `xcrun devicectl`.
- [ ] Confirm the home-screen icon, app launch, and permission prompts. Check realtime connection independently and report device-network failures separately from package validity.
