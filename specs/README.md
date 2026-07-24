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

## Index

- [Telos](TELOS.md)
- [Local-first builder persona](personas/local-first-builder.md)
- [Start a durable local Buzz journey](journeys/start-durable-local-buzz.md)
- [Run without hosted infrastructure story](stories/local-relay/run-without-hosted-infrastructure.md)
- [Local event log model](models/local-event-log/local-event-log.model.yaml)
- [Durable local event log behavior](features/local-relay/durable-local-event-log.feature)
- [HTTP bridge contract](contracts/openapi/local-relay.yaml)
- [WebSocket contract](contracts/asyncapi/local-relay.yaml)
