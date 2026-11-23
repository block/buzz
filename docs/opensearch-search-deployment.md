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

Messages stored before step 3 are not indexed; there is no backfill yet.

If you stream Postgres changes through logical replication, leave
`search_outbox` out of the publication. Its rows are short-lived queue
entries.

## Local run

```sh
docker compose --profile search up -d opensearch
BUZZ_OPENSEARCH_URL=http://localhost:9200 just relay
curl -s 'localhost:9200/buzz-messages/_search?pretty'
```
