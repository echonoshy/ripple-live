# Responses API、模型回复判断与 Android 单链路 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将远程 Agent 统一迁移到 Responses API，使用独立模型 Gate 判断每段语音是否回复，删除旧协议与唤醒词，并交付可在 Android 真机运行的 arm64 Debug 包。

**Architecture:** 服务端代码只在 `140.143.229.103:/home/lake/workspace/ripple-live` 修改和部署，Android 代码只在本机 `/Users/lake/workspace/ripple-live` 修改和构建。模型服务先改用与 Qwen3-VL 实际 JSON `<tool_call>` 输出匹配的 vLLM `hermes` parser，使 `/v1/responses` 产出结构化 `function_call`；Gateway 只消费 Responses 事件和 `function_call_output`，Gate 复用同一模型、只暴露一个判断函数并要求模型调用它。

**Tech Stack:** Rust 2024、Axum、Tokio、Reqwest、Serde JSON、SQLite、vLLM 0.14、Qwen3-VL-8B-Instruct、React 19、TypeScript 6、Tauri 2、Android/Gradle/JDK 17。

## Global Constraints

- 服务端只能在 `140.143.229.103:/home/lake/workspace/ripple-live` 修改、测试和部署。
- 移动端只能在本机 `/Users/lake/workspace/ripple-live` 修改、测试和构建。
- 直接在当前 `master` 分支工作，不创建 worktree 或功能分支。
- 模型上游只使用 `POST /v1/responses`，不得保留 `/v1/chat/completions` 工具调用路径。
- vLLM 0.14 对 named/required `tool_choice` 会生成畸形 tagged arguments；所有请求使用 `tool_choice: "auto"`，需要指定工具时只暴露该工具并在 instructions 中明确要求调用。
- 字面量 `<tool_call>` 不得在 Gateway 内被解析或执行。
- 实时协议只接受 `protocol_version = 3`，不保留旧客户端兼容。
- Gate 输入为当前 ASR、最近 2 至 4 轮对话、助手是否刚回复；Gate 阶段不得请求或上传画面。
- Gate `ignore` 不写用户对话、不触发画面、工具或 TTS；开发日志保留 transcript、decision、reason、latency 和 fallback。
- Gate 超时或非法输出采用 `fail-open`。
- 本轮不修改、构建或回归 iOS。
- 最终 Android APK 必须只包含 arm64 ABI，并在 Android 真机完成安装、启动和端到端验证。

---

## File Map

### Remote server and deployment

- Create: `/home/lake/workspace/ripple-live/services/agent-gateway/src/response_gate.rs` — Gate 类型、提示词、严格结果解析和 fail-open 包装。
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/adapters.rs` — Responses 请求、SSE 事件、输出项、工具结果和多模态输入适配。
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/config.rs` — `/v1/responses` 默认地址和 Gate 超时配置。
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/lib.rs` — 导出 `response_gate`，移除 `activation`。
- Delete: `/home/lake/workspace/ripple-live/services/agent-gateway/src/activation.rs` — 删除规则唤醒模块。
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/tools.rs` — 将工具 schema 转换为 Responses 扁平函数格式。
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/orchestrator.rs` — Responses 工具循环、Gate 调用、结构化事件解析，删除 Chat Completions 和标签回退。
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/main.rs` — 严格协议 v3、统一 ASR→Gate→Agent 流程、按需画面请求，删除唤醒状态。
- Modify: `/home/lake/workspace/ripple-live/deploy/agent-stack/start.sh` — 默认 `hermes` tool parser 和 Responses URL。
- Modify: `/home/lake/workspace/ripple-live/deploy/agent-stack/.env.example` — Responses 与 Gate 配置示例。
- Modify: `/home/lake/workspace/ripple-live/deploy/agent-stack/smoke-test.py` — Responses 工具闭环、Gate 和协议 v3 冒烟。
- Modify: `/home/lake/workspace/ripple-live/deploy/agent-stack/test-smoke-contract.py` — 固化新冒烟契约。

### Local Android client

- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/realtime/protocol.ts` — 协议 v3 和无 activation 的 session.start。
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/realtime/RealtimeSession.ts` — 删除唤醒状态、事件和 API，保持统一 listening 生命周期。
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/App.tsx` — 删除唤醒设置、手动唤醒和旧文案；视频帧只响应服务端请求。
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/App.css` — 删除仅供唤醒设置使用的样式。
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/media/LiveMedia.ts` — 删除 `lowPower` 激活耦合，使用固定 Android 摄像头参数。
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/tests/realtime-session.test.ts` — 协议 v3、统一失败恢复和按需画面测试。
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/tests/mobile-package.test.mjs` — 删除唤醒断言，增加 Android-only 和无旧字符串断言。

---

### Task 1: Correct vLLM Responses Tool Parsing

**Files:**
- Modify: `/home/lake/workspace/ripple-live/deploy/agent-stack/start.sh:23-26,113-125,148`
- Modify: `/home/lake/workspace/ripple-live/deploy/agent-stack/.env.example:15-16,41`
- Modify: `/home/lake/workspace/ripple-live/deploy/agent-stack/smoke-test.py`
- Test: `/home/lake/workspace/ripple-live/deploy/agent-stack/test-smoke-contract.py`

**Interfaces:**
- Consumes: Qwen3-VL output shaped as `<tool_call>{"name":"calculate","arguments":{"expression":"7 * 8"}}</tool_call>`.
- Produces: `/v1/responses` output item `{type:"function_call", name:"calculate", call_id:"call_1", arguments:"{\"expression\":\"7 * 8\"}"}` where `arguments` is directly parseable JSON and contains no XML tags.

- [ ] **Step 1: Add a failing smoke-contract test for structured Responses calls**

```python
def test_responses_tool_call_rejects_tagged_arguments(self) -> None:
    item = SMOKE.require_function_call(
        {"output": [{
            "type": "function_call",
            "name": "calculate",
            "call_id": "call_1",
            "arguments": "{\"expression\":\"7 * 8\"}",
        }]}
    )
    self.assertEqual(item["name"], "calculate")
    self.assertEqual(json.loads(item["arguments"]), {"expression": "7 * 8"})
    self.assertNotIn("<tool_call>", item["arguments"])
```

Add `import json` to the contract test. `SMOKE.require_function_call(payload)` must select exactly one `output` item with `type == "function_call"`, require non-empty `call_id`, and run `json.loads(arguments)` before returning it.

- [ ] **Step 2: Run the contract test and verify the current fixture/path fails**

Run:

```bash
cd /home/lake/workspace/ripple-live
python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v
```

Expected: FAIL because the current smoke path accepts Chat Completions or tagged arguments.

- [ ] **Step 3: Switch the runtime and environment defaults**

Change the exact defaults to:

```bash
AGENT_TOOL_CALL_PARSER="${AGENT_TOOL_CALL_PARSER:-hermes}"
```

Set `.env.example`:

```dotenv
RIPPLE_AGENT_URL=http://127.0.0.1:8712/v1/responses
RIPPLE_GATE_TIMEOUT_MS=3000
AGENT_TOOL_CALL_PARSER=hermes
```

- [ ] **Step 4: Make the live smoke execute a full Responses tool loop**

The smoke must:

1. POST 只包含 `calculate` 的工具列表，以 instructions 要求调用并使用 `tool_choice: "auto"`。
2. Assert one structured `function_call` and JSON-decodable `arguments`.
3. Execute the test calculation locally.
4. POST a second Responses request whose `input` is the first response `output` plus:

```python
{
    "type": "function_call_output",
    "call_id": call["call_id"],
    "output": json.dumps({"ok": True, "result": 56}),
}
```

5. Assert the final response contains an `output_text` item and no executable tag fallback.

- [ ] **Step 5: Restart the development stack and verify the parser live**

Run:

```bash
cd /home/lake/workspace/ripple-live
./deploy/agent-stack/stop.sh
./deploy/agent-stack/start.sh
python3 deploy/agent-stack/smoke-test.py --responses-only
```

Expected: auto tool output is a structured `function_call`; `json.loads(arguments)` succeeds; the second request returns final text.

- [ ] **Step 6: Commit the runtime contract**

```bash
git add deploy/agent-stack/start.sh deploy/agent-stack/.env.example deploy/agent-stack/smoke-test.py deploy/agent-stack/test-smoke-contract.py
git commit -m "fix(runtime): normalize responses tool calls"
```

### Task 2: Implement the Responses Adapter

**Files:**
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/adapters.rs:1-180,284-315,330-end`
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/tools.rs:14-112,237-240`
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/config.rs:3-42,75-128`
- Modify: `/home/lake/workspace/ripple-live/deploy/agent-stack/start.sh:145-175`

