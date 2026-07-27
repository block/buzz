# Self-hosting a full Buzz backend (relay + spawner)

This guide stands up the entire server side of Buzz from scratch: your own
relay, the optional `buzz-spawner` for server-hosted agents, and the desktop
app pointed at both. It also collects the tricks and failure modes we hit
while doing exactly this, so you don't have to rediscover them. The single
most important rule: **every client — desktop, CLI, spawner, and agent
containers — must reach the relay via the same public hostname**, because the
relay derives the community (tenant) from the HTTP `Host` header.

It covers the **single-node VPS** path — the production Docker Compose bundle
in [`deploy/compose/`](../deploy/compose/README.md). (For hacking on Buzz
itself from source, see [CONTRIBUTING.md](../CONTRIBUTING.md).)

---

## 1. The relay

### VPS (Docker Compose)

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env              # replace every CHANGE_ME value
./run.sh start            # or BUZZ_COMPOSE_TLS=true ./run.sh start for Let's Encrypt
curl -fsS "http://127.0.0.1:3000/_liveness"
```

See [`deploy/compose/README.md`](../deploy/compose/README.md) for the full
production notes (image pinning, secrets, backups).

### Relay tricks we learned

- **The `Host` header IS the community boundary.** The relay derives the tenant
  from the HTTP host it was reached at. `ws://localhost:3000`,
  `ws://relay:3000` (compose-internal), and `wss://your.domain` are *three
  different communities*. Every client — desktop, CLI, spawner, agents — must
  use the same public host, or they end up in communities that can't see each
  other (or hit a 404 because the internal name has no community row).
  The failure is **silent**: a spawner on a different host than the desktop
  never sees the desktop's agent specs or deletions — deleting an agent shows
  "Removing…" but nothing happens, and a stale status card (e.g. FAILED with an
  old error) sticks around forever because no spawner in that community is
  alive to replace or tombstone it. Pick one hostname and use it everywhere.
- **Keep `BUZZ_RELAY_PRIVATE_KEY` stable.** It anchors relay identity across
  restarts; rotating it is a new relay as far as clients are concerned.
- **Migrations:** local dev auto-applies from `migrations/`; the compose bundle
  requires `BUZZ_AUTO_MIGRATE=true` (or a manual `buzz-admin migrate`) on a
  fresh database.
