# ADR 002: Real Business API boundary

Status: accepted; production integration pending.

## Decision

MCP calls fixed Business API routes and never reads a database directly. The
Business API authenticates the service, resolves authoritative user scope,
intersects filters, verifies the turn Delegation before and after work, and
returns minimal typed facts/Findings.

## Rationale

The API is the only component able to enforce current business permissions and
source semantics. Direct database/MCP access would bypass row authorization,
couple tools to schema and expose broad query surfaces. Stable ids and source
versions preserve reproducibility. Debug mock is rejected in release so test
data cannot silently become production data.

## Consequences

No production READY decision is possible until a named authoritative API,
workload identity and real permission adapter replace the desensitized
acceptance reference. The Agent remains read-only and may suggest review only.
