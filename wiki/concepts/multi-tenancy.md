# Multi-Tenancy

Buzz supports multi-tenant deployments where multiple communities share the same Postgres/Redis/MinIO infrastructure while maintaining strict isolation.

## Isolation Model

- **Community** is the tenant boundary — one workspace, one URL, one isolated world
- All data is scoped by `community_id` at the database level
- The relay extracts the community from the Host header on connection
- `TenantContext` is established before any handler runs

## Formal Verification

The multi-tenant relay behavior has been mechanized in **TLA+** for correctness verification. Authorization properties are verified in **Tamarin** (a security protocol verifier). These formal specs live in `docs/formal/`.

**Related:**
- [Community](../entities/community)
- [Architecture](architecture)
- [buzz-conformance](../components/buzz-conformance)
