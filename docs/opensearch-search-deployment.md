# OpenSearch message index deployment

The relay can keep an OpenSearch index of messages up to date. Searches still
run on Postgres full-text search; the index is built ahead of switching reads.

## How it works

- Admission writes a `search_outbox` row in the same transaction as each
  stored event of a searchable kind (9, 40002, 40008, 45001, 45003). A
  rolled-back write queues nothing, and admission never calls OpenSearch.
- One relay instance holds the indexer lease (a Postgres session advisory
  lock). It claims a batch of one community's rows, re-reads those events,
  writes or removes their documents in one bulk request, and deletes the rows
  it claimed. A failed request leaves the rows queued; the indexer retries with
  backoff capped at 60 seconds. A batch the cluster rejects outright (HTTP 400
  or 413) skips its community for that pass, so other communities keep
  indexing.
- The index (`buzz-messages`) is created by the leader on its first attempt,
  so an unreachable cluster never delays startup. Writes go through the alias
  and never create an index, so a deleted index is recreated with its mapping
  on the next attempt. Documents hold the message
  text in an indexed field only; `_source` does not store it.
- Community deletion fences and purges `search_outbox` with the community's
  other rows.

## Settings

| Variable | Meaning |
| --- | --- |
| `BUZZ_OPENSEARCH_URL` | HTTP(S) cluster URL without credentials, query or fragment. Unset disables the outbox and indexer. |
| `BUZZ_OPENSEARCH_USERNAME`, `BUZZ_OPENSEARCH_PASSWORD` | Optional basic auth. Set both or neither. |

## Rollout order

1. Deploy a release that knows the `search_outbox` table as a pending
   deletion-scoped table, before the release with migration `0106`. That
   keeps community deletion working in a mixed fleet and after a rollback.
2. Deploy the release with the migration. Without `BUZZ_OPENSEARCH_URL`
   nothing is queued and nothing changes.
3. Set `BUZZ_OPENSEARCH_URL` on every relay instance. Each instance queues
   only the messages it admits itself, so an instance without the setting
   leaves gaps. Startup fails if migration `0106` has not run. Only messages
   admitted after the whole fleet has the setting are covered; older ones need
   a backfill, which is not part of this release.

If you stream Postgres changes through logical replication, keep
`search_outbox` out of the publication: its rows are short-lived queue
entries, not data.

## Local run

```sh
docker compose --profile search up -d opensearch
BUZZ_OPENSEARCH_URL=http://localhost:9200 just relay
curl -s 'localhost:9200/buzz-messages/_search?pretty'
```

Watch the queue drain with
`psql -c 'SELECT count(*) FROM search_outbox'`.
