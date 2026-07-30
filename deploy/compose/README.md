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

## Server-hosted agents (buzz-spawner)

By default every Buzz agent is spawned by the desktop app, so agents stop when
the laptop sleeps and cannot be created from mobile or web at all. The optional
`spawner` service fixes that: it watches the relay for agent specs you publish
and reconciles them into one isolated container per agent.

```bash
cd deploy/compose
$EDITOR .env                              # set BUZZ_SPAWNER_NSEC + ANTHROPIC_API_KEY
BUZZ_COMPOSE_SPAWNER=true ./run.sh start
```

### ⚠️ Trust boundary: the Docker socket

**`compose.spawner.yml` mounts `/var/run/docker.sock` into the spawner
container. That is root-equivalent access to the host** — anything that can talk
to that socket can start a privileged container and take over the machine. This
is why the service is opt-in rather than on by default.

The spawner needs it because creating containers is its entire job. Agents run
arbitrary shell and file-edit tools through `buzz-dev-mcp`, so running several as
bare subprocesses of one daemon would let any agent read every other agent's
workspace and secret key. Per-agent containers are the isolation.

If you want to reduce the blast radius, point the spawner at a **rootless
Docker or Podman socket** instead of the system one:

```bash
BUZZ_SPAWNER_DOCKER_SOCKET=/run/user/1000/docker.sock \
  BUZZ_COMPOSE_SPAWNER=true ./run.sh start
```

### Notes

- `BUZZ_ALLOW_NIP_OA_AUTH=true` is required. Spawned agents authenticate by
  owner attestation; without it none of them can connect.
- Agent containers are deliberately **not** attached to `buzz-net`. They reach
  the relay at `BUZZ_SPAWNER_AGENT_RELAY_URL` (defaulting to `RELAY_URL`) the
  same way any external client does, so an agent's shell cannot reach Postgres,
  Redis, or MinIO directly.
- **Both spawner URLs must use the relay's public host.** The relay derives the
  community from the HTTP `Host` header, so `ws://relay:3000` is a *different
  tenant* from `wss://your.domain`. Pointing the spawner at the internal compose
  name puts it in a community your owner account is not in — status events never
  reach you — or fails the connection outright with a 404, because internal
  names have no community row.
- `BUZZ_SPAWNER_NSEC` must stay stable. Clients address specs to this pubkey,
  and it is the pubkey an owner approves when signing an attestation — rotating
  it strands every existing agent.
- `BUZZ_SPAWNER_AGENT_ENV` takes variable **names**, not `KEY=VALUE` pairs. The
  values are read from the spawner's own environment, so a rotated credential is
  picked up by restarting the spawner rather than by editing agent config.
- Back up the `buzz-spawner-data` volume. It holds each agent's secret key,
  minted on this host and never transmitted anywhere. Losing it orphans every
  running agent: their pubkeys remain in your channels and relay membership, but
  nothing can sign as them again.
- Creating an agent needs a one-time approval from the owner's client. The
  spawner mints the key, then asks the owner to sign a NIP-OA attestation for it
  — it cannot self-attest. Until that is approved the agent's status event
  reports `pending_attestation`.

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