**Interfaces:**
- Consumes: internal history messages, `VideoFrame`, current tool schemas, Responses tool choice.
- Produces: `ResponsesOutput { text: String, function_calls: Vec<FunctionCall>, output_items: Vec<Value> }`, `function_call_output(call_id, result)`, and Responses-format input/tool JSON.

- [ ] **Step 1: Write adapter unit tests for exact Responses shapes**

```rust
#[test]
fn parses_completed_responses_output() {
    let output = parse_responses_output(&json!({"output": [
        {"type":"message","content":[{"type":"output_text","text":"结果"}]},
        {"type":"function_call","call_id":"call_7","name":"calculate","arguments":"{\"expression\":\"7*8\"}"}
    ]})).unwrap();
    assert_eq!(output.text, "结果");
    assert_eq!(output.function_calls[0].call_id, "call_7");
    assert_eq!(output.function_calls[0].name, "calculate");
}

#[test]
fn literal_tool_call_tag_is_only_text() {
    let output = parse_responses_output(&json!({"output": [{
        "type":"message","content":[{"type":"output_text","text":"<tool_call>{}</tool_call>"}]
    }]})).unwrap();
    assert!(output.function_calls.is_empty());
}

#[test]
fn builds_function_call_output_item() {
    assert_eq!(
        function_call_output("call_7", &json!({"ok":true,"result":56})),
        json!({"type":"function_call_output","call_id":"call_7","output":"{\"ok\":true,\"result\":56}"})
    );
}
```

- [ ] **Step 2: Run the focused adapter tests and verify they fail**

Run:

```bash
cd /home/lake/workspace/ripple-live
cargo test --manifest-path services/agent-gateway/Cargo.toml adapters::tests -- --nocapture
```

Expected: FAIL because `parse_responses_output` and `function_call_output` do not exist.

- [ ] **Step 3: Replace Chat Completions types with Responses types**

Define:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug)]
pub struct ResponsesOutput {
    pub text: String,
    pub function_calls: Vec<FunctionCall>,
    pub output_items: Vec<Value>,
}
```

Remove `ToolCall`, `AgentReply`, Chat `messages`, `max_tokens`, `/choices/0/message`, and `role:"tool"` builders.

- [ ] **Step 4: Add Responses request methods**

Implement exact public methods:

```rust
pub async fn respond(
    &self,
    input: &[Value],
    tools: &[Value],
    tool_choice: Value,
    instructions: &str,
) -> anyhow::Result<ResponsesOutput>;

pub async fn respond_stream(
    &self,
    input: &[Value],
    tools: &[Value],
    tool_choice: Value,
    instructions: &str,
) -> anyhow::Result<AgentStream>;
```

Request JSON uses `model`, `instructions`, `input`, `tools`, `tool_choice`, `temperature`, `max_output_tokens`, and `stream`. SSE parsing must yield the decoded `data:` payload unchanged; it must not interpret Chat `choices`.

- [ ] **Step 5: Convert tools and multimodal content to Responses format**

Each Chat-style schema:

```json
{"type":"function","function":{"name":"calculate","description":"计算表达式","parameters":{"type":"object"}}}
```

must become:

```json
{"type":"function","name":"calculate","description":"计算表达式","parameters":{"type":"object"}}
```

The user item must be:

```rust
json!({"role":"user","content":[
    {"type":"input_text","text": text},
    {"type":"input_image","image_url": data_url}
]})
```

- [ ] **Step 6: Change settings defaults and add Gate timeout**

Add `pub gate_timeout: Duration` and initialize it with:

```rust
gate_timeout: Duration::from_millis(parsed("GATE_TIMEOUT_MS", 3_000)),
```

Change the agent default URL to `http://127.0.0.1:8712/v1/responses`.
Pass the same value into the Gateway process in `start.sh`:

```bash
RIPPLE_AGENT_URL="${RIPPLE_AGENT_URL:-http://127.0.0.1:8712/v1/responses}" \
```

- [ ] **Step 7: Run adapter tests and the full library suite**

