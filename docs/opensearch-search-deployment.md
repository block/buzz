# OpenSearch message index deployment

With `BUZZ_OPENSEARCH_URL` set, the relay keeps an OpenSearch index of
messages up to date through the `search_outbox` table, and backfills the
messages stored before it was set. Searches run on Postgres unless the
`search.backend` flag says otherwise.

| Variable | Meaning |
| --- | --- |
| `BUZZ_OPENSEARCH_URL` | HTTP(S) cluster URL without credentials, query or fragment. Unset turns indexing off. |
| `BUZZ_OPENSEARCH_USERNAME`, `BUZZ_OPENSEARCH_PASSWORD` | Optional basic auth. Set both or neither. |
| `BUZZ_OPENSEARCH_AWS_SIGV4_SERVICE` | Optional AWS SigV4 request signing, for Amazon OpenSearch Service: `es` for a managed domain, `aoss` for a Serverless collection. Unset turns signing off. Not combined with basic auth. |
| `BUZZ_OPENSEARCH_AWS_REGION` | Region to sign for. Defaults to `AWS_REGION`, then `AWS_DEFAULT_REGION`. Signing needs one of them. |
| `BUZZ_SEARCH_INDEXER_WORKERS` | Indexer workers on the relay that holds the lease: 1 to 32, default 1. Each holds one connection from `BUZZ_DB_POOL_SIZE` for the length of a bulk request, so leave the pool room for them. |
| `BUZZ_OPENSEARCH_PREVIEW` | Required with the URL for now, because deleting a community leaves its messages in the index. Use it only on relays without real data. |

## AWS request signing

With signing on, the relay takes credentials from the standard AWS chain:
environment variables, the shared profile, a web identity token, then the
container or instance role. It reloads them before they expire, and static
keys every 15 minutes, so role credentials and a key changed in the shared
profile are picked up without a restart. The identity needs HTTP access to
the domain or, for Serverless, a data access policy on the collection and its
`buzz-*` indexes plus `aoss:APIAccessAll`.

When a load fails, the relay logs only the kind of error. For the cause, add
`aws_config=warn` to `RUST_LOG`. That output can include what a
`credential_process` helper printed, so make sure the helper prints no
secrets. A helper should also bound its own run time: the relay gives up on a
load that takes too long but does not stop a helper that hangs.

## Rollout order

