# Smoke Contract Cleanup Design

## Goal

Make the deployment smoke test exercise the currently deployed authenticated realtime protocol, including semantic voice endpointing and on-demand JPEG video, so it catches client/server protocol drift before release.

## Scope

The smoke test will require `RIPPLE_SMOKE_ACCESS_TOKEN` without printing it, initialize protocol v3, and keep its existing health, Responses tool-loop, first-playback, cancellation, and recovery checks.

For each synthetic voice turn it will generate one UUID `turn_id` and use that same ID in `input.speech_started` and `input.commit`. The unrelated-speech probe will assert the server records a Gate `ignore` decision. A second probe will request a visual description, wait for `input.frame.requested`, and send one embedded valid JPEG plus the exact server `response_id`; it then requires a successful terminal response.

The test will fail on an error event, timeout, mismatched response/turn identifier, duplicate terminal event, or missing playback milestone. A small stdlib contract test will cover URL token encoding, voice-event identifier correlation, JPEG event shape, and terminal-event rejection without invoking models.

## Cleanup

Delete `services/agent-gateway/src/context.rs.orig`. Git history shows it is a tracked pre-edit backup; no source, build, deployment, or test path references it. Retain existing tests and historical design/plan documents because they remain runnable or serve change-traceability.

## Deployment and Verification

Run contract tests and Python compilation locally. Apply the same focused change to the remote service checkout, run the contract test and `--responses-only` smoke there, then run the full authenticated smoke only with a supplied `RIPPLE_SMOKE_ACCESS_TOKEN`. No secret is logged or committed.
