# Task 3 report: Live result sheet and reliable receipts

## Scope

- Added `LiveResultSheet.tsx`, which accepts only `LiveResult[]` and exhaustively renders memory receipts, todo receipts/lists, search sources, weather, and generic statuses.
- Added `liveResultsReducer` with a deterministic latest-three policy. A duplicate `callId` replaces its current entry in place; dismissal removes only the selected result.
- Wired correlated `onToolResult` events through `parseLiveResult` in `App.tsx`. Results clear on the next user transcript, a new call, a fatal session failure, leave, or explicit dismissal. A response-level failure leaves already-confirmed receipts visible, and audio callbacks do not read or change result state.
- Guarded result and artifact callbacks by active session identity and null the session owner before close, preventing late completions from updating result state after leave.
- Coalesced repeated leave requests into one in-flight close so a second tap cannot finish early and allow the first close to overwrite a later call lifecycle.
- Stacked result and authenticated artifact sheets in one bounded output tray above the call controls. Only result presence applies the 280 ms stage shift; reduced-motion removes the stage/card motion.
- Kept all work in the local mobile client. No backend or iOS files changed.

## Rendering and safety

- Success receipts are emitted only by validated success variants from the existing parser. Generic failures use muted-red card and label styling.
- Search cards render at most three canonical, unique, upstream-validated HTTP(S) sources. An accessible button opens each source through Tauri's native opener on mobile or an isolated browser tab in development; no raw anchor navigation or raw HTML rendering is used.
- Todo lists render at most five rows. Each result has its own labelled 44 px dismissal control, and the sheet is a labelled polite live region.
- Out-of-range finite todo timestamps cannot throw during rendering; the receipt remains visible and simply omits an invalid date label.
- The shared output tray lets artifacts and results coexist without adding any playback-side effects or audio lifecycle calls.

## TDD evidence

1. Added the structural/safety package test before the component existed. `npm run test:mobile` failed with `LiveResultSheet.tsx should exist` (12 passed, 1 failed).
2. Added reducer tests before the reducer behavior. The initial stub failed all four intended cases: latest-three capping, duplicate replacement, sequential dismissal, and clearing.
3. Implemented the reducer and reran the focused suite with those behaviors passing.
4. Review identified a render crash for finite timestamps outside JavaScript's date range. Added a server-render regression first; it failed with `RangeError: Invalid time value`, then passed after validating `Date.getTime()`.
5. Review also identified repeated leave-call re-entry. Added a single-flight regression first; the stub returned two different promises, then passed after coalescing repeated requests until close completion.

## Verification

All commands ran from `apps/mobile` after the review fixes:

- `npm run test:tool-results` — 32 passed.
- `npm run test:realtime` — 26 passed.
- `npm run test:mobile` — 13 package tests plus 5 library tests passed.
- `npm run test:playback` — 5 passed.
- `npm run test:media` — 6 passed.
- `npm run test:live-ui` — 23 passed.
- `npm run lint` — passed without warnings.
- `npm run build` — passed. Vite emitted only its existing large-chunk advisory.
- `git diff --check` — passed before report creation and is rerun before commit.

## Review disposition and remaining concern

- Independent review found no critical issues. Both important findings (invalid-date rendering and re-entrant leave) were fixed with RED/GREEN regression coverage; the muted-red failure-label minor finding was also fixed.
- Automated checks establish bounded layout and coexistence structurally, but this task did not perform physical-device visual QA of every three-card-plus-artifact combination. That remains an Android acceptance check rather than a code/test failure.

## Controller review round 1

