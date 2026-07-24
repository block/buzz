# Telos

## North star

Make the smallest durable, portable Buzz environment in which one person and
one accountable agent can build shared memory without operating hosted
infrastructure.

The first stable node is the relay on a laptop. It gives identity, coherence,
and agent experiments one dependable place to meet while the surrounding
cognitive architecture continues to evolve.

## Why this exists

Ideas about identity, DID Beacons, coherence observation, reporting, and agent
orchestration currently live across repositories and conversations. That
plurality is healthy, but it needs a stable substrate: an event history that is
easy to run, inspect, move, and promote.

Buzz should help these projects become more coherent without forcing them into
one application or one ontology. Signed events provide the shared language;
the relay provides continuity.

## Principles

1. **Local before hosted.** A meaningful experiment starts with one process and
   one data path on a laptop.
2. **Identity before automation.** Every durable contribution has a
   cryptographic author; agents remain attributable participants.
3. **Events before integrations.** Capabilities meet through signed Nostr
   events rather than repository-specific coupling.
4. **Durability before scale.** Acknowledged durable events survive restart.
5. **Portable memory.** The local event log is readable, copyable, and suitable
   for later import into a hosted relay.
6. **Honest compatibility.** The local node implements a named protocol subset
   and fails clearly outside it.
7. **Progressive infrastructure.** Postgres, Redis, and object storage enter
   only when an experiment actually needs their scaling or media properties.

## First horizon

The local relay accepts and verifies signed events, persists non-ephemeral
events to an append-only file, restores its effective state after restart, and
serves the Nostr WebSocket and Buzz HTTP bridge operations used by lightweight
CLI and agent experiments.

It does not initially promise hosted-relay parity, multi-node fan-out, full
authorization policy, full-text search, media storage, workflows, or production
operations.

## Promotion path

An experiment should be able to begin against the local relay and later move to
the hosted architecture without changing its event vocabulary or identity.
Promotion means changing operational adapters and policy, not rewriting the
meaning of the work.
