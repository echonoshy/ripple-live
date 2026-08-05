# Disable Voice Reasoning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Disable reasoning on the first normal Agent and Gate requests without changing TTS or client behavior.

**Architecture:** Keep reasoning serialization in the shared Responses request builder. Pass an explicit `none` effort from the Gate and every Agent generation round, while retaining the existing recovery path and observability.

**Tech Stack:** Rust, Tokio, reqwest, Axum test servers, SQLite event assertions, Qwen Responses API.

## Global Constraints

- Responses API remains the only model protocol.
- Backend changes are deployed and verified on `lake@140.143.229.103`.
- Android remains the mobile delivery target; iOS is untouched.
- TTS segmentation and client playback buffering are out of scope.

---

### Task 1: Request contracts

**Files:**
- Modify: `services/agent-gateway/src/adapters.rs`
- Test: `services/agent-gateway/src/adapters.rs`

**Interfaces:**
- Consumes: `responses_request_body(..., reasoning_effort: Option<&str>)`.
- Produces: `ModelAdapters::respond(..., reasoning_effort: Option<&str>)` and normal request bodies containing `{"reasoning":{"effort":"none"}}`.

- [ ] Add a failing request-body test asserting the normal Agent request uses `none`.
- [ ] Run the targeted test and confirm it fails because the normal request omits reasoning.
- [ ] Pass the explicit effort through `ModelAdapters::respond` and its callers.
- [ ] Run the targeted adapter tests and confirm they pass.

### Task 2: First-attempt Agent behavior

**Files:**
- Modify: `services/agent-gateway/src/orchestrator.rs`
- Test: `services/agent-gateway/src/orchestrator.rs`

**Interfaces:**
- Consumes: `ModelAdapters::respond_stream(..., reasoning_effort)`.
- Produces: every Agent generation round uses `Some("none")`; recovery events and tool routing remain unchanged.

- [ ] Change the HTTP integration test to require `reasoning.effort=none` on the first request.
- [ ] Run it and confirm failure because only recovery currently disables reasoning.
- [ ] Make every generation round pass `Some("none")` and keep recovery state independent of reasoning.
- [ ] Run Agent recovery and tool-loop integration tests.

### Task 3: Gate behavior

**Files:**
- Modify: `services/agent-gateway/src/orchestrator.rs`
- Test: `services/agent-gateway/src/orchestrator.rs`

**Interfaces:**
- Consumes: `ModelAdapters::respond(..., Some("none"))`.
- Produces: the Gate request disables reasoning while preserving structured tool parsing and timeout fallback.

- [ ] Add a Gate HTTP contract assertion for `reasoning.effort=none`.
- [ ] Run it and confirm it fails against the current Gate request.
- [ ] Pass `Some("none")` from `gate_transcript`.
- [ ] Run Gate unit and integration tests.

### Task 4: Validation and publication

**Files:**
- Verify: `services/agent-gateway/src/adapters.rs`
- Verify: `services/agent-gateway/src/orchestrator.rs`
- Preserve: `services/agent-gateway/src/endpointing.rs`

**Interfaces:**
- Consumes: local and server working trees plus the live Responses endpoint.
- Produces: one verified commit pushed to the configured origin and checked out cleanly on both machines.

- [ ] Run `cargo fmt --check`, all gateway tests, strict Clippy, and `git diff --check` locally.
- [ ] Run direct live-model probes for normal text, Gate structured output, and a tool call.
- [ ] Synchronize the exact files to the server and run tests, release build, readiness, and smoke checks.
- [ ] Stage only reviewed files, commit once, push the current branch, align the server checkout to that commit, and verify both work trees are clean at the same SHA.
