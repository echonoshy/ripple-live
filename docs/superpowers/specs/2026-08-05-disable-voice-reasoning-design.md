# Disable Voice Reasoning Design

## Goal

Reduce voice-response latency by disabling model reasoning for normal Agent and response-Gate requests from their first attempt.

## Scope

- Send `reasoning: {"effort":"none"}` on every normal streaming Agent request.
- Send the same setting on every response-Gate request.
- Preserve the existing endpoint-classifier setting and empty-response recovery behavior.
- Preserve tool schemas, tool routing, token limits, TTS segmentation, audio playback buffering, Android behavior, and frozen iOS code.

## Design

The Responses request builder remains the single place that serializes the optional reasoning setting. `ModelAdapters::respond` accepts a reasoning-effort option so the Gate can explicitly disable reasoning, while `AgentOrchestrator` passes `none` for every streaming generation round. Recovery metadata continues to distinguish recovery attempts, but recovery no longer changes the reasoning mode because the first request is already in the low-latency mode.

## Failure Handling

The existing single retry for an empty or `max_output_tokens` response remains active. Tool restrictions and the original input remain unchanged during recovery. Upstream transport, malformed tool-call, and non-recoverable incomplete responses retain their existing error paths.

## Verification

- Request-body tests prove normal Agent and Gate requests contain `reasoning.effort=none`.
- Orchestrator integration tests prove the first Agent HTTP request uses `none` and tool calls still complete.
- All Rust tests, formatting, linting, release build, server readiness, Responses smoke tests, and direct live-model latency probes must pass.

