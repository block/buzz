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

Mobile QR pairing (NIP-AB, kind 24134) runs through a **separate** relay, and it
has to. A device mid-pairing holds a freshly generated ephemeral key that is not
a relay member yet, so `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true` — the production
default in `.env.example` — makes the main relay reject it outright. The
`buzz-pair-relay` sidecar exists for exactly this: no auth, no persistence, no
history, just an in-flight match between two kind-24134 subscriptions.

The binary already ships inside the relay image, so this bundle runs it as the
`pairing-relay` service. Two things have to line up:

1. **A route to the sidecar.** With `BUZZ_COMPOSE_TLS=true` the bundled
   `Caddyfile` routes `/pair` and `/pair/*` to `pairing-relay:5000`; everything
   else still goes to the relay.
2. **An advertised URL.** `BUZZ_PAIRING_RELAY_URL` is published in the relay's
   NIP-11 document, and clients use it directly. Leave it at
   `wss://<your-domain>/pair` to match the Caddy route. The value must be
   `ws://` or `wss://`; the relay refuses to start otherwise.

### Bringing your own reverse proxy

The sidecar is published on **loopback only** (`BUZZ_PAIR_RELAY_PORT`, default
`127.0.0.1:5000`). It performs no authentication and enforces no membership, so
putting it on a public interface would expose an unauthenticated WebSocket
endpoint to the internet — don't. A proxy running on the same host reaches it at
`127.0.0.1:5000`; a proxy in another container can join `buzz-net` and use
`pairing-relay:5000` instead.

Whatever proxy you use, it must terminate TLS, route only `/pair`, pass the
WebSocket upgrade headers, and keep read timeouts tight — the sidecar caps each
connection at 120 seconds itself and delegates slowloris protection to the proxy
(see `crates/buzz-pair-relay/src/lib.rs`). Then point `BUZZ_PAIRING_RELAY_URL` at
whatever public URL that proxy serves.

If something genuinely off-box must reach the sidecar without a proxy, set
`BUZZ_PAIR_RELAY_PORT=0.0.0.0:5000` — but note that pairing then runs
unencrypted over `ws://`, which iOS will refuse, and the endpoint is open to
anyone who can reach the port.

### Checking it works

```bash
# Is the URL advertised?
curl -sS -H 'Accept: application/nostr+json' "https://<your-domain>" \
  | grep -o '"pairing_relay_url":"[^"]*"'

# Is something answering on /pair?
curl -sS -o /dev/null -w "%{http_code}\n" "https://<your-domain>/pair"
```

A bare **400** is the healthy answer: the sidecar serves no NIP-11 document and
rejects any request that is not a WebSocket upgrade. **404** means no route or
no sidecar — pairing will fail. **401/403**, or a socket that closes while the
desktop waits for EOSE, means `/pair` is reaching the *main* relay, which is
refusing the not-yet-member device.

Note that the sidecar requires a `#p` filter: a `REQ` for `kinds:[24134]`
without one is closed with `#p filter required`. Real clients always send it;
this only surprises people probing by hand.

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
