# ADR 001: Read-only per-Turn Agent Delegation

Status: accepted for V4.

## Decision

Use a short-lived opaque AgentReadDelegation for one signed user event and one
Agent Turn. The Host injects it through MCP process environment; the model never
sees it. Expose only eight fixed read tools. The Business System still performs
final authorization, and raw results never enter long-term memory.

## Reasons

- Workbench access tokens identify an interactive OIDC client and have broader
  lifetime/audience than a tool call.
- Authentik tokens must not cross into the Agent/MCP trust boundary.
- Business Cookies and Business Sessions represent iframe interaction and may
  authorize writes; reuse would collapse browser and automation boundaries.
- Opaque random tokens can be hashed at rest, atomically budgeted and revoked
  without placing claims in model-visible context.
- Fixed tools make schema, scope, pagination, payload and audit behavior
  reviewable; SQL/HTTP proxies cannot be safely bounded by a prompt.
- MCP scope cannot know ERP roles or current data ownership, so final Business
  authorization is mandatory.
- Business results are time-varying and sensitive; remembering them creates
  stale facts and an uncontrolled secondary data store.

## Consequences

MCP environment is fixed per ACP Session, so dedicated Business Turns create a
fresh Session and revoke on every terminal path. The API integration must carry
service identity plus validated minimal context. Without the real API and auth
adapter, production remains disabled.
