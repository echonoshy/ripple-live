# Ripple Live Conversation Continuity and Results Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep live calls inside one resumable conversation and render validated tool results, memory receipts, todo receipts, and persisted action chips without interrupting speech.

**Architecture:** Add a typed validation boundary around the already-emitted `response.tool.completed.result` payload, then render a small union of approved cards. Wire `conversation_id` through call start/end so calls return to the same timeline. Extend the server conversation-message response with action references derived from existing `memory_items.source_turn_id` and `todos.source_turn_id`; no new database table is required.

**Tech Stack:** React 19, TypeScript 6, Tauri 2 Android, Rust 2024, Axum, SQLx/SQLite, Node test runner, Cargo tests.

## Global Constraints

- Android APK is the only mobile implementation target; do not modify or regenerate iOS.
- Responses API remains the only allowed Agent API protocol.
- Backend edits, tests, builds, and deployment run on `lake@140.143.229.103` in the remote Ripple Live checkout.
- Unknown or malformed tool JSON falls back to a text status and is never rendered as arbitrary HTML or an arbitrary card.
- A success receipt appears only when the actual tool result contains `ok: true` and a validated target object.
- Memory is saved only after an explicit user request; this plan does not add automatic save behavior.
- A call started from an existing conversation must reuse its `conversation_id`; ending the call returns to that conversation.
- Live result cards must not stop, restart, or delay audio playback.

## File Structure

- Create `apps/mobile/src/realtime/toolResults.ts`: validated `LiveResult` union and parser.
- Create `apps/mobile/src/components/LiveResultSheet.tsx`: weather/list/memory/todo/generic result presentation.
- Create `apps/mobile/src/components/ConversationActions.tsx`: persisted action chips in history.
- Create `apps/mobile/tests/tool-results.test.ts`: parser tests.
- Modify `apps/mobile/src/realtime/RealtimeSession.ts`: emit typed tool-completion callbacks.
- Modify `apps/mobile/tests/realtime-session.test.ts`: correlation and malformed-result coverage.
- Modify `apps/mobile/src/api.ts`: conversation-detail API and message action types.
- Modify `apps/mobile/src/App.tsx`: current conversation ownership, resume flow, result state, and post-call routing.
- Modify `apps/mobile/src/components/LiveCallScreen.tsx`: render `LiveResultSheet`.
- Modify `apps/mobile/src/live/LiveCall.css`: result and receipt transitions.
- Modify `apps/mobile/tests/mobile-package.test.mjs`: continuity and safe-result assertions.
- Modify remote `services/agent-gateway/src/context.rs`: serialize `ConversationAction` entries from existing source relationships.
- Modify remote `services/agent-gateway/src/main.rs`: extend route tests for message actions.

---

### Task 1: Validated tool-result union

**Files:**
- Create: `apps/mobile/src/realtime/toolResults.ts`
- Create: `apps/mobile/tests/tool-results.test.ts`
- Modify: `apps/mobile/package.json`

**Interfaces:**
- Consumes: `{ callId, name, result }` from a correlated realtime tool-completion event.
- Produces: `LiveResult` and `parseLiveResult(event): LiveResult`.

- [ ] **Step 1: Write failing parser tests**

```ts
import assert from 'node:assert/strict'
import test from 'node:test'
import { parseLiveResult } from '../src/realtime/toolResults.ts'

test('creates a memory receipt only for a successful validated memory', () => {
  const result = parseLiveResult({
    callId: 'call-1', name: 'remember',
    result: { ok: true, memory: { id: 'mem_1', user_note: '65W 充电器' } },
  })
  assert.deepEqual(result, {
    kind: 'memory_receipt', callId: 'call-1', memoryId: 'mem_1',
    title: '65W 充电器', status: 'success',
  })
})

test('creates a todo receipt with an optional due time', () => {
  const result = parseLiveResult({
    callId: 'call-2', name: 'create_todo',
    result: { ok: true, todo: { id: 'todo_1', title: '带充电器', due_at: 1786323600 } },
  })
  assert.equal(result.kind, 'todo_receipt')
  if (result.kind === 'todo_receipt') assert.equal(result.dueAt, 1786323600)
})

test('bounds web search cards to three validated sources', () => {
  const result = parseLiveResult({
    callId: 'call-3', name: 'web_search',
    result: { ok: true, data: { results: [
      { title: 'One', url: 'https://one.example', snippet: 'First' },
      { title: 'Two', url: 'https://two.example', snippet: 'Second' },
      { title: 'Three', url: 'https://three.example', snippet: 'Third' },
      { title: 'Four', url: 'https://four.example', snippet: 'Fourth' },
    ] } },
  })
  assert.equal(result.kind, 'search')
  if (result.kind === 'search') assert.equal(result.items.length, 3)
})

test('never trusts malformed or failed result payloads', () => {
  assert.deepEqual(
    parseLiveResult({ callId: 'bad', name: 'remember', result: '<script>' }),
    { kind: 'generic', callId: 'bad', label: '记忆操作未完成', status: 'error' },
  )
  assert.deepEqual(
    parseLiveResult({ callId: 'bad-2', name: 'create_todo', result: { ok: false } }),
    { kind: 'generic', callId: 'bad-2', label: '待办创建未完成', status: 'error' },
  )
})
```

