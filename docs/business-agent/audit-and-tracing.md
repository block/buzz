# Audit and tracing

One UUID Trace id starts at Host authorization and is reused by Delegation,
MCP, Business API and MCP audit. Business resource evidence remains within the
same result envelope.

Events implemented by the Gateway/MCP path include:

- `AGENT_TURN_AUTHORIZED`, `AGENT_TURN_REJECTED`;
- `AGENT_DELEGATION_ISSUED`, `EXPIRED`, `REVOKED`, `EXHAUSTED`;
- `BUSINESS_MCP_TOOL_CALLED`, `SUCCEEDED`, `FAILED`;
- `BUSINESS_READ_AUTHORIZATION_DENIED`, `BUSINESS_READ_PARTIAL_RESULT`;
- `BUSINESS_ANOMALY_RUN_STARTED`, `COMPLETED`, `PARTIAL`, `FAILED`;
- `BUSINESS_ANOMALY_FINDING_CREATED` as one aggregate event per returned run;
- `BUSINESS_ANOMALY_AUTHORIZATION_DENIED` when anomaly scope consumption fails;
- `BUSINESS_ANOMALY_DATA_QUALITY_BLOCKED` when required profit inputs block a conclusion;
- `BUSINESS_ANOMALY_RESPONSE_EMITTED` after a Finding-bearing answer is accepted by Buzz;
- `AGENT_BUSINESS_RESPONSE_EMITTED` and `FAILED`: no publish tool is exposed to
  the model. The ACP host captures only the final ACP assistant message (bounded
  to 128 KiB), signs it with the managed Agent identity, submits it as a reply to
  the trusted source event, then retains only accepted/event id, duration and
  aggregate Finding/ResourceRef counts for audit before revoking the turn
  Delegation. Answer text and business fields are not written to the audit.
  Builder/capture/unit coverage is present; a live production Buzz publication
  remains part of the production cutover gate.

Columns include user/binding, shortened pubkey, Agent/Turn/source ids,
Delegation, tool, result/finding/resource counts, ruleset/run id, duration,
Trace id, result and reason. The audit
table remains append-only. Token, cookie, access token, raw query result and
customer details are forbidden.
