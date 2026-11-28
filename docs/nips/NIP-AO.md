NIP-AO
======

Agent Observability
-------------------

`draft` `optional`

This NIP defines ephemeral, encrypted event kinds for streaming internal session telemetry between AI agent processes and their owners' desktop clients via Nostr relays.

## Motivation

AI agent harnesses execute long-running sessions that invoke tools, send protocol
frames to models, and emit intermediate reasoning. Owners need real-time visibility
into this activity for debugging, auditing, and control — without that telemetry
being stored on any relay or visible to third parties.

Kind 24200 provides a dedicated, encrypted, ephemeral channel for this purpose.
It is strictly scoped to the agent↔owner relationship and carries no durable state.

## Definitions

- **Agent**: An AI process with its own Nostr keypair, executing a session on behalf of an owner.
- **Owner**: The human (or system) whose pubkey the agent was provisioned under.
- **Observer Frame**: A single kind 24200 event carrying one unit of telemetry or control.
- **Session**: A bounded agent execution correlated by a shared `sessionId`.

## Event Kinds

| Kind  | Name                  | Direction         |
|-------|-----------------------|-------------------|
| 24200 | Agent Observer Frame  | agent↔owner (both)|

Kind 24200 falls in the ephemeral range (20000–29999) defined by NIP-01. Relays
MUST NOT persist it.

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
context.

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
| `control_result`   | The agent's answer to a control frame (see below)        |
| `batch`            | Several frames of one channel, sent together (see below) |

A `batch` frame's `payload` is `{ "events": [ ... ] }`, a list of complete
`ObserverEvent` objects for the same channel, each with its own `seq`,
`timestamp`, `kind` and `payload`. Its own metadata mirrors the last inner
event. Clients MUST process each inner event as if it had arrived on its own,
in list order. An agent MAY batch any frame kind, including `control_result`,
so a client that ignores `batch` can miss the answer to a control frame.

### Control (`frame=control`)

The `content` field decrypts to a JSON object whose `type` names the command.
The defined types are `cancel_turn`, `cancel_thread_turn`, `switch_model` and
`publish_project_owner_announcements`. Implementations MUST ignore events with
unrecognized `type` values, and SHOULD drop a frame whose required fields are
missing or invalid without answering it.

The agent answers a control frame with a telemetry frame of `kind`
`control_result`. Its `payload` carries the command's `type`, a `status` and
the frame's `requestId`, so the owner can match the answer to the command that
caused it and ignore answers to earlier ones. A sender that gets no answer
cannot tell a dropped frame from an offline or outdated agent, and SHOULD time
out.

#### `cancel_turn`

Stops the agent's running turn in a channel.

```json
{
  "type":      "cancel_turn",
  "channelId": "<channel_uuid>",
  "requestId": "<opaque_id>"
}
```

| Field       | Required | Meaning                                  |
|-------------|----------|------------------------------------------|
| `channelId` | yes      | Channel the turn runs in.                |
| `requestId` | no       | Opaque correlator, echoed in the result. |

The target is the whole channel. An agent with more than one session in the
channel MUST NOT pick one and answers `ambiguous_target`. An agent MUST ignore
any other field, such as a `threadRootId`, and MUST NOT narrow the target from
it; a sender that stops one thread uses `cancel_thread_turn`.

The result is a `control_result` telemetry frame with `channelId` set:

```json
{ "type": "cancel_turn", "status": "sent", "requestId": "<opaque_id>" }
```

| `status`           | Meaning                                                          |
|--------------------|------------------------------------------------------------------|
| `sent`             | A running turn was signalled to stop. It may not have ended yet. |
| `no_active_turn`   | No turn was running there, or it was already stopping.           |
| `ambiguous_target` | The channel has more than one session.                           |

`sent` is not proof that the turn ended. A sender that needs that SHOULD watch
the turn's own telemetry, or other evidence of activity, end.

#### `cancel_thread_turn`

Stops the agent's running turn in one thread of a channel.

```json
{
  "type":         "cancel_thread_turn",
  "channelId":    "<channel_uuid>",
  "threadRootId": "<event_id_hex>",
  "requestId":    "<opaque_id>"
}
```