Run:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml adapters::tests -- --nocapture
cargo test --manifest-path services/agent-gateway/Cargo.toml --lib
```

Expected: all adapter tests and library tests PASS.

- [ ] **Step 8: Commit the Responses adapter**

```bash
git add services/agent-gateway/src/adapters.rs services/agent-gateway/src/tools.rs services/agent-gateway/src/config.rs deploy/agent-stack/start.sh
git commit -m "refactor(agent): adopt responses protocol"
```

### Task 3: Migrate Both Agent Tool Loops

**Files:**
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/orchestrator.rs:1-616,717-814,919-end`

**Interfaces:**
- Consumes: `ModelAdapters::respond`, `respond_stream`, `FunctionCall`, Responses-format tool schemas and `function_call_output`.
- Produces: text and realtime turns whose continuation input contains model `output_items` followed by matching `function_call_output` entries.

- [ ] **Step 1: Replace Chat delta tests with Responses event tests**

```rust
#[test]
fn reads_responses_text_and_function_arguments() {
    assert_eq!(responses_text_delta(&json!({
        "type":"response.output_text.delta","delta":"你好"
    })), Some("你好"));

    let mut calls = BTreeMap::new();
    merge_responses_function_event(&mut calls, &json!({
        "type":"response.output_item.added","output_index":0,
        "item":{"type":"function_call","call_id":"call_1","name":"calculate","arguments":""}
    })).unwrap();
    merge_responses_function_event(&mut calls, &json!({
        "type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"expression\":\"7*8\"}"
    })).unwrap();
    assert_eq!(calls[&0].arguments, "{\"expression\":\"7*8\"}");
}
```

Add a negative assertion that a `response.output_text.delta` containing `<tool_call>` never inserts a function call.

- [ ] **Step 2: Run the focused orchestrator tests and verify failure**

Run:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml orchestrator::tests -- --nocapture
```

Expected: FAIL because the current helpers only parse `/choices/0/delta` and embedded tags.

- [ ] **Step 3: Migrate `run_text_response`**

Use `input: Vec<Value>` initialized from recent history plus the current user item. For every round:

```rust
let reply = self.adapters.respond(&input, &tools, tool_choice, &system_prompt()).await?;
input.extend(reply.output_items.clone());
if reply.function_calls.is_empty() { /* persist and return reply.text */ }
for call in reply.function_calls {
    let outcome = self.tools.execute(&execution, &call.name, &call.arguments).await;
    input.push(function_call_output(&call.call_id, &outcome.value));
}
```

当 `forced_route` 存在时，只把匹配 `name` 的一个 schema 传给模型，并在 instructions 末尾追加 `本轮必须调用工具 {name}，不得直接作答。`；`tool_choice` 仍为 `"auto"`。后续轮次恢复完整工具列表和普通 instructions。

- [ ] **Step 4: Migrate realtime streaming**

Handle only these upstream event types:

- `response.output_text.delta` → append text, emit Ripple text delta, feed speech segmenter;
- `response.output_item.added` with `item.type == "function_call"` → initialize call by `output_index`;
- `response.function_call_arguments.delta` → append arguments by `output_index`;
- `response.output_item.done` → retain the exact completed output item;
- `response.failed` or `response.incomplete` → return `AGENT_FAILED`;
- `response.completed` → end the round.

After the round, append completed output items to `input`, execute only structured calls, then append one `function_call_output` per `call_id`.

- [ ] **Step 5: Delete all text-tag and Chat helpers**

Delete `merge_tool_call_deltas`, `embedded_tool_calls`, `streamed_raw_message`, `tool_result_message` imports, and their tests. Keep `tool_call_id` database fields because they are idempotency keys, but populate them from Responses `call_id`.

- [ ] **Step 6: Run focused and full tests**

Run:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml orchestrator::tests -- --nocapture
cargo test --manifest-path services/agent-gateway/Cargo.toml
```

Expected: all tests PASS and `grep -R "embedded_tool_calls\|/choices/0/delta/tool_calls\|role.*tool" services/agent-gateway/src` returns no executable compatibility path.

- [ ] **Step 7: Commit both migrated loops**

```bash
git add services/agent-gateway/src/orchestrator.rs
git commit -m "refactor(agent): stream responses tool loops"
```

### Task 4: Add the Independent Model Gate

**Files:**
- Create: `/home/lake/workspace/ripple-live/services/agent-gateway/src/response_gate.rs`
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/lib.rs:1-14`
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/orchestrator.rs:44-97,212-340`

