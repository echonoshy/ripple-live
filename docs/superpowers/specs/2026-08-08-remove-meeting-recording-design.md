# Remove Meeting Recording — Design

Date: 2026-08-08
Status: Approved

## Goal

Permanently remove the meeting-recording capability from the current product and production runtime while preserving Git history. The removal covers the Gateway API, audio ingestion and storage, transcription and organization workers, meeting-specific model adapters, database schema and rows, production audio files, deployment configuration, and meeting-specific tests.

The Android application currently contains no meeting-recording client implementation, so this work does not change Android or iOS code.

## Non-goals

- Do not rewrite or force-push Git history.
- Do not modify or extend iOS.
- Do not remove ffmpeg from the host operating system because other services may use it.
- Do not create a backup containing meeting audio, transcripts, summaries, or todos.
- Do not change the Responses API, realtime voice, accounts, conversations, memories, or global todos.
- Do not remove cloud-provider snapshots or external backups through SSH. Those must be audited separately in the cloud console.

## Current Scope

The meeting feature was introduced by the 20 commits from `fc7acde` through `48504f7`. Relative to the parent baseline `07405c6`, it adds approximately 10,545 lines across 14 files.

The production database currently contains one completed meeting with a recorded duration of 58,840 milliseconds, six chunk rows, seven transcript rows, two meeting todo rows, and two completed processing jobs. The meeting audio directory contains seven M4A files totaling approximately 980 KB. These records and files will be permanently deleted.

The code surface includes:

- `services/agent-gateway/src/meeting/`
- `services/agent-gateway/src/meeting_api.rs`
- meeting routes, state, initialization, workers, and tests in `main.rs`
- meeting-specific transcription and organization behavior in `adapters.rs`
- meeting configuration and exports in `config.rs`, `context.rs`, and `lib.rs`
- meeting environment variables and ffmpeg reporting in deployment scripts

The runtime surface includes:

- `/v1/meetings`
- `/v1/meetings/{meeting_id}`
- meeting todo, chunk, finalize, retry, audio-ticket, and audio sub-routes
- `meetings` and all `meeting_*` SQLite tables
- `runtime-data/agent-gateway/meeting-recordings`

## Selected Approach

Use one maintenance window and one auditable removal commit. Build and verify the no-meeting Gateway before stopping production. During the maintenance window, stop the old Gateway, permanently destroy meeting database content and audio files, then start the pre-verified no-meeting Gateway.

This approach is preferred over a staged `410 Gone` release because no shipped Android client uses the meeting API. It is preferred over rebuilding the entire SQLite database because a full migration would unnecessarily risk unrelated account, conversation, memory, and todo data.

## Code and Git Design

Create one new commit named `refactor(server): remove meeting recording subsystem`. Preserve the original 20 commits in history.

The removal commit will:

1. Delete the complete `meeting` module and `meeting_api.rs`.
2. Remove meeting routes, `AppState` members, service initialization, background workers, test helpers, and meeting API tests from `main.rs`.
3. Remove meeting-only transcription probes, section summarization, artifact organization, structured meeting tool schemas, and their tests from `adapters.rs`.
4. Remove meeting-only settings, pool exposure, and module exports from `config.rs`, `context.rs`, and `lib.rs`.
5. Remove `RIPPLE_MEETING_MAX_CHUNK_BYTES`, the meeting-specific `RIPPLE_FFMPEG_BIN` path, and Gateway ffmpeg status output from deployment scripts.
6. Leave all non-meeting behavior intact, including the Responses API as the only agent API protocol.

The implementation may use the inverse of `07405c6..48504f7` as the mechanical starting point because the range is isolated to this feature. The resulting diff must still be reviewed file by file; no branch reset or history rewrite is allowed.

## Permanent Data Destruction

The production Gateway must be stopped and confirmed inactive before any database or filesystem deletion. No meeting-content backup will be created.

### SQLite

