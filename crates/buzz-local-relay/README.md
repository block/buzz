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
just local-relay --require-auth
```

The listener is loopback-only by default. Binding another address exposes an
unauthenticated relay unless `--require-auth` is set and should be an
intentional local-network experiment, not an internet deployment.

## Implemented surface

- NIP-01 WebSocket `EVENT`, `REQ`, `CLOSE`, `OK`, and `EOSE`
- Buzz HTTP bridge `POST /events`, `POST /query`, and `POST /count`
- `GET /health`
- Event ID and Schnorr signature verification
- NIP-01 regular, replaceable, parameterized replaceable, and ephemeral kinds
- Append-before-acknowledgement NDJSON persistence
- Strict verified replay on restart
- Live in-process subscription fan-out
- Portable durable-history replication source and policy-gated sink ports
- Peer-bound HTTP replication sink (`POST /replication`, requires
  `--require-auth` plus a `--peer-trust` binding for the source stream)
- Optional laptop identity adapter (`--require-auth`):
  - NIP-42 challenge authentication for WebSocket connections
  - payload-bound NIP-98 authentication for HTTP writes, queries, and counts
  - one-use proof replay protection, persisted beside a durable event log
    (`<data>.auth-proofs`) so restarts inside the freshness window still
    reject replayed evidence
  - direct writes bound to the authenticated event author, with the narrow
    NIP-59 gift-wrap exception
  - equivalent result-level disclosure policy for query, count, historical
    subscription, and live delivery
  - configured replication-source bindings to stable node identifiers and
    active Nostr verification keys

Without `--require-auth`, the relay represents one trusted local community and
preserves the original lightweight iteration path. The secured mode introduces
no Postgres, Redis, MinIO, external identity provider, or DID resolver.

## Not implemented

- NIP-29 membership policy, delegated append authority, or public-read grants
- dynamic DID resolution
- Postgres FTS or indexed search filters
- Redis or multi-node fan-out
- automatic peer discovery or a continuous relay-to-relay transport
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
cargo test -p buzz-local-relay --test portable_conformance
cargo test -p buzz-local-relay --test replication_port
cargo test -p buzz-local-relay --test identity_adapter
cargo clippy -p buzz-local-relay --all-targets -- -D warnings
```

The intent and acceptance behavior live under [`specs/`](../../specs/README.md).
This crate is the laptop reference adapter for the
[`portable-relay-core-v0.1`](../../specs/architecture/portable-relay-boundary.md)
behavioral boundary. Its first protocol-level conformance test consumes the
shared signed-event vector without querying relay internals.
