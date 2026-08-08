# Task 6 fix report: total conversation action normalization

## Review finding

`normalizeConversationMessages` read action fields directly. Inherited fields could be copied into an own-property action and rendered, while throwing proxies could abort the complete history response.

## Fix

- Reads message/action/array fields only through guarded own data-property descriptors.
- Rejects inherited fields and accessors without invoking them.
- Safely bounds messages to 500, actions to 10, and attachments to 100.
- Contains throwing/revoked-style array, action, and message proxies.
- Drops only malformed rows; valid messages and valid sibling actions remain available.
- Keeps legacy missing `actions` behavior as an empty list and finite `due_at` normalization unchanged.

## TDD evidence

- RED: inherited action plus a throwing action proxy caused the normalization call to throw.
- RED: a throwing messages/actions array proxy caused the complete normalization call to throw.
- GREEN: direct entrypoint tests verify inherited fields are rejected, throwing action rows are dropped, throwing arrays become empty, malformed message proxies are skipped, and later valid rows survive.

## Verification

Passed:

- `npm run test:tool-results` (44)
- `npm run test:realtime` (33)
- `npm run test:mobile` (17 package + 6 conversation action + 5 library)
- `npm run test:playback` (5)
- `npm run test:media` (6)
- `npm run test:live-ui` (23)
- `npm run lint`
- `npm run build`
- Android universal release build with the configured JDK 17
- APK ZIP integrity check

APK after fix:

- Path: `/Users/lake/workspace/ripple-live/.worktrees/gpt-live-alignment/apps/mobile/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk`
- Built: `2026-08-08 18:05:13`
- Size: `61,065,464` bytes
- SHA-256: `d61e8d5be7d681ebd87049add71b2ee74cad3012a556189e979db4dadf1efd22`
