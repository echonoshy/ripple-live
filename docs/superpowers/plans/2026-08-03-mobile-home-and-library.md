# Ripple Live Mobile Home and Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give voice and video equal prominence on the home screen and add server-synchronized organization controls to chat history and visual memory.

**Architecture:** Extend the Rust/SQLite gateway in its canonical remote repository with additive library metadata, authenticated mutation endpoints, and archived-memory exclusion. Add small pure TypeScript library helpers plus reusable React controls around the existing local shared Tauri app; keep realtime call code unchanged. Build and deploy the gateway only on `140.143.229.103`, while building and testing the mobile client locally.

**Tech Stack:** Rust 2024, Axum 0.8, SQLx/SQLite, React 19, TypeScript 6, Vite 8, Tauri 2, Node test runner, CSS.

## Global Constraints

- Preserve the fixed hidden endpoint `140.143.229.103:8700`; do not expose it in login or settings.
- Keep bundle identifiers, authentication, realtime WebSocket protocol, model routing, camera/audio behavior, and signing configuration unchanged.
- Use exactly this core palette: canvas `#100E15`, raised surface `#191621`, strong surface `#211D2A`, hairline `#35303E`, primary ink `#F4F1F7`, Ripple violet `#A97BFF`.
- Keep every interactive target at least 44 by 44 CSS pixels and provide accessible names and visible focus states.
- Honor `prefers-reduced-motion`; only the ready mark may animate continuously.
- Preserve uncommitted user changes in `services/agent-gateway/src/orchestrator.rs` and unrelated mobile signing/ATS files.
- Run Tasks 1 and 2 in `/home/lake/workspace/ripple-live` on `140.143.229.103`; run Tasks 3 through 7 in `/Users/lake/workspace/ripple-live` on the local Mac.

---

### Task 1: Add persistent library metadata and migration coverage

**Files:**
- Remote modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/context.rs`
- Remote modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/memory.rs`

**Interfaces:**
- Produces: `LibraryScope`, `LibraryAction`, and additive `is_pinned`/`archived_at` response fields.
- Produces: nullable `MemoryRecord.conversation_id` source relationship internally while keeping the public memory response backward compatible.

- [ ] **Step 1: Write failing store tests**

Add tests that open a temporary database, create two users, and assert defaults, ownership, ordering, archive filtering, atomic batch rejection, archived recall exclusion, and memory survival after conversation deletion. Use these exact public calls in assertions:

```rust
let active = store
    .list_conversations(&user.id, LibraryScope::Active, false, "", 50)
    .await
    .unwrap();
assert!(!active[0].is_pinned);
assert_eq!(active[0].archived_at, None);

store
    .mutate_conversations(&user.id, &[conversation.clone()], LibraryAction::Pin)
    .await
    .unwrap();
assert!(store.list_conversations(&user.id, LibraryScope::Active, false, "", 50)
    .await.unwrap()[0].is_pinned);

store
    .mutate_conversations(&user.id, &[conversation.clone()], LibraryAction::Archive)
    .await
    .unwrap();
assert!(store.list_conversations(&user.id, LibraryScope::Active, false, "", 50)
    .await.unwrap().is_empty());
assert_eq!(store.list_conversations(&user.id, LibraryScope::Archived, false, "", 50)
    .await.unwrap().len(), 1);
```

For memory recall, archive the created memory and assert:

```rust
assert!(service.recall(&user.id, "蓝色转接头", 5).await.unwrap().is_empty());
assert_eq!(service.list(&user.id, LibraryScope::Archived, false, "", 10)
    .await.unwrap().len(), 1);
```

- [ ] **Step 2: Run the tests and confirm the missing APIs fail compilation**

Run:

```bash
ssh 140.143.229.103
cd /home/lake/workspace/ripple-live
cargo test --manifest-path services/agent-gateway/Cargo.toml context::tests memory::tests
```

Expected: compilation fails because `LibraryScope`, `LibraryAction`, and mutation arguments do not exist.

- [ ] **Step 3: Add library types and schema migration**

Define shared types in `context.rs`:

```rust
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryScope { Active, Archived, All }

impl Default for LibraryScope {
    fn default() -> Self { Self::Active }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryAction { Pin, Unpin, Archive, Unarchive, Delete }
```

Extend both serialized records:

```rust
pub is_pinned: bool,
pub archived_at: Option<f64>,
```

