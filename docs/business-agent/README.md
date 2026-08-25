# Business Agent V5

V5 extends the dedicated, read-only business query path with a Production
Adapter contract and deterministic cross-domain anomaly analysis. It does not
change Nostr kinds, Relay behavior, Business Dock authentication, or the
Business System write surface.

```text
trusted Buzz event -> active identity binding -> short AgentReadDelegation
-> business-read-mcp -> Production Adapter -> Business Read API final authorization
-> structured minimal result -> Buzz answer -> biz:// link -> Business Dock
```

This repository still has no customer ERP credentials or authoritative
production endpoint. It now includes an independently runnable reference
Business Read API backed by a versioned, explicitly desensitized acceptance
snapshot. That path exercises service authentication, final delegation checks,
formal multi-dimensional scope intersection, V4 reads, V5 rules, Trace and
`biz://` links. It must not be described as real production data access.

Start with [architecture.md](architecture.md), then
[real-business-api-integration.md](real-business-api-integration.md),
[production-adapter.md](production-adapter.md), and [operations.md](operations.md).
The security decision is recorded in
[ADR 001](adr/001-read-only-agent-delegation.md).
