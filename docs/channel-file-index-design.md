# Persistent channel file index — design

## Why

`listChannelFiles` (in `desktop/src/shared/api/channelFiles.ts`) re-scans a
channel's **entire** history over the network every time the Files tab opens:
it pages all top-level messages, then sweeps every thread for replies. Cost is
O(channel size) per open, all at load time.

A persistent local index moves that cost off the read path. Events (including
thread replies and deletion tombstones) already reach the client — the
cross-channel `unread_catch_up` pass and the live relay subscription both fetch
channel events with **no `top_level` scoping** — so we can index files as those
events flow through, and the Files tab reads pre-computed rows from SQLite with
no network at open. This also subsumes the main (non-threaded) files, not just
the thread-reply gap.

## Non-goals / phasing

Delivered in phases so the persisted schema settles before it holds live data:

- **Phase 1 (this doc's core):** the SQLite store — schema, migrations, the
  writer that turns events into rows (file attachments, supersedes links,
  deletion tombstones), and the read query that reproduces
  `ChannelFileEntry[]`. Pure, unit-tested, wired to nothing yet.
- **Phase 2:** backfill each channel once from a single non-`top_level` history
  scan (reusing the `channel_reconnect_repair` filter via `query_relay`), and
  hook the writer into the live stream and the `unread_catch_up` drain.
- **Phase 3:** a Tauri command reads the index; `listChannelFiles` calls it,
  falling back to today's live-query scan until the index is proven.

**Link entries** (URLs in message bodies, today's `collectChannelLinkEntries`)
are deliberately out of Phase 1 — they need message-body parsing, not just tag
parsing. The `kind` column already distinguishes `'file'` from `'link'` so link
rows slot in later without a migration. Until then the read path keeps deriving
links client-side (Phase 3 decides the exact seam).

## Store conventions (mirrors `archive/store.rs`)

- One SQLite file, opened per operation (no long-lived connection in
  `AppState`), `busy_timeout=5000`, WAL with the same retry-on-busy loop.
- Schema is `CREATE TABLE/INDEX IF NOT EXISTS`. Migrations use a **marker
  table** (`channel_file_index_migrations(name PRIMARY KEY, applied_at)`), not
  `PRAGMA user_version`, each migration guarded + committed last so a crash
  re-runs it. Phase 1 ships zero migrations (fresh table); the scaffold is
  present so later changes follow the archive convention.
- Rows are scoped by `identity_pubkey` (a column, like `archived_events`) so
  switching identity never leaks another account's file list.

## Schema

```
channel_file_index(
  identity_pubkey, channel_id, event_id, url,   -- PK (url defaults '' when absent)
  kind,            -- 'file' | 'link'
  uploaded_by, uploaded_at,
  filename, sha256, size, mime,
  supersedes,      -- event_id this file's own tag supersedes, or NULL
  deleted,         -- 1 once a tombstone targets event_id
  indexed_at
)
channel_file_tombstones(identity_pubkey, channel_id, event_id)  -- PK
```

`supersededBy` is **not stored** — it's derived at read time from other rows'
`supersedes`, exactly like the current JS second pass, so it's always correct
as new versions arrive with no row rewrites. A `channel_file_tombstones` table
makes deletion order-independent: a tombstone that arrives before the file it
deletes still wins, because inserts consult it.

## Writer (`index_events`)

Takes a batch of events (as a plain `IndexableEvent { id, kind, pubkey,
content, created_at, tags }` — trivially built from `nostr::Event`, matching
`unread_catch_up`'s `EventView`) and, in one transaction:

- kind **9 / 40002** (channel message): parse each `imeta` tag → upsert one
  `'file'` row per attachment; read the `["e", id, _, "supersedes"]` marker into
  `supersedes`; set `deleted` from the tombstone table.
- kind **40099** (system): if the JSON body is
  `{"type":"message_deleted","target_event_id":…}`, record the tombstone and
  mark any existing rows for that `event_id` deleted.

Idempotent: `INSERT … ON CONFLICT … DO UPDATE`, so re-seeing an event is a
no-op. That makes backfill + live + catch-up safe to overlap.

## Reader (`query_channel_files`)

Fetch non-deleted rows for `(identity, channel)`, newest first, then in memory:
back-fill `supersededBy` from the `supersedes` graph and null out any
`supersedes`/`supersededBy` whose other end isn't present — the same two passes
`channelFiles.ts` does today. Output is `#[serde(rename_all = "camelCase")]` so
it deserializes into the existing `ChannelFileEntry` TS type unchanged.

## Consistency notes

- **Rebuild:** the index is a cache. A version bump or corruption can drop and
  rebuild it from the Phase 2 backfill; nothing authoritative lives here.
- **Completeness:** live + catch-up cover new events; the one-time per-channel
  backfill covers history predating the index. Both feed the same idempotent
  writer.