In `initialize`, use `ensure_column` for `is_pinned` and `archived_at`. Add a private `migrate_memory_sources_nullable` transaction that inspects `PRAGMA table_info(memory_items)`, rebuilds `memory_items` only when `conversation_id` is still `NOT NULL`, copies every named column, recreates `idx_memory_items_user`, runs `PRAGMA foreign_key_check`, and commits only when the check returns no rows.

- [ ] **Step 4: Implement filtered list and transactional mutation methods**

Change the conversation signature to:

```rust
pub async fn list_conversations(
    &self,
    user_id: &str,
    scope: LibraryScope,
    pinned_only: bool,
    query: &str,
    limit: i64,
) -> anyhow::Result<Vec<ConversationSummary>>
```

Use bound SQL predicates for ownership, scope, optional `is_pinned = 1`, and `LIKE` search; order by `is_pinned DESC, updated_at DESC`. Add:

```rust
pub async fn mutate_conversations(
    &self,
    user_id: &str,
    ids: &[String],
    action: LibraryAction,
) -> anyhow::Result<usize>
```

Reject empty lists and more than 100 IDs, verify `COUNT(*) == ids.len()` inside the transaction, update pin/archive fields for reversible actions, and for delete clear memory source relationships before deleting `turn_attachments`, `turns`, `events`, `sessions`, and `conversations`.

Change memory list to:

```rust
pub async fn list(
    &self,
    user_id: &str,
    scope: LibraryScope,
    pinned_only: bool,
    query: &str,
    limit: usize,
) -> anyhow::Result<Vec<MemoryRecord>>
```

Add `MemoryService::mutate(user_id, ids, action)` with the same validation and transaction boundary. Reuse the existing single-memory asset cleanup after the database transaction for delete actions. Add `archived_at IS NULL` to recall queries before scoring.

- [ ] **Step 5: Run store tests**

Run:

```bash
cd /home/lake/workspace/ripple-live
cargo fmt --manifest-path services/agent-gateway/Cargo.toml -- --check
cargo test --manifest-path services/agent-gateway/Cargo.toml context::tests memory::tests
```

Expected: formatting check and all store tests pass.

- [ ] **Step 6: Commit the data layer**

```bash
git add services/agent-gateway/src/context.rs services/agent-gateway/src/memory.rs
git commit -m "feat(library): persist organized history states"
```

---

### Task 2: Expose authenticated library APIs

**Files:**
- Remote modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/main.rs`
- Remote modify: `/home/lake/workspace/ripple-live/services/agent-gateway/Cargo.toml`
- Remote modify: `/home/lake/workspace/ripple-live/services/agent-gateway/Cargo.lock`

**Interfaces:**
- Consumes: `LibraryScope`, `LibraryAction`, `ContextStore::mutate_conversations`, and `MemoryService::mutate` from Task 1.
- Produces: list query parameters plus conversation and memory PATCH/DELETE/batch HTTP routes.

- [ ] **Step 1: Write failing route-level request tests**

Extract router construction into `fn app(state: AppState) -> Router` so a test can call routes without opening a public listener. Add `tower = { version = "0.5", features = ["util"] }` under dev dependencies and test authenticated requests with `oneshot`:

```rust
let response = app(state.clone())
    .oneshot(
        Request::patch(format!("/v1/conversations/{conversation}"))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"is_pinned":true}"#))
            .unwrap(),
    )
    .await
    .unwrap();
