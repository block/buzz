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

## Coolify

Coolify can deploy this production Compose bundle directly from Git. Create a
new resource with these settings:

- Repository: `https://github.com/block/buzz`
- Build pack: **Docker Compose**
- Base directory: `/deploy/compose`
- Docker Compose location: `/compose.yml`
- Raw Compose: disabled

Use only the base `compose.yml`; do not add `compose.caddy.yml`. Coolify's proxy
terminates HTTPS and WebSockets, so the bundled Caddy service is unnecessary.

In Coolify's service settings, assign the relay a domain such as
`https://buzz.example.com:3000`. The `:3000` suffix selects the relay's internal
container port; clients still connect over normal HTTPS/WSS on port 443.

Add the variables from `.env.example` to Coolify's environment settings and
replace every `CHANGE_ME` value. At minimum, update the public URLs for your
domain:

```dotenv
BUZZ_DOMAIN=buzz.example.com
RELAY_URL=wss://buzz.example.com
BUZZ_MEDIA_BASE_URL=https://buzz.example.com/media
BUZZ_MEDIA_SERVER_DOMAIN=buzz.example.com
BUZZ_CORS_ORIGINS=https://buzz.example.com
```

Keep all generated keys, passwords, and S3 credentials stable across deploys.
Because the base Compose file also publishes the relay port, bind its fallback
host listener to loopback on any unused port:

```dotenv
BUZZ_HTTP_PORT=127.0.0.1:33000
```

After the deployment is healthy, verify the public route:

```bash
curl -fsS https://buzz.example.com/_liveness
```

Then connect the Buzz desktop app to `wss://buzz.example.com`. The
`minio-init` service is a one-shot setup job; an exit code of 0 after it creates
the bucket is expected.

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
