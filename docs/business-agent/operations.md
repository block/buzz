# Operations

## Production

Run migrations through `0003_business_anomaly_audit.sql`, provision a minimum 32-byte
service credential from the server secret store, deploy the Gateway and
`business-read-mcp`, then configure the dedicated `buzz-acp` process:

```text
BUSINESS_AGENT_READ_ENABLED=true
BUZZ_ACP_AGENT_COMMAND=buzz-agent # recommended; other ACP runtimes are supported
# When using buzz-agent, configure exactly one model provider, for example:
BUZZ_AGENT_PROVIDER=openai
OPENAI_COMPAT_API_KEY=<secret-store reference>
OPENAI_COMPAT_MODEL=<approved model id>
BUSINESS_AUTH_GATEWAY_BASE_URL=https://business-auth.example.com/
BUSINESS_READ_API_BASE_URL=https://business-api.example.com/
BUSINESS_READ_MCP_COMMAND=/opt/buzz/bin/business-read-mcp
BUSINESS_READ_ADAPTER=production
BUSINESS_READ_SERVICE_AUTH_MODE=shared_secret
BUSINESS_READ_SERVICE_AUDIENCE=business-read-api
BUSINESS_READ_SERVICE_CREDENTIAL=<secret-store reference>
BUSINESS_AUTHORIZATION_ENABLED=true
BUSINESS_ANOMALY_ENABLED=true
BUSINESS_ANOMALY_RULESET_PATH=/etc/buzz/trade-risk-v1.0.json
BUSINESS_ANOMALY_DEFAULT_RULESET_VERSION=trade-risk-v1.0
BUSINESS_DATA_STALE_AFTER_MINUTES=1440
BUSINESS_ANOMALY_MAX_FINDINGS=100
BUSINESS_ANOMALY_MAX_PAYLOAD_BYTES=131072
BUSINESS_ANOMALY_SCHEDULE_ENABLED=false
AGENT_DELEGATION_TTL_SECONDS=300
AGENT_DELEGATION_MAX_CALLS=20
AGENT_TURN_TIMEOUT_SECONDS=120
BUSINESS_TOOL_TIMEOUT_SECONDS=10
BUSINESS_TOOL_MAX_PAYLOAD_BYTES=131072
BUSINESS_AGENT_RATE_LIMIT_PER_MINUTE=10
BUZZ_ACP_HEARTBEAT_INTERVAL=0
```

Do not set `BUZZ_ACP_MCP_COMMAND`; dedicated mode drops ordinary MCP servers
anyway. Use `examples/business-query-agent` for ordinary lookups and the
separate `examples/business-anomaly-agent` Persona for broad analysis. Do not
give either runtime Shell, filesystem, browser, SQL, generic HTTP, or memory
tools. Codex, Goose, Claude and other ACP runtimes are supported. Dedicated mode
still removes ordinary configured MCP servers and injects only the per-turn
Business MCP server, but a general-purpose runtime may independently expose its
own built-in tools; operators must evaluate and restrict those capabilities.

For `codex-acp`, select a model that exposes session-scoped MCP tools as direct
tool calls. The verified local acceptance path uses `gpt-5.5`. Do not use
`gpt-5.6-sol` for this path yet: its code-mode tool broker snapshots namespaces
before the per-turn Business MCP server is attached, so the server can report
`ready` while its tools remain unavailable to the model. This is a runtime/model
compatibility constraint, not a reason to switch the dedicated host back to a
hard-coded `buzz-agent` runtime.

With `BUSINESS_AGENT_READ_ENABLED=false` or missing real integration, ordinary
Buzz agents continue unchanged. Enabling with a missing credential/API URL
fails startup instead of using fixtures.

When deploying with `buzz-agent`, run `just business-agent-runtime-acceptance`. The probe uses
the real `buzz-agent -> session/new -> business-read-mcp` path and a loopback
model stub, then asserts that the model sees exactly the 28 fixed business read
tools and no general-purpose tool. It does not call a model, consume a real
Delegation, or read business data.

## Debug fixture

Debug builds may use `BUSINESS_READ_ADAPTER=mock` plus the exact acknowledgement
from `.env.example`. Every result stays `partial`, names the mock source and
warns that production is disabled.

Monitor denial/rate/timeout counts and audit continuity. Cleanup runs with the
Gateway sweep. Revoking a binding also revokes associated Delegations.
