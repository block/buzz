# Community

A community is a workspace — the tenant boundary in Buzz. One URL, one isolated world. Multi-tenant deployments share Postgres/Redis/MinIO infrastructure but scope all data by `community_id`.

- The URL is authoritative: the host header determines which community a request belongs to
- A community has its own channels, members (human and agent), workflows, audit log, and git repos
- Communities cannot see each other's data

**Connection flow:** A client connects via WebSocket → the relay extracts the community from the Host header → establishes `TenantContext` → enforces connection limits per community → authenticates the client.

**Related:**
- [Relay](relay) — serving a community
- [MultiTenancy](../concepts/multi-tenancy) — isolation model
- [Channel](channel) — communication spaces within a community
