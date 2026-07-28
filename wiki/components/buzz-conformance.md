# buzz-conformance

Multi-tenant conformance tests. Verifies that the relay behaves correctly under multi-tenant deployment, including:

- Tenant isolation (community A cannot see community B's data)
- Auth across tenants
- Correct scoping of events, channels, and memberships by community_id
- Rate limits per tenant

**Related:**
- [MultiTenancy](../concepts/multi-tenancy)
- [buzz-test-client](buzz-test-client)
- [Community](../entities/community)