- [ ] **Step 2: Add and run the focused test command**

Add:

```json
"test:tool-results": "tsx --test tests/tool-results.test.ts"
```

Run: `cd apps/mobile && npm run test:tool-results`
Expected: FAIL because `toolResults.ts` does not exist.

- [ ] **Step 3: Implement the closed result union and validators**

```ts
export type LiveResult =
  | { kind: 'memory_receipt'; callId: string; memoryId: string; title: string; status: 'success' }
  | { kind: 'todo_receipt'; callId: string; todoId: string; title: string; dueAt: number | null; status: 'success' }
  | { kind: 'todo_list'; callId: string; titles: string[]; completed: boolean; status: 'success' }
  | { kind: 'search'; callId: string; items: Array<{ title: string; url: string; snippet: string }>; status: 'success' }
  | { kind: 'weather'; callId: string; location: string; summary: string; temperature: number | null; status: 'success' }
  | { kind: 'generic'; callId: string; label: string; status: 'success' | 'error' }

export type ToolCompletion = { callId: string; name: string; result: unknown }
```

Use `isRecord(value): value is Record<string, unknown>` plus exact string/number checks. Cap labels at 120 characters, todo-list rows at five, search rows at three, snippets at 240 characters, and accept only absolute `http:`/`https:` source URLs. Recognize `remember`, `create_todo`, `list_todos`, `web_search`, and `weather_lookup`; all other tools return a generic completed/failed label. External tools read their stable payload under `result.data`; built-in tools read their target directly under `result`.

- [ ] **Step 4: Run parser tests**

Run: `cd apps/mobile && npm run test:tool-results`
Expected: all parser tests PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/mobile/src/realtime/toolResults.ts apps/mobile/tests/tool-results.test.ts apps/mobile/package.json
git commit -m "feat(mobile): validate live tool results"
```

---

### Task 2: Correlated tool completions from `RealtimeSession`

**Files:**
- Modify: `apps/mobile/src/realtime/RealtimeSession.ts`
- Modify: `apps/mobile/tests/realtime-session.test.ts`

**Interfaces:**
- Consumes: `response.tool.completed` containing `response_id`, `call_id`, `name`, and `result`.
- Produces: `SessionOptions.onToolResult(event: ToolCompletion): void` only for the current response and non-empty call ID.

- [ ] **Step 1: Extend the test harness and add failing correlation tests**

```ts
const results: ToolCompletion[] = []
// pass onToolResult: (result) => results.push(result)

test('emits a correlated completed tool result', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-1' })
  harness.receive({
    type: 'response.tool.completed', response_id: 'response-1',
    call_id: 'call-1', name: 'remember', result: { ok: true },
  })
  assert.deepEqual(harness.results, [
    { callId: 'call-1', name: 'remember', result: { ok: true } },
  ])
})

