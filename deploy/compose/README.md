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
`pairing-relay` service. With `BUZZ_COMPOSE_TLS=true` the bundled `Caddyfile`
routes `/pair` and `/pair/*` to `pairing-relay:5000`; everything else still goes
to the relay. That is the whole setup — there is nothing to configure.

`BUZZ_PAIRING_RELAY_URL` is left **unset** deliberately. A client reads the
relay's NIP-11 document and uses `pairing_relay_url` if it is there; with no
value, it falls back to `<RELAY_URL>/pair`, which is precisely the route above.
Set it only when pairing lives somewhere else — a dedicated host name, or a
proxy that exposes a different path. The value must be `ws://` or `wss://` or
the relay refuses to start.

Be careful with it. A stale value is quiet: the relay starts, the site works,
and pairing sends devices to whatever host name is in that string. The handshake
payload is a private key. Change it only alongside the route that serves it.

One precondition on that fallback: clients only try `<RELAY_URL>/pair` when the
relay advertises NIP-43, which it does when it has a stable
`BUZZ_RELAY_PRIVATE_KEY` **and** `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true` — both
defaults here. On an open relay (`BUZZ_REQUIRE_RELAY_MEMBERSHIP=false`) clients
pair against the main relay directly, which works because there is no membership
gate to fail; the sidecar is then unused.

### Bringing your own reverse proxy

The sidecar is published on **loopback only** (`BUZZ_PAIR_RELAY_HOST_IP`, default
`127.0.0.1`). It performs no authentication and enforces no membership, so
putting it on a public interface would expose an unauthenticated WebSocket
endpoint to the internet — don't. A proxy running on the same host reaches it at
`127.0.0.1:5000`; a proxy in another container can join `buzz-net` and use
`pairing-relay:5000` instead.

Whatever proxy you use, it must terminate TLS, route only `/pair`, pass the
WebSocket upgrade headers, and keep read timeouts tight — the sidecar caps each
connection at 120 seconds itself and delegates slowloris protection to the proxy
(see `crates/buzz-pair-relay/src/lib.rs`). If it serves `/pair` on the same host
name as the relay, you still need no `BUZZ_PAIRING_RELAY_URL`; set it only if the
public URL differs.

If something genuinely off-box must reach the sidecar without a proxy, set
`BUZZ_PAIR_RELAY_HOST_IP=0.0.0.0` — but note that pairing then runs unencrypted
over `ws://`, which iOS will refuse, and the endpoint is open to anyone who can
reach the port.

### Checking it works

```bash
curl -sS -o /dev/null -w "%{http_code}\n" "https://<your-domain>/pair"
```

A bare **400** is the healthy answer: the sidecar serves no NIP-11 document and
rejects any request that is not a WebSocket upgrade. **404** means no route or
no sidecar — pairing will fail. **401/403**, or a socket that closes while the
desktop waits for EOSE, means `/pair` is reaching the *main* relay, which is
refusing the not-yet-member device.

Only if you set `BUZZ_PAIRING_RELAY_URL`, check that it is advertised:

```bash
curl -sS -H 'Accept: application/nostr+json' "https://<your-domain>" \
  | grep -o '"pairing_relay_url":"[^"]*"'
```

Empty output is correct on a default install — the field is omitted entirely
when unset, and clients use the `/pair` fallback.

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
