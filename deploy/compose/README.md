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
- The bundled Compose stack fixes the relay endpoint to `http://minio:9000` and
  `BUZZ_S3_ADDRESSING_STYLE=path`: Docker DNS resolves `minio`, not
  `<bucket>.minio`. It is not configurable for an external S3 provider through
  `.env`; use the Helm chart or a custom Compose configuration for providers
  such as new Railway Storage Buckets that require `virtual` addressing.

Run `./run.sh backup-hint` for the backup checklist.

## Mobile pairing

`BUZZ_REQUIRE_RELAY_MEMBERSHIP=true` (the production default in `.env.example`)
makes the relay advertise NIP-43 device pairing in its NIP-11 document,
regardless of whether a pairing relay is actually reachable. Without the
wiring below, the desktop/mobile app's pairing probe finds NIP-43 advertised,
falls back to the legacy `<relay>/pair` convention, and gets
`HTTP error: 404 Not Found` — there is nothing listening on `/pair`.

To make mobile pairing work out of the box, this bundle includes:

- A `pairing-relay` service in `compose.yml` — the same image as `relay`,
  running the bundled `buzz-pair-relay` binary instead. It's stateless and
  needs no database/Redis/secrets.
- A `BUZZ_PAIRING_RELAY_URL` env var (see `.env.example`) that the main relay
  advertises directly in NIP-11, so the client connects to the pairing relay
  without needing the legacy fallback at all.
- With `BUZZ_COMPOSE_TLS=true` (`compose.caddy.yml`), a `/pair` route in the
  `Caddyfile` that proxies to `pairing-relay:5000`, including the WebSocket
  upgrade.

If you're **not** using `compose.caddy.yml` (bringing your own reverse
proxy), you must add an equivalent `/pair` → `pairing-relay:5000` route
yourself — see `crates/buzz-pair-relay`'s module docs for the requirements
(route only `/pair`, terminate TLS, enforce read timeouts). Without that
route, either wire it up or set `BUZZ_REQUIRE_RELAY_MEMBERSHIP=false` and
clear `BUZZ_PAIRING_RELAY_URL` to disable device pairing entirely on an open
relay.

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
