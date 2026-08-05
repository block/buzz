# Writer-fence cutover

Migrations `0027_writer_epoch_fence.sql` and
`0028_writer_epoch_commit_and_effect_fence.sql` install the deployment-global
`buzz_writer_fence` authority, the server-side `buzz_writer_fence_config` mode
row, `ENABLE ALWAYS` triggers, and the commit/effect serialization boundary.
The relay acquires an epoch/lease at startup
when `BUZZ_WRITER_FENCE_REQUIRED=true`, stamps every writer-pool connection,
renews the lease, and turns readiness off if renewal or live revalidation
fails. The process environment controls startup behavior; the database config
row controls mutation enforcement and cannot be disabled with `SET` or
`SET LOCAL`.

## Preconditions

- The backup has a successful independent restore proof.
- All old writers are stopped and drained before the database-wide fence is
  enabled.
- The migration/control role owns `buzz_writer_fence` and its functions.
- The migration/control role owns `buzz_writer_fence_config` and can update its
  singleton row.
- The relay database role is a non-owner, non-superuser with no direct table
  privileges on either writer-fence control table.
- Existing sessions for the old runtime role are drained before role rotation;
  the post-rotation audit proves no old session remains and the new runtime role
  owns no public relation.

## Install the migration without serving traffic

The first installation must run with process fencing disabled because the
authority tables do not exist until migration 0027 completes and the commit/
effect boundary is not installed until migration 0028 completes. The migrations
creates the config row with `required=false`; the relay has a one-shot
migration mode for this bootstrap:

```sh
BUZZ_MIGRATE_ONLY=true \
BUZZ_AUTO_MIGRATE=true \
BUZZ_WRITER_FENCE_REQUIRED=false \
docker compose run --rm --no-deps relay
```

The process exits immediately after SQLx records the migration; it does not
bind the public relay or connect to Redis.

## Harden ownership and privileges

Run `scripts/writer_fence_hardening.sql` as the existing migration
administrator, substituting the actual runtime role and passwords for the
deployment. It terminates and rechecks old `buzz` sessions before renaming the
role, then audits ownership and superuser state. The important server-side part
also includes the effect permit:

```sql
CREATE ROLE buzz_writer_fence_owner NOLOGIN NOINHERIT;
ALTER TABLE public.buzz_writer_fence OWNER TO buzz_writer_fence_owner;
ALTER TABLE public.buzz_writer_fence_config OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_acquire(text, text, integer) OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_renew(text, bigint, text, integer) OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_state(text) OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_check(text, bigint, text) OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_guard() OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_effect_check(text, bigint, text) OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_commit_guard() OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_truncate_guard() OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_begin_effect(text, bigint, text, text) OWNER TO buzz_writer_fence_owner;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_acquire(text, text, integer) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_renew(text, bigint, text, integer) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_state(text) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_begin_effect(text, bigint, text, text) TO buzz;
ALTER ROLE buzz NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;
UPDATE public.buzz_writer_fence_config
   SET required = TRUE, updated_at = clock_timestamp()
 WHERE singleton;
```

The role/ownership checks are a hard gate. Do not set the config row to
`required=true` while the old relay is still writing. A database/session GUC
named `buzz.writer_fence_required` is not the enforcement boundary.

## Close the commit and external-effect races

Migration 0028 adds a deferred `ENABLE ALWAYS` constraint trigger that
revalidates the epoch/holder at transaction commit. A transaction that passed
the row-level check but lost the epoch before commit is therefore rejected.

Redis publication, presence, connection-control publication, and push delivery
must begin a writer-fence effect permit and keep its PostgreSQL transaction open
until the Redis/HTTP operation returns. Epoch takeover takes the incompatible
row lock and cannot pass the permit while the external operation is in flight.
Use the stable event ID or push outbox UUID as the retry key; never create a new
effect identity merely because permit commit or the network result is
ambiguous.

## Start the fenced relay

Set these in the relay environment and restart the new image:

```dotenv
BUZZ_WRITER_FENCE_REQUIRED=true
BUZZ_WRITER_FENCE_RESOURCE=buzz-relay
BUZZ_WRITER_FENCE_LEASE_SECONDS=30
BUZZ_WRITER_FENCE_RENEW_INTERVAL_SECONDS=10
BUZZ_AUTO_MIGRATE=false
```

Validate the private health port, not the public app port:

```sh
curl -fsS http://127.0.0.1:8080/_readiness
```

`/_readiness` remains unauthenticated only on the health router so Docker/K8s
can probe it; it is no longer mounted on the public application router. The
response contains only booleans and never exposes the holder identity or raw
lease token.

## Rollback boundary

Stop the new relay, set the server-side config row to `required=false`, and
restore the previous image only after confirming the backup/restore path. The
migration objects can remain installed; the config row is what keeps the old
image compatible while the server-side enforcement is off:

```sql
UPDATE public.buzz_writer_fence_config
   SET required = FALSE, updated_at = clock_timestamp()
 WHERE singleton;
```
