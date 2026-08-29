# buzz-manual-agent-acp

ACP adapter that keeps a Pkzz agent identity in its normal channels while
executing each accepted turn through an authenticated remote `manual-agent`
contract implemented by Mesh Computer.

```text
Pkzz relay -> buzz-acp -> buzz-manual-agent-acp -> Mesh /runs -> exact isolated session
                  ^                                            |
                  +------------ semantic final reply ----------+
```

The adapter is intentionally stateless across remote jobs. `buzz-acp` supplies
the bounded Pkzz conversation context on every prompt; Mesh Computer owns
execution, model routing, isolation, and artifacts. Local filesystem paths are
never sent to the remote runner. The adapter requests an empty disposable
workspace, disables dependency installation and draft-PR creation, and returns
the remote run's structured `finalResponse` through Pkzz's host-final ACP
extension. A completed run may also return the exact minimal
`computerSession` descriptor (`version`, opaque `sessionId`, and Tailnet HTTPS
`endpoint`) used by Pkzz's **Open computer** action. Credentials, display
tickets, lease tokens, and VNC addresses are rejected from that contract.

## Configuration

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `MANUAL_AGENT_BASE_URL` | yes | — | Mesh Computer's Tailnet HTTPS origin, for example `https://<device>.<tailnet>.ts.net:8443`. Public HTTPS origins are rejected. |
| `MANUAL_AGENT_TOKEN` | yes | — | App password sent as a bearer; minimum 32 characters. Never place it in a message or agent card. |
| `MANUAL_AGENT_MODEL` | no | runner default | Optional model override accepted by the runner. |
| `MANUAL_AGENT_REASONING_EFFORT` | no | runner default | `minimal`, `low`, `medium`, `high`, or `xhigh`. |
| `MANUAL_AGENT_POLL_INTERVAL_MS` | no | `2000` | Status polling interval, bounded to 250–30000 ms. |
| `MANUAL_AGENT_RUN_TIMEOUT_SECS` | no | `3600` | Hard turn limit, bounded to 60–14400 seconds. |

Only Tailnet HTTPS DNS names ending in `.ts.net` are accepted. Public HTTPS
origins, IP literals, credentials, query strings, fragments, and path prefixes
are rejected before any bearer-authenticated request is built.

## Pkzz Desktop

Choose **Remote agent computer** as the agent harness. Pkzz shows first-class
**Manual Agent Tailnet URL** and masked **Manual Agent app password** fields;
the app password is persisted in the OS keyring, stripped from managed-agent
JSON and renderer projections, and hydrated only into the selected adapter
process. The fields are hidden from the generic Advanced editor. The adapter ships as a desktop sidecar. Discovery
reports **Connection setup needed** until the required fields are configured;
finding the sidecar binary alone never means the runtime is ready. Connection
values are definition/instance scoped and are rejected from Global Defaults so
the bearer cannot fan out to unrelated agent processes.

Mesh Computer must provide authenticated `GET /runs?limit=1`, `POST /runs`,
`GET /runs/{runId}`, and `POST /runs/{runId}/stop` endpoints compatible with
the manual-agent OpenAPI contract. Before it accepts ACP input, the adapter also
requires authenticated `GET /readiness` to return
`{"ok":true,"executionBackend":"crabbox","backendReady":true}`. Missing or
weaker attestation keeps live spawning fail-closed; API health or binary
presence alone is not operational readiness.

When a trusted completed status contains `computerSession`, the terminal ACP
tool update retains that descriptor and the desktop renders **Open computer**.
The descriptor's normalized Tailnet origin must exactly match the configured
`MANUAL_AGENT_BASE_URL`; a different host or port fails the turn closed.
The action launches `mesh-computer://session/<id>?endpoint=<tailnet-origin>`;
the app password never enters the URI. Mesh Computer validates the link again,
uses its own encrypted credential, and loads the named session instead of
creating a replacement.

MeshiNfer stays behind the remote guest runner as its model-routing layer. Configure
the Mesh/Crabbox service to use the MeshiNfer endpoint and aliases; do not put
the MeshiNfer router URL or credentials into an agent card or Pkzz event. The
optional `MANUAL_AGENT_MODEL` value selects a runner-supported alias but does
not transfer provider ownership to Pkzz.

## Security boundary

- The bearer exists only in the adapter process environment and HTTP header.
- Pkzz events carry prompts and final replies, never the remote bearer.
- The adapter does not SSH, mount the desktop workspace, accept arbitrary
  remote URLs per message, or expose remote logs to the channel.
- Cancellation succeeds only after the runner acknowledges a terminal stop;
  stop failures are surfaced as failed turns.
- Progress notifications keep the ACP turn alive without publishing remote
  logs or intermediate model text as a final answer.