**Interfaces:**
- Consumes: transcript, `ContextStore::recent_messages(session_id, 2)`, same Responses model and a 3000 ms timeout.
- Produces: `GateOutcome { decision: GateDecision, reason: String, latency_ms: u128, fallback: bool }`.

- [ ] **Step 1: Write Gate parser and failure-policy tests**

```rust
#[test]
fn parses_gate_function_arguments() {
    let outcome = parse_gate_arguments("{\"decision\":\"ignore\",\"reason\":\"background_conversation\"}").unwrap();
    assert_eq!(outcome.decision, GateDecision::Ignore);
}

#[test]
fn unknown_decision_is_rejected() {
    assert!(parse_gate_arguments("{\"decision\":\"maybe\",\"reason\":\"unclear\"}").is_err());
}

#[test]
fn gate_error_fails_open() {
    let outcome = GateOutcome::fallback("timeout", 3000);
    assert_eq!(outcome.decision, GateDecision::Respond);
    assert!(outcome.fallback);
}
```

- [ ] **Step 2: Run the Gate tests and verify failure**

Run:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml response_gate::tests -- --nocapture
```

Expected: FAIL because the module does not exist.

- [ ] **Step 3: Implement Gate types and single-function schema**

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision { Respond, Ignore }

#[derive(Clone, Debug)]
pub struct GateOutcome {
    pub decision: GateDecision,
    pub reason: String,
    pub latency_ms: u128,
    pub fallback: bool,
}
```

The only function exposed to Gate is:

```json
{
  "type":"function",
  "name":"decide_response",
  "description":"判断当前语音是否需要助手回复",
  "parameters":{
    "type":"object",
    "properties":{
      "decision":{"type":"string","enum":["respond","ignore"]},
      "reason":{"type":"string"}
    },
    "required":["decision","reason"],
    "additionalProperties":false
  }
}
```

- [ ] **Step 4: Implement `AgentOrchestrator::gate_transcript`**

Exact signature:

```rust
pub async fn gate_transcript(
    &self,
    session_id: &str,
    transcript: &str,
) -> GateOutcome;
```

Load `recent_messages(session_id, 2)` before adding the current user turn. Compute:

```rust
let assistant_just_replied = history.last().and_then(|item| item.get("role"))
    .and_then(Value::as_str) == Some("assistant");
```

Call `respond` under `tokio::time::timeout(self.settings.gate_timeout, request)` with only `decide_response`, instructions requiring exactly one call, and `tool_choice: "auto"`; require exactly one function call with that name. Define `GateOutcome::fallback(reason: impl Into<String>, latency_ms: u128)` and convert every error into that fail-open result.

- [ ] **Step 5: Add Gate flow logging without persisting ignored turns**

Record `server.gate.completed` with:

```rust
json!({
    "response_id": response_id,
    "transcript": transcript,
    "gate_decision": outcome.decision.as_str(),
    "gate_reason": outcome.reason,
    "gate_latency_ms": outcome.latency_ms,
    "gate_fallback": outcome.fallback,
})
```

Use this single event family as the source for the required development metrics: count by `gate_decision` for `gate_respond_total` and `gate_ignore_total`, count `gate_fallback=true` for `gate_fallback_total`, and aggregate `gate_latency_ms` for latency. Existing `server.agent.first_delta` and `server.tts.first_audio` events remain the post-Gate first-text and first-audio measurements.

Do not call `add_turn`, emit `input.transcript.final`, request a frame, create a response, call tools, or start TTS when decision is `Ignore`.

- [ ] **Step 6: Run Gate and full service tests**

Run:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml response_gate::tests -- --nocapture
cargo test --manifest-path services/agent-gateway/Cargo.toml
```

Expected: PASS; invalid/timeout paths all return Respond with `fallback=true`.

- [ ] **Step 7: Commit the model Gate**

```bash
git add services/agent-gateway/src/response_gate.rs services/agent-gateway/src/lib.rs services/agent-gateway/src/orchestrator.rs
git commit -m "feat(gate): let model choose replies"
```

### Task 5: Enforce Realtime Protocol v3 and Remove Wake Activation

**Files:**
- Delete: `/home/lake/workspace/ripple-live/services/agent-gateway/src/activation.rs`
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/lib.rs:1-14`
- Modify: `/home/lake/workspace/ripple-live/services/agent-gateway/src/main.rs:20,49-81,982-1609,1633-end`

