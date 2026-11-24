# OpenSearch message index deployment

With `BUZZ_OPENSEARCH_URL` set, the relay keeps an OpenSearch index of
messages up to date through the `search_outbox` table. Searches still run on
Postgres.

| Variable | Meaning |
| --- | --- |
| `BUZZ_OPENSEARCH_URL` | HTTP(S) cluster URL without credentials, query or fragment. Unset turns indexing off. |
| `BUZZ_OPENSEARCH_USERNAME`, `BUZZ_OPENSEARCH_PASSWORD` | Optional basic auth. Set both or neither. |

## Rollout order

1. Deploy a release that lists `search_outbox` as a pending deletion-scoped
   table ([block/buzz#9871](https://github.com/block/buzz/pull/9871)).
   Deletion code that predates it rejects the catalog once migration `0106`
   runs, which stops community deletion.
2. Deploy the release with migration `0106`. Nothing is queued until the
   setting is on.
3. Set `BUZZ_OPENSEARCH_URL` on every relay. Each relay queues only the
   messages it stores, so one without the setting leaves gaps. A relay with
   the setting refuses to start before `0106` has run.

Don't set it on a relay with real data yet. Deletes, edits and account
scrubs don't reach the index until
[block/buzz#9862](https://github.com/block/buzz/pull/9862), and nothing
purges a deleted community's documents. Searches never return them, but
their words stay in the index. That release also caps bulk requests at
8 MiB; until then, a cluster with a 10 MiB request limit can reject a full
batch.

Messages stored before step 3 are not indexed; there is no backfill yet.

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
`search_outbox` out of the publication. Its rows are short-lived queue
entries.

## Local run

```sh
docker compose --profile search up -d opensearch
BUZZ_OPENSEARCH_URL=http://localhost:9200 just relay
curl -s 'localhost:9200/buzz-messages/_search?pretty'
```
