# Ripple Live Mobile Home and Library Design

## Goal

Make voice and video equally discoverable on the mobile home screen, adopt a restrained Kiro-inspired dark interface, and keep growing chat history and visual memory collections manageable across devices.

## Product Direction

The redesign uses Kiro as a visual reference rather than copying its desktop layout. Ripple Live keeps its own identity: a near-black plum canvas, quiet elevated panels with precise one-pixel borders, and violet reserved for live state and primary feedback. Information density increases in library screens, while the call entry screen remains calm and immediate.

The signature interaction is a low-amplitude Ripple pulse around the central mark. It runs only while the app is ready, becomes a responsive waveform during a call, and stops when reduced-motion is enabled. No other element uses continuous animation.

## Home Screen

- Preserve the brand header and the three utility destinations: visual memory, chat history, and settings.
- Keep the central Ripple mark, ready status, headline, and supporting sentence, but tighten the vertical rhythm so the primary actions remain fully visible on smaller iPhones.
- Replace the large voice button plus small video button with two equal-width cards in a two-column grid.
- Each card contains an icon, a short label, and a one-line description. Voice uses `语音通话 / 只听声音`; video uses `视频通话 / 看见现场`.
- Neither mode receives permanent visual priority. Press, focus, connection, and permission states use the same treatment for both cards.
- Use a 3.5-second ambient pulse on the mark, a brief 180-millisecond press response on controls, and honor `prefers-reduced-motion`.

## Shared Library Model

Chat history and visual memory use one shared organization model so the two screens do not teach different behaviors.

- Group active items by local calendar day: `今天`, `昨天`, a localized month/day label for the preceding six days, then `更早`.
- Pinned items appear in a `已标记` section before chronological groups and do not appear a second time below it.
- Provide text search across chat title/preview and memory note/visual summary.
- Provide an `全部 / 已标记 / 已归档` filter. Archived items are excluded from `全部` and from agent recall, but remain recoverable.
- Tapping a card opens its detail. Swiping left reveals `标记`, `归档`, and `删除`; long press enters multi-select mode for the same actions.
- Destructive deletion always requires confirmation and states that it cannot be undone. Archive and pin are immediate, reversible actions with inline feedback.
- Empty, loading, error, no-search-result, and partial-operation states must each have distinct copy and recovery actions.

## Chat History

- Use compact full-width rows with title, two-line preview, relative time, and a pin indicator.
- Preserve the existing conversation transcript view and attachments.
- Add server-persisted pin, archive, unarchive, delete, and batch mutation APIs.
- Deleting a conversation removes its turns and conversation record. Visual memories created from that conversation remain in the memory library; their source relationship becomes nullable so saved memories are not unexpectedly lost.
- Opening a pinned conversation does not change its pin ordering. Within each section, order by `updated_at` descending.

## Visual Memory

- Default to a compact two-column cover grid. A memory without an image uses a branded text cover.
- Each card shows a short note, capture date, and pin indicator; the full visual summary appears only in detail or search results to reduce scroll length.
- Preserve note editing and image viewing. Add the same pin, archive, unarchive, delete, and batch operations used by chat history.
- Archived memories must be excluded from the context compiler and memory recall/search used by the agent, while remaining visible in the app's archive filter.
- Within each section, order by `captured_at` when present, otherwise `created_at`, descending.

## Service and Data Design

The service remains on `140.143.229.103:8700`; server code and deployment are changed through passwordless SSH on that host. The mobile client remains local.

Add `is_pinned INTEGER NOT NULL DEFAULT 0` and `archived_at REAL` to both `conversations` and `memory_items` using the existing additive `ensure_column` migration pattern. Change `memory_items.conversation_id` and `source_turn_id` to nullable relationships. Because SQLite cannot relax an existing `NOT NULL` constraint in place, rebuild `memory_items` inside one migration transaction for existing databases, copy every row, recreate its indexes, and run `PRAGMA foreign_key_check` before committing. Conversation deletion first clears the two source columns, then removes turns and the conversation so saved memories survive.

Extend serialized records with:

```text
ConversationSummary.is_pinned: boolean
ConversationSummary.archived_at: number | null
MemoryRecord.is_pinned: boolean
MemoryRecord.archived_at: number | null
```

Add authenticated routes:

```text
PATCH  /v1/conversations/{id}       { is_pinned?: boolean, archived?: boolean }
DELETE /v1/conversations/{id}
POST   /v1/conversations/batch      { ids: string[], action: pin|unpin|archive|unarchive|delete }

PATCH  /v1/memories/{id}            { user_note?: string, is_pinned?: boolean, archived?: boolean }
POST   /v1/memories/batch           { ids: string[], action: pin|unpin|archive|unarchive|delete }
```

List endpoints accept `scope=active|archived|all`, `query`, and a bounded `limit`. The service validates ownership for every ID and performs batch operations transactionally. A batch containing an unknown or unauthorized ID fails without partially mutating other items.

## Visual System

- Canvas `#100E15`
- Raised surface `#191621`
- Strong surface `#211D2A`
- Hairline `#35303E`
- Primary ink `#F4F1F7`
- Ripple violet `#A97BFF`

Use the system sans-serif stack with three roles: compact uppercase/medium status labels, semibold display headlines, and regular body text. Corners remain soft but decrease from the current oversized controls: 22 pixels for primary cards, 16 pixels for library rows, and 12 pixels for chips. Shadows are avoided; depth comes from tone, border, and spacing.

All interactive elements have visible focus treatment, at least a 44-by-44-point target, VoiceOver labels, and non-color state indicators. Layout must work from 320 CSS pixels wide through tablet widths and respect safe-area insets.

## Error Handling and Optimistic Updates

Pin and archive may update optimistically, then roll back with a visible error if the request fails. Delete waits for the service response before removing the card. Search input is debounced locally; the current bounded collection is filtered immediately, while the API query keeps results correct once the dataset exceeds the first page.

Existing sessions and records receive defaults during migration. Older clients continue to function because new response properties are additive and existing routes retain their current behavior.

## Verification

- Rust store tests cover migration defaults, ownership, pin/archive ordering, archived recall exclusion, surviving memories after conversation deletion, and atomic batch failure.
- Route tests cover authentication, request validation, filtering, search, and all mutation actions.
- Mobile tests cover date grouping, search/filter behavior, optimistic rollback, confirmation before deletion, batch selection, equal home actions, and reduced motion.
- Run `cargo test -p ripple-agent-gateway`, `npm run test:mobile`, `npm run lint`, and `npm run build` before deployment.
- Deploy only the gateway changes to the remote host, verify the active listener and health endpoint, then make authenticated create/update/list/delete smoke requests against port `8700`.
- Run `npm run ios:dev` locally and visually verify the home, chat history, visual memory, archive, search, multi-select, small-screen, and reduced-motion states in the iOS simulator.

## Out of Scope

- User-created folders, arbitrary tags, cloud photo import, sharing, export, and undo-after-delete.
- Changes to realtime audio/video protocol, authentication, call behavior, model routing, or the hidden fixed service address.
- A separate native SwiftUI implementation or platform-specific feature fork.
