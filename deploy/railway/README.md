# Buzz on Railway

This directory contains Buzz's first-party Railway template draft. It is intentionally scoped to Railway only; the repository README deploy button should be wired after the public image and automatic migrations PRs land.

```markdown
[![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/deploy/buzz?referralCode=buzz)
```

## Template shape

`railway.template.json` models the current full Buzz deployment as six Railway services:

- `Buzz` (`ghcr.io/block/buzz:main`) — the relay and bundled web UI. `:main` is the rolling pre-release image published from the repository's default branch; production operators should pin `:sha-<7>` or a semver tag once releases exist.
- `Postgres` — persistent database volume with daily backups requested in the template.
- `Redis` — authenticated persistent pub/sub/cache service.
- `Typesense` — persistent search service.
- `MinIO` — S3-compatible media/object storage.
- `MinIO Init` — idempotently creates the `buzz-media` bucket.

The only required user input is `RELAY_OWNER_PUBKEY`, a 64-character hex Nostr public key. Railway generates the relay key, database password, Redis password, Typesense key, MinIO password, and git hook HMAC secret with `${{secret(...)}}`.

## Buzz-specific defaults

The template uses the environment names read by `crates/buzz-relay/src/config.rs`. The app router itself exposes `/_readiness`, so Railway can health-check the public service without using Buzz's separate health-only port:

- `RELAY_URL=wss://${{RAILWAY_PUBLIC_DOMAIN}}`
- `RELAY_OWNER_PUBKEY=<required user input>`
- `DATABASE_URL=${{Postgres.DATABASE_URL}}`
- `REDIS_URL=${{Redis.REDIS_URL}}`
- `TYPESENSE_URL=http://${{Typesense.RAILWAY_PRIVATE_DOMAIN}}:${{Typesense.PORT}}`
- `TYPESENSE_API_KEY=${{Typesense.TYPESENSE_API_KEY}}`
- `BUZZ_S3_*` from the MinIO service; MinIO also sets `MINIO_DOMAIN=${{RAILWAY_PRIVATE_DOMAIN}}` so path-style S3 requests work on Railway private networking.
- `BUZZ_GIT_REPO_PATH=/data/git`
- `BUZZ_REQUIRE_AUTH_TOKEN=true`
- `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`
- `BUZZ_ALLOW_NIP_OA_AUTH=true`

Railway terminates TLS, so the public relay URL is `wss://` and CORS/media URLs use `https://`.

## Validation status

This template is mechanically valid JSON and follows the v2 template mechanics observed in Railway's Plausible, NocoDB, and Typesense templates: service refs, generated secrets, `serviceDomains`, `volumeMounts`, and app health checks.

End-to-end click-through validation is intentionally blocked until:

1. `ghcr.io/block/buzz:main` is publicly published by the image pipeline.
2. Buzz owns fresh-database migrations at startup or through the same image.

Until both land, this template should be treated as first-party deploy wiring, not a proven production install.

## Operational notes for users

Back up these values and volumes before relying on a Railway deployment:

- The owner private key corresponding to `RELAY_OWNER_PUBKEY` — Railway only stores the public key.
- `BUZZ_RELAY_PRIVATE_KEY` and `BUZZ_GIT_HOOK_HMAC_SECRET`.
- Postgres data.
- MinIO media/object data.
- The Buzz `/data` volume that stores git name-reservation state.
