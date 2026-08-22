# Ripple Passport Realtime Protocol

This document is the external service contract required by the standalone
firmware. The Gateway implementation is intentionally not part of this branch.

## Transport

- WebSocket URL: `ws://<configured-host>/v1/agent/realtime`
- Protocol version: `5`
- Client build: `passport-0.2`
- JSON text frames only; fragmented WebSocket messages are reassembled by the
  firmware before JSON parsing.
- Audio payload byte order is the native little-endian order used by ESP32-C3.

The validation firmware does not send an authorization token. The Gateway must
map this device connection to a configured anonymous validation account.

## Session startup

Client:

```json
{
  "type": "session.start",
  "protocol_version": 5,
  "client_build": "passport-0.2",
  "mode": "audio"
}
```

Server:

```json
{"type":"session.ready"}
```

The device does not enable PTT until the WebSocket is connected and
`session.ready` has been received.

## Input turn

Pressing `OK` first cancels any active response and clears stale input:

```json
{"type":"response.cancel","clear_input":true}
```

Then the client starts a turn:

```json
{"type":"input.speech_started","turn_id":"passport-<time>-<counter>"}
```

Microphone data is read as 16 kHz mono PCM16 in 40 ms blocks, converted to
little-endian Float32 samples in `[-1, 1]`, base64 encoded, and sent as:

```json
{
  "type": "input.audio.append",
  "sample_rate": 16000,
  "audio": "<base64 little-endian Float32>"
}
```

Releasing `OK` commits the same turn ID:

```json
{"type":"input.commit","turn_id":"passport-<time>-<counter>"}
```

## Response stream

The normal server sequence is:

```json
{"type":"response.created","response_id":"response-123"}
```

```json
{
  "type": "response.audio.delta",
  "response_id": "response-123",
  "sample_rate": 24000,
  "audio": "<base64 little-endian Float32>"
}
```

```json
{"type":"response.done","response_id":"response-123","text":"optional final text"}
```

Every response event must carry the same non-empty `response_id`. The firmware
rejects stale response IDs, sample rates other than 24 kHz, deltas larger than
2,400 samples (100 ms), and invalid Float32/base64 payloads. Playback starts
after the actual queued duration reaches about 400 ms; a completed short
response may start with less buffered audio.

## Cancellation and errors

The server may acknowledge cancellation:

```json
{"type":"response.cancelled","response_id":"response-123"}
```

Failures must use either event type and may include a human-readable message:

```json
{"type":"response.failed","response_id":"response-123","message":"..."}
```

```json
{"type":"error","message":"..."}
```

On cancellation or disconnect, the firmware stops accepting response audio and
frees every queued PCM block. Network disconnects are retried automatically.
The Gateway must send `session.ready` within 15 seconds, begin a committed
response within 60 seconds, and avoid a 30-second gap between response events.

## Gateway compatibility checklist

- Accept protocol v5 session startup without a device token in the validation
  environment.
- Return `session.ready` before accepting PTT turns.
- Accept 16 kHz mono Float32 input in 40 ms blocks.
- Produce 24 kHz mono Float32 output, preferably in 100 ms blocks.
- Preserve `turn_id` semantics and cancel the active response promptly.
- Attach one stable `response_id` to every event in a response lifecycle.
- Never interleave audio from a cancelled response with a newer response.
- Use the Responses API for model orchestration; other model API protocols are
  outside this product contract.
