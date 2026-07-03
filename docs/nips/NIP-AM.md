NIP-AM
======

Agent Turn Metrics
------------------

`draft` `optional` `relay`

This NIP defines a durable, encrypted event kind for recording per-turn token
usage and estimated cost of AI agent sessions. An agent publishes one
`kind:44200` event per completed turn, NIP-44 encrypted to its owner, so the
owner can account for token usage across agents and harnesses without the
relay — or any third party — learning what the agent did or what it cost.

## Motivation

AI agent harnesses consume model tokens on every turn. Owners running fleets
of agents need durable, harness-independent usage accounting — the equivalent
of a metered bill — for cost attribution, budgeting, and capacity planning.

[NIP-AO](NIP-AO.md) (kind 24200) already streams encrypted session telemetry
between agent and owner, but it is deliberately ephemeral: relays MUST NOT
persist it, so it cannot answer "how many tokens did my agents use last
week?". Transcript-grade durable telemetry is explicitly out of scope — the
persistence-averse reasoning behind NIP-AO's ephemerality contract applies to
conversation content, not to a small usage record. Kind 44200 stores only the
metric: token counts, an estimated cost, and correlation identifiers, all
encrypted to the owner.

## Definitions

- **Agent**: an AI process with its own Nostr keypair, executing sessions on
  behalf of an owner.
- **Owner**: the human (or system) whose pubkey the agent was provisioned under.
- **Turn**: one prompt→response cycle of an agent session, as bounded by the
  harness (e.g. one ACP `session/prompt` round trip).
- **Turn metric**: a single kind 44200 event recording the usage of one turn.

## Event

`kind:44200` is a regular event by Buzz convention (alongside 44100/44101):
stored,
append-only, never replaced. Each completed turn produces exactly one event.

```json
{
  "kind": 44200,
  "pubkey": "<agent_pubkey>",
  "created_at": <unix_timestamp>,
  "content": "<NIP-44 v2 ciphertext>",
  "tags": [
    ["p",     "<owner_pubkey>"],
    ["agent", "<agent_pubkey>"]
  ],
  "sig": "..."
}
```

Events MUST have exactly one `p` tag (the owner) and exactly one `agent` tag
(equal to `pubkey`). The tag layout deliberately mirrors NIP-AO telemetry
frames so existing owner-scoped tooling applies unchanged.

No channel (`h`) tag is used. The channel a turn served is private usage
metadata and lives inside the encrypted payload; keeping it out of the tags
avoids leaking per-channel activity rates to the relay operator and keeps the
event community-global (owner-scoped) rather than channel-scoped.

## Encryption

`content` MUST be encrypted with NIP-44 v2 using `(agent_privkey,
owner_pubkey)` — identical to NIP-AO telemetry. Plaintext SHOULD be zeroized
after encrypt/decrypt. Decrypted payload MUST NOT exceed 65,535 bytes
(payloads are typically well under 1 KB).

## Decrypted Payload

The `content` field decrypts to a UTF-8 JSON object:

```jsonc
{
  "harness":   "goose",                  // REQUIRED: harness identifier
  "model":     "claude-sonnet-4-5",      // model id, or null if unknown
  "channelId": "<channel_uuid>" | null,
  "sessionId": "<session_id>"  | null,   // REQUIRED when "cumulative" is present
  "turnId":    "<turn_id>"     | null,
  "turnSeq":   17 | null,                // REQUIRED when "cumulative" is present
  "timestamp": "2026-07-01T20:11:03.213Z", // REQUIRED: RFC 3339, end of turn

  // Usage for THIS turn (computed delta). Fields are null when the harness
  // does not report them — a null MUST NOT be recorded or summed as zero.
  "turn": {
    "inputTokens":  1234  | null,
    "outputTokens": 567   | null,
    "totalTokens":  1801  | null,
    "costUsd":      0.0123 | null        // estimated
  },

  // Session-cumulative usage as reported at the end of this turn.
  "cumulative": {
    "inputTokens":  45210 | null,
    "outputTokens": 9876  | null,
    "totalTokens":  55086 | null,
    "costUsd":      0.41  | null         // estimated
  },

  // false when the publisher could not observe the previous turn's
  // cumulative baseline (e.g. harness restart mid-session), making the
  // "turn" object unreliable for this event.
  "deltaReliable": true,

  "stopReason": "end_turn"               // optional
}
```

`harness` and `timestamp` are REQUIRED. All other fields are OPTIONAL or
nullable, except as constrained below. Consumers MUST ignore unknown fields
(forward compatibility).

### Ordering and delta recomputation

When a `cumulative` object is present, `sessionId` and `turnSeq` are
REQUIRED. `turnSeq` is a per-session monotonically increasing integer
starting at any value, incremented by the publisher on every published turn
metric for that session; a publisher restart that loses the counter MUST
start a new `sessionId` rather than reuse the old one with a reset `turnSeq`.
Cumulative values form a series only *within* one `sessionId`, ordered by
`turnSeq` — consumers MUST NOT diff cumulative values across different
`sessionId`s, and MUST NOT rely on `created_at` (seconds precision, ambiguous
for same-second turns) for ordering within a session.