1. Deploy a release that lists `search_outbox` and `search_backfill` as
   pending deletion-scoped tables
   ([block/buzz#9871](https://github.com/block/buzz/pull/9871)). Deletion
   code that predates it rejects the catalog once migration `0106` runs,
   which stops community deletion.
2. Deploy the release with migrations `0106` to `0108`. Nothing is queued
   until the setting is on.
3. Set `BUZZ_OPENSEARCH_URL` and `BUZZ_OPENSEARCH_PREVIEW=true` on every
   relay in one deploy. A relay queues only its own writes, so one without
   the settings leaves the index without the messages it stores and with the
   ones it deletes. A relay with them refuses to start before the migrations
   have run. The backfill starts with the first relay to hold the lease.

If a relay without the settings ran after the backfill started, wait until
every relay has them, then delete the index while searches run on
Postgres. The relay clears the backfill progress, creates the index again,
waits 30 seconds before its first write, and backfills every community. It
does the same whenever the index goes missing. Running
`DELETE FROM search_backfill` without deleting the index is
not enough: the backfill reads only live messages, so it would leave the
documents of messages deleted in the meantime. Deleting one community's row
backfills that community again.

To turn indexing off, unset the URL and restart the relays. Queued rows stay
until it is back on.

## Operating

While OpenSearch is down the queue grows without bound. Check it with:

```sh
psql -c 'SELECT count(*), min(created_at) FROM search_outbox'
```

The index holds the words of every indexed message, DMs included, so anyone
who can query the cluster can search them. Limit cluster access to the relays.

If you stream Postgres changes through logical replication, leave
`search_outbox` and `search_backfill` out of the publication. Their rows are
short-lived queue entries and progress markers.

After a point-in-time restore, or a cut-over to a cluster built by logical
replication (which copies no sequence values), the version sequence can be
far below the versions the index holds. The index then refuses changes to the
messages it holds: `buzz_search_index_version_conflicts_total` rises and their
rows stay in `search_outbox`. Rebuild the index (above), or move the sequence
past any version handed out before. The indexer takes far fewer than a
million values a second, so a value tied to the clock works after every
restore or cut-over:

```sh
psql -c "SELECT setval('search_index_version', GREATEST((SELECT last_value FROM search_index_version) + 1000000000000, 10000000000000000 + (EXTRACT(EPOCH FROM now())::bigint * 1000000)))"
```

## What to watch

| Metric | Act when |
| --- | --- |
| `buzz_search_index_lag_seconds` | It keeps growing: the indexer is failing or too slow. After an outage it stays up for up to a minute. |
| `buzz_search_outbox_depth` | It keeps growing, as with the lag. Counted up to 100,000 rows. |
| `buzz_search_index_rejected_total` | It rises steadily: the cluster refuses those documents for good, and their rows stay queued. |
| `buzz_search_index_version_conflicts_total` | It is not zero: the version sequence handed out values again (below). |
| `buzz_search_backfill_complete` | `1` means every stored message of the community is queued, not yet indexed. Move searches to OpenSearch once it is `1` for every community and `search_outbox` has emptied. |
| `buzz_search_shadow_overlap_ratio` | In shadow mode: the share of the top 10 hits both backends returned. |
| `buzz_search_requests_total{backend="opensearch", outcome="error"}` | It rises: at `2` each one is a failed search. `outcome="timeout"` counts the searches that ran out of time apart. |
| `buzz_search_opensearch_skipped_total` | Searches at `1` or `2` that ran on Postgres anyway, by `reason`. `shadow_busy` rising means shadows are being dropped because 32 are already in flight. |
| `buzz_search_backend_flag_invalid_total` | It is not zero: `search.backend` holds a value other than `0`, `1` or `2`, which means `0`. |

Only the relay that holds the indexer lease samples the lag and depth, every 15 seconds. A relay that lost the lease keeps reporting its last values, as it does for `buzz_search_backfill_complete`.

## Choosing the search backend

The integer flag `search.backend` holds one value for the whole relay, so a
relay never serves some communities from one backend and some from the
other:

| Value | Results from | OpenSearch |
| --- | --- | --- |
| `0` (default, and any unknown value) | Postgres | not queried |
| `1` (shadow) | Postgres | runs the same first-page query alongside; only compared and measured |
| `2` | OpenSearch | a failed search fails the request; nothing falls back to Postgres |

Move a relay `0` → `1` → `2`, and back to `0` to roll back. With LaunchDarkly
(the `relay` context) a change applies within seconds. The `environment`
provider is read at startup, so a change there takes a restart:

```sh
BUZZ_FEATURE_FLAGS_PROVIDER=environment BUZZ_FEATURE_FLAG_SEARCH_BACKEND=1 just relay
```

Prefix-mode searches, and searches that name a kind other than a message
kind (people search) or no kinds, stay on Postgres at every value.

The backends differ on edited messages: Postgres matches the text a message
was stored with, and the index holds its latest edit. So shadow overlap
drops for searches that touch edited messages, and moving to `2` changes
their results.

If `buzz_search_index_rejected_total` rises, investigate the rejection and
fix the cause. The relay retries rejected rows automatically. A sustained
spike can block newer messages in the affected community and pause backfill
across communities. After the fix, verify that the queue drains.

## Known gaps

- **Deleted communities.** Deleting a community does not remove its
  documents from the index. No search can return them, but their words stay
  in the index.
- **Postgres crash or failover.** The version sequence can hand out values
  again after a Postgres crash or the promotion of an asynchronous replica.
  Rows then stay queued until the sequence passes the versions the index
  holds. If a bulk request sent before the crash also lands after a newer
  write of the same message, the index keeps that message's older text
  until it changes again.
- **Writes applied late.** OpenSearch forgets a delete's version after 60
  seconds (`gc_deletes`), so an older write that lands later, such as a
  previous lease holder's bulk request, brings the deleted document back.
- **Removed text on disk.** A removed document can no longer be found, but
  its text stays in the index's files until OpenSearch merges them. The
  relay does not force that merge.

In the two cases before this one, a search can match text a message no longer has. Every
hit is re-read from Postgres before it is returned, so that text is never
shown.

## Local run

```sh
docker compose --profile search up -d opensearch
BUZZ_OPENSEARCH_URL=http://localhost:9200 BUZZ_OPENSEARCH_PREVIEW=true just relay
curl -s 'localhost:9200/buzz-messages/_search?pretty'
```
