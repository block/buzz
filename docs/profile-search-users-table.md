# People search from the `users` table

`BUZZ_PROFILE_SEARCH_USERS_TABLE` switches `kinds == [0]` NIP-50 searches from
Postgres full-text search over `events` to the `users` profile projection.
This document covers what the projection is, what the search does and does
not honour on that path, and the order to roll it out in.

## Why

The desktop and mobile people pickers send
`{"kinds":[0],"search":"<text>","search_mode":"prefix","limit":N,"page":P}`.
Full-text search over the whole kind-0 JSON ranks on lexeme hits, so a profile
whose `name` is `wes` can lose to an unrelated profile whose `about` mentions
"wes", and prefix mode is a scan over every stored kind-0 row rather than a
lookup over people. The `users` table already holds one row per pubkey; this
setting makes it the row source for people search, with results hydrated back
to real signed kind-0 events so clients see the same event shape as before.

## The projection

`users` carries a per-pubkey projection of the current kind-0 head:

| Column | Kind-0 source |
|---|---|
| `name` | `name` (migration 0083 adds this column) |
| `display_name` | `display_name`, falling back to `displayName` |
| `avatar_url` | `picture`, falling back to `image` |
| `about` | `about` |
| `nip05_handle` | `nip05`, only when its domain is the bound tenant host |
| `metadata_event_id` | the kind-0 event id |

Before this change `display_name` held `display_name` *or* `name`. Readers
that want a label (`channel_members`, `dm`, media upload attribution) now
`COALESCE(display_name, name)`, so their output is unchanged.

The projection is written **inside the kind-0 replace transaction**
(`Db::replace_profile_event`): an accepted profile and its `users` row commit
or roll back together. The previous implementation wrote the row as a
post-storage side effect, so a failure after the event was durable left the
profile unsearchable with no retry record. Every kind-0 write path uses this
seam, including the relay's own "`<host>` Moderation" profile (re-published
with every moderation notice), so the relay's own writes never add an
unprojected publisher.

A NIP-05 handle another row already holds is written under a savepoint and
then resolved against the **holder's current kind-0 head**, not its row. A
holder whose head still carries the handle keeps it, and the new profile
projects without it (NIP-05 resolution is identity-bearing, so a live claim is
never displaced). A holder whose head no longer carries it — a row that
predates this seam, or was left behind when its owner moved on — is stale and
releases the handle, so the claim lands on the first write. The holder's head
is read under the holder's own replace lock; a holder mid-replace is treated
as contested and the claimant's next write or backfill pass picks it up.

## Semantics on the `users` path

With the setting on, a filter whose `kinds` is non-empty and all `0`:

- runs `search_users` over `name`, `display_name`, and `nip05_handle` as a
  case-insensitive substring match, and over the hex pubkey as a prefix match
  only (a one- or two-character query drawn from `[0-9a-f]` would otherwise
  substring-match a large share of every pubkey);
- ranks exact > prefix > contains (name/display_name before NIP-05 before
  pubkey within each tier), then by label, then by pubkey, so paging is total
  and stable;
- considers only rows projected from a live head (`metadata_event_id IS NOT
  NULL`). A row `ensure_user` created for a pubkey that only authenticated can
  never hydrate to a kind-0, so it does not consume a `limit`/`offset` slot;
- honours `authors` (`pubkey = ANY(...)`; an empty list means any author, as on
  the FTS path);
- excludes deactivated users (`deactivated_at IS NOT NULL`);
- clamps `limit` to `1..=500`; on the HTTP bridge, `page` (1-based) skips
  `(page - 1) * limit` ranked people;
- hydrates each hit to that pubkey's **current** kind-0 head
  (`DISTINCT ON (pubkey) ... ORDER BY created_at DESC, id ASC`, live rows
  only) and returns the heads in `users` rank order. A hit with no live head
  is dropped rather than replaced by a stale row;
- applies the normal NIP-01 post-filter and reader-visibility gates to the
  hydrated events.

Three filter fields are **no-ops** on this path, by design:

- `search_mode` — the `users` match is already substring semantics, so
  prefix vs full-text has nothing to act on.
- `since` / `until` — the table has one row per pubkey with no event
  timestamps; the hydrated head is whatever is current.

A kind-0 filter that also carries `#h` returns nothing (heads are global, not
channel-scoped), and a connection without global read access skips the query,
matching what the FTS scope would have produced.

Filters with any other `kinds`, or no `kinds`, and every search with the
setting off, keep the FTS path in `buzz-search` unchanged.

## Rollout

The row source must be complete before the switch flips, or people with a
kind-0 but no projected row vanish from search. The order is:

1. **Migrate.** Apply migration `0083_users_name.sql` (`buzz-admin migrate`,
   or `BUZZ_AUTO_MIGRATE=true` on the next relay start).
2. **Precheck.**

   ```bash
   buzz-admin backfill-user-profiles --precheck
   ```

   Prints, per live community, the number of distinct kind-0 publishers, the
   number of `users` rows, and `unprojected` — publishers whose
   `users.metadata_event_id` is not their current head. This is read-only and
   covers every community.
3. **Backfill** each community whose `unprojected` is non-zero. `buzz-admin`
   is single-community per invocation, resolved from `RELAY_URL`:

   ```bash
   RELAY_URL=wss://<community-host> buzz-admin backfill-user-profiles --batch-size 500
   ```

   Each batch is one transaction over up to `--batch-size` pubkeys. It takes
   the same per-pubkey advisory lock live kind-0 replaces use, so running it
   while the relay serves traffic never overwrites a newer profile with an
   older head. One pass converges: a handle still held by a stale row is
   released when its claimant is projected, whichever batch each lands in,
   so a publisher is never left without a handle its head claims. Re-running
   is a no-op over current heads. The one exception is a holder that was
   mid-replace when its claimant was visited; that claimant is projected
   without the handle and picks it up on its next publish or backfill pass.
   Moderation profiles published by builds that predate this seam are
   ordinary unprojected publishers and are projected like any other.
4. **Re-run the precheck.** It should read `unprojected 0` for every
   community after a single pass; a non-zero count means kind-0 heads landed
   through a path that bypassed the projection (file a bug), not that another
   pass is due. Relay traffic does not move the count either: its moderation
   profile is projected when it is published.
5. **Enable** the setting on the relay (`BUZZ_PROFILE_SEARCH_USERS_TABLE=true`;
   in the Helm deployment, via `relay.extraEnv`) and restart.

Flipping the setting back to `false` restores the FTS path immediately; the
projection keeps being written either way. A later cleanup can remove the
kind-0 branch from `buzz-search` once every deployment runs with the setting
on.
