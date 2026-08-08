# Remove Meeting Recording Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Permanently remove the meeting-recording subsystem from the current Gateway and production runtime while preserving Git history and every non-meeting record.

**Architecture:** Apply the inverse of the isolated 20-commit meeting range as one auditable removal commit, then build and verify that exact commit before entering a maintenance window. With the old Gateway stopped, prove the destructive SQLite transaction is isolated by running it once with `ROLLBACK`, commit the same child-to-parent table deletion, vacuum the database, permanently delete the exact meeting-audio directory, and start only the no-meeting Gateway.

**Tech Stack:** Rust 2024, Axum, Tokio, SQLx with SQLite, Bash, Python `unittest`, systemd user services, curl, Git and GitHub.

## Global Constraints

- Responses API remains the only allowed agent API protocol.
- All service code changes, builds, deployment, and runtime inspection run on `lake@140.143.229.103` in `/home/lake/workspace/ripple-live`.
- Android and iOS source code are not modified.
- Git history is preserved; no reset, force-push, or history rewrite is allowed.
- Do not create a backup containing meeting audio, transcript, summary, or todo content.
- Do not uninstall host ffmpeg; only remove the Gateway's meeting-specific configuration and status output.
- The only audio deletion target is `/home/lake/workspace/ripple-live/runtime-data/agent-gateway/meeting-recordings` after exact canonical-path and non-symlink validation.
- After permanent deletion, never restart a meeting-capable Gateway binary.
- Cloud snapshots and external backups are outside SSH scope and must be reported as not modified.

## File Map

**Delete:**

- `services/agent-gateway/src/meeting/mod.rs`
- `services/agent-gateway/src/meeting/processor.rs`
- `services/agent-gateway/src/meeting/storage.rs`
- `services/agent-gateway/src/meeting/store.rs`
- `services/agent-gateway/src/meeting/types.rs`
- `services/agent-gateway/src/meeting/worker.rs`
- `services/agent-gateway/src/meeting_api.rs`

**Restore to their pre-meeting behavior:**

- `services/agent-gateway/src/main.rs` — routes, state, initialization, helpers, and API tests
- `services/agent-gateway/src/adapters.rs` — meeting-only ASR probes, summarization, organization, and tool schema
- `services/agent-gateway/src/config.rs` — meeting chunk and ffmpeg settings
- `services/agent-gateway/src/context.rs` — meeting-only pool exposure
- `services/agent-gateway/src/lib.rs` — meeting module export
- `deploy/agent-stack/start.sh` — meeting environment variables
- `deploy/agent-stack/status.sh` — meeting-driven ffmpeg status output

**Preserve:** `apps/mobile/**`, all iOS files, the approved design, this plan, and all historical Git commits.

---

### Task 1: Publish the Approved Design and Plan

**Files:**
- Existing: `docs/superpowers/specs/2026-08-08-remove-meeting-recording-design.md`
- Create: `docs/superpowers/plans/2026-08-08-remove-meeting-recording.md`

**Interfaces:**
- Consumes: approved design commit `65c12df`
- Produces: synchronized local, origin, and server `master` before code removal

- [ ] **Step 1: Verify that only approved documentation is ahead of origin**

```bash
cd /Users/lake/workspace/ripple-live
git status --short --branch
git log --oneline origin/master..HEAD
git diff --check origin/master..HEAD
```

Expected: this plan is the only uncommitted file and `65c12df` is the only committed change ahead of `origin/master`.

- [ ] **Step 2: Commit the implementation plan**

```bash
cd /Users/lake/workspace/ripple-live
git add docs/superpowers/plans/2026-08-08-remove-meeting-recording.md
git diff --cached --check
git commit -m "docs: plan meeting recording removal"
```

Expected: one commit containing only the plan.

- [ ] **Step 3: Push the approved documentation commits**

```bash
cd /Users/lake/workspace/ripple-live
git push origin master
git status --short --branch
```

