# Task 7 report: Android APK and production acceptance

## Outcome

The in-call camera upgrade passed the complete fresh mobile regression, production builds, APK integrity/signature inspection, Android cold-start smoke, and production Responses/realtime compatibility smokes at local HEAD `b3f9e2e445ab43a058d81d1e2f776f476ceefeec`.

No product defect was found and no application or server source was changed. The only material acceptance caveat is physical interaction: device `80e0a09e` remained on the authenticated lock screen with `NotificationShade` focused, so the lock was not bypassed and the nine touch/visual scenarios could not be manually exercised on the device. Their lifecycle, ordering, timing, failure, and ownership rules are covered by the fresh automated suites and production protocol smokes described below; unlocked-device visual and permission-sheet confirmation remains manual.

## Fresh local verification

All commands ran sequentially from `apps/mobile` and exited 0:

- `npm run test:live-ui`: 26/26
- `npm run test:tool-results`: 44/44
- `npm run test:live-media`: 44/44
- `npm run test:realtime`: 52/52
- `npm run test:mobile`: package 18/18, conversation actions 6/6, library 5/5
- `npm run test:media`: 21/21
- `npm run test:playback`: 5/5
- `npm run lint`: clean
- `npm run build`: TypeScript and Vite production build passed

Vite emitted only the existing advisory that the main minified chunk is larger than 500 kB.

With `JAVA_HOME=/opt/homebrew/Cellar/openjdk@17/17.0.20/libexec/openjdk.jdk/Contents/Home`:

- `npm run android:build` passed and produced the unsigned universal release APK.
- `npx tauri android build --apk --debug` passed and produced the signed universal debug APK used for device acceptance.

A direct standalone Gradle debug invocation was not used as acceptance evidence because Tauri's generated `rustBuild*` task requires the CLI-owned IPC server. The canonical Tauri debug build above supplied that server and completed all four ABIs.

## APK evidence

### Release

- Path: `/Users/lake/workspace/ripple-live/.worktrees/gpt-live-alignment/apps/mobile/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`
- Size: `61,085,880` bytes
- mtime: `2026-08-08T20:31:56+0800`
- SHA-256: `e3db1562523c5c249c012956d564c13fecd1932946812d0c394da9e8c0b91297`
- `unzip -tq`: no compressed-data errors
- `apksigner verify`: expected `DOES NOT VERIFY` / missing signature metadata because this artifact is explicitly unsigned
- `aapt`: package `cn.minicpm.live`, version `0.1.1` (`1001`), min SDK 24, target SDK 36

### Debug installed on device

- Path: `/Users/lake/workspace/ripple-live/.worktrees/gpt-live-alignment/apps/mobile/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`
- Size: `727,163,285` bytes
- mtime: `2026-08-08T20:33:19+0800`
- SHA-256: `3dd090f0e1655005c091fb2d7db154ed6fc1009106d65ba54e3b2276cde829d5`
- `unzip -tq`: no compressed-data errors
- `apksigner verify --verbose --print-certs`: verifies with APK Signature Scheme v2, one Android Debug signer; certificate SHA-256 `fd9ebd45a23774898e55aa9128156846c1130091518f83cf478d4eb8029a8a1f`
- `aapt`: package `cn.minicpm.live.debug`, launchable activity `cn.minicpm.live.MainActivity`, version `0.1.1` (`1001`), min SDK 24, target SDK 36, native code for `arm64-v8a`, `armeabi-v7a`, `x86`, and `x86_64`

`git diff --name-only master...HEAD -- apps/mobile/src-tauri/gen/apple apps/mobile/src-tauri/Info.ios.plist apps/mobile/src-tauri/tauri.ios.conf.json` produced no output. The frozen iOS source/configuration surface is unchanged.

## Android device acceptance

Device `80e0a09e` (`24117RK2CC`) accepted the freshly built debug APK with `adb install -r -t`. Package `lastUpdateTime` became `2026-08-08 20:35:01`.