| Field          | Required | Meaning                                                  |
|----------------|----------|----------------------------------------------------------|
| `channelId`    | yes      | Channel the turn runs in.                                |
| `threadRootId` | yes      | Root event id (64 hex characters) of the thread to stop. |
| `requestId`    | no       | Opaque correlator, echoed in the result.                 |

The agent stops the turn of the session that serves that thread. An agent that
keeps one session for the whole channel (or a DM) stops that session's turn,
since every thread of the channel runs in it. An agent that keeps a session per
thread MUST NOT stop a session that may serve another thread. A frame without a
valid `threadRootId` is dropped unanswered.

An agent that predates this type ignores it, as it does any unrecognized
`type`, so a sender gets no answer rather than a stop of another thread's turn,
and SHOULD time out.

The result is a `control_result` telemetry frame with `channelId` set:

```json
{ "type": "cancel_thread_turn", "status": "sent", "requestId": "<opaque_id>" }
```

| `status`         | Meaning                                                          |
|------------------|------------------------------------------------------------------|
| `sent`           | A running turn was signalled to stop. It may not have ended yet. |
| `no_active_turn` | No turn was running in that thread, or it was already stopping.  |

As with `cancel_turn`, `sent` is not proof that the turn ended.

#### `switch_model`

Changes the model the agent uses in a channel.

```json
{
  "type":      "switch_model",
  "channelId": "<channel_uuid>",
  "modelId":   "<model_id>",
  "requestId": "<opaque_id>"
}
```

`channelId` and `modelId` are required; `requestId` is an optional string,
echoed in every result for this command. If a turn is running, the agent stops
it and runs its request again under the new model. If none is, the new model
applies from the next turn. Like `cancel_turn`, the target is the whole
channel.

The first `control_result` frame, with `channelId` set, carries `type`,
`status`, `modelId` and `requestId`:

| `status`            | Meaning                                                             |
|---------------------|---------------------------------------------------------------------|
| `sent`              | A running turn was signalled; a later frame reports the outcome.    |
| `turn_ending`       | The running turn was already stopping, so the switch did not land.  |
| `switched`          | No turn was running; the next turn uses the new model.              |
| `unsupported_model` | The agent's model list does not include `modelId`.                  |
| `no_active_turn`    | The agent has no idle session in the channel.                       |
| `ambiguous_target`  | The channel has more than one session.                              |

After `sent`, the agent sends one more `control_result` with the same fields
when it creates the session under the new model: `switched`,
`unsupported_model`, or `failure` if the agent runtime rejected the model.
After `switched`, the same session creation can still send `unsupported_model`
or `failure`, when the agent had not yet learned its model list.

#### `publish_project_owner_announcements`

Asks the agent to sign and publish, as itself, git project announcements the
owner prepared.

```json
{
  "type":      "publish_project_owner_announcements",
  "requestId": "<uuid>",
  "announcements": [
    {
      "kind":      30617,
      "content":   "",
      "createdAt": 1777464041,
      "tags":      [["d", "<project_id>"]]
    }
  ]
}
```

`requestId` MUST be a UUID. `announcements` holds one or two event templates.
Each template's `kind` MUST be 30617 or 30621 and its `tags` MUST include a
non-empty `d` tag. `createdAt` is optional; the agent uses the current time
when it is absent and rejects a value more than five minutes in the future. The
agent publishes the templates in order and stops at the first failure.

The result is a `control_result` telemetry frame with `channelId` null:

```json
{
  "type":      "publish_project_owner_announcements",
  "requestId": "<uuid>",
  "status":    "ok",
  "events":    [ { "...": "the signed events it published" } ],
  "error":     null
}
```

`status` is `ok` when every template was published, or `error`, with `error`
describing the failure and `events` holding the events published before it.

## Ephemerality Contract

- Relays MUST NOT persist kind 24200 events to any durable storage.
- Relays MUST NOT include kind 24200 events in search indexes.
- Relays MUST NOT include kind 24200 events in audit logs.
- Relays SHOULD fan out kind 24200 events only via in-memory pub/sub,
  never via a database write path.
- Clients SHOULD subscribe with `since=<now>`; historical replay is not supported.
- Clients SHOULD buffer received events in a bounded in-memory ring buffer.

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
subscribe attempts MUST be rejected with `AUTH required`.