Expected: local `master` and `origin/master` are synchronized and clean.

- [ ] **Step 4: Fast-forward the server checkout**

```bash
ssh lake@140.143.229.103
cd /home/lake/workspace/ripple-live
git status --short --branch
git pull --ff-only origin master
git status --short --branch
```

Expected: the server worktree is clean at the plan commit.

---

### Task 2: Remove the Meeting Subsystem in One Auditable Change

**Files:**
- Delete: the seven meeting files in the File Map
- Modify: the seven pre-existing files in the File Map

**Interfaces:**
- Consumes: isolated feature range `07405c6..48504f7`
- Produces: affected source files identical to baseline `07405c6`, without changing later documentation commits

- [ ] **Step 1: Run the removal contract before editing and verify it fails**

```bash
cd /home/lake/workspace/ripple-live
test ! -e services/agent-gateway/src/meeting
test ! -e services/agent-gateway/src/meeting_api.rs
! git grep -nE '/v1/meetings|MeetingWorker|RIPPLE_MEETING_MAX_CHUNK_BYTES|save_meeting_artifact' -- services/agent-gateway deploy/agent-stack
```

Expected: non-zero because the subsystem still exists.

- [ ] **Step 2: Apply the inverse feature range without committing**

```bash
cd /home/lake/workspace/ripple-live
git revert --no-commit 07405c6..48504f7
git status --short
git diff --cached --stat
```

Expected: the seven meeting files are deleted and only the 14 feature-touched files change.

- [ ] **Step 3: Prove the affected tree matches the pre-meeting baseline**

```bash
cd /home/lake/workspace/ripple-live
git diff --exit-code 07405c6 -- \
  deploy/agent-stack/start.sh deploy/agent-stack/status.sh \
  services/agent-gateway/src/adapters.rs services/agent-gateway/src/config.rs \
  services/agent-gateway/src/context.rs services/agent-gateway/src/lib.rs \
  services/agent-gateway/src/main.rs services/agent-gateway/src/meeting_api.rs \
  services/agent-gateway/src/meeting
```

Expected: exit code 0 and no output.

- [ ] **Step 4: Run the removal contract again and verify it passes**

```bash
cd /home/lake/workspace/ripple-live
test ! -e services/agent-gateway/src/meeting
test ! -e services/agent-gateway/src/meeting_api.rs
! git grep -nE '/v1/meetings|MeetingWorker|RIPPLE_MEETING_MAX_CHUNK_BYTES|save_meeting_artifact' -- services/agent-gateway deploy/agent-stack
```

Expected: exit code 0.

- [ ] **Step 5: Review the staged inverse**

```bash
cd /home/lake/workspace/ripple-live
git diff --cached --check
git diff --cached --name-status
git diff --cached -- deploy/agent-stack/start.sh deploy/agent-stack/status.sh services/agent-gateway/src/config.rs services/agent-gateway/src/context.rs services/agent-gateway/src/lib.rs
```

Expected: no whitespace errors, no Android or iOS changes, no removal of non-meeting settings, and Responses remains the only agent API protocol.

---

### Task 3: Verify, Commit, and Push the No-Meeting Gateway

**Files:**
- Test: `services/agent-gateway/src/**/*.rs`
- Test: `deploy/agent-stack/test-smoke-contract.py`
- Build: `services/agent-gateway/Cargo.toml`

**Interfaces:**
- Consumes: staged no-meeting source from Task 2
- Produces: pushed removal commit and release binary built from that exact source

- [ ] **Step 1: Run Rust formatting and the complete test suite**

```bash
export PATH=/home/lake/.cargo/bin:$PATH
cd /home/lake/workspace/ripple-live
cargo fmt --check --manifest-path services/agent-gateway/Cargo.toml
cargo test --manifest-path services/agent-gateway/Cargo.toml -- --test-threads=1
```

