---
id: sovereign-node-operator
type: persona
---

# Persona: Sovereign node operator

## Role

An operator responsible for one independently owned Buzz node and its durable
event history. The operator may collaborate with other sovereign nodes through
a rendezvous custodian without transferring ownership or policy authority.

## Goals

- Share a deliberately selected event stream with a named counterparty.
- Keep application ownership, node identity, transport keys, and event authorship distinct.
- Understand whether a relationship is proposed, active, drifted, or revoked.
- Move referenced artifacts without creating a separate, broader access policy.
- Delegate observation and reporting without delegating configuration authority.

## Frustrations

- Trust is duplicated across environment variables, files, scripts, and remote deployments.
- A public key is ambiguous unless its role is explicit.
- A successful transport request can be mistaken for mutual agreement.
- Artifact access can accidentally become broader than the event stream that introduced it.
- Configuration drift is often discovered only after replication stops or leaks data.

## Context

- Tech comfort: high
- Usage frequency: weekly operations with occasional incident response
- Key devices: workstation and server administration surfaces

## Quotes

> "Show me exactly what I offered, who accepted it, which key moved the bytes,
> and whether the current heads still agree."
