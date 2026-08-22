# Agent Gateway PostgreSQL migration

The Gateway now requires PostgreSQL with the `vector` and `pg_trgm` extensions.
The asset directory is unchanged; JPEG files remain under
`RIPPLE_DATA_DIR/assets` and only their metadata is migrated.

## Required settings

```bash
export RIPPLE_DATABASE_URL='postgres://ripple_live:REDACTED@127.0.0.1:5432/ripple_live'
export RIPPLE_DATABASE_MAX_CONNECTIONS=16
```

On startup the Gateway applies the versioned SQL migrations in `migrations/`.
Do not point a new build at production before importing and validating the
existing SQLite database.

## Offline import

First create a consistent SQLite backup. Do not copy the live database, WAL,
and SHM files independently.

```bash
sqlite3 runtime-data/agent-gateway/context.sqlite3 \
  '.backup /path/to/context.snapshot.sqlite3'
```

The target PostgreSQL database must be empty. The importer refuses to overwrite
a non-empty target.

```bash
RIPPLE_SQLITE_PATH=/path/to/context.snapshot.sqlite3 \
RIPPLE_DATABASE_URL='postgres://ripple_live:REDACTED@127.0.0.1:5432/ripple_live' \
cargo run --release --bin migrate_sqlite_to_postgres
```

The importer preserves IDs and source relationships, resets PostgreSQL identity
sequences, and verifies row counts for users, conversations, sessions, events,
explicit memories, assets, attachments, and todos. Legacy session memories are
copied to `legacy_memory_archive`; rows whose conversation still identifies an
owner are also converted into user-owned `memory_items`.

## Production cutover

1. Put the Gateway into drain mode and wait for active turns to finish.
2. Create a final SQLite `.backup` snapshot and retain it for rollback.
3. Import into a new empty PostgreSQL database.
4. Start one Gateway instance with `RIPPLE_DATABASE_URL` and the existing
   `RIPPLE_DATA_DIR`.
5. Verify `/ready`, authentication, conversation listing, image content, todos,
   project creation, project conversation creation, and a real `/v1/responses`
   request.
6. Reopen traffic only after all checks pass.

Rollback is safe before PostgreSQL writes are accepted: stop the new Gateway and
restart the previous build against the retained SQLite snapshot. After accepting
new PostgreSQL writes, rollback requires exporting those new rows first.
