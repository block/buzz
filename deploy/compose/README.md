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

## Windows notes

- `run.sh` is a bash script. Run it from **Git Bash** or WSL, not PowerShell —
  Windows hands `.sh` files to the associated editor instead of executing them.
  In Git Bash, drive paths are written `/d/path/to/buzz/deploy/compose`.
- `$EDITOR .env` is a Unix convention; in PowerShell use `notepad .env`
  (or any editor). A file named `.env` can look nameless in File Explorer
  unless "File name extensions" is enabled.
- Git Bash (MSYS) rewrites absolute container paths in command arguments,
  which breaks `docker compose exec` container paths with "OCI runtime exec
  failed". `run.sh` disables this itself via `MSYS_NO_PATHCONV=1`; export the
  same variable before running `docker compose exec ...` by hand.
- For a **local (loopback) deployment**, set `BUZZ_DOMAIN=127.0.0.1` and use
  `ws://127.0.0.1:3000` everywhere — clients canonicalize loopback URLs to
  `127.0.0.1`, so a relay bound to the literal host `localhost` is unreachable
  by agent harnesses (they get 404 on the WebSocket upgrade). This applies to
  all platforms, not just Windows.
- Generate hex secrets in PowerShell with:

  ```powershell
  -join ((0..31) | ForEach-Object { '{0:x2}' -f (Get-Random -Maximum 256) })
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