**Interfaces:**
- Consumes: `session.start` with exact `protocol_version: 3`, audio commit, optional correlated video frames.
- Produces: ASR→Gate; `ignore` ends silently; `respond` requests a correlated frame only in video mode, then starts Agent.

- [ ] **Step 1: Replace legacy protocol tests with strict v3 tests**

```rust
#[test]
fn protocol_three_requires_exact_version() {
    assert!(validate_protocol_version(Some(3)).is_ok());
    assert_eq!(validate_protocol_version(None).unwrap_err().code, "unsupported_protocol");
    assert_eq!(validate_protocol_version(Some(2)).unwrap_err().code, "unsupported_protocol");
    assert_eq!(validate_protocol_version(Some(4)).unwrap_err().code, "unsupported_protocol");
}

#[test]
fn pending_frame_requires_exact_response_id() {
    assert!(matches!(correlate_pending_frame("r1", Some("r1")), FrameCorrelation::Matched));
    assert!(matches!(correlate_pending_frame("r1", None), FrameCorrelation::Stale));
}
```

Define the validator contract used by the test:

```rust
#[derive(Debug)]
struct ProtocolError {
    code: &'static str,
    message: &'static str,
}

fn validate_protocol_version(version: Option<u32>) -> Result<(), ProtocolError>;
```

- [ ] **Step 2: Run the service tests and verify failure**

Run:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml --bin ripple-agent-gateway protocol -- --nocapture
```

Expected: FAIL because version 2 and legacy correlation are still accepted.

- [ ] **Step 3: Make `session.start` mandatory and exact**

Set `REALTIME_PROTOCOL_VERSION` to 3. Before accepting audio or video events, require a successful `session.start`. On missing or mismatched version emit:

```json
{"type":"error","code":"unsupported_protocol","message":"客户端与服务端协议版本不一致，需要使用协议 v3"}
```

Then break the socket loop. `session.ready` contains only `session_id`, `protocol_version`, sample rates, and mode—no legacy or activation fields.

- [ ] **Step 4: Replace `input.commit` with the single Gate path**

On commit:

1. Transcribe once with `transcribe_candidate`.
2. Call `gate_transcript`.
3. If Ignore: clear frames and continue without client-visible transcript.
4. If Respond and audio mode: call `spawn_turn` with `transcript_override`.
5. If Respond and video mode: store `PendingTurn` and emit `input.frame.requested` with the response ID.
6. On correlated `input.video.commit`, call `spawn_turn` with the stored transcript and frame.

Move `response.created` and `input.transcript.final` after Gate acceptance so ignored speech never appears as a user turn.

- [ ] **Step 5: Delete all activation and legacy state**

Delete `ActivationMode`, `evaluate_activation`, `activation_mode`, `awake_until`, `session.wake`, `session.awake`, `input.activation.*`, `server.activation.*`, `FrameCorrelation::Legacy`, `legacy_frame_correlation`, and their tests. Delete `activation.rs` and remove its `lib.rs` export.

- [ ] **Step 6: Run full tests and stale-symbol scans**

Run:

```bash
cargo test --manifest-path services/agent-gateway/Cargo.toml
grep -RInE 'ActivationMode|awake_until|session\.wake|input\.activation|server\.activation|Legacy\(' services/agent-gateway/src && exit 1 || true
```

Expected: all tests PASS; stale-symbol scan prints nothing.

- [ ] **Step 7: Commit protocol v3**

```bash
git add services/agent-gateway/src/main.rs services/agent-gateway/src/lib.rs services/agent-gateway/src/activation.rs
git commit -m "refactor(realtime): require protocol version three"
```

### Task 6: Simplify the Android Client to Protocol v3

**Files:**
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/realtime/protocol.ts:1-23`
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/realtime/RealtimeSession.ts:1-80,220-370,437-548`
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/App.tsx:72,130-190,250-265,468-551,1683-1724,1739-1745,1780-1800,1850-1866`
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/App.css:1881-1934`
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/src/media/LiveMedia.ts:3-10,333-343`
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/tests/realtime-session.test.ts`
- Modify: `/Users/lake/workspace/ripple-live/apps/mobile/tests/mobile-package.test.mjs:92-110`

**Interfaces:**
- Consumes: strict server protocol v3 and `input.frame.requested` only after Gate Respond.
- Produces: `session.start` without activation mode, continuous VAD audio commits, on-demand correlated video frame, unified listening state.

- [ ] **Step 1: Write failing v3 and no-wake client tests**

```ts
test('session start declares only protocol v3 fields', () => {
  const event = createSessionStart('video')
  assert.equal(event.type, 'session.start')
  assert.equal(event.protocol_version, 3)
  assert.equal(event.mode, 'video')
  assert.equal(event.client_build.length > 0, true)
  assert.equal('activation_mode' in event, false)
})