test('ignores stale and uncorrelated tool results', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'response-current' })
  harness.receive({ type: 'response.tool.completed', response_id: 'stale', call_id: 'call-1', name: 'remember', result: { ok: true } })
  harness.receive({ type: 'response.tool.completed', response_id: 'response-current', name: 'remember', result: { ok: true } })
  assert.deepEqual(harness.results, [])
})
```

- [ ] **Step 2: Run realtime tests to verify failure**

Run: `cd apps/mobile && npm run test:realtime`
Expected: TypeScript/test failure because `onToolResult` is missing.

- [ ] **Step 3: Add event fields and callback**

Add `call_id?: string` to `RealtimeEvent`, add `onToolResult` to `SessionOptions`, and in `response.tool.completed` call:

```ts
if (event.call_id && event.name) {
  this.options.onToolResult({
    callId: event.call_id,
    name: event.name,
    result: event.result,
  })
}
```

Keep the current `isCurrentResponse(event)` guard before the callback.

- [ ] **Step 4: Update every session constructor and run tests**

Run: `cd apps/mobile && npm run test:realtime && npm run build`
Expected: all tests and the build PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/mobile/src/realtime/RealtimeSession.ts apps/mobile/tests/realtime-session.test.ts apps/mobile/src/App.tsx
git commit -m "feat(mobile): surface correlated tool completions"
```

---

### Task 3: Live result sheet and reliable receipts

**Files:**
- Create: `apps/mobile/src/components/LiveResultSheet.tsx`
- Modify: `apps/mobile/src/components/LiveCallScreen.tsx`
- Modify: `apps/mobile/src/live/LiveCall.css`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: `LiveResult[]` and existing authenticated artifact rendering.
- Produces: a bottom sheet that keeps at most three results per response and uses `callId` for deduplication.

- [ ] **Step 1: Add failing structural and safety assertions**

```js
const resultSource = readFileSync(path.join(appRoot, 'src/components/LiveResultSheet.tsx'), 'utf8')
const callSource = readFileSync(path.join(appRoot, 'src/components/LiveCallScreen.tsx'), 'utf8')
assert.match(resultSource, /memory_receipt/)
assert.match(resultSource, /todo_receipt/)
assert.match(resultSource, /weather/)
assert.match(resultSource, /search/)
assert.doesNotMatch(resultSource, /dangerouslySetInnerHTML/)
assert.match(callSource, /<LiveResultSheet/)
```

- [ ] **Step 2: Run package tests to verify failure**

Run: `cd apps/mobile && npm run test:mobile`
Expected: FAIL because `LiveResultSheet.tsx` is absent.

- [ ] **Step 3: Implement result-specific rendering**

Render memory and todo results as compact success receipts; weather as a summary card; web search as at most three source rows with safe external links; todo lists as at most five rows; generic results as a one-line status. The component accepts only `LiveResult`, never raw `unknown`.

```ts
export type LiveResultSheetProps = {
  results: LiveResult[]
  onDismiss(callId: string): void
}
```

- [ ] **Step 4: Wire response lifecycle in `App.tsx`**

On `onUserText`, clear previous response results. On `onToolResult`, parse and deduplicate by `callId`. Do not clear results when audio deltas arrive. Clear them on a new user turn, leave-call, or explicit dismissal.

- [ ] **Step 5: Add 280ms entry and reduced-motion styles**

The core moves upward/scales down only when a result sheet is present. Receipts use the same cold-black/blue palette; failures use muted red. No card uses an independent infinite animation.

- [ ] **Step 6: Verify mobile checks**

