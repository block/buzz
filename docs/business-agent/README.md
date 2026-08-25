# Business Agent

Business Agent provides a dedicated query path plus six fixed draft-creation
commands. It does not change Nostr kinds, Relay behavior, or Business Dock
authentication. Confirm, approve, reverse, allocate, post and payment-execution
operations remain unavailable to the Agent.

```text
trusted Buzz event -> active identity binding -> short turn delegation
-> business-read-mcp -> Production Adapter -> Business API final authorization
-> Business Core user permission/scope check -> result -> Buzz answer -> biz:// link
```

This repository still has no customer ERP credentials or authoritative
production endpoint. It now includes an independently runnable reference
Business Read API backed by a versioned, explicitly desensitized acceptance
snapshot for anomaly acceptance. The live draft path exercises service authentication, final delegation checks,
formal multi-dimensional scope intersection, V4 reads, V5 rules, Trace and
`biz://` links, while Business Core remains authoritative for every write.

Start with [architecture.md](architecture.md), then
[real-business-api-integration.md](real-business-api-integration.md),
[production-adapter.md](production-adapter.md), and [operations.md](operations.md).
The security decision is recorded in
[ADR 001](adr/001-read-only-agent-delegation.md).