- Package resolution selected `cn.minicpm.live.debug/cn.minicpm.live.MainActivity`.
- `am start -S -W` reported `Status: ok`, `LaunchState: COLD`, and a 397 ms total launch.
- The new process remained alive as PID `21798`; ActivityManager retained the correct MainActivity as resumed/focused app ownership.
- A fresh post-launch logcat scan found no `FATAL EXCEPTION`, package ANR, AndroidRuntime crash, abort, SIGSEGV/SIGABRT, or Chromium fatal crash.
- Process exit history after installation contained only the expected `PACKAGE UPDATED` exits and isolated WebView cleanup, not a crash/ANR for the current process.
- CAMERA and RECORD_AUDIO AppOps access times did not advance across the cold start. Camera service reported `Active Camera Clients: []`. This is device-level evidence that simply starting the app does not open the camera or microphone.

The device simultaneously reported `mDreamingLockscreen=true`, `mCurrentFocus=NotificationShade`; UIAutomator exposed only `com.android.systemui`. No unlock or authentication bypass was attempted.

## Nine-scenario acceptance matrix

| Scenario | Acceptance evidence | Physical caveat |
| --- | --- | --- |
| Voice begins without camera permission/access | `voice activation becomes ready without requesting camera`; package static no-auto-camera assertion; cold-start AppOps unchanged and no active camera client | Voice button could not be tapped while locked |
| Explicit camera tap keeps the same conversation/socket | Camera orchestration opens only after first frame plus matching v5 ACK; App creates audio session and reuses its exact media/session owner; realtime send-generation tests pass | Permission sheet and live preview not visually observed |
| Permission denial remains in audio | `camera permission failure returns to the orb with an explicit open retry` | Android permission-sheet wording not visually observed |
| First frame transitions in 420 ms | `camera closes only after audio acknowledgement and the 420ms transition` plus UI timing assertions | Crossfade not visually judged on hardware |
| Focus appears only for a requested frame | Realtime frame-request callback bracket tests and 160 ms minimum-visible UI tests pass | Focus frame not visually judged on hardware |
| Closing camera preserves audio/conversation | Server-first audio acknowledgement ordering, reverse 420 ms transition, and video-only track disposal tests pass | Live microphone continuity not listened to on hardware |
| Switching with a pending frame yields one response | Deployed server Task 2 sequencing tests cover pending video-to-audio release before ACK; realtime correlation/one-terminal-response tests pass | Pending-frame gesture sequence not manually repeated |
| Front/back flip does not restart audio | Camera replacement/latest-wins tests preserve microphone/playback ownership and dispose video tracks only | Physical front/back preview not observed |
| Network/mode failure reports last acknowledged/unknown mode | Failed video/audio correction, timeout, retained preview, retry, stale ACK, and send-generation isolation tests pass | Network failure banner not visually judged on hardware |

The automated evidence establishes the state-machine and resource-ownership contracts. The remaining unlocked-device pass is specifically visual/haptic/permission UX confirmation, not an observed functional failure.

## Production server and smoke acceptance

The remote checkout stayed clean at deployed commit `7a61d696e6b75f39a7bc5741edd65a54164a9676`. ASR, Agent, TTS, and Gateway remained running and healthy. The running Gateway image and the on-disk release binary both had SHA-256 `e89de218e2b98260d7086f7d514cc7c594c3dcaf0d78dc45ce965aff7414667c`.

Fresh production smokes used a random one-hour test identity inserted into the existing auth store. The token was never printed. A shell trap removed its sessions and user-owned rows; read-only follow-up queries reported zero `camera-qa-*` users, conversations, or auth sessions.

- Responses API-only function continuation passed: `calculate(7 * 8)` produced an opaque call ID and final result 56. No alternate Agent API was used.
- Dedicated v4 compatibility passed: audio session negotiated v4; correlated `session.mode.set` rejection returned `unsupported_protocol`; the same socket then completed a response.
- Dedicated v5 compatibility passed: invalid mode returned correlated `invalid_mode`; the same socket acknowledged audio-to-video and video-to-audio, then completed a response.
- Full v4 realtime smoke passed Gateway health/readiness, model-gate ignore, on-demand JPEG request/commit, first-result milestones, Responses-backed tool loop, barge-in, response isolation, and four unique terminal responses. First text was 0.617 s, first audio 0.735 s, with 43 audio chunks / 407,040 transport bytes.
- Final production status remained healthy for all four services.

## Final status

Task 7 is complete with no source fix required. Release and signed debug APKs are fresh and verified; the current debug build is installed and cold-start stable; production remains v4/v5 compatible and Responses-only. The sole remaining human action is to unlock the connected Android device and visually/tactually confirm the nine interaction scenarios.