## Relay Behavior

On receiving a kind 24200 event, a relay MUST:

1. Validate the event signature per NIP-01.
2. Verify authorization per the rules above.
3. Fan out to matching subscribers via in-memory pub/sub.
4. NOT invoke the normal event ingestion or persistence path.

Relays SHOULD enforce a rate limit of 100 events/second per agent pubkey.
Relays are RECOMMENDED to reject events whose `created_at` falls outside a ±5-minute
freshness window to prevent replay of captured events.

## Client Behavior

Clients subscribe with:

```json
{"kinds": [24200], "#p": ["<own_pubkey>"], "since": <now>}
```

On receiving an event, a client MUST:

1. Verify the event signature.
2. Decrypt `content` using own secret key and `event.pubkey`.
3. Parse the decrypted payload and dispatch on `kind` (telemetry) or `type` (control).
4. Ignore unknown `kind`/`type` values.

Clients SHOULD verify that the `agent` tag matches a known/trusted agent pubkey
before decrypting.

Clients SHOULD buffer events in a bounded ring buffer (RECOMMENDED maximum: 800 events).
Clients MUST NOT request historical kind 24200 events (no `since` in the past, no
`until`, no `ids` queries).

## Security Considerations

**Metadata leakage.** Routing tags (`p`, `agent`, `frame`, `created_at`) are
cleartext. A relay operator can observe that agent X is streaming to owner Y at what
rate. For maximum metadata privacy, implementors MAY wrap events in NIP-59 gift wrap.

**No forward secrecy.** NIP-44 does not provide forward secrecy; compromise of the
agent's private key allows decryption of any captured ciphertext.

**Replay attacks.** A captured, signed event could be replayed without a freshness
check. Relays are RECOMMENDED to enforce a `created_at` freshness window.

**Rogue relays.** The ephemerality contract is relay policy, not cryptography.
NIP-44 encryption ensures stored events remain opaque to the relay operator absent
key compromise.

**Best-effort delivery.** Control frames can be dropped during reconnect or queue
overflow. Control commands SHOULD be treated as advisory with idempotent semantics.
Agents MUST NOT rely on guaranteed delivery of control frames.

**Operational persistence vectors.** Telemetry may transiently exist in process
memory, crash dumps, and application logs. Implementations SHOULD minimize logging
of decrypted payloads and MUST NOT log it at INFO level or above.

## Relationship to Other NIPs

- **NIP-01**: Kind 24200 is in the ephemeral range (20000–29999); standard event
  structure and signature rules apply.
- **NIP-42**: Recommended for relay-side authentication gating.
- **NIP-44**: Required encryption algorithm for all `content` fields.
- **NIP-29**: An `h` tag MAY be included when the agent session is scoped to a
  NIP-29 group.
- **NIP-XX (PR #2226)**: NIP-XX defines the agent *output* plane; this NIP defines
  the *observability* plane (internal agent activity). They are complementary and
  non-overlapping.

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

### 2. Control Event — `cancel_thread_turn` frame

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
  "type":         "cancel_thread_turn",
  "channelId":    "52a85618-0f8f-4542-94ec-599e6e1c6f2e",
  "threadRootId": "9f2c4e1a7b3d5f6081a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708",
  "requestId":    "0b6f2d4e-8a1c-4f3e-9d2b-7c5a1e0f4b6d"
}
```

---

### 3. Telemetry Event — `control_result` for that cancel

**Decrypted payload** (wire event shaped as in example 1):

```json
{
  "seq":        43,
  "timestamp":  "2026-04-29T12:00:42.120Z",
  "kind":       "control_result",
  "agentIndex": null,
  "channelId":  "52a85618-0f8f-4542-94ec-599e6e1c6f2e",
  "sessionId":  null,
  "turnId":     null,
  "payload": {
    "type":      "cancel_thread_turn",
    "status":    "sent",
    "requestId": "0b6f2d4e-8a1c-4f3e-9d2b-7c5a1e0f4b6d"
  }
}
```

## Reference Implementation

[block/sprout PR #421](https://github.com/block/sprout/pull/421)