assert_eq!(response.status(), StatusCode::OK);
```

Cover unauthorized access, another user's ID, archive filtering, query matching, empty batch rejection, atomic mixed-owner batch rejection, and delete returning `204`.

- [ ] **Step 2: Run the route tests and confirm failure**

Run:

```bash
ssh 140.143.229.103
cd /home/lake/workspace/ripple-live
cargo test --manifest-path services/agent-gateway/Cargo.toml main::tests
```

Expected: requests return `404 Method Not Allowed` or fail to compile before routes and payloads exist.

- [ ] **Step 3: Add exact request/query structures and routes**

Define:

```rust
#[derive(Debug, Deserialize)]
struct LibraryListQuery {
    #[serde(default)]
    scope: LibraryScope,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    query: String,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LibraryPatch {
    is_pinned: Option<bool>,
    archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BatchMutation {
    ids: Vec<String>,
    action: LibraryAction,
}

#[derive(Debug, Deserialize)]
struct MemoryPatch {
    user_note: Option<String>,
    is_pinned: Option<bool>,
    archived: Option<bool>,
}
```

Register static batch routes before item routes and preserve existing GET behavior:

```rust
.route("/v1/conversations/batch", post(batch_conversations))
.route("/v1/conversations/{conversation_id}", patch(update_conversation).delete(delete_conversation))
.route("/v1/memories/batch", post(batch_memories))
.route("/v1/memories/{memory_id}", get(get_memory).patch(update_memory).delete(delete_memory))
```

Return the updated record for PATCH, `204` for DELETE, `{ "updated": number }` for batch success, `400` for invalid combinations, `401` for missing auth, and `404` for unknown or unauthorized records.

- [ ] **Step 4: Run gateway verification**

Run:

```bash
cd /home/lake/workspace/ripple-live
cargo fmt --manifest-path services/agent-gateway/Cargo.toml
cargo test --manifest-path services/agent-gateway/Cargo.toml
cargo clippy --manifest-path services/agent-gateway/Cargo.toml -- -D warnings
```

Expected: all tests pass and clippy exits with code 0.

- [ ] **Step 5: Commit the API**

```bash
git add services/agent-gateway/Cargo.toml services/agent-gateway/Cargo.lock services/agent-gateway/src/main.rs
git commit -m "feat(library): expose history management APIs"
```

---

### Task 3: Add mobile library types, API calls, and grouping helpers

**Files:**
- Create: `apps/mobile/src/library.ts`
- Create: `apps/mobile/tests/library.test.mjs`
- Modify: `apps/mobile/src/api.ts`
- Modify: `apps/mobile/package.json`

**Interfaces:**
- Produces: `LibraryApiScope`, `LibraryView`, `LibraryAction`, `LibraryItem`, `groupLibraryItems`, `matchesLibraryQuery`, `conversationMutation`, `memoryMutation`, and batch API functions.
- Consumed by: Tasks 4 through 6.

- [ ] **Step 1: Write failing helper tests**

Create deterministic tests by passing `now` explicitly:

```js
const groups = groupLibraryItems(items, new Date('2026-08-03T12:00:00+08:00'))
assert.deepEqual(groups.map((group) => group.label), ['已标记', '今天', '昨天', '7月29日', '更早'])
assert.equal(groups.flatMap((group) => group.items).length, items.length)
assert.equal(matchesLibraryQuery(items[0], '红茶 配料'), true)
assert.equal(matchesLibraryQuery(items[0], '不存在'), false)
```

Also assert archived items only appear under the archived scope and pinned items are not duplicated in date groups.

- [ ] **Step 2: Run the tests and confirm module-not-found failure**

Run:

```bash
cd apps/mobile && node --test tests/library.test.mjs
```

Expected: failure because `src/library.ts` has not been compiled for Node import.

- [ ] **Step 3: Implement pure helpers and compile them for tests**

Add `"test:library": "tsc src/library.ts --outDir .test-dist --module nodenext --moduleResolution nodenext --target es2022 && node --test tests/library.test.mjs"` and make `test:mobile` run both package and library tests. Define:

```ts
export type LibraryApiScope = 'active' | 'archived' | 'all'
export type LibraryView = 'all' | 'pinned' | 'archived'
export type LibraryAction = 'pin' | 'unpin' | 'archive' | 'unarchive' | 'delete'

export type LibraryItem = {
  id: string
  title: string
  searchableText: string
  timestamp: number
  isPinned: boolean
  archivedAt: number | null
}

export type LibraryGroup = { label: string; items: LibraryItem[] }
```

`groupLibraryItems` first extracts pinned active items, then groups the remainder by local midnight boundaries. `matchesLibraryQuery` lowercases and splits trimmed whitespace so every query token must appear in `searchableText`. List API options use `{ scope: LibraryApiScope; pinned?: boolean; query: string; limit: number }`; the UI maps `all` to `{ scope: 'active' }`, `pinned` to `{ scope: 'active', pinned: true }`, and `archived` to `{ scope: 'archived' }`.

- [ ] **Step 4: Extend API types and calls**

Add `is_pinned: boolean` and `archived_at: number | null` to both record types. Change list calls to accept `{ scope, pinned, query, limit }` and URL-encode `URLSearchParams`. Add:

```ts
export function updateConversation(
  server: string,
  token: string,
  id: string,
  patch: { is_pinned?: boolean; archived?: boolean },
): Promise<ConversationSummary>

export function batchConversations(
  server: string,
  token: string,
  ids: string[],
  action: LibraryAction,
): Promise<{ updated: number }>

export function batchMemories(
  server: string,
  token: string,
  ids: string[],
  action: LibraryAction,
): Promise<{ updated: number }>
```

Broaden `updateMemory` to accept `{ user_note?, is_pinned?, archived? }` while preserving the existing edit caller.

- [ ] **Step 5: Run mobile helper checks**

Run:

```bash
cd apps/mobile && npm run test:mobile && npm run lint && npm run build
```

Expected: helper tests and package contract pass; build exits with code 0.

- [ ] **Step 6: Commit mobile data utilities**

```bash
git add apps/mobile/src/api.ts apps/mobile/src/library.ts apps/mobile/tests/library.test.mjs apps/mobile/package.json
git commit -m "feat(mobile): add synchronized library utilities"
```

---

### Task 4: Build shared library controls and interaction state

**Files:**
- Create: `apps/mobile/src/components/LibraryToolbar.tsx`
- Create: `apps/mobile/src/components/LibrarySection.tsx`
- Create: `apps/mobile/src/components/LibraryActions.tsx`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: Task 3 `LibraryView`, `LibraryAction`, and grouped item types.
- Produces: reusable toolbar, date section, swipe/long-press action presentation, selection mode, confirmation dialog, and optimistic rollback callback contract.

- [ ] **Step 1: Add failing source and accessibility contracts**

Extend the Node package test to require component imports and stable accessible copy:

```js
for (const file of ['LibraryToolbar.tsx', 'LibrarySection.tsx', 'LibraryActions.tsx']) {
  assert.equal(existsSync(path.join(appRoot, 'src/components', file)), true)
}
assert.match(appSource, /aria-label="搜索聊天历史"/)
assert.match(appSource, /删除后无法恢复/)
assert.match(cssSource, /@media \(prefers-reduced-motion: reduce\)/)
```

- [ ] **Step 2: Run the package test and confirm failure**

Run `cd apps/mobile && npm run test:mobile`.

Expected: missing component files and copy assertions fail.

- [ ] **Step 3: Implement reusable controls**

Use these prop contracts:

```ts
type LibraryToolbarProps = {
  kind: '聊天历史' | '视觉记忆'
  query: string
  scope: LibraryView
  selectionCount: number
  onQueryChange(value: string): void
  onScopeChange(value: LibraryView): void
  onBatchAction(action: LibraryAction): void
  onCancelSelection(): void
}

type LibraryActionsProps = {
  pinned: boolean
  archived: boolean
  onAction(action: LibraryAction): void
}

type LibrarySectionProps = {
  label: string
  count: number
  children: ReactNode
}
```

Render search with a visible label, three scope chips (`全部`, `已标记`, `已归档`), a selection toolbar when `selectionCount > 0`, and icon-plus-text actions. Use pointer start/end distance for left-swipe reveal and a 500-millisecond pointer timer for long-press selection; cancel the timer on movement, pointer cancellation, or unmount.

- [ ] **Step 4: Add shared optimistic mutation state to App**

Maintain separate history and memory query/scope/selection state. Implement a generic local updater:

```ts
async function optimisticMutation<T extends { id: string; is_pinned: boolean; archived_at: number | null }>(
  items: T[],
  ids: string[],
  action: LibraryAction,
  persist: () => Promise<unknown>,
  setItems: (items: T[]) => void,
  setError: (message: string) => void,
) {
  const previous = items
  setItems(applyLibraryAction(items, ids, action))
  try { await persist() } catch (error) {
    setItems(previous)
    setError(error instanceof Error ? error.message : '操作失败，请重试')
  }
}
```

For delete, show an in-app `role="alertdialog"` confirmation with `删除后无法恢复` and remove items only after the API succeeds.

- [ ] **Step 5: Run checks and commit**

Run `cd apps/mobile && npm run test:mobile && npm run lint && npm run build` and expect all commands to succeed.

```bash
git add apps/mobile/src/components apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): add shared library controls"
```

---

### Task 5: Redesign chat history management

**Files:**
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: Task 3 APIs/helpers and Task 4 controls.
- Produces: grouped, searchable, pinnable, archivable, deletable, and multi-select chat history.

- [ ] **Step 1: Add failing chat-history contract assertions**

Assert the source contains the group and state copy and no longer directly maps the flat collection:

```js
assert.match(appSource, /groupLibraryItems/)
assert.match(appSource, /没有找到相关对话/)
assert.match(appSource, /已归档的对话会保留，但不会出现在最近记录中/)
assert.doesNotMatch(appSource, /historyItems\.map\(\(item\)/)
```

- [ ] **Step 2: Run the package test and confirm failure**

Run `cd apps/mobile && npm run test:mobile`.

Expected: new chat-history source assertions fail.

- [ ] **Step 3: Render grouped chat rows and actions**

Map each `ConversationSummary` to:

```ts
{
  id: item.id,
  title: item.title || '未命名对话',
  searchableText: `${item.title} ${item.preview}`,
  timestamp: item.updated_at,
  isPinned: item.is_pinned,
  archivedAt: item.archived_at,
}
```

Render `LibraryToolbar`, then `LibrarySection` groups. Each row includes checkbox state in selection mode, title, two-line preview, time, a visible pin icon, and `LibraryActions`. Tapping opens the transcript only when selection mode is off. Archived empty state explains retention; no-results state clears the query.

- [ ] **Step 4: Wire server persistence**

Single actions call `updateConversation` or the existing DELETE route. Batch actions call `batchConversations`. Clear selected IDs after success; keep them selected after failure. Reload the current scope after archive/unarchive so pagination remains correct.

- [ ] **Step 5: Run checks and commit**

Run `cd apps/mobile && npm run test:mobile && npm run lint && npm run build` and expect success.

```bash
git add apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(history): organize and manage conversations"
```

---

### Task 6: Redesign visual memory management

**Files:**
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: Task 3 memory APIs/helpers and Task 4 controls.
- Produces: grouped two-column memory library with preserved edit and image behavior.

- [ ] **Step 1: Add failing visual-memory contract assertions**

```js
assert.match(appSource, /aria-label="搜索视觉记忆"/)
assert.match(appSource, /memory-library-grid/)
assert.match(appSource, /没有找到相关记忆/)
assert.match(cssSource, /grid-template-columns:\s*repeat\(2, minmax\(0, 1fr\)\)/)
```

- [ ] **Step 2: Run the package test and confirm failure**

Run `cd apps/mobile && npm run test:mobile`.

Expected: visual-memory assertions fail.

- [ ] **Step 3: Render compact grouped memory cards**

Use `captured_at ?? created_at` for grouping and search both `user_note` and `visual_summary`. Cards render a 4:3 authenticated cover or branded text cover, two-line note, time, pin indicator, and action reveal. Tapping opens a detail sheet containing the full summary, existing note editor, and delete action. Selection mode replaces tap behavior with checkbox toggling.

- [ ] **Step 4: Wire memory persistence and archive behavior**

Single pin/archive/note updates use `updateMemory`; batch mutations use `batchMemories`; deletion continues to wait for the server. When archive scope is active, expose `取消归档` instead of `归档`.

- [ ] **Step 5: Run checks and commit**

Run `cd apps/mobile && npm run test:mobile && npm run lint && npm run build` and expect success.

```bash
git add apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(memory): organize visual memory library"
```

---

### Task 7: Apply the Kiro-inspired home treatment and equal call entries

**Files:**
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: existing `openCall('audio' | 'video')` behavior without changing realtime code.
- Produces: equal voice/video cards and reduced-motion Ripple pulse.

- [ ] **Step 1: Add failing equality and motion assertions**

```js
assert.match(appSource, /<strong>语音通话<\/strong>/)
assert.match(appSource, /<small>只听声音<\/small>/)
assert.match(appSource, /<strong>视频通话<\/strong>/)
assert.match(appSource, /<small>看见现场<\/small>/)
assert.match(cssSource, /\.launch-actions\s*{[^}]*grid-template-columns:\s*repeat\(2, minmax\(0, 1fr\)\)/s)
assert.match(cssSource, /animation:\s*ripple-ready-pulse 3\.5s/)
```

- [ ] **Step 2: Run the package test and confirm failure**

Run `cd apps/mobile && npm run test:mobile`.

Expected: copy, grid, and animation assertions fail.

- [ ] **Step 3: Update markup and token system**

Remove primary/secondary launch semantics. Give both buttons `className="launch-button call-entry"`, equal icons, labels, and descriptions. Replace root color values with the six specified palette values, reduce library row radii to 16 pixels, chips to 12 pixels, and action cards to 22 pixels. Use border/tone instead of drop shadows.

- [ ] **Step 4: Add the signature motion and responsive rules**

Add:

```css
.ready-mark::before {
  content: "";
  position: absolute;
  inset: -12px;
  border: 1px solid rgba(169, 123, 255, 0.32);
  border-radius: inherit;
  animation: ripple-ready-pulse 3.5s ease-in-out infinite;
}

@keyframes ripple-ready-pulse {
  0%, 100% { opacity: 0.32; transform: scale(0.94); }
  50% { opacity: 0.9; transform: scale(1.04); }
}

@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    scroll-behavior: auto !important;
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

At widths below 360 pixels, retain two equal columns but reduce the gap and icon size; at tablet widths, cap the home content at 520 pixels and the library at 720 pixels.

- [ ] **Step 5: Run checks and commit**

Run `cd apps/mobile && npm run test:mobile && npm run lint && npm run build` and expect success.

```bash
git add apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(home): balance voice and video entries"
```

---

### Task 8: Verify, deploy, and smoke-test the complete flow

**Files:**
- Remote verify: `/home/lake/workspace/ripple-live/services/agent-gateway`
- Local verify: `/Users/lake/workspace/ripple-live/apps/mobile`

**Interfaces:**
- Consumes: all preceding deliverables.
- Produces: current remote gateway, authenticated API evidence, and local iOS simulator evidence.

- [ ] **Step 1: Run the complete verification suite**

```bash
ssh 140.143.229.103 'cd /home/lake/workspace/ripple-live && cargo fmt --manifest-path services/agent-gateway/Cargo.toml -- --check && cargo test --manifest-path services/agent-gateway/Cargo.toml && cargo clippy --manifest-path services/agent-gateway/Cargo.toml -- -D warnings'
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:mobile
npm run lint
npm run build
```

Expected: every command exits with code 0.

- [ ] **Step 2: Back up the remote database**

Resolve the active `RIPPLE_DATA_DIR` from `systemctl --user show ripple-agent-gateway.service --property=ExecStart` and validate it equals `/home/lake/workspace/ripple-live/.cache/agent-gateway`. Create a new timestamped directory under `/home/lake/workspace/ripple-live/.cache/backups/` and use SQLite's online `.backup` command to produce a consistent `context.sqlite3` copy without stopping the live service. Run `PRAGMA integrity_check` against the copy and require `ok`. Do not copy local SQLite, assets, credentials, or mobile files to the host.

- [ ] **Step 3: Build and restart the gateway on the remote host**

On `140.143.229.103`, build the release binary from the already-modified canonical remote repository, stop only `ripple-agent-gateway.service`, and restart only that unit. Then run:

```bash
cd /home/lake/workspace/ripple-live
cargo build --release --manifest-path services/agent-gateway/Cargo.toml
systemctl --user restart ripple-agent-gateway.service
./deploy/agent-stack/status.sh
curl --fail --silent http://127.0.0.1:8700/health
```

Expected: gateway unit is active and health returns JSON with `"ok":true`; ASR, agent, and TTS services remain untouched.

- [ ] **Step 4: Run authenticated API smoke requests**

Use an existing test account or create one with a configured invitation code without printing credentials. Exercise active list, pin, archive, archived list, unarchive, search, and delete for a disposable conversation and memory. Confirm `401` without a token, `404` for another user's ID, and archived memory no longer appears in agent recall.

- [ ] **Step 5: Run the iOS preview and inspect all requested states**

Run `cd /Users/lake/workspace/ripple-live/apps/mobile && npm run ios:dev`, launch the iPhone simulator, and verify:

```text
Home: equal voice/video cards, ready pulse, small-width layout
History: Today/yesterday/older groups, search, pin, archive, delete, batch selection
Memory: two-column grid, detail/edit, search, pin, archive, delete, batch selection
Accessibility: VoiceOver names, 44px targets, visible focus, reduced-motion behavior
Persistence: relaunch and re-login retain server-backed pin/archive state
```

- [ ] **Step 6: Record final verification and commit scoped fixes**

If local verification required a correction, rerun the directly affected failing test first and then the full checks before committing:

```bash
git add apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/src/api.ts apps/mobile/src/library.ts apps/mobile/src/components apps/mobile/tests apps/mobile/package.json
git commit -m "fix(mobile): correct library interaction defects"
```

If remote verification required a correction, commit only gateway files from Tasks 1 and 2 in the remote repository:

```bash
ssh 140.143.229.103
cd /home/lake/workspace/ripple-live
git add services/agent-gateway/Cargo.toml services/agent-gateway/Cargo.lock services/agent-gateway/src/context.rs services/agent-gateway/src/memory.rs services/agent-gateway/src/main.rs
git commit -m "fix(library): correct management API defects"
```

If no correction was needed, leave both worktrees unchanged and report the exact successful commands, remote health, and simulator observations.