test('failed response always returns to listening', () => {
  const harness = failureHarness()
  harness.receive({ type: 'response.created', response_id: 'failed' })
  harness.receive({ type: 'response.failed', response_id: 'failed', message: '失败' })
  assert.equal(harness.states.at(-1), 'listening')
})
```

In the package test, assert source does not contain `ActivationMode`, `ripple-activation-mode`, `静默唤醒`, `session.wake`, or `input.activation`.

- [ ] **Step 2: Run the focused tests and verify failure**

Run:

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
npm run test:mobile
```

Expected: FAIL because protocol v2 and activation paths still exist.

- [ ] **Step 3: Make protocol.ts strict v3**

```ts
export type RealtimeMode = 'audio' | 'video'
export const REALTIME_PROTOCOL_VERSION = 3

export function createSessionStart(mode: RealtimeMode) {
  return {
    type: 'session.start',
    protocol_version: REALTIME_PROTOCOL_VERSION,
    client_build: clientBuild,
    mode,
  }
}
```

- [ ] **Step 4: Remove activation from `RealtimeSession`**

Delete `ActivationMode`, the `activationMode` option, `silent`/`verifying` states, `onActivated`, activation event cases, and `forceWake()`. `connect()` and `finishResponse()` always transition to `listening`. Keep `forceListen()` as the user-visible interruption action.

`commitInput()` must never upload a frame before Gate acceptance:

```ts
async commitInput() {
  if (!this.transport || !this.ready || this.closed) return
  await this.sendEvent({ type: 'input.commit' })
}
```

Only `input.frame.requested` may call `createRequestedFrameEvents(responseId, frame, Date.now())`.

- [ ] **Step 5: Remove wake configuration and UI**

Delete the activation state/effect, interaction settings panel, call header wake label, manual wake branch, and wake-specific status copy. The third call control always invokes `forceListen()` and is labeled `打断回答` / `打断`.

Instantiate media/session as:

```ts
const media = new LiveMedia({
  video: videoRef.current,
  canvas: canvasRef.current,
  withVideo: nextMode === 'video',
  facingMode: cameraFacing,
  onPlaybackStarted: (bufferedMs) => session.outputPlaybackStarted(bufferedMs),
})

session = new RealtimeSession({
  server,
  accessToken,
  mode: nextMode,
  onState: setSessionState,
  onError: (message) => {
    setErrorMessage(message)
    setSessionState('error')
  },
  onAssistantText: setAssistantText,
  onUserText: (text) => {
    setUserText(text)
    setLiveArtifacts([])
  },
  onTool: setToolStatus,
  onAudio: (audio) => media.enqueueOutput(audio),
  onAudioDone: () => media.finishOutput(),
  onInterrupted: () => media.clearOutput(),
  onFrameRequested: () => media.captureFrame(),
  onArtifact: (artifact) => {
    setLiveArtifacts((items) =>
      items.some((item) => item.id === artifact.id)
        ? items
        : [...items, artifact],
    )
  },
  onReady: async () => {
    await media.start(
      (audio) => void session.sendInput(audio),
      () => void session.speechStarted(),
      () => void session.commitInput(),
      (level) => {
        visualizerRef.current?.style.setProperty('--audio-level', String(level))
      },
    )
  },
  onConversation: () => {},
})
```

- [ ] **Step 6: Remove activation-only CSS and low-power coupling**

Delete `.interaction-panel` and `.settings-choice` rules. Remove `lowPower` from `LiveMediaOptions`; use Android video constraints `1280x720` with no activation-dependent frame-rate branch.

- [ ] **Step 7: Run all Android-facing source tests**

