# Buzz execution node

`buzz-node` is the runtime-neutral relay client for paired execution nodes. It
currently implements the encrypted deploy slice with `FakeWorkloadRuntime`.

Run it with:

```bash
BUZZ_RELAY_URL=ws://localhost:3000 cargo run -p buzz-node -- run
```

Configuration:

- `BUZZ_RELAY_URL` — required relay WebSocket URL.
- `BUZZ_AUTH_TAG` — optional JSON-encoded NIP-OA authentication tag.
- `BUZZ_NODE_DATA_DIR` — optional durable data directory; defaults to
  `.buzz-node`. It stores the node identity, paired owners, and command
  idempotency state.
- `BUZZ_NODE_NAME` — optional display name used in node announcements.

Pair an owner with `buzz-node pair --qr <desktop-qr-uri>` before running the
node. Command payloads contain only safe workload data and credential
references; credential material remains node-local.