- Added connection-generation cancellation to `RealtimeSession`. Closing during a pending connection invalidates the attempt; a transport that opens or resolves late is closed without being assigned, activated, started, or allowed to emit callbacks. Text and transport callbacks also ignore closed sessions.
- Guarded every `App` session callback capable of mutating UI, media, or playback state with active-session ownership. Unmount and leave clear ownership before asynchronous teardown, so an old session cannot affect a replacement session.
- Preserved confirmed live results across `onResponseFailed`; only the response error message changes. Receipts still clear at the next user turn, call/leave boundary, fatal session error, or explicit dismissal.
- Deduplicated search results by canonical URL within the original first-three source boundary, preserving first-source order and stable URL keys.
- Replaced raw search anchors with `@tauri-apps/plugin-opener` 2.5.4. On supported targets, its Tauri capability is scoped to `https://*` and `http://*` only. The browser/dev fallback uses a new `noopener,noreferrer` tab and never replaces the live call page; opener errors are contained as a no-op and do not touch playback.
- Updated both `package-lock.json` and `Cargo.lock` with the corresponding opener dependencies. No generated Apple/iOS file changed.

### Round 1 TDD and verification

- Deferred WebSocket regressions cover close-before-open, late ready/message suppression with transport closure, isolation from a newer session, and settlement when a stale socket closes before opening.
- External-link regressions cover isolated browser opening, contained native failure, rejection of non-HTTP schemes, absence of raw anchor navigation, native dependency/init metadata, and the HTTP(S)-only capability scope.
- Parser regression proves canonical duplicate URLs are skipped while the first three unique results are retained. A response-failure regression proves confirmed receipts are not cleared.
- Final suites: mobile package 16/16 plus library 5/5; playback 5/5; media 6/6; live UI 23/23; realtime 29/29; tool results 36/36. `npm run lint` and `npm run build` passed; Vite emitted only its existing large-chunk advisory.
- The initial online Cargo check stalled while updating the registry index and was bounded/terminated. With dependencies present and lockfile resolved, `cargo check --offline --quiet` and `cargo metadata --offline --no-deps` both passed. `git diff --check` passed.

## Controller review round 2

- Added a synchronous call-lifecycle generation guard with explicit opening, active, leaving, failed, and idle phases. Leave invalidates ownership and navigates home before awaiting transport close; both the scheduled auto-start and `startCall` claim the same owner. A delayed close cannot create replacement media or a second session, and a failed connect cannot enter an automatic retry loop.
- App connect failure now invalidates the current owner, clears both owner refs, stops media, triggers/awaits `session.close()`, and presents the error without leaving the call eligible for auto-start.
- `RealtimeSession.connect()` now treats connection/start rejection as terminal for that generation: it marks the session closed, closes an assigned transport, and ignores later transport/text callbacks. Browser messages and native queued messages are activated only after the initial session-start send succeeds, so a ready arriving while that send is pending cannot start media after failure. Existing close-before-connect-resolution behavior remains covered.
- Search parsing validates only the original first three input rows, then deduplicates their canonical URLs. A proxied 10,000-row duplicate payload proves exactly four outer descriptor reads (length plus indices 0–2), independent of unique output count.
- The Rust opener dependency is target-scoped with `cfg(not(target_os = "ios"))`, plugin registration has the same compile-time guard, and its capability applies only to Linux, macOS, Windows, and Android. The JavaScript opener checks for iOS before native detection, dynamically imports the plugin only on the supported native path, and performs no plugin or browser operation on iOS. No Apple/iOS source or generated file changed.

### Round 2 TDD and verification

- RED/GREEN regressions cover delayed leave with exactly one session/media owner, failed-connect generation invalidation without auto-retry, rejected/deferred session-start transport closure with ready suppression, bounded duplicate-heavy parser work, and side-effect-free iOS link handling.
- Structural native tests cover the target-specific Cargo dependency, compile-time Rust registration guard, non-iOS capability platforms, and unchanged HTTP(S)-only scope.
- Final verification after the hook-lint cleanup: mobile package 16/16 plus library 5/5; playback 5/5; media 6/6; live UI 23/23; realtime 30/30; tool results 40/40. `npm run lint` completed without diagnostics, `npm run build` passed with only the existing Vite large-chunk advisory, offline Cargo check/target metadata passed, `git diff --check` passed, and the diff contains no Apple/iOS paths.
