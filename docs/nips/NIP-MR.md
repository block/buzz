# NIP-MR: Agent Mention Receipt

`draft` `optional`

An agent-published receipt for a mention, so that a mention nothing picks up is
distinguishable from a mention still being worked on.

## Motivation

Buzz relays fan out; they do not track delivery. An event that `p`-tags an agent
is pushed to whatever subscriptions match at the instant of fan-out and is
retained for pull queries, but nothing is retried, queued, or reported. If the
agent's harness is not connected and subscribed at that instant, the mention
reaches nobody and nothing is published in response.

The harness has its own reasons to not act. It drops events whose author is
outside `respond_to` (which defaults to `owner-only`, so a co-worker mentioning
the agent is dropped), events matching no subscription rule, events whose filter
expression failed closed, and — under `--dedup=drop` — events arriving while the
channel is already in flight. Each of those exits at a `debug!` log.

Every one of these produces the same observable: nothing. The sender cannot tell
"no agent is running", "the agent will not talk to you", and "the agent is
thinking" apart, so a mention that went nowhere looks exactly like one that is
being handled. That is the dead-end this NIP closes.

An agent already posts a 👀 reaction on pickup, but it is explicitly cosmetic —
a 500 ms timeout with failures swallowed — so it cannot be the signal a sender
relies on, and it is absent entirely on the decline paths.

## Event

**Kind `44102`** — regular, stored, channel-scoped.

| Tag | Cardinality | Value |
|-----|-------------|-------|
| `h` | 1 | Channel UUID the mention occurred in |
| `e` | 1 | Event id of the mention being acknowledged |
| `p` | 1 | Pubkey of the mention's author |
| `status` | 1 | `accepted` or `declined` |
| `reason` | 0–1 | Machine-readable slug; present only when `declined` |

`content` is empty. The signer is the acknowledging agent.

```jsonc
{
  "kind": 44102,
  "pubkey": "<agent pubkey>",
  "content": "",
  "tags": [
    ["h", "3f2b...-uuid"],
    ["e", "<mention event id>"],
    ["p", "<mention author pubkey>"],
    ["status", "declined"],
    ["reason", "sender-not-allowed"]
  ]
}
```

### `status`

- **`accepted`** — the harness queued the mention; a turn is coming. It is *not*
  a completion guarantee: the turn may still fail, be cancelled, or time out.
- **`declined`** — the harness received the mention and will not act on it.

### `reason` slugs

| Slug | Meaning |
|------|---------|
| `sender-not-allowed` | The author is outside the agent's `respond_to` set. The likeliest cause of a dead-ended mention in practice, since the default is `owner-only`. |
| `no-matching-rule` | No subscription rule matched, or a filter expression failed closed. |
| `busy` | The agent was mid-turn and is configured to drop rather than queue. |

Readers MUST treat an unknown slug as a bare decline rather than rejecting the
event, so new slugs can be added without breaking older clients.

## Agent behaviour

An agent SHOULD publish exactly one receipt per event that `p`-tags it, at the
point it decides what to do with that event and before any turn output exists.

An agent MUST NOT acknowledge:

- events that do not `p`-tag it — in a wildcard subscription the harness sees
  every message in the channel, and acknowledging those would bury it;
- its own events;
- **kind `44102` itself.** A receipt `p`-tags the author it answers, so a
  receipt is itself a mention of that agent. Without this rule two sibling
  agents on wildcard subscriptions would acknowledge each other's receipts
  indefinitely, with no human involved and no terminating condition.

Publishing is best-effort and MUST NOT block dispatch.

## Relay behaviour

Kind `44102` requires the `messages:write` scope and an `h` tag, which routes
the write through the ordinary channel-membership check. A non-member therefore
cannot inject receipts into a channel they cannot read.

The kind is deliberately **not** p-gated: a receipt is channel-visible like the
👀 reaction it accompanies, so co-members and sibling agents can see that a
mention was received.

## Client behaviour

A relay cannot verify that a signing key belongs to an agent. Clients MUST
therefore ignore any receipt whose author is not a pubkey the acknowledged
message actually mentioned. Without that check, any member could publish a
well-formed receipt for someone else's message and suppress a genuine warning.

Because no receipt is published when nothing is running, absence is meaningful
and only the sending client can observe it. A client SHOULD track its own
outgoing agent mentions and, after a bounded wait, surface those that were never
acknowledged. The wait should comfortably exceed a cold agent start — the
reference desktop implementation uses 30 s — since a premature "nobody picked
this up" on an agent that was merely booting is worse than a late one.

An actual reply supersedes the receipt path entirely.

## Limitations

`accepted` marks the start of a turn, not its completion. An agent that accepts
and then crashes, hangs, or has its work superseded publishes no further
receipt, so a client that stops watching on `accepted` will not learn about it.
Covering that requires a completion deadline or a terminal `dropped` receipt,
and is left to a follow-up.

## Reference implementation

- Kind and tag vocabulary: `crates/buzz-core/src/kind.rs`
- Relay admission: `crates/buzz-relay/src/handlers/ingest.rs`
- Publisher: `crates/buzz-acp/src/pool.rs`, wired in `crates/buzz-acp/src/lib.rs`
- Client tracking: `desktop/src/features/agents/pendingMentionAckStore.ts`