Expected: formatting exits 0 and all library, binary, and doc tests pass.

- [ ] **Step 2: Run deployment smoke-contract tests**

```bash
cd /home/lake/workspace/ripple-live
python3 -m unittest deploy/agent-stack/test-smoke-contract.py -v
```

Expected: every smoke-contract test passes.

- [ ] **Step 3: Build the release Gateway while production remains online**

```bash
export PATH=/home/lake/.cargo/bin:$PATH
cd /home/lake/workspace/ripple-live
cargo build --release --manifest-path services/agent-gateway/Cargo.toml
test -x services/agent-gateway/target/release/ripple-agent-gateway
```

Expected: release build exits 0 and the binary exists.

- [ ] **Step 4: Commit the verified removal**

```bash
cd /home/lake/workspace/ripple-live
git diff --cached --check
git commit -m "refactor(server): remove meeting recording subsystem"
git status --short --branch
```

Expected: one removal commit and a clean worktree.

- [ ] **Step 5: Push without rewriting history and rebuild the exact pushed commit**

```bash
cd /home/lake/workspace/ripple-live
git push origin master
git fetch origin
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/master)"
git merge-base --is-ancestor 48504f7 HEAD
cargo build --release --manifest-path services/agent-gateway/Cargo.toml
```

Expected: push and build succeed, tips match, and the original meeting tip remains an ancestor.

---

### Task 4: Freeze Production State and Validate the Destructive Scope

**Files:**
- Inspect: `/home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3`
- Inspect: `/home/lake/workspace/ripple-live/runtime-data/agent-gateway/meeting-recordings`
- Inspect: `/home/lake/.config/systemd/user/ripple-agent-gateway.service`

**Interfaces:**
- Consumes: the still-running meeting-capable production service and its current data
- Produces: exact pre-cleanup counts held in the maintenance shell for later equality checks

- [ ] **Step 1: Confirm the old service is still online before maintenance**

```bash
ssh lake@140.143.229.103
systemctl --user is-active ripple-agent-gateway.service
curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8700/v1/meetings
```

Expected: service state is `active`; the authenticated meeting route returns `401`, proving the old route is present before cleanup.

- [ ] **Step 2: Record and compare the meeting-data inventory**

```bash
DB=/home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3
test -f "$DB"
test ! -L "$DB"
test "$(realpath "$DB")" = /home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3
sqlite3 -readonly "$DB" "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'meeting%' ORDER BY name;"
sqlite3 -readonly "$DB" "
SELECT 'meetings', COUNT(*) FROM meetings
UNION ALL SELECT 'meeting_chunks', COUNT(*) FROM meeting_chunks
UNION ALL SELECT 'meeting_transcript_segments', COUNT(*) FROM meeting_transcript_segments
UNION ALL SELECT 'meeting_todos', COUNT(*) FROM meeting_todos
UNION ALL SELECT 'meeting_processing_jobs', COUNT(*) FROM meeting_processing_jobs;
SELECT state, COUNT(*), COALESCE(SUM(duration_ms), 0) FROM meetings GROUP BY state ORDER BY state;
SELECT stage, status, COUNT(*) FROM meeting_processing_jobs GROUP BY stage, status ORDER BY stage, status;
"
```

Expected: exactly 1 meeting, 6 chunks, 7 transcript segments, 2 todos, and 2 jobs; the meeting is `completed` with total duration `58840`; transcript and organization each have one completed job. Stop and re-evaluate the scope if any value differs.

- [ ] **Step 3: Capture canonical counts for every non-meeting application table**

Keep this SSH shell open through Tasks 5 and 6 so `COUNT_SQL` and `BEFORE_COUNTS` remain available.