- **Relay queries must specify `kinds`.** An open-ended filter trips the
  relay's p-gate and returns 403 — this bites scripts and `buzz messages
  search` (pass `--kinds 9,45001,45003`).
- **Ephemeral kinds don't go through `POST /events`.** The handshake kinds
  (24200/24201) are routed only by the WebSocket handler; the HTTP ingest path
  rejects them with `restricted: unknown event kind`.

---

## 2. The spawner (server-hosted agents)

By default agents are spawned by the desktop app and die when the laptop
sleeps. `buzz-spawner` watches the relay for agent specs (kind:30178) and
reconciles them into one Docker container per agent.

### VPS (Docker Compose)

```bash
cd deploy/compose
$EDITOR .env                              # set BUZZ_SPAWNER_NSEC
BUZZ_COMPOSE_SPAWNER=true ./run.sh start
```

> ⚠️ **Trust boundary:** the compose service mounts the Docker socket, which is
> root-equivalent on the host. That's the price of per-agent container
> isolation. Consider a rootless Docker/Podman socket
> (`BUZZ_SPAWNER_DOCKER_SOCKET=/run/user/1000/docker.sock`). Details in
> [`deploy/compose/README.md`](../deploy/compose/README.md).

### Per-owner Claude credentials (how agents get API access)

Server agents run on **each owner's own token**, not a host-global key:

1. Each user opens **Settings → Agents → Server agents** in the desktop app,
   finds the connected spawner, and pastes their credential into **"Your
   Claude credential"** — either a Claude Code OAuth token (`sk-ant-oat…`, from
   `claude setup-token`) or an Anthropic API key (`sk-ant-api…`).
2. The token travels NIP-44-encrypted over the ephemeral attestation channel
   (kind:24201), is stored only in the spawner's `credentials.json` (mode
   0600), and is injected into that owner's agent containers as
   `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` (classified by prefix).
3. Saving a token restarts that owner's agents; clearing it stops them.
   **There is no fallback:** an agent whose owner has provisioned no token is
   held stopped with a **"Needs credential"** badge (`needs_credential` on its
   kind:30179 status).

Host-global `ANTHROPIC_API_KEY` / `CLAUDE_CODE_OAUTH_TOKEN` passthrough via
`BUZZ_SPAWNER_AGENT_ENV` no longer reaches agent containers — keep that
variable for non-Anthropic env only.

### Spawner tricks we learned

- **Point the spawner at the relay's *public* host** (both
  `BUZZ_SPAWNER_RELAY_URL` and `BUZZ_SPAWNER_AGENT_RELAY_URL`). Internal
  compose names are a different tenant — see the Host-header trick above.
  `BUZZ_SPAWNER_AGENT_RELAY_URL` is the address handed to agent *containers*:
  it defaults to `BUZZ_SPAWNER_RELAY_URL`, and must be a host the containers
  can reach — never `localhost`, which inside a container is the container
  itself (agents crash-loop with `Connection refused (os error 111)`). Avoid
  `host.docker.internal` too: it works network-wise but is a different Host
  header, i.e. a different community than the one your desktop is in.
- **mDNS `.local` hostnames resolve to `127.0.0.1` first inside containers.**
  Docker's DNS can return both `127.0.0.1` and the LAN IP for `<host>.local`;
  clients that try addresses in order fall through to the LAN IP and work, but
  expect confusing intermittent `Connection refused` noise. A plain DNS name
  or LAN IP is more predictable.
- **`BUZZ_ALLOW_NIP_OA_AUTH=true` on the relay is required.** Spawned agents
  authenticate by owner attestation; without it none of them can connect.
- **`BUZZ_SPAWNER_NSEC` must stay stable.** Specs are addressed to this pubkey
  and it's what owners approve; rotating it strands every existing agent.
- **The agent harness refuses to start without a provider.** If a spec/persona
  doesn't name one, the container exits deep inside the harness complaining
  about `BUZZ_AGENT_PROVIDER` — set `BUZZ_SPAWNER_DEFAULT_PROVIDER` /
  `BUZZ_SPAWNER_DEFAULT_MODEL` as the host fallback.
- **The agent runtime binary is spawner config, not passthrough.** Which
  binary runs (`buzz-agent` vs `claude-agent-acp`) is a code-execution surface,
  so it comes only from `BUZZ_SPAWNER_AGENT_COMMAND` — it cannot be set from a
  spec or from `BUZZ_SPAWNER_AGENT_ENV` (both are filtered against reserved
  keys). Use `claude-agent-acp` to run agents on Claude subscription OAuth
  tokens.
- **`BUZZ_SPAWNER_AGENT_ENV` takes variable *names*, not `KEY=VALUE`.** Values
  are read from the spawner's own environment at startup; naming an unset
  variable is a hard startup error (better than an agent failing obscurely
  later).
- **The default agent image lives on GHCR and may need auth.** A
  `denied`/`unauthorized` pull of `ghcr.io/block/buzz-acp:main` means the
  Docker daemon isn't logged in to ghcr.io (`docker login ghcr.io` with a PAT
  that has `read:packages`), or the image/tag isn't public. Build your own
  from the repo root with `docker build -f Dockerfile.acp -t buzz-acp:local .`
  and point `BUZZ_SPAWNER_AGENT_IMAGE=buzz-acp:local` at it.
- **Creating an agent needs a one-time owner approval.** The spawner mints the
  agent's key, then asks the owner's client to sign a NIP-OA attestation — it
  cannot self-attest. Until approved, status reports `pending_attestation`; if
  you miss the prompt (it's ephemeral), the spawner re-asks after a timeout.
- **Back up the state dir** (`agents.json` + `credentials.json`, or the
  `buzz-spawner-data` volume in compose). `agents.json` holds each agent's
  secret key, minted on this host and never transmitted — losing it orphans
  every agent permanently.

---

## 3. The desktop app

Install a desktop build (see [RELEASING.md](../RELEASING.md)), then point it at
your relay: set `BUZZ_RELAY_URL` before launching, or switch the relay from
inside the app. Use the same public host as the spawner and agents — the Host
header is the tenant (see § 1).

Wiring it all together, in order:

1. Start the relay, then the spawner, then the app.
2. In **Settings → Agents → Server agents**, your spawner appears under
   "Spawners on this relay" (it announces itself, kind:10180) — **Connect**.
3. Add your Claude credential to the spawner's credential card.
4. **Deploy agent** → pick a persona → approve the attestation prompt when it
   appears. The agent goes `pending_attestation → starting → running`.
5. Mention the agent in a channel and it answers — even with your laptop
   closed, because it now lives on the server.
