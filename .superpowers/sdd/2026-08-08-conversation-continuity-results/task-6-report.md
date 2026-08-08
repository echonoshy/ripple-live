# Task 6 report: Render persisted actions in Android history

## Outcome

- Added backward-compatible `ConversationMessage.actions` normalization. Older responses without `actions` become an empty list; malformed action rows are ignored and non-finite `due_at` values become `null`.
- Added safe memory/todo action chips to conversation history. Unknown kinds, empty/inherited/throwing targets, and unsafe labels do not become navigable success UI; React escapes labels and no raw HTML path exists.
- Memory actions navigate to the memory library, fetch the exact target, and open its detail sheet. Missing/deleted or failed targets surface the real load error.
- Todo actions navigate to the todo library, search active then completed items, switch the status tab when needed, and focus/scroll the exact target with reduced-motion support. Missing/deleted or failed targets degrade to an error without inventing success.
- Added 44 px action targets and dark GPT Live-aligned chip styling without changing the live result tray.
- No backend or iOS files were modified.

## TDD evidence

- RED: `npm run test:mobile` failed because `ConversationActions.tsx` did not exist.
- RED: the throwing-proxy action test failed at the unsafe property descriptor access.
- RED: the legacy normalization test failed because `normalizeConversationMessages` did not exist.
- GREEN: behavior tests now cover legacy missing actions, finite due normalization, recognized kinds only, malformed/inherited/throwing targets, safe escaped rendering, and memory/todo activation routing.

## Verification

All passed from `apps/mobile`:

- `npm run test:tool-results` (44 tests)
- `npm run test:realtime` (33 tests)
- `npm run test:mobile` (17 package + 4 conversation action + 5 library tests)
- `npm run test:playback` (5 tests)
- `npm run test:media` (6 tests)
- `npm run test:live-ui` (23 tests)
- `npm run lint` (no warnings)
- `npm run build`
- `JAVA_HOME=/opt/homebrew/Cellar/openjdk@17/17.0.20/libexec/openjdk.jdk/Contents/Home npm run android:build`
- `unzip -t` on the produced APK

APK:

- Path: `/Users/lake/workspace/ripple-live/.worktrees/gpt-live-alignment/apps/mobile/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`
- Built: `2026-08-08 17:59:18`
- Size: `61,061,224` bytes
- SHA-256: `a9b251e0f1f4856cbd33f53035a46c071e6179a9fe2f32135fee15590e4b5f20`
- ZIP integrity: no errors detected
