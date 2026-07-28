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
`message/send`/`tasks/get` for A2A 0.3. It sends the matching `A2A-Version`
header on every standard A2A request. It also has an explicit vendor
compatibility path for a non-standard card shape (`serviceEndpoint` plus
`agent/sendMessage` and `agent/getTask`). The compatibility path is not an A2A
release contract. Task responses are polled with bounded backoff until a
terminal state or the configured timeout.

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

The adapter requires the OASF descriptor fields `digest`, `media_type`, and
`size` for every artifact. It validates the SHA-256 digest and exact size, and
it accepts only JSON media types. It accepts the OASF `data.card_data` field
only as an explicit deprecated compatibility fallback because current OASF
schemas prefer an artifact descriptor.

Remote records and card/artifact endpoints must use HTTPS. HTTP is accepted
only for loopback hosts. Redirects are disabled. A bearer token is sent only
when `--bearer-token-endpoint` normalizes to the resolved A2A endpoint. This
keeps operator credentials out of arbitrary endpoints selected by a public
record. The adapter resolves each hostname, applies the address policy, and
pins the request client to the checked address. It does not perform a second
unchecked DNS lookup for the request.

The source record does not supply commands, environment variables, or
credentials. New conversations use random UUID context identifiers by
default. An operator can supply a stable identifier with `--context-id` or
`BUZZ_A2A_CONTEXT_ID` when the host has a durable A2A conversation reference
to preserve intentionally.

## A2A extensions

Use `--extensions-json` or `BUZZ_A2A_EXTENSIONS_JSON` to activate optional A2A
protocol extensions. The value must be a JSON object. Each key must be an exact
extension URI advertised by the Agent Card. Each value is the metadata that the
adapter sends for that extension.

```json
{
  "https://example.com/a2a/extensions/work-context/v1": {
    "organizationRef": "https://example.com/organizations/acme",
    "spaceRef": "https://example.com/spaces/project-1"
  }
}
```

For a standard A2A interface, the adapter sends the configured URIs in the
`A2A-Extensions` header. It also sends the URI list and metadata on the A2A
message. If no extensions are configured, the adapter omits the header and both
message fields. The adapter rejects unadvertised extensions and rejects a card
whose required extension is not configured. The vendor compatibility path does
not support A2A extensions.

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

ACP cancellation currently stops the local Buzz turn and aborts the adapter's
request future. The adapter does not yet send an A2A task cancellation request,
so a task that the source runtime already accepted can continue remotely.

OASF defines the record schema; it does not define how records are discovered
or transported. The current adapter resolves a reviewed local path or HTTPS
URL. Authenticity is therefore based on the operator's review and, for remote
records, the HTTPS connection. OASF 1.1 records do not carry a general record
signature. Domain-JWKS verification is the next planned trust layer. Optional
AGNTCY Directory resolution can follow when interoperable Directory identity
and verification are required.