```bash
COUNT_SQL="$(sqlite3 -readonly "$DB" "SELECT 'SELECT ' || quote(name) || ', COUNT(*) FROM \"' || replace(name, '\"', '\"\"') || '\";' FROM sqlite_master WHERE type='table' AND name NOT LIKE 'meeting%' AND name NOT LIKE 'sqlite_%' ORDER BY name;")"
test -n "$COUNT_SQL"
BEFORE_COUNTS="$(sqlite3 -readonly -separator '|' "$DB" "$COUNT_SQL")"
printf '%s\n' "$BEFORE_COUNTS"
```

Expected: a deterministic, non-empty `table_name|row_count` list. It is only retained in shell memory and is not a content backup.

- [ ] **Step 4: Resolve the exact recording directory without following a symlink**

```bash
AUDIO=/home/lake/workspace/ripple-live/runtime-data/agent-gateway/meeting-recordings
test -d "$AUDIO"
test ! -L "$AUDIO"
test "$(realpath "$AUDIO")" = /home/lake/workspace/ripple-live/runtime-data/agent-gateway/meeting-recordings
find "$AUDIO" -xdev -type f -name '*.m4a' | sort
test "$(find "$AUDIO" -xdev -type f -name '*.m4a' | wc -l | tr -d ' ')" = 7
du -sh "$AUDIO"
```

Expected: exactly 7 M4A files and approximately 980 KiB. Stop if the resolved path, file count, or contents differ materially.

---

### Task 5: Stop the Gateway and Permanently Remove Meeting Data

**Files:**
- Modify in place: `/home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3`
- Permanently delete: `/home/lake/workspace/ripple-live/runtime-data/agent-gateway/meeting-recordings`
- Possibly unlink empty SQLite sidecars: `/home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3-wal`, `/home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3-shm`

**Interfaces:**
- Consumes: `DB`, `AUDIO`, `COUNT_SQL`, and `BEFORE_COUNTS` from Task 4
- Produces: a valid database with no meeting tables and a filesystem with no meeting recordings

- [ ] **Step 1: Stop only the Gateway and prove port 8700 is closed**

```bash
systemctl --user stop ripple-agent-gateway.service
test "$(systemctl --user is-active ripple-agent-gateway.service || true)" = inactive
! ss -ltn '( sport = :8700 )' | tail -n +2 | grep -q .
```

Expected: the Gateway is inactive and nothing listens on TCP port 8700.

- [ ] **Step 2: Dry-run the exact drop transaction and roll it back**

```bash
sqlite3 "$DB" "PRAGMA wal_checkpoint(TRUNCATE);"
DRY_RUN_OUTPUT="$(sqlite3 -separator '|' "$DB" "
PRAGMA foreign_keys=ON;
PRAGMA secure_delete=ON;
BEGIN IMMEDIATE;
DROP TABLE IF EXISTS meeting_audio_tickets;
DROP TABLE IF EXISTS meeting_processing_jobs;
DROP TABLE IF EXISTS meeting_todos;
DROP TABLE IF EXISTS meeting_transcript_segments;
DROP TABLE IF EXISTS meeting_chunks;
DROP TABLE IF EXISTS meetings;
$COUNT_SQL
PRAGMA integrity_check;
ROLLBACK;
")"
DRY_COUNTS="$(printf '%s\n' "$DRY_RUN_OUTPUT" | sed '$d')"
DRY_INTEGRITY="$(printf '%s\n' "$DRY_RUN_OUTPUT" | tail -n 1)"
test "$DRY_COUNTS" = "$BEFORE_COUNTS"
test "$DRY_INTEGRITY" = ok
```

Expected: the transaction can drop all six possible meeting tables, every non-meeting table count remains identical, integrity is `ok`, and `ROLLBACK` leaves production unchanged.

- [ ] **Step 3: Commit the verified table removal and compact deleted content**

This is the first irreversible step and is covered by the user's explicit approval to permanently delete meeting data without a backup.

