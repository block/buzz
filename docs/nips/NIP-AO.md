NIP-AO
======

Agent Observability
-------------------

`draft` `optional` `relay`

This NIP defines encrypted event kinds for streaming and retaining internal
session telemetry between AI agent processes and their owners' desktop clients
via Nostr relays.

## Motivation

AI agent harnesses execute long-running sessions that invoke tools, send protocol
frames to models, and emit intermediate reasoning. Owners need real-time visibility
into this activity for debugging, auditing, and control — without that telemetry
being visible to third parties.

Kind 24200 provides a dedicated, encrypted, **durable** channel for this purpose.
It is strictly scoped to the agent↔owner relationship. Relays persist frames so
owners can REQ historical activity (thinking, tool calls, session boundaries)
alongside live fan-out. Token-usage accounting remains on [NIP-AM](NIP-AM.md)
kind 44200.

## Definitions

- **Agent**: An AI process with its own Nostr keypair, executing a session on behalf of an owner.
- **Owner**: The human (or system) whose pubkey the agent was provisioned under.
- **Observer Frame**: A single kind 24200 event carrying one unit of telemetry or control.
- **Session**: A bounded agent execution correlated by a shared `sessionId`.

## Event Kinds

| Kind  | Name                  | Direction         | Persistence |
|-------|-----------------------|-------------------|-------------|
| 24200 | Agent Observer Frame  | agent↔owner (both)| durable     |

### Durability special-case (kind number 24200)

Kind 24200 sits in the NIP-01 ephemeral numeric range (20000–29999). Buzz
**special-cases** it as a durable stored kind so existing publishers
(`buzz-acp`, desktop control frames) keep the historical kind number without a
breaking renumber. Relays that implement this NIP:

- MUST persist kind 24200 to durable storage (same append-only model as kind 44200).
- MUST NOT treat kind 24200 as fan-out-only ephemera.
- MUST document this departure from the default NIP-01 ephemeral-range contract.

Relays that do not implement this NIP MAY continue to discard 24200; clients
MUST tolerate missing history on non-implementing relays (local archive remains
a fallback).

## Event Structure

```json
{
  "kind": 24200,
  "pubkey": "<sender_pubkey>",
  "created_at": <unix_timestamp>,
  "content": "<NIP-44 v2 ciphertext>",
  "tags": [
    ["p",     "<recipient_pubkey>"],
    ["agent", "<agent_pubkey>"],
    ["frame", "telemetry" | "control"]
  ]
}
```

Events MUST have exactly one `p` tag, exactly one `agent` tag, and exactly one
`frame` tag.

**Telemetry** (agent → owner): `pubkey`=agent, `p`=owner, `agent`=agent.
**Control** (owner → agent): `pubkey`=owner, `p`=agent, `agent`=agent (target).

`frame` MUST be `"telemetry"` or `"control"`. Relays SHOULD silently drop events
with unrecognized `frame` values (returning OK to the publisher for forward
compatibility). Clients MUST ignore events with unrecognized `frame` values. An `h` tag MAY be included when the session runs within a NIP-29 group
context; Buzz relays store the event as community-global (`channel_id = NULL`)
regardless — channel identity for filtering lives primarily in the encrypted
payload (`channelId`).

## Encryption

All `content` fields MUST be encrypted with NIP-44 v2 (XChaCha20-Poly1305 over a
secp256k1 ECDH shared secret).

- **Telemetry**: encrypted with `(agent_privkey, owner_pubkey)`
- **Control**: encrypted with `(owner_privkey, agent_pubkey)`

Plaintext SHOULD be zeroized from memory immediately after encrypt/decrypt.
Decrypted payload MUST NOT exceed 65,535 bytes.

## Decrypted Payload

### Telemetry (`frame=telemetry`)

The `content` field decrypts to an `ObserverEvent` JSON object:

```json
{
  "seq":         <monotonic_integer>,
  "timestamp":   "<rfc3339_string>",
  "kind":        "<frame_kind>",
  "agentIndex":  <integer> | null,
  "channelId":   "<channel_uuid>" | null,
  "sessionId":   "<session_id>" | null,
  "turnId":      "<turn_id>" | null,
  "payload":     { ... }
}
```

`seq`, `timestamp`, `kind`, and `payload` are REQUIRED. `agentIndex`, `channelId`, `sessionId`,
and `turnId` are OPTIONAL — they MAY be `null` when the value is not yet known
(e.g., `sessionId` before session establishment). Clients MUST handle `null` values
gracefully.

