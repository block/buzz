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

With a local relay already running, verify the standalone contract with:

```bash
./scripts/smoke-buzz-node.sh ws://127.0.0.1:3000
```

Pair an owner with `buzz-node pair --qr <desktop-qr-uri>` before running the
node. Command payloads contain only safe workload data and credential
references; credential material remains node-local.