Run:

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
npm run test:mobile
npm run lint
npm run build
```

Expected: all commands PASS and stale-symbol scan is empty:

```bash
rg -n 'ActivationMode|ripple-activation-mode|静默唤醒|session\.wake|input\.activation|forceWake' src tests
```

- [ ] **Step 8: Commit the Android v3 client**

```bash
git add apps/mobile/src apps/mobile/tests
git commit -m "feat(android): use model gated protocol"
```

### Task 7: Deploy and Perform End-to-End Android Verification

**Files:**
- No planned source edits; any discovered defect returns execution to the owning Task 1-6 before this verification task is rerun.
- Verify: `/home/lake/workspace/ripple-live/deploy/agent-stack/status.sh`
- Verify: `/home/lake/workspace/ripple-live/deploy/agent-stack/smoke-test.py`
- Build artifact: `/Users/lake/workspace/ripple-live/apps/mobile/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`

**Interfaces:**
- Consumes: remote Gateway protocol v3 and local Android v3 client.
- Produces: live service evidence, structured tool-loop evidence, arm64-only APK evidence, and installed/launched Android evidence.

- [ ] **Step 1: Run final remote tests before deployment**

Run:

```bash
ssh 140.143.229.103 'cd /home/lake/workspace/ripple-live && cargo test --manifest-path services/agent-gateway/Cargo.toml && python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v'
```

Expected: all Rust and Python tests PASS.

- [ ] **Step 2: Build release Gateway, restart the development stack, and verify readiness**

Run:

```bash
ssh 140.143.229.103 'cd /home/lake/workspace/ripple-live && cargo build --release --manifest-path services/agent-gateway/Cargo.toml && ./deploy/agent-stack/stop.sh && ./deploy/agent-stack/start.sh && ./deploy/agent-stack/status.sh'
```

Expected: ASR, agent, TTS, and Gateway listeners are healthy; Gateway `/ready` reports all required dependencies ready.

- [ ] **Step 3: Run authenticated Responses/Gate/realtime smoke**

Run:

```bash
ssh 140.143.229.103 'cd /home/lake/workspace/ripple-live && python3 deploy/agent-stack/smoke-test.py --authenticated --responses --gate --protocol 3'
```

Expected:

- single-tool auto calculate produces structured `function_call` and `function_call_output` continuation;
- literal `<tool_call>` text does not execute;
- a direct request Gate result is Respond;
- a background-conversation fixture Gate result is Ignore;
- invalid protocol 2 is rejected;
- protocol 3 completes ASR, Agent, TTS and interruption flows.

- [ ] **Step 4: Run final local Android tests and build arm64 Debug APK**

Run with JDK 17:

```bash
cd /Users/lake/workspace/ripple-live/apps/mobile
npm run test:realtime
npm run test:mobile
npm run lint
npm run build
JAVA_HOME=$(/usr/libexec/java_home -v 17) npm run android:build -- --target aarch64
```

Expected: all tests/builds PASS and the APK exists at the path above.

- [ ] **Step 5: Verify APK ABI contents**

Run:

```bash
unzip -l /Users/lake/workspace/ripple-live/apps/mobile/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk | rg 'lib/[^/]+/.*\.so$'
```

Expected: native libraries exist only under `lib/arm64-v8a/`; no `armeabi-v7a`, `x86`, or `x86_64` entries.

- [ ] **Step 6: Install, launch, and verify on the observed Android device**

Run:

```bash
adb devices
adb install -r /Users/lake/workspace/ripple-live/apps/mobile/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb shell monkey -p cn.minicpm.live -c android.intent.category.LAUNCHER 1
adb shell pidof cn.minicpm.live
```

Expected: one authorized device, install success, launch succeeds, and `pidof` returns a process ID.

Then perform these device checks and correlate them with Gateway logs:

1. Ask a direct question and hear a reply.
2. Say a short contextual follow-up and hear a reply.
3. Play unrelated background speech and confirm no user message, frame request, tool, or TTS occurs.
4. Trigger a calculation/tool request and confirm structured tool execution.
5. Interrupt playback and start a new turn.

- [ ] **Step 7: Record final evidence**

Do not create a verification-only commit. If verification finds a defect, return to the task that owns the affected file, add the failing regression test there, implement the minimal correction, rerun that task, commit it with that task's commit format, and then restart Task 7.

Final handoff must list exact passing commands, remote readiness result, tool `call_id` evidence, APK path and ABI list, device serial, install result, package PID, and any remaining limitation. Do not claim iOS validation.