Run: `cd apps/mobile && npm run test:tool-results && npm run test:realtime && npm run test:mobile && npm run lint && npm run build`
Expected: all commands PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/mobile/src/components/LiveResultSheet.tsx apps/mobile/src/components/LiveCallScreen.tsx apps/mobile/src/live/LiveCall.css apps/mobile/src/App.tsx apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): render live results and action receipts"
```

---

### Task 4: Conversation ID ownership and resume flow

**Files:**
- Modify: `apps/mobile/src/api.ts`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/components/ConversationHome.tsx`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`
- Modify: `apps/mobile/tests/realtime-session.test.ts`

**Interfaces:**
- Consumes: optional `conversationId` in the existing `RealtimeSession` constructor and `onConversation(id)`.
- Produces: `openCall(mode, conversationId?)`; `conversation(server, token, id)` API; post-call routing to `screen === 'conversation'`.

- [ ] **Step 1: Add failing API and UI assertions**

```js
const apiSource = readFileSync(path.join(appRoot, 'src/api.ts'), 'utf8')
assert.match(apiSource, /export async function conversation\(/)
assert.match(appSource, /conversationId:\s*activeConversationId/)
assert.match(appSource, /onConversation:\s*setActiveConversationId/)
assert.match(appSource, /openCall\('audio', selectedConversation\.id\)/)
```

- [ ] **Step 2: Add a realtime URL regression test**

Create a test transport/WebSocket harness that connects with `conversationId: 'conv_existing'` and assert the URL contains `conversation_id=conv_existing`. Retain the existing test that a new session omits the query parameter.

- [ ] **Step 3: Run tests to verify failure**

Run: `cd apps/mobile && npm run test:realtime && npm run test:mobile`
Expected: FAIL on the missing App/API wiring.

- [ ] **Step 4: Add the conversation-detail API**

```ts
export async function conversation(server: string, token: string, id: string) {
  const payload = await request<{ data: ConversationSummary }>(
    server,
    `/v1/conversations/${encodeURIComponent(id)}`,
    {},
    token,
  )
  return payload.data
}
```

- [ ] **Step 5: Wire call ownership**

Add `activeConversationId: string | null`. `openCall(mode, conversationId?)` stores the optional ID; `RealtimeSession` receives it; `onConversation` stores the server-confirmed ID. On leave, fetch the summary by ID, set `selectedConversation`, and route to `conversation`. If the fetch fails, route to the chat home and show a non-blocking history refresh error.

- [ ] **Step 6: Add “继续语音” to conversation detail**

The button calls `openCall('audio', selectedConversation.id)`. Ending that call returns to the same detail screen and reloads its messages.

- [ ] **Step 7: Verify and commit**

Run: `cd apps/mobile && npm run test:realtime && npm run test:mobile && npm run lint && npm run build`
Expected: all commands PASS.

```bash
git add apps/mobile/src/api.ts apps/mobile/src/App.tsx apps/mobile/src/components/ConversationHome.tsx apps/mobile/tests/mobile-package.test.mjs apps/mobile/tests/realtime-session.test.ts
git commit -m "feat(mobile): continue realtime conversations"
```

---

### Task 5: Persisted conversation actions from existing server data

**Files:**
- Modify on remote host: `services/agent-gateway/src/context.rs`
- Modify on remote host: `services/agent-gateway/src/main.rs`

**Interfaces:**
- Consumes: existing `memory_items.source_turn_id` and `todos.source_turn_id` rows owned by the conversation user.
- Produces: `ConversationMessage.actions: Vec<ConversationAction>` where action kind is `memory` or `todo`.

- [ ] **Step 1: Verify the remote checkout and create the implementation branch**

Run locally:

```bash
git push -u origin codex/gpt-live-alignment
ssh lake@140.143.229.103 '
  cd ~/workspace/ripple-live &&
  git status --short &&
  git fetch origin &&
  (git switch codex/gpt-live-alignment || git switch --track -c codex/gpt-live-alignment origin/codex/gpt-live-alignment) &&
  git pull --ff-only &&
  git rev-parse HEAD
'
```

Expected: clean server worktree on the fixed branch `codex/gpt-live-alignment` at the plan base commit.

- [ ] **Step 2: Write a failing `ContextStore` test on the remote host**

The test creates one memory and one todo with the same user turn, calls `conversation_messages()`, and asserts:

```rust
assert_eq!(messages[0].actions.len(), 2);
assert_eq!(messages[0].actions[0].kind, "memory");
assert_eq!(messages[0].actions[1].kind, "todo");
assert_eq!(messages[0].actions[1].label, "周一带充电器");
```

- [ ] **Step 3: Run the focused server test to verify failure**

Run remotely: `cargo test --manifest-path services/agent-gateway/Cargo.toml conversation_messages_include_source_actions`
Expected: compile failure because `ConversationMessage.actions` is missing.

- [ ] **Step 4: Add the serialized action type and queries**

```rust
#[derive(Clone, Debug, Serialize)]
pub struct ConversationAction {
    pub kind: String,
    pub target_id: String,
    pub label: String,
    pub due_at: Option<f64>,
}
```

Add `actions: Vec<ConversationAction>` to `ConversationMessage`. For each user turn, query `memory_items` and `todos` by `user_id` and `source_turn_id`, order memories before todos and each group by `created_at`, and cap the combined result at ten. Assistant turns return an empty vector.

- [ ] **Step 5: Extend the authenticated route test**

Assert `/v1/conversations/{id}/messages` returns `actions`, does not leak another user's targets, and still returns image attachments unchanged.

- [ ] **Step 6: Run server verification and commit remotely**

```bash
cargo fmt --check --manifest-path services/agent-gateway/Cargo.toml
cargo test --manifest-path services/agent-gateway/Cargo.toml
cargo build --release --manifest-path services/agent-gateway/Cargo.toml
python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v
git add services/agent-gateway/src/context.rs services/agent-gateway/src/main.rs
git commit -m "feat(server): expose conversation action references"
git push origin HEAD
```

Expected: all verification passes and the remote commit is pushed.

---

### Task 6: Render persisted actions in Android history

**Files:**
- Modify: `apps/mobile/src/api.ts`
- Create: `apps/mobile/src/components/ConversationActions.tsx`
- Modify: `apps/mobile/src/App.tsx`
- Modify: `apps/mobile/src/App.css`
- Modify: `apps/mobile/tests/mobile-package.test.mjs`

**Interfaces:**
- Consumes: `ConversationMessage.actions` from Task 5.
- Produces: memory/todo chips that navigate to the corresponding library entry without inventing success state.

- [ ] **Step 1: Sync the remote server commit into the local implementation branch**

Run: `git fetch origin && git merge --ff-only origin/codex/gpt-live-alignment`
Expected: local branch contains the server action response.

- [ ] **Step 2: Add failing type and component assertions**

```js
const apiSource = readFileSync(path.join(appRoot, 'src/api.ts'), 'utf8')
const actionSource = readFileSync(path.join(appRoot, 'src/components/ConversationActions.tsx'), 'utf8')
assert.match(apiSource, /actions: ConversationAction\[\]/)
assert.match(appSource, /<ConversationActions/)
assert.doesNotMatch(actionSource, /dangerouslySetInnerHTML/)
```

- [ ] **Step 3: Define the API type**

```ts
export type ConversationAction = {
  kind: 'memory' | 'todo' | string
  target_id: string
  label: string
  due_at: number | null
}
```

- [ ] **Step 4: Implement action chips**

`ConversationActions` renders recognized memory/todo kinds only. Memory selects the matching memory after loading the memory tab; todo selects the todo tab and scrolls to the target. Unknown action kinds are ignored rather than guessed.

- [ ] **Step 5: Verify full stack and build APK**

Run locally:

```bash
cd apps/mobile
npm run test:tool-results
npm run test:realtime
npm run test:mobile
npm run lint
npm run build
npm run android:build
```

Expected: all commands PASS and an APK is produced.

- [ ] **Step 6: Commit**

```bash
git add apps/mobile/src/api.ts apps/mobile/src/components/ConversationActions.tsx apps/mobile/src/App.tsx apps/mobile/src/App.css apps/mobile/tests/mobile-package.test.mjs
git commit -m "feat(mobile): show conversation action history"
```

---

### Task 7: Deploy backward-compatible server response and run end-to-end acceptance

**Files:**
- Remote deployment only; no source edits expected.

**Interfaces:**
- Consumes: verified server binary and Android APK.
- Produces: production conversation actions plus verified live receipts and resume behavior.

- [ ] **Step 1: Deploy on the required remote host**

Run remotely from the pushed implementation commit:

```bash
cd ~/workspace/ripple-live
git pull --ff-only
cargo build --release --manifest-path services/agent-gateway/Cargo.toml
./deploy/agent-stack/stop.sh
./deploy/agent-stack/start.sh
./deploy/agent-stack/status.sh
```

Expected: `ripple-agent-gateway` is active and health/readiness checks are healthy.

- [ ] **Step 2: Run production smoke verification**

Run remotely: `uv run --with httpx deploy/agent-stack/smoke-test.py`
Expected: Responses API, auth, realtime, memory, todo, and conversation smoke checks PASS.

- [ ] **Step 3: Perform Android end-to-end checks**

Verify: a new call stores its server conversation ID; ending returns to the same timeline; “继续语音” reuses the ID; successful memory/todo operations show one receipt each; malformed/failed results never show success; tool cards do not stop audio; history action chips reopen the correct memory/todo; no camera opens automatically.
