# Buzz Docker Compose deployment

This is the single-node/VPS deployment bundle. It is intentionally separate from
the root `docker-compose.yml`, which remains local development infrastructure.

## Quick start

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env
cd ../..
```

In `.env`, replace every `CHANGE_ME` value and set `BUZZ_IMAGE` to either the
published image digest or the full 40-character commit-SHA tag. From the
repository root, validate and start through the Launchpad guard:

```bash
./launchpad/deploy/run.sh check
./launchpad/deploy/run.sh start
```

For a public VPS with automatic Let's Encrypt certificates:

```bash
BUZZ_COMPOSE_TLS=true ./launchpad/deploy/run.sh start
```

The bootstrap script should eventually replace manual `.env` editing for normal
users. It is responsible for generating stable secrets and, optionally, an owner
keypair.

## Production notes

- Requires Docker Compose v2.24.4 or newer; the TLS override uses Compose's
  `!reset` tag to remove the direct relay port when Caddy terminates HTTPS.
- GitHub Actions builds the root `Dockerfile` when a commit reaches the
  `launchpad` branch and publishes `ghcr.io/launchpad-26/buzz`. Normal VPS
  deployment consumes that prebuilt image; it does not build the Dockerfile.
- `BUZZ_IMAGE` has no default. Compose fails when it is missing. Production
  should use `ghcr.io/launchpad-26/buzz@sha256:<digest>` or the workflow's
  `ghcr.io/launchpad-26/buzz:sha-<full-40-character-commit>` tag. The moving
  `:launchpad` tag is only a convenience pointer and is rejected by the
  Launchpad guard unless explicitly allowed for development/testing.
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

Run `./launchpad/deploy/run.sh backup-hint` from the repository root for the
backup checklist.

## Image lifecycle and traceability

The command behavior is intentionally distinct:

- `start` runs `docker compose up -d --wait`. It does not explicitly pull; an
  absent image may be fetched by Compose, while an already-present tag may be
  reused.
- `upgrade` runs `docker compose pull` and then `up -d --wait`. Use this after
  intentionally changing `BUZZ_IMAGE`.
- `restart` force-recreates only the relay and does not pull an image.

To see the configured image and the immutable digest of the running relay:

```bash
cd deploy/compose
docker compose config --images
container_id=$(docker compose ps -q relay)
docker inspect --format 'configured={{.Config.Image}} image_id={{.Image}}' "$container_id"
image_id=$(docker inspect --format '{{.Image}}' "$container_id")
docker image inspect --format 'repo_digests={{json .RepoDigests}}' "$image_id"
```

The full `sha-<40-character-commit>` tag maps directly to Git. A digest is the
strongest runtime pin; the corresponding workflow run summary records both the
digest and full commit-SHA tag and includes the provenance verification command.

To upgrade, replace only `BUZZ_IMAGE` in the local, untracked `.env` with the
new verified digest or full commit-SHA tag, run
`./launchpad/deploy/run.sh check`, back up state, and run
`./launchpad/deploy/run.sh upgrade`. To roll back, restore the previous
immutable `BUZZ_IMAGE` value and run the same check and upgrade commands. An
image-only rollback is safe only when intervening database migrations are
backward-compatible; otherwise restore the matching pre-upgrade database and
object/git snapshots as a coordinated recovery.

## Validation

Before sharing an install link publicly, verify a fresh install with:

```bash
./launchpad/deploy/run.sh check
./launchpad/deploy/run.sh start
curl -fsS "http://127.0.0.1:$(grep -E '^BUZZ_HTTP_PORT=' deploy/compose/.env | cut -d= -f2-)/_liveness"
./launchpad/deploy/run.sh status
```
