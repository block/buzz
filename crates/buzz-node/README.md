# Buzz execution node

`buzz-node` is a standalone, runtime-neutral relay client for paired execution
nodes. It can run under any process supervisor; the supervisor only needs to
provide environment variables and a writable persistent data directory.

Run it with:

```bash
BUZZ_RELAY_URL=ws://localhost:3000 cargo run -p buzz-node -- run
```

Configuration:

- `BUZZ_RELAY_URL` — required relay WebSocket URL.
- `BUZZ_AUTH_TAG` — optional JSON-encoded NIP-OA authentication tag.
- `BUZZ_NODE_DATA_DIR` — optional durable data directory; defaults to
  `.buzz-node`. It must survive restarts and stores the node identity, paired
  owners, workload/credential state, and command idempotency journal. Identity
  and state files are created with owner-only permissions on Unix.
- `BUZZ_NODE_NAME` — optional display name used in node announcements.
- `BUZZ_NODE_HEALTH_ADDR` — optional local HTTP bind address for probes;
  defaults to `127.0.0.1:8081`.
- `BUZZ_NODE_MAX_CONCURRENT_COMMANDS` — optional positive limit for concurrent
  encrypted command processing; defaults to `8`.

The data directory contains `identity.nsec` (the node identity), `owners.json`
(paired owner public keys), and `execution-state.json` (workloads, encrypted
credential state, sequences, and the idempotency journal). Back up and restore
the directory as one unit; restoring only part of it can detach workloads from
their identity or replay protection. Keep backups access-controlled because
the identity file can authenticate the node.

Operational endpoints:

- `GET /health` (or `/healthz`) returns `200` while the process is alive.
- `GET /ready` (or `/readiness`) returns `200` only after relay
  authentication, announcement, and command subscription succeed; it returns
  `503` while connecting or reconnecting.

The service reconnects with exponential backoff up to 30 seconds, drains
in-flight command work before a connection attempt is retried, logs lifecycle
and relay failures through `tracing`, and exits cleanly on Ctrl-C or SIGTERM.

While connected, the node publishes an ephemeral kind:20001 presence
heartbeat every 60 seconds over its WebSocket connection — the same presence
mechanism members and managed agents use. The relay keeps that presence in
Redis with a 180-second TTL and clears it on clean disconnect, so Desktop
derives node availability from live presence rather than announcement
freshness: a node with a Ready announcement and live presence shows as
connected, a Ready node without presence shows as unavailable within roughly
three minutes of an unclean drop, and a graceful shutdown (which publishes an
`offline` presence before disconnecting) flips immediately. The kind:30630
announcement itself stays slow-moving capability and pairing state; it is
only republished when owners or workload status change.

With a local relay already running, verify the standalone contract with:

```bash
./scripts/smoke-buzz-node.sh ws://localhost:3000
```

The relay URL must use the same host as the Desktop community it should join;
relay communities are scoped by host.

Pair an owner with `buzz-node pair --qr <desktop-qr-uri>`. Pairing runs as a
separate process and persists the owner attestation to `owners.json` in the
shared data directory; a node started afterwards announces it immediately. An
already-running `buzz-node run` process re-reads `owners.json` every few
seconds and, when the paired owners change, republishes its replaceable
announcement with the updated attestations and starts authorizing commands
from the new owner — no restart required. Command payloads contain only safe
workload data and credential references; credential material remains
node-local.
