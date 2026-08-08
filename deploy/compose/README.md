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

## Device pairing

Mobile QR pairing (NIP-AB) needs the `buzz-pair-relay` sidecar: a
membership-gated (NIP-43) relay rejects unpaired devices, so the pairing
handshake runs through a separate ephemeral relay instead. Without it, phones
scanning the desktop QR fail with a WebSocket 404 on `<relay>/pair`.

The TLS stack includes the sidecar by default (`compose.pair.yml`):

- Runs `buzz-pair-relay` from the same relay image.
- Caddy routes `/pair` to it; everything else still goes to the relay.
- Sets `BUZZ_PAIRING_RELAY_URL=wss://$BUZZ_DOMAIN/pair` on the relay so the
  pairing URL is advertised in NIP-11.

Set `BUZZ_COMPOSE_PAIRING=false` to opt out — `/pair` then returns 502 from
Caddy. Pairing is not wired for the non-TLS stack: there is no reverse proxy
to route `/pair`, and iOS requires `wss://` anyway.

Verify after `./run.sh start`:

```bash
curl -fsS "https://<your-domain>" -H 'Accept: application/nostr+json' | grep -o 'pairing_relay_url[^,]*'
curl -is "https://<your-domain>/pair" | head -1   # expect HTTP 400 (non-WebSocket request rejected)
```

Run `./run.sh backup-hint` for the backup checklist.

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