`seq` is monotonically increasing per session (drop detection). `timestamp` is an
RFC 3339 datetime string with sub-second precision (e.g., `"2026-04-29T12:00:41.500Z"`).
`agentIndex` identifies the agent in multi-agent scenarios. `sessionId`/`turnId`
correlate frames across a session and turn. `payload` is kind-specific (MAY be `{}`).
Unknown `kind` values MUST be ignored.

### Frame Kinds

| `kind`             | Description                                              |
|--------------------|----------------------------------------------------------|
| `acp_read`         | Inbound ACP protocol frame (model → harness)             |
| `acp_write`        | Outbound ACP protocol frame (harness → model)            |
| `turn_started`     | A new agent turn has begun                               |
| `session_resolved` | Session completed or terminated                          |

### Control (`frame=control`)

The `content` field decrypts to:

```json
{
  "type":      "cancel_turn",
  "channelId": "<channel_uuid>"
}
```

The only defined control type is `cancel_turn`. Implementations MUST ignore
events with unrecognized `type` values.

## Persistence, Retention, and Privacy

- Relays MUST persist kind 24200 events to durable storage.
- Relays MUST NOT include kind 24200 events in search indexes (NIP-50 FTS).
  Content is NIP-44 ciphertext and transcript-like; indexing it would waste
  storage and risk leaking activity metadata through search hits.
- Relays MUST NOT expose kind 24200 content or cleartext routing tags to
  unauthenticated or non-`#p` readers (same `#p`-gate as kind 44200).
- Relays SHOULD retain kind 24200 for at least 30 days. Longer retention is
  operator policy. Relays MAY apply retention TTL or owner-requested deletion;
  clients SHOULD treat relay history as best-effort and MAY keep a local
  encrypted archive.
- Clients MAY subscribe for live frames and separately REQ historical frames
  with a past `since` / `until` / `limit` for the owner (`#p` = self).
- Clients SHOULD buffer received events in a bounded in-memory ring buffer for
  the live UI; historical pages come from relay REQ and/or local archive.

## Authorization

**Telemetry** (agent → owner):
- `event.pubkey` MUST equal the agent pubkey.
- `p` tag MUST equal the owner pubkey.
- Relay MUST verify `is_agent_owner(agent, owner)` via authenticated ownership lookup.

**Control** (owner → agent):
- `event.pubkey` MUST equal the owner pubkey.
- `p` tag MUST equal the agent pubkey.
- Relay MUST verify `is_agent_owner(agent, owner)` where agent is resolved from the
  `agent` tag.

Both directions require relay confirmation of the agent-owner relationship via
database lookup. `#p` tag matching alone is insufficient. Unauthorized publish or
subscribe attempts MUST be rejected with `AUTH required` / `restricted:`.

**Reads**: Kind 24200 is `#p`-gated and result-gated. A REQ that can match kind
24200 MUST include `#p` equal to the authenticated reader's pubkey. Knowing an
event `id` alone is NOT authorization — filters that explicitly name kind 24200
MUST still satisfy the `#p` owner check (same rule as NIP-AM kind 44200).

## Relay Behavior

On receiving a kind 24200 event, a relay MUST:

1. Validate the event signature per NIP-01.
2. Verify authorization per the rules above.
3. Enforce a freshness window: reject events whose `created_at` falls outside
   ±5 minutes (replay defense for live publish; historical REQ still serves
   already-stored events).
4. Persist the event via the normal ingest path (community-global storage).
5. Fan out to matching subscribers.

Relays SHOULD enforce a rate limit of **20 events/second per agent pubkey** for
telemetry frames (lower than a pure fan-out budget because each accepted frame
is a durable write). Control frames (owner → agent) SHOULD bypass this limiter.
Relays SHOULD count kind 24200 against durable message admission quotas (not
ephemeral-exempt quotas).

## Client Behavior

Clients subscribe for live frames with:

```json
{"kinds": [24200], "#p": ["<own_pubkey>"], "since": <now_minus_lookback>}
```

Clients MAY request historical frames with a past `since` (and optional `until`,
`limit`, `#agent`):

```json
{"kinds": [24200], "#p": ["<own_pubkey>"], "#agent": ["<agent_pubkey>"], "since": <window_start>, "limit": 500}
```

On receiving an event, a client MUST:

