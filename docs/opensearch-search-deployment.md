# OpenSearch message index deployment

With `BUZZ_OPENSEARCH_URL` set, the relay keeps an OpenSearch index of
messages up to date through the `search_outbox` table. Searches still run on
Postgres.

| Variable | Meaning |
| --- | --- |
| `BUZZ_OPENSEARCH_URL` | HTTP(S) cluster URL without credentials, query or fragment. Unset turns indexing off. |
| `BUZZ_OPENSEARCH_USERNAME`, `BUZZ_OPENSEARCH_PASSWORD` | Optional basic auth. Set both or neither. |
| `BUZZ_OPENSEARCH_AWS_SIGV4_SERVICE` | Optional AWS SigV4 request signing, for Amazon OpenSearch Service: `es` for a managed domain, `aoss` for a Serverless collection. Unset turns signing off. Not combined with basic auth. |
| `BUZZ_OPENSEARCH_AWS_REGION` | Region to sign for. Defaults to `AWS_REGION`, then `AWS_DEFAULT_REGION`. Signing needs one of them. |
| `BUZZ_OPENSEARCH_PREVIEW` | Required with the URL until [block/buzz#9862](https://github.com/block/buzz/pull/9862), because deleted messages stay in the index. Use it only on relays without real data. |

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

1. Deploy a release that lists `search_outbox` as a pending deletion-scoped
   table ([block/buzz#9871](https://github.com/block/buzz/pull/9871)).
   Deletion code that predates it rejects the catalog once migration `0106`
   runs, which stops community deletion.
2. Deploy the release with migration `0106`. Nothing is queued until the
   setting is on.
3. Set `BUZZ_OPENSEARCH_URL` and `BUZZ_OPENSEARCH_PREVIEW=true` on every
   relay. Each relay queues only the messages it stores, so one without the
   settings leaves gaps. A relay with them refuses to start before `0106` has
   run. Until [block/buzz#9862](https://github.com/block/buzz/pull/9862)
   caps bulk requests, a cluster with a 10 MiB request limit can reject a
   full batch.

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
BUZZ_OPENSEARCH_URL=http://localhost:9200 BUZZ_OPENSEARCH_PREVIEW=true just relay
curl -s 'localhost:9200/buzz-messages/_search?pretty'
```
