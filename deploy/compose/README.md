# Buzz Docker Compose deployment

This is the single-node/VPS deployment bundle. It is intentionally separate from
the root `docker-compose.yml`, which remains local development infrastructure.

## Quick start

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env       # replace every CHANGE_ME value
./run.sh start
```

For a public VPS with automatic Let's Encrypt certificates:

```bash
cd deploy/compose
BUZZ_COMPOSE_TLS=true ./run.sh start
```

The bootstrap script should eventually replace manual `.env` editing for normal
users. It is responsible for generating stable secrets and, optionally, an owner
keypair.

## Production notes

- Requires Docker Compose v2.24.4 or newer; the TLS override uses Compose's
  `!reset` tag to remove the direct relay port when Caddy terminates HTTPS.
- Default `BUZZ_IMAGE` tracks `ghcr.io/block/buzz:main` for early testing. Pin it to `ghcr.io/block/buzz:sha-<7>` or a semver release tag for production once available.
- Keep `BUZZ_RELAY_PRIVATE_KEY`, `BUZZ_GIT_HOOK_HMAC_SECRET`, database/Redis,
  and S3 secrets stable across restarts.
- `RELAY_OWNER_PUBKEY` is intentionally not prefixed with `BUZZ_`; it must be a
  64-character hex Nostr pubkey when closed relay mode is enabled.
- `BUZZ_AUTO_MIGRATE` is opt-in. Set `BUZZ_AUTO_MIGRATE=true` or run
  `buzz-admin migrate` before starting the relay when bootstrapping a fresh
  database. Auto-migration requires an image that includes embedded SQLx
  migrations.
- The stack uses Postgres, Redis, MinIO, and a git data volume because
  those are real Buzz dependencies today. Minimal mode can simplify this later.

Run `./run.sh backup-hint` for the backup checklist.

## Key-value backend: Redis or Valkey

The relay talks to its key-value store through a single `REDIS_URL` connection
string using the RESP protocol, so it works unchanged against Redis **or**
[Valkey](https://valkey.io) — the permissively-licensed (BSD-3) fork of Redis.
This was verified by inventorying every command Buzz issues (including the
commands inside its five Lua scripts) and replaying them against a live Valkey
8.1.6 instance: all behaved identically, and Buzz's own Redis-dependent test
suites (pub/sub presence + NIP-98 replay protection, the fenced-lease tunnel
suite, and the mesh registry suite) passed in full. No Redis modules, streams,
cluster APIs, or keyspace notifications are used.

Redis remains the default. To run the bundled key-value service on Valkey
instead — for a fully Apache-2.0/BSD self-host stack — enable the override:

```bash
BUZZ_COMPOSE_VALKEY=true ./run.sh start
```

This swaps the container image to `valkey/valkey:8-alpine`; the service keeps
the name `redis`, so `REDIS_URL` and `REDIS_PASSWORD` are unchanged.

**Managed Valkey (e.g. AWS ElastiCache):** skip the local service entirely and
point `REDIS_URL` in `.env` at the managed endpoint — the relay dials whatever
RESP-compatible host the connection string names:

```bash
# .env
REDIS_URL=rediss://my-valkey-endpoint.cache.amazonaws.com:6379
```

## Validation

Before sharing an install link publicly, verify a fresh install with:

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env
./run.sh config
./run.sh start
curl -fsS "http://127.0.0.1:$(grep -E '^BUZZ_HTTP_PORT=' .env | cut -d= -f2-)/_liveness"
./run.sh status
```
