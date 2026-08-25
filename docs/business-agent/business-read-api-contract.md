# Business Read API contract

This is the production integration contract. `services/business-read-api`
implements a desensitized acceptance reference for the contract; it is not an
ERP connector.

`business-read-mcp` sends `POST /v1/read/{fixed-tool-name}` for reads and
`POST /v1/write/{fixed-tool-name}` for one of six draft creates. JSON must match
the strict tool schema. The request carries:

- `x-business-service-credential` (service identity);
- `x-business-service-audience: business-read-api`;
- `x-enterprise-user-id`, `x-identity-binding-id`;
- `x-agent-delegation-id`, `x-agent-id`, `x-agent-turn-id`;
- `x-source-buzz-event-id`, `x-source-channel-id`, `x-trace-id`.

It never carries the opaque Delegation token, Workbench/Authentik token,
Business Cookie, Embed code, or Business Session token.

The API authenticates the service, resolves the Enterprise user's current
permissions, intersects requested and allowed data scope, and returns the
schema-version-1 envelope documented in `tool-contracts.md`. It generates all
resource links and evidence. It returns exact-id miss/denial identically and
must not expose existence through timing or error detail where practical.

The fourteen Business Core reads, eight anomaly reads, six action-lifecycle
reads and six draft creates are fixed allowlists. Missing read fields use `partial` plus warnings;
invented data is forbidden. Response max is 100 rows/128 KiB and the server
must use decimal strings for monetary amounts and quantities.

Draft creation requires complete input. The API verifies the delegation again,
derives the idempotency key, forwards only to `/v1/agent-drafts/*`, and accepts
only a `draft` result with the same Trace id. Business Core then repeats the
current user's domain permission and data-scope checks.

The V5 response adds `runId`, `ruleSetVersion`, `dataAsOf`, effective scope
hash, totals by currency, paginated Findings, severity, confidence, threshold,
versioned Evidence and server-generated ResourceRefs. The API re-verifies the
binding/user/delegation immediately before returning so a revoked result is
discarded.
