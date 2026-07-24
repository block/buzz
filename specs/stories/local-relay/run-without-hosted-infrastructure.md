# Story: Run Buzz without hosted infrastructure

As a local-first builder, I want a single-process relay with portable durable
storage so that humans and agents can develop shared memory before operating a
hosted stack.

## Acceptance criteria

- The process starts without Docker, Postgres, Redis, or MinIO.
- The default listener is loopback-only.
- The relay verifies the ID and Schnorr signature of every submitted event.
- A durable event is appended before a successful acknowledgement is sent.
- Valid records are recovered after process restart.
- Duplicate event IDs are idempotent.
- NIP-01 replaceable and parameterized replaceable events expose only the
  current effective event in queries.
- NIP-01 ephemeral kinds are delivered live but not written to disk.
- Clients can use `EVENT`, `REQ`, `CLOSE`, `EOSE`, and `OK` WebSocket frames.
- Clients can use `POST /events`, `POST /query`, `POST /count`, and
  `GET /health`.
- Unsupported production capabilities are not simulated as if they were safe
  or complete.

## Out of scope

NIP-29 authorization policy, NIP-42 authentication, media, search indexes,
workflow execution, audit chains, multiple relay nodes, and production
availability guarantees.
