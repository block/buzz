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

## Device pairing

Mobile pairing needs a dedicated pairing relay (`buzz-pair-relay`). The desktop
generates the QR, then resolves where the phone should connect: it first reads
the main relay's NIP-11 `pairing_relay_url`, and if that is unset it falls back
to the legacy `/pair` path on the main relay. The base relay does **not** serve
`/pair`, so the stack runs a `pair` sidecar (bundled in the same relay image)
and the deployment just has to route traffic to it. Three cases:

- **TLS (`compose.caddy.yml`) — works out of the box.** Caddy routes
  `/pair` on your main domain to the sidecar, so the desktop's legacy fallback
  (`wss://<domain>/pair`) Just Works with no extra DNS and nothing to set. The
  overlay unpublishes the sidecar's host port; Caddy is the only way in.
- **Split domain or your own reverse proxy.** Expose the sidecar at its own
  host name and advertise it so the desktop uses it directly:
  set `BUZZ_PAIRING_RELAY_URL=wss://pair.<domain>` in `.env` and point that
  name at the sidecar in your proxy — the base stack publishes it on
  `BUZZ_PAIR_RELAY_PORT` (default 5000). If your proxy runs *outside* this
  compose project and you would rather not publish the port, attach the proxy
  to the project's `buzz-net` network and target `pair:5000` directly. The
  relay then advertises the URL in NIP-11 and the desktop skips the `/pair`
  fallback.
- **Non-TLS / direct (no Caddy).** The base stack publishes the sidecar on
  `BUZZ_PAIR_RELAY_PORT` (default 5000); set
  `BUZZ_PAIRING_RELAY_URL=ws://<host>:5000` so the QR points at it. Without
  that, the desktop QR points at a `/pair` endpoint the relay 404s and pairing
  fails.

`BUZZ_PAIRING_RELAY_URL` must be a `ws://` or `wss://` URL; the relay rejects
anything else at startup.

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