1. Verify the event signature.
2. Decrypt `content` using own secret key and `event.pubkey`.
3. Parse the decrypted payload and dispatch on `kind` (telemetry) or `type` (control).
4. Ignore unknown `kind`/`type` values.

Clients SHOULD verify that the `agent` tag matches a known/trusted agent pubkey
before decrypting.

Clients SHOULD buffer live events in a bounded ring buffer (RECOMMENDED maximum:
3000 events per agent). Channel filtering for activity views SHOULD use the
decrypted `channelId` field (cleartext `h` tags are optional and not required
for storage routing on Buzz).

## Security Considerations

**Metadata leakage.** Routing tags (`p`, `agent`, `frame`, `created_at`) are
cleartext. A relay operator can observe that agent X is streaming to owner Y at what
rate, and — with durable storage — can observe historical activity volume. For
maximum metadata privacy, implementors MAY wrap events in NIP-59 gift wrap.

**No forward secrecy.** NIP-44 does not provide forward secrecy; compromise of the
agent's private key allows decryption of any captured ciphertext, including
relay-retained history.

**Replay attacks.** A captured, signed event could be replayed without a freshness
check on publish. Relays MUST enforce a `created_at` freshness window on ingest.
Already-stored events remain available via historical REQ by design.

**Rogue relays.** Persistence is relay policy backed by NIP-44 opacity. Encryption
ensures stored events remain opaque to the relay operator absent key compromise,
but retention policy and metadata leakage are operator-trust concerns.

**Best-effort delivery.** Control frames can be dropped during reconnect or queue
overflow. Control commands SHOULD be treated as advisory with idempotent semantics.
Agents MUST NOT rely on guaranteed delivery of control frames.

**Operational persistence vectors.** Telemetry may exist in process memory, crash
dumps, application logs, relay databases, and local client archives.
Implementations SHOULD minimize logging of decrypted payloads and MUST NOT log
them at INFO level or above.

## Relationship to Other NIPs

- **NIP-01**: Kind 24200 uses the ephemeral numeric range but is a documented
  durable special-case on Buzz relays implementing this NIP.
- **NIP-42**: Required for relay-side authentication gating of publish and REQ.
- **NIP-44**: Required encryption algorithm for all `content` fields.
- **NIP-29**: An `h` tag MAY be included when the agent session is scoped to a
  NIP-29 group; Buzz stores events as community-global regardless.
- **NIP-AM**: Kind 44200 records durable per-turn token usage metrics. Kind 24200
  is the observability / activity plane; they are complementary and non-overlapping.
- **NIP-XX (PR #2226)**: NIP-XX defines the agent *output* plane; this NIP defines
  the *observability* plane (internal agent activity).

## Examples

### 1. Telemetry Event — `acp_write` frame

**Wire event (encrypted):**

```json
{
  "id":         "a1b2c3d4...",
  "kind":       24200,
  "pubkey":     "agent_pubkey_hex",
  "created_at": 1777464041,
  "content":    "<NIP-44 v2 ciphertext>",
  "tags": [
    ["p",     "owner_pubkey_hex"],
    ["agent", "agent_pubkey_hex"],
    ["frame", "telemetry"]
  ],
  "sig": "..."
}
```

**Decrypted payload:**

```json
{
  "seq":        42,
  "timestamp":  "2026-04-29T12:00:41.500Z",
  "kind":       "acp_write",
  "agentIndex": 0,
  "channelId":  "52a85618-0f8f-4542-94ec-599e6e1c6f2e",
  "sessionId":  "a1b2c3d4",
  "turnId":     "e5f6g7h8",
  "payload": {
    "jsonrpc": "2.0",
    "method":  "tools/call",
    "params":  { "name": "shell", "arguments": { "command": "ls -la" } }
  }
}
```

---

### 2. Control Event — `cancel_turn` frame

**Wire event (encrypted):**

```json
{
  "id":         "e5f6a7b8...",
  "kind":       24200,
  "pubkey":     "owner_pubkey_hex",
  "created_at": 1777464042,
  "content":    "<NIP-44 v2 ciphertext>",
  "tags": [
    ["p",     "agent_pubkey_hex"],
    ["agent", "agent_pubkey_hex"],
    ["frame", "control"]
  ],
  "sig": "..."
}
```

**Decrypted payload:**

```json
{
  "type":      "cancel_turn",
  "channelId": "52a85618-0f8f-4542-94ec-599e6e1c6f2e"
}
```

## Reference Implementation

[block/sprout PR #421](https://github.com/block/sprout/pull/421)
