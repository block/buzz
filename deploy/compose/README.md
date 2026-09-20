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

## Import the owner key into Buzz Desktop

`buzz-admin generate-key` prints a hex `Public key` and `Secret key`. The
server configuration and desktop import use different encodings:

| Key | Where it belongs |
|-----|------------------|
| Owner **public** key (64-character hex) | `RELAY_OWNER_PUBKEY` in `.env` |
| Owner **secret** key (`nsec1…`) | Desktop's **Use an existing key** flow |
| Separate relay **secret** key (64-character hex) | `BUZZ_RELAY_PRIVATE_KEY` in `.env` |

### Convert an existing hex secret key to nsec

Use the owner secret key that corresponds to `RELAY_OWNER_PUBKEY`, **not**
`BUZZ_RELAY_PRIVATE_KEY`. Hex and [NIP-19](https://github.com/nostr-protocol/nips/blob/master/19.md)
`nsec` encode the same secret: conversion does not create a new identity or
require changing the owner public key in `.env`. A public key cannot be
converted into a secret key.

1. Install [nak](https://github.com/fiatjaf/nak#installation), a standalone
   Nostr CLI, on a trusted local machine and ensure `nak` is on your `PATH`.
   Prebuilt binaries are available on its
   [releases page](https://github.com/fiatjaf/nak/releases).
2. Run this command in a private terminal. At the prompt, paste only the
   owner's 64-character hex **secret** key and press Enter (input is hidden):

   ```bash
   bash -c '
     set +x
     IFS= read -r -s -p "Owner secret key (hex): " owner_secret || exit 1
     printf "\n" >&2
     printf "%s\n" "$owner_secret" | nak encode nsec
   '
   ```

   The conversion runs locally, without contacting a relay. The secret is
   passed on stdin rather than in command arguments or shell history, and the
   temporary shell exits afterward.
3. Copy the resulting `nsec1…` value into Buzz Desktop's **Use an existing key**
   flow, then connect to your relay. Keep `RELAY_OWNER_PUBKEY` unchanged.

Both the hex secret and its `nsec` form grant control of the owner identity;
`nsec` is **not encryption**. Never paste either into an online converter,
issue, chat, or screenshot. The output is visible in terminal scrollback (and
invalid input may be echoed in an error), so avoid recorded/shared terminals
and clear the clipboard and scrollback afterward. Keep a secure backup of the
owner secret; do not put it in the relay's `.env` or reuse the relay signing key
as your desktop identity.

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
- Mobile push remains off by default. To use the public gateway, keep the
  template's explicit `BUZZ_PUSH_GATEWAY_DELIVERY_URL` and set
  `BUZZ_PUSH_ENABLED=true`. To use another gateway, replace the exact HTTPS
  `/v1/deliveries/apns` URL before enabling push.
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
