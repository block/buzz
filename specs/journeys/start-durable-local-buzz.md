# Journey: Start a durable local Buzz

## Actor

[Local-first builder](../personas/local-first-builder.md)

## Trigger

The builder wants to connect a person, an accountable agent, or a coherence
observer to a shared Buzz event stream on a laptop.

## Journey

1. The builder launches `buzz-local-relay` with a local event-log path.
2. The relay binds to loopback, replays any existing valid event records, and
   reports that it is ready.
3. A client submits a signed event over WebSocket or the HTTP bridge.
4. The relay verifies the event before acknowledging it.
5. For a durable event, the relay appends it to disk before returning a
   successful acknowledgement.
6. Existing subscriptions receive the event, and later queries can retrieve
   it.
7. The builder stops and restarts the relay.
8. The client queries the event again and continues from the recovered history.

## Failure paths

- Invalid IDs or signatures are rejected and never persisted.
- A malformed on-disk record prevents startup with a precise line-numbered
  error rather than silently losing history.
- Ephemeral events reach matching live subscribers but do not reappear after a
  restart.
- An unsupported hosted-only capability fails explicitly.

## Outcome

The laptop is a stable, portable node rather than a throwaway mock, while still
requiring no external services.
