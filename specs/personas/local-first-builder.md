# Persona: Local-first builder

## Context

The builder is evolving several identity, coherence, and agent projects at
once. Their laptop is the first trustworthy environment and may be offline.

## Goals

- Start a durable Buzz node with one command.
- Let humans and agents contribute under stable cryptographic identities.
- Inspect and carry the shared history between experiments.
- Discover which capabilities deserve production infrastructure through use.

## Frictions

- Docker Compose introduces several services before the experiment has proved
  what it needs.
- Project memory is fragmented across repositories and conversations.
- A disposable mock is easy to run but cannot become a stable node.
- A production-compatible deployment can obscure the essential event model.

## Success

After a restart, the builder can query the same signed events and continue an
agent experiment without reconstructing context or starting infrastructure.