If a consumer recomputing deltas observes a cumulative counter that decreases
between consecutive `turnSeq` values (counter reset, harness bug), it MUST
treat the affected turn's usage as unknown (null), not as negative usage.
Publishers likewise MUST NOT emit negative values in `turn`; when the
computed delta would be negative or the previous baseline is unknown, the
publisher sets the affected `turn` counters to null and `deltaReliable:
false`.

Where the harness reports only cumulative counters, the publisher computes
`turn` as the difference between consecutive cumulative snapshots within one
session. Consumers doing exact accounting SHOULD prefer recomputing deltas
from consecutive `cumulative` values and treat `turn` as a convenience.

### Numeric validity and token semantics

All token counts MUST be non-negative integers. `costUsd` MUST be a finite,
non-negative number. `totalTokens` is the harness- or provider-reported
total when available; publishers MUST NOT derive it by summing `inputTokens`
and `outputTokens` (providers may count categories a simple sum misses) —
when no total is reported, `totalTokens` is null. `inputTokens` is the
inclusive input-side total: where the provider reports cache reads/writes
separately (e.g. Anthropic `cache_read_input_tokens` /
`cache_creation_input_tokens`), the publisher folds them into `inputTokens`.
Publishers MAY additionally report the cache components in optional
`cacheReadTokens` / `cacheWriteTokens` fields inside `turn` and `cumulative`;
when present these are informational subsets of `inputTokens`, not additions
to it.

`costUsd` values are estimates (provider list prices at publish time, or a
harness-reported estimate). They are advisory, not billing records.

`stopReason`, when present, MUST be one of `end_turn`, `max_tokens`,
`cancelled`, `error`, `unknown`. Consumers MUST treat unrecognized
`stopReason` values as `unknown`; the token counts remain valid.

## Publisher Behavior

- Publish exactly one event per completed turn, at turn completion, including
  turns that end in cancellation or error when usage was observed.
- Do NOT publish an event for a turn with no observed usage (all counters
  unknown); an all-null metric carries no information.
- `created_at` SHOULD equal the payload `timestamp` truncated to seconds.

## Relay Behavior

On receiving a kind 44200 event, a relay MUST:

1. Validate the event signature per NIP-01.
2. Verify `event.pubkey` equals the `agent` tag and that
   `is_agent_owner(agent, owner)` holds for the `p` tag via authenticated
   ownership lookup. Tag matching alone is insufficient.
3. Store the event durably, scoped to the owner (community-global; no channel
   scope).
4. NOT index the event in any full-text search (the ciphertext is not
   searchable and must not enter search indexes).

Reads MUST be gated: only an authenticated ([NIP-42](42.md)) reader whose
pubkey equals the `#p` tag value may receive the event. This gate applies to
**every** read path, including explicit `ids` filters — knowing an event id
MUST NOT grant access. (Some p-gated kinds exempt id-addressed lookups on the
theory that knowing the id implies authorization; kind 44200 events are
long-lived and their cleartext envelope leaks turn activity, so no such
exemption is permitted.) Unauthenticated publish or subscribe attempts MUST be
rejected with `AUTH required`; authenticated attempts from a pubkey that is not
the event owner MUST be rejected with `restricted:`.

Relays SHOULD rate-limit kind 44200 to a rate consistent with real turn
frequency (RECOMMENDED: 60 events/minute per agent pubkey).

## Client Behavior

Owners recover usage history with:

```json
{"kinds": [44200], "#p": ["<own_pubkey>"], "since": <window_start>}
```

On receiving an event, a client MUST verify the signature, decrypt with its
own secret key and `event.pubkey`, and ignore events that fail to decrypt or
parse. Clients SHOULD deduplicate by event id. For within-session ordering,
clients MUST use `(sessionId, turnSeq)` from the decrypted payload as
described above; `created_at` is suitable only for coarse time-window
queries.

## Relationship to Other NIPs

- [NIP-AO](NIP-AO.md): same agent↔owner encryption and tag scoping, but
  ephemeral and transcript-grade. NIP-AM events MUST NOT carry conversation
  content, tool calls, or protocol frames — usage numbers and identifiers only.
- [NIP-09](09.md): the authoring agent (or its owner via relay policy) may
  request deletion; relays apply standard deletion semantics.
- [NIP-40](40.md): publishers MAY set `expiration` to bound retention.

## Security Considerations

**Metadata leakage.** `p`, `agent`, and `created_at` are cleartext: a relay
operator learns that agent X completed turns for owner Y at some rate. Turn
rate is already observable from the agent's channel messages; the token
counts, cost, model, and channel remain encrypted.

**No forward secrecy.** NIP-44 does not provide forward secrecy; compromise
of the agent's private key allows decryption of captured ciphertexts.

**Integrity of accounting.** Metrics are self-reported by the agent process.
A compromised agent can under- or over-report. Owners requiring stronger
guarantees must reconcile against provider-side billing.
