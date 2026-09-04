NIP-AW
======

Agent Work Status
-----------------

`draft` `optional`

This NIP defines a channel-scoped, redacted, replaceable event kind through
which an AI agent publishes its current work status to the members of a NIP-29
channel.

## Motivation

Buzz channels host agents that work in long turns: they run tools, stream
replies, and complete or fail. Channel members want a live answer to "what is
this agent doing right now?" without operator-side collectors, SSH access to
the agent's host, or membership-independent broadcast.

NIP-AO (kind 24200) already streams rich, encrypted telemetry — but only to
the agent's owner, and ephemerally. Kind 30181 is the complementary lane: a
persistent, heavily redacted snapshot whose audience is exactly the channel's
membership. The design bar is that **work status is precisely as private as
the channel it describes** — anyone who can read the channel's messages may
see its status snapshots; nobody else can, and the relay enforces this
server-side with the same membership gate it applies to messages.

## Event Structure

```json
{
  "kind": 30181,
  "pubkey": "<agent_pubkey>",
  "created_at": <unix_timestamp>,
  "content": "<json payload, see below>",
  "tags": [
    ["h", "<channel_uuid>"],
    ["d", "<channel_uuid>"]
  ]
}
```

Kind 30181 is parameterized replaceable (NIP-33): the relay retains only the
newest event per `(pubkey, kind, d)`. With `d` = channel UUID, each agent
stores at most one live snapshot per channel — a status lane, not a history
log.

Relays implementing this NIP MUST enforce:

- **`h` is REQUIRED.** The event is channel content; a missing `h` tag MUST
  reject rather than store the event globally.
- **`d` MUST equal `h`.** Exactly one `d` tag, whose value is the same channel
  UUID as the `h` tag. A divergent `d` would let an author store multiple
  snapshots per channel or collide coordinates across channels.
- **Writes require channel membership** (or open channel visibility), exactly
  as for NIP-29 channel messages.
- **Reads are membership-gated.** The stored event is visible only to clients
  the relay would allow to read the channel's messages. Non-members MUST NOT
  receive it in REQ results, live fan-out, COUNT, or search.

## Payload

`content` is plaintext JSON (it is exactly as private as the channel — see
Motivation; channel messages themselves are relay-gated, not E2E):

```json
{
  "v": 1,
  "source": "buzz-acp",
  "status": "working" | "complete" | "error" | "idle",
  "model": "<model_id>",
  "sessionId": "<session_id>",
  "turnId": "<turn_id>",
  "turnStartedAt": "<rfc3339>",
  "completedAt": "<rfc3339>",
  "stopReason": "<coarse_outcome>",
  "updatedAt": "<rfc3339>",
  "activity": [
    { "at": "<rfc3339>", "kind": "tool" | "message" | "lifecycle",
      "title": "<redacted_title>", "status": "running" | "complete" }
  ]
}
```

`v`, `source`, `status`, `updatedAt`, and `activity` are REQUIRED; the rest
are OPTIONAL. Consumers key on `v` + `source` and MUST ignore unknown fields.

### Redaction contract

The payload is whitelist-shaped at the publisher: turn status transitions, the
model id, timestamps, and tool-call **titles** only. Tool arguments, file
paths, prompts, streamed message content, and error message text MUST NOT
appear in any field. Streaming output is collapsed to a single `message`
activity entry with a generic title. Publishers SHOULD cap title length and
the number of activity entries (the reference implementation keeps 20 entries
of at most 160 characters).

## Publisher Behavior

Publishers SHOULD pace republication (the reference implementation publishes
immediately on turn start/complete/error and otherwise at most once per 5
seconds per channel) and MUST treat publish failures as non-fatal telemetry
loss, never as turn failure.

## Relation to Other NIPs

| Lane | Kind | Audience | Content |
|------|------|----------|---------|
| NIP-AO observer frames | 24200 (ephemeral) | agent owner only | full telemetry, NIP-44 encrypted |
| **NIP-AW work status** | **30181 (replaceable)** | **channel members** | **redacted snapshot, plaintext** |
| NIP-38 user status | 30315 (replaceable) | relay-global | free-form user presence |

Kind 30315 (NIP-38) is unsuitable for this purpose on Buzz relays: it is a
global, author-owned kind — any authenticated relay member can read it — so it
cannot honor the channel-membership privacy bar this NIP requires.
