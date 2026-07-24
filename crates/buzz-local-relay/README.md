# buzz-local-relay

`buzz-local-relay` is the smallest durable Buzz node: one process, one
append-only event log, and no external services.

It is intended for laptop-scale identity, coherence, and agent-orchestration
experiments. It uses Buzz's real Nostr signature verification and filter
matching, so events retain their identity and can later move into a fuller
deployment.

## Run

From the repository root:

```bash
. ./bin/activate-hermit
just local-relay
```

Defaults:

- WebSocket: `ws://127.0.0.1:3000/`
- HTTP: `http://127.0.0.1:3000`
- Event log: `.buzz-local/events.ndjson`

Options:

```bash
just local-relay --bind 127.0.0.1:3100
just local-relay --data /absolute/path/events.ndjson
just local-relay --ephemeral
```

The listener is loopback-only by default. Binding another address exposes an
unauthenticated relay and should be an intentional local-network experiment,
not an internet deployment.

## Implemented surface

- NIP-01 WebSocket `EVENT`, `REQ`, `CLOSE`, `OK`, and `EOSE`
- Buzz HTTP bridge `POST /events`, `POST /query`, and `POST /count`
- `GET /health`
- Event ID and Schnorr signature verification
- NIP-01 regular, replaceable, parameterized replaceable, and ephemeral kinds
- Append-before-acknowledgement NDJSON persistence
- Strict verified replay on restart
- Live in-process subscription fan-out

HTTP auth headers sent by existing Buzz clients are tolerated but not enforced.
The local relay represents one trusted local community.

## Not implemented

- NIP-42/NIP-98 authorization and NIP-29 membership policy
- Postgres FTS or indexed search filters
- Redis or multi-node fan-out
- MinIO/S3 media
- audit chains, workflows, git hosting, huddles, and administrative APIs
- production hardening or availability guarantees

These are promotion boundaries, not silently simulated features. Use the
production `buzz-relay` when an experiment needs them.

## Inspect and move the log

Each line is a complete signed Nostr event. The relay replays all valid lines
and applies replacement semantics to rebuild the effective state:

```bash
wc -l .buzz-local/events.ndjson
head -n 1 .buzz-local/events.ndjson | jq
cp .buzz-local/events.ndjson /path/to/backup.ndjson
```

Ephemeral kinds (`20000..29999`) are delivered live and never written.

## Verify

```bash
cargo test -p buzz-local-relay
cargo clippy -p buzz-local-relay --all-targets -- -D warnings
```

The intent and acceptance behavior live under [`specs/`](../../specs/README.md).
