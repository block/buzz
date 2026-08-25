# AgentReadDelegation

`agent_read_delegations` binds one opaque credential to one real Buzz event,
Enterprise user/binding, IAM decision, Agent principal/id, Turn id, channel,
audience, effective capability/data-scope list, Trace id, expiry, and call budget.

- entropy: 32 cryptographically random bytes;
- encoding: Base64URL without padding (43 characters);
- storage: SHA-256 hash only;
- audience: exactly `business-read-mcp`;
- default/max TTL: 300/900 seconds;
- default call limit: 20;
- scopes: a Business IAM-approved subset of the fixed read capability catalog;
- issuance idempotency: unique `(source_buzz_event_id, agent_id)`;
- state: `active`, `expired`, `revoked`, or `exhausted`.

The issue endpoint verifies the Nostr id/signature, author, `h` channel tag,
active binding, active user, requested read scopes, and per-user rate limit,
then evaluates Business IAM. Unregistered Agents and empty effective permission
intersections are rejected.

The token is never a tool parameter or prompt field. `buzz-acp` passes it in
the MCP process environment; `business-read-mcp` sends it only in the gateway
Authorization header. The Business API receives the validated context and
Delegation id, not the opaque token.

Each consume uses a PostgreSQL `UPDATE ... FROM ... RETURNING` that checks
status, expiry, audience, Agent/Turn ids, binding/user state, scope and budget,
then increments `used_calls`. The call that reaches the maximum succeeds and
marks the record exhausted; later calls fail.

Normal turn completion synchronously revokes the delegation. IAM principal,
direct-grant, role-binding, role-permission, role, or permission changes revoke
related active delegations transactionally; TTL remains an abnormal-path limit.