```bash
sqlite3 "$DB" "
PRAGMA foreign_keys=ON;
PRAGMA secure_delete=ON;
BEGIN IMMEDIATE;
DROP TABLE IF EXISTS meeting_audio_tickets;
DROP TABLE IF EXISTS meeting_processing_jobs;
DROP TABLE IF EXISTS meeting_todos;
DROP TABLE IF EXISTS meeting_transcript_segments;
DROP TABLE IF EXISTS meeting_chunks;
DROP TABLE IF EXISTS meetings;
COMMIT;
VACUUM;
PRAGMA wal_checkpoint(TRUNCATE);
"
AFTER_COUNTS="$(sqlite3 -readonly -separator '|' "$DB" "$COUNT_SQL")"
test "$AFTER_COUNTS" = "$BEFORE_COUNTS"
test "$(sqlite3 -readonly "$DB" 'PRAGMA integrity_check;')" = ok
test -z "$(sqlite3 -readonly "$DB" "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'meeting%' ORDER BY name;")"
```

Expected: no meeting-prefixed tables exist, SQLite integrity is `ok`, and all non-meeting row counts match exactly.

- [ ] **Step 4: Remove only empty SQLite sidecars**

```bash
for SIDECAR in \
  /home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3-wal \
  /home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3-shm
do
  if [ -e "$SIDECAR" ]; then
    test ! -L "$SIDECAR"
    test ! -s "$SIDECAR"
    unlink "$SIDECAR"
  fi
done
```

Expected: sidecars are absent, or were verified as non-symlink empty files before being unlinked. Any non-empty sidecar stops execution.

- [ ] **Step 5: Permanently delete only the validated recording directory**

```bash
test -d "$AUDIO"
test ! -L "$AUDIO"
test "$(realpath "$AUDIO")" = /home/lake/workspace/ripple-live/runtime-data/agent-gateway/meeting-recordings
find "$AUDIO" -xdev -type f -delete
find "$AUDIO" -xdev -depth -type d -empty -delete
test ! -e "$AUDIO"
```

Expected: the exact meeting-recordings directory and its files are permanently removed. No recursive broad delete or glob is used.

---

### Task 6: Start the No-Meeting Release and Verify Production

**Files:**
- Execute: `/home/lake/workspace/ripple-live/deploy/agent-stack/start.sh`
- Execute: `/home/lake/workspace/ripple-live/deploy/agent-stack/status.sh`
- Execute: `/home/lake/workspace/ripple-live/deploy/agent-stack/smoke-test.py`
- Inspect: `/home/lake/workspace/ripple-live/runtime-data/agent-gateway/context.sqlite3`

**Interfaces:**
- Consumes: the pushed no-meeting release binary and cleaned production data
- Produces: a healthy production service whose former meeting routes return `404`

- [ ] **Step 1: Start the release and verify core health**

```bash
cd /home/lake/workspace/ripple-live
./deploy/agent-stack/start.sh
systemctl --user is-active ripple-agent-gateway.service
curl -fsS http://127.0.0.1:8700/health
curl -fsS http://127.0.0.1:8700/ready
./deploy/agent-stack/status.sh
```

Expected: service state is `active`, health and readiness succeed, and status output has no meeting worker or ffmpeg dependency line.

- [ ] **Step 2: Prove every former meeting route is gone**

```bash
while read -r METHOD PATH
do
  STATUS="$(curl -sS -o /dev/null -w '%{http_code}' -X "$METHOD" "http://127.0.0.1:8700$PATH")"
  printf '%s %s -> %s\n' "$METHOD" "$PATH" "$STATUS"
  test "$STATUS" = 404
done <<'EOF'
POST /v1/meetings
GET /v1/meetings
GET /v1/meetings/removal-probe
POST /v1/meetings/removal-probe/chunks
POST /v1/meetings/removal-probe/complete
GET /v1/meetings/removal-probe/transcript
GET /v1/meetings/removal-probe/summary
GET /v1/meetings/removal-probe/todos
POST /v1/meetings/removal-probe/todos/removal-probe/confirm
DELETE /v1/meetings/removal-probe
EOF
```

