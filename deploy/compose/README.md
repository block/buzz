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
  `!reset` tag to remove the direct gateway port when Caddy terminates HTTPS.
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

## Gateway and device pairing

The Compose bundle runs device pairing through Buzz's dedicated, stateless
`buzz-pair-relay`. Nginx is the single gateway in front of both services:

```text
public :3000 -> gateway:3000 -> /pair, /pair/* -> pairing-relay:5000
                              -> everything else -> relay:3000
```

Only the gateway publishes a host port. The main relay and pairing relay remain
on the private Compose network, and the pairing relay has no persistent volume,
database, or identity. Its sessions are ephemeral by design.

NIP-43 advertises relay membership support; it does not advertise a pairing
endpoint. Keep the explicit public URL in the environment file so the main
relay publishes it as NIP-11 `pairing_relay_url`:

```dotenv
BUZZ_PAIRING_RELAY_URL=wss://buzz.example.com/pair
```

The URL must use `ws://` or `wss://` and include a host. The sample same-host
`/pair` path is routed by Nginx. A separate pairing hostname can also target the
gateway when its advertised URL ends in `/pair`; keep external proxies pointed
at the gateway rather than bypassing it.

### Cloudflare Tunnel

Cloudflare Tunnel can target Nginx directly; Caddy is not required. For a
host-installed `cloudflared`, bind the gateway to loopback and point the tunnel
at that port:

```dotenv
BUZZ_HTTP_BIND_IP=127.0.0.1
BUZZ_HTTP_PORT=3000
```

```yaml
ingress:
  - hostname: buzz.example.com
    service: http://127.0.0.1:3000
  - service: http_status:404
```

Keep `RELAY_URL=wss://buzz.example.com` and
`BUZZ_PAIRING_RELAY_URL=wss://buzz.example.com/pair`; Cloudflare terminates TLS
while Nginx routes both WebSocket upgrades. If `cloudflared` runs as a container
attached to `buzz-net`, target `http://gateway:3000` instead. Do not target
`relay:3000`, because that bypasses `/pair` routing.

The optional Caddy overlay follows the same architecture: it removes the
gateway's host port and proxies all traffic to `gateway:3000`, leaving Nginx as
the only component that decides between the main and pairing relays.

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