Before deletion, record only schema names and row counts for audit. Do not export content. Capture row counts for all non-meeting tables so they can be compared before the destructive transaction commits.

With the Gateway stopped:

1. Run a WAL checkpoint with truncation.
2. Enable SQLite secure deletion.
3. Begin an immediate transaction.
4. Drop the following tables if present, in child-to-parent order:
   - `meeting_audio_tickets`
   - `meeting_processing_jobs`
   - `meeting_todos`
   - `meeting_transcript_segments`
   - `meeting_chunks`
   - `meetings`
5. Confirm all non-meeting row counts are unchanged and run an integrity check before committing.
6. Roll back on any mismatch or integrity failure.
7. Commit only after the checks pass.
8. Run `VACUUM` to rewrite the database and remove deleted content from free pages.
9. Truncate WAL again, close SQLite, and remove only empty WAL/SHM sidecar files associated with this database.
10. Run a final integrity check and confirm that `sqlite_master` contains no table matching `meeting%`.

### Audio Files

The only permitted deletion target is:

`/home/lake/workspace/ripple-live/runtime-data/agent-gateway/meeting-recordings`

Before deletion, resolve and validate the canonical path, confirm it exactly matches the expected target, confirm it is not a symbolic link, and recount the expected files and size. Permanently delete files and directories without moving them to trash. After deletion, confirm that the target does not exist and that the Gateway runtime-data tree contains no remaining meeting M4A files.

Operational deletion cannot remove copies already held in cloud disk snapshots or external backup systems. Those systems require a separate audit and deletion action outside this repository and SSH host.

## Deployment Sequence

1. Produce and review the removal diff.
2. Run formatting, the complete Gateway test suite, release build, and smoke-contract tests.
3. Commit and push the removal code.
4. Build the exact pushed commit on the server while the current service is still running.
5. Start the maintenance window and stop `ripple-agent-gateway.service`.
6. Confirm the process and port are inactive.
7. Execute the SQLite and audio destruction procedure.
8. Start the pre-verified no-meeting Gateway.
9. Run health, readiness, route-removal, database, filesystem, and functional smoke checks.

The old meeting-capable Gateway must not be restarted after permanent deletion because it would recreate meeting tables and expose removed routes.

## Failure Handling

- If code verification or the release build fails, do not stop production and do not delete data.
- If any database check fails before transaction commit, roll back and leave the old service available for restart.
- If `VACUUM` fails after the destructive transaction commits, keep the Gateway stopped and repair the database maintenance operation before startup.
- If the new Gateway fails after data destruction, repair or deploy a pre-verified no-meeting binary. Do not restart a meeting-capable binary.
- Meeting data has no rollback path after the destructive transaction and filesystem deletion. This is intentional and approved.

## Verification and Acceptance

Code acceptance requires:

- no meeting module, API handler, route, worker, configuration, deployment variable, or meeting-specific test remains
- `cargo fmt --check --manifest-path services/agent-gateway/Cargo.toml` passes
- `cargo test --manifest-path services/agent-gateway/Cargo.toml` passes
- `cargo build --release --manifest-path services/agent-gateway/Cargo.toml` passes
- `python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v` passes
- repository searches confirm the removal without flagging historical documentation or Git history

Production acceptance requires:

- `ripple-agent-gateway.service` is active
- `/health` and `/ready` return healthy results
- `/v1/meetings` and all former meeting sub-routes return `404`
- SQLite integrity check returns `ok`
- no `meeting%` table exists
- all audited non-meeting table row counts are unchanged
- the meeting-recordings directory and meeting M4A files do not exist
- Responses API, realtime voice, authentication, conversations, memories, and global todos pass smoke verification

## Audit Trail

The final handoff must include:

- removal commit hash
- test and release-build results
- production service status and health results
- meeting route status after deployment
- pre-deletion meeting counts and audio size
- post-deletion schema and filesystem checks
- confirmation that Git history was preserved
- explicit note that cloud snapshots and external backups were not modified unless separately verified
