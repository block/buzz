# Buzz Specifications

This directory is the durable design memory for Buzz work. It connects the
project's purpose to concrete behavior before implementation:

```text
TELOS.md
  -> personas and journeys
  -> stories
  -> domain models
  -> executable behavior
  -> transport contracts
```

The first vertical slice is the local relay: a single-process Buzz node that
preserves signed events on a laptop without Postgres, Redis, MinIO, or Docker.
The portable relay boundary separates that observable behavior from its
runtime adapters so the same signed-event vocabulary can later run on a
cloud-native node or the hosted relay.

## Index

- [Telos](TELOS.md)
- [Local-first builder persona](personas/local-first-builder.md)
- [Start a durable local Buzz journey](journeys/start-durable-local-buzz.md)
- [Run without hosted infrastructure story](stories/local-relay/run-without-hosted-infrastructure.md)
- [Local event log model](models/local-event-log/local-event-log.model.yaml)
- [Durable local event log behavior](features/local-relay/durable-local-event-log.feature)
- [HTTP bridge contract](contracts/openapi/local-relay.yaml)
- [WebSocket contract](contracts/asyncapi/local-relay.yaml)
- [Portable relay boundary](architecture/portable-relay-boundary.md)
- [Portable relay model](models/portable-relay/portable-relay-boundary.model.yaml)
- [Adapter conformance behavior](features/portable-relay/adapter-conformance.feature)
- [Core conformance vector](fixtures/portable-relay/core-v0.1.json)
- [Portable relay capability](capabilities/portable-relay.capability.yaml)
