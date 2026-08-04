# Semantic turn endpointing and stop command design

## Goal

Prevent the voice agent from answering during a natural pause inside a user's
sentence, without making clearly complete requests unnecessarily slow.  A
spoken stop command must silence an active reply immediately and must not
produce a new Agent or TTS reply.

The selected silence fallback is **1.5 seconds**.

## Current behavior and problem

`LiveMedia` reports Silero VAD's `onSpeechEnd` after 500 ms of silence.  The
mobile app immediately translates that callback to `input.commit`.  The
gateway then transcribes the captured audio and starts the Agent/TTS turn.
VAD detects the absence of voice, not whether the user completed a thought,
so an ordinary hesitation can become an unwanted response.

When new speech starts while audio is playing, the client already clears local
playback and sends `response.cancel` with high priority.  However, the newly
spoken phrase is later committed as an ordinary user turn; therefore an
utterance such as “停下” can itself receive a spoken reply.

## Scope

This change applies to continuous audio and video sessions.  It does not add a
new streaming-ASR provider or change wake-word activation policy.

## Protocol and state model

The protocol version increments from 3 to 4.  Each user utterance receives a
client-generated `turn_id`.

Client events:

| Event | Required fields | Meaning |
| --- | --- | --- |
| `input.speech_started` | `turn_id` | Starts a new utterance and immediately cancels active output. |
| `input.speech_resumed` | `turn_id` | Continues the same utterance after a tentative pause. |
| `input.turn.pause` | `turn_id` | VAD has observed a tentative end; the gateway should evaluate the accumulated audio without consuming it. |
| `input.commit` | `turn_id` | Finalizes the currently accumulated utterance. |

Server events:

| Event | Fields | Meaning |
| --- | --- | --- |
| `input.turn.decision` | `turn_id`, `decision`, `reason` | `decision` is `complete`, `continue`, or `uncertain`. |
| `input.command.handled` | `turn_id`, `command: "stop"` | A pure stop command was consumed and no Agent reply will be created. |

The client keeps a single endpointing state for its active `turn_id`:

1. **Speaking** — append captured audio normally.
2. **Pause pending** — after VAD ends, send `input.turn.pause` and retain the
   utterance id.  Do not send `input.commit` yet.
3. **Continue wait** — after an `continue` or `uncertain` decision, start a
   1.5-second silence timer.  If VAD starts again, cancel the timer, emit
   `input.speech_resumed`, and continue appending to the same audio buffer.
4. **Finalize** — send one `input.commit` after a `complete` decision, or when
   the 1.5-second timer expires.  Ignore delayed decisions whose `turn_id` no
   longer matches the pending utterance.

The gateway retains its current audio buffer through `input.turn.pause`; only
`input.commit`, `input.clear`, or a handled command consumes it.  A pause
evaluation must not block later audio events: it runs in a cancellable task
keyed by `turn_id`.  A new `speech_resumed`, `speech_started`, `input.clear`,
or commit invalidates its result.

## Semantic endpoint decision

On `input.turn.pause`, the gateway snapshots the current audio, transcribes the
snapshot, and evaluates whether it is safe to answer now.  The audio remains
buffered so the final Agent turn can reuse the accepted transcript rather than
transcribing a second time.

The decision is deliberately conservative:

1. Normalize whitespace and Chinese punctuation.
2. Use deterministic rules for obvious cases.  Examples of likely continuation
   include a trailing connective or incomplete lead-in such as “然后”, “但是”,
   “因为”, “如果”, “我想”, “我觉得”, “这个”, or “那个”.  Obvious complete
   questions, completed imperative requests, and sentence-final wording can be
   accepted without another model request.
3. For all remaining cases, call a small, tool-free classifier through the
   existing model adapter.  Its fixed JSON result is `complete` or `continue`
   plus a confidence score.  The prompt instructs it to choose `continue`
   whenever a natural continuation is plausible.
4. Accept only a high-confidence `complete` result.  Low confidence, malformed
   output, ASR errors, timeouts, and classifier errors yield `uncertain`.

`complete` makes the client finalize at once.  Both `continue` and `uncertain`
wait for resumed speech or the 1.5-second silence fallback.  This keeps the
system responsive for clearly complete speech and avoids a false early answer
when the semantic signal is weak.

The classifier is not provided conversation tools, user credentials, or video
frames.  Its output is parsed as data, never treated as instructions.

## Stop command behavior

The existing barge-in behavior remains: the first confirmed user speech clears
the AudioWorklet buffer locally and sends `response.cancel` before collecting
new audio.  This is the immediate-silence path and does not wait for ASR.

During pause evaluation and final commit, the gateway normalizes the transcript
and applies an exact stop-command matcher before creating a response.  Accepted
forms include “停”, “停下”, “停一下”, “停止”, “别说了”, “不要说了”,
“不用说了”, “先别说”, and “安静”, optionally preceded by a wake name or
polite filler.  The matcher must cover the entire normalized utterance; it must
not use substring matching, so “停止计时” and “不要说这个” remain ordinary
requests.

On a match, the gateway cancels any active response, clears buffered audio and
frames, does not create a user or assistant turn, and emits
`input.command.handled`.  The mobile client clears any pending endpoint timer,
keeps the session in `listening`, and does not display or play a reply.

## Error handling and observability

- Treat semantic evaluation failure as `uncertain`, then use the 1.5-second
  fallback; never fail the live session solely because endpoint classification
  failed.
- Record endpoint events with `turn_id`, audio duration, transcript character
  count, decision, reason, classifier latency, and whether fallback finalized
  the turn.  Do not log raw audio.
- Record handled stop commands with the command kind but without storing them
  as conversation turns.
- Ignore stale decisions and stale commits by `turn_id`, with a warning-level
  event for diagnostics.

## Tests

Mobile unit tests cover protocol v4, a complete decision committing once, a
continue/uncertain decision delaying exactly 1.5 seconds, resumed speech
cancelling the timer, and stale decisions being ignored.  Gateway tests cover
exact stop-command matching, non-stop near matches, classifier fallbacks,
buffer preservation across pause evaluation, and invalidation of an evaluation
when speech resumes.  Existing interruption tests continue to assert that
playback is cleared immediately on new speech.