Expected: all ten former endpoint shapes return `404`, not `401`, proving the router no longer recognizes them.

- [ ] **Step 3: Prove non-meeting authentication and read APIs still work**

```bash
for PATH in /v1/conversations /v1/memories /v1/todos
do
  STATUS="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:8700$PATH")"
  printf 'GET %s -> %s\n' "$PATH" "$STATUS"
  test "$STATUS" = 401
done
test -n "${RIPPLE_SMOKE_ACCESS_TOKEN:-}"
for PATH in /v1/auth/me /v1/conversations /v1/memories /v1/todos
do
  STATUS="$(curl -sS -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $RIPPLE_SMOKE_ACCESS_TOKEN" \
    "http://127.0.0.1:8700$PATH")"
  printf 'authenticated GET %s -> %s\n' "$PATH" "$STATUS"
  test "$STATUS" = 200
done
```

Expected: representative non-meeting routes return `401` without credentials, then account, conversation, memory, and global-todo reads return `200` with the existing smoke credential. The token is never printed.

- [ ] **Step 4: Recheck the database and filesystem after startup**

```bash
test "$(sqlite3 -readonly "$DB" 'PRAGMA integrity_check;')" = ok
test -z "$(sqlite3 -readonly "$DB" "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'meeting%' ORDER BY name;")"
RESTART_COUNTS="$(sqlite3 -readonly -separator '|' "$DB" "$COUNT_SQL")"
test "$RESTART_COUNTS" = "$BEFORE_COUNTS"
test ! -e /home/lake/workspace/ripple-live/runtime-data/agent-gateway/meeting-recordings
test -z "$(find /home/lake/workspace/ripple-live/runtime-data -xdev -type f -name '*.m4a' -print -quit)"
```

Expected: startup does not recreate meeting tables or audio, database integrity is `ok`, and all non-meeting counts remain identical.

- [ ] **Step 5: Run the Responses API production smoke test**

```bash
cd /home/lake/workspace/ripple-live
RIPPLE_SMOKE_PYTHON=/home/lake/workspace/ripple-live/.venv-qwen3-asr-1.7b/bin/python \
  python3 deploy/agent-stack/smoke-test.py --responses-only
```

Expected: the Responses-only model smoke passes; no alternate agent API protocol is exercised.

- [ ] **Step 6: Run the authenticated Gateway realtime smoke without exposing its token**

```bash
test -n "${RIPPLE_SMOKE_ACCESS_TOKEN:-}"
cd /home/lake/workspace/ripple-live
RIPPLE_SMOKE_PYTHON=/home/lake/workspace/ripple-live/.venv-qwen3-asr-1.7b/bin/python \
  python3 deploy/agent-stack/smoke-test.py
```

Expected: authenticated Gateway health, realtime voice, Responses-backed tool use, TTS-to-ASR loopback, video-frame handling, cancellation, and response isolation all pass. The access token is consumed only from the existing shell environment and is never printed, stored, or committed; stop for the missing credential instead of weakening authentication.

- [ ] **Step 7: Run final source, history, and deployment guards**

```bash
cd /home/lake/workspace/ripple-live
! rg -n '/v1/meetings|MeetingRepository|MeetingWorker|meeting-recordings|RIPPLE_MEETING|ffmpeg' services/agent-gateway deploy/agent-stack
git status --short --branch
git fetch origin
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/master)"
git merge-base --is-ancestor 48504f7 HEAD
systemctl --user is-active ripple-agent-gateway.service
```

Expected: no meeting implementation references remain, the remote worktree is clean and synchronized, commit `48504f7` remains in history, and the Gateway is active. Report explicitly that host-local meeting data is gone while provider/cloud snapshots or external backups, if any, were outside this SSH cleanup and were not altered.
