# buzz-a2a-acp

`buzz-a2a-acp` is a small BYOH subprocess adapter. It lets Buzz host an agent
that is described by an [AGNTCY/OASF Agent Record](https://docs.agntcy.org/oasf/agent-record-guide/)
and invoked with [A2A](https://a2a-protocol.org/latest/).

The adapter reads the record, resolves the `integration/a2a` module (OASF id
`203`), and exposes the remote agent through Buzz's existing ACP stdio seam:

```
OASF Agent Record -> A2A Agent Card -> A2A JSON-RPC -> ACP stdio -> Buzz
```

The current adapter prefers an A2A JSON-RPC interface declared in
`supportedInterfaces`. It selects `SendMessage`/`GetTask` for A2A 1.x and
`message/send`/`tasks/get` for A2A 0.3. It also has a named compatibility path
for an older service card (`serviceEndpoint` plus `agent/sendMessage` and
`agent/getTask`). The compatibility path is not a new protocol. Task responses
are polled until a terminal state or the bounded timeout.

## Configure in Buzz

Register the binary as a BYOH ACP runtime with:

```text
command: buzz-a2a-acp
args: --record,/absolute/path/to/agent-record.json
```

The same configuration can be represented by a custom-harness JSON object:

```json
{
  "id": "remote-oasf-agent",
  "label": "Remote OASF agent",
  "command": "buzz-a2a-acp",
  "args": ["--record", "/absolute/path/to/agent-record.json"],
  "env": {}
}
```

This is an operator-owned configuration example. The adapter is not added to
Buzz's compiled-in runtime gallery and does not auto-import or auto-trust
records.

Or set `BUZZ_A2A_AGENT_RECORD`. The record can be a local file or an HTTP(S)
URL. Use `BUZZ_A2A_BEARER_TOKEN` only when the remote A2A endpoint requires it.
The token is supplied by the operator and is never read from the public record.

The adapter requires and validates an OASF artifact `sha256` digest for every
artifact fetched over the network. It also validates a declared artifact size
when present. It accepts the OASF `data.card_data` field only as an explicit
deprecated compatibility fallback because current OASF schemas prefer an
artifact descriptor.

Remote records and card/artifact endpoints must use HTTPS. HTTP is accepted
only for loopback hosts. Redirects are disabled. A bearer token is sent only
when `--bearer-token-endpoint` normalizes to the resolved A2A endpoint. This
keeps operator credentials out of arbitrary endpoints selected by a public
record. The adapter resolves each hostname, applies the address policy, and
pins the request client to the checked address. It does not perform a second
unchecked DNS lookup for the request.

The optional `--agency-ref`, `--space-ref`, and `--agent-ref` flags project
stable host context references into A2A `metadata`. The source record does
not supply commands, environment variables, or credentials.

## Scope and trust boundary

The adapter projects public discovery metadata and A2A results. It does not
copy an Agency's private prompts, memory, tools, local files, or signing keys
into Buzz. The source runtime remains responsible for authentication,
authorization, execution, and any Nostr or Git signing. Buzz receives the
ACP-visible response or task status.

This is an experimental adapter. It does not implement AGNTCY Directory
registration, OASF custom taxonomy exchange, A2A streaming, push notifications,
or Surface rendering. Those are separate integration layers that can build on
the real invocation seam without inventing a parallel agency protocol.
