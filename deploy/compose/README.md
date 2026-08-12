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
- The proposed NIP-FI configuration contract is future-facing; this bundle
  does not imply that the current relay parses or enforces it. See the
  [identity configuration contract](../../docs/CORPORATE_IDENTITY.md).
- The stack uses Postgres, Redis, MinIO, and a git data volume because
  those are real Buzz dependencies today. Minimal mode can simplify this later.

Run `./run.sh backup-hint` for the backup checklist.

## NIP-FI readiness

This Compose bundle does not provision a NIP-FI runtime, trusted edge, issuer
integration, or conformance runner. It makes no claim that the proposed
`BUZZ_NIP_FI_V1_CONFIG_JSON` document is parsed or enforced. Do not advertise
or enforce NIP-FI from this bundle, and do not add a provider-specific sidecar
or unsigned corporate identity header as a substitute.

An activating deployment must pin an exact image, isolate verifier ingress
when `trusted-proxy-hmac-v2` is enabled, deliver HMAC secrets through a
secret store rather than `.env`, and pass the complete exact-head behavioral
matrix before activation. A valid Compose render or healthy relay does not
close those gates. See the
[provider-neutral deployment guide](../../docs/NIP_FI_DEPLOYMENT.md) and
[runtime operations guide](../../docs/NIP_FI_RUNTIME_OPERATIONS.md).

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
