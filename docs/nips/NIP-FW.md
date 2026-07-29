NIP-FW
======

Message Forwarding
------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-18 (reposts), NIP-29 (relay-based
groups), NIP-42 (authentication)

This NIP defines `kind:40009` message forwards — a message that carries a
complete, independently verifiable copy of another message into a different
channel, together with an optional note written by the forwarder. It is the
Slack "Forward message…" primitive expressed as a Nostr event.

## Motivation

Buzz issue [#3264](https://github.com/block/buzz/issues/3264) asks for Slack
parity on the single most-used sharing gesture in a chat product: take a
message that is in the wrong place and put it in the right place, with a line
of context. Today the only way to do that in Buzz is to copy the text by hand,
which loses the author, the timestamp, the attachments, and any link back to
the original.

The agent case is the sharper one. Handing work off — "this is the failure the
build agent reported, take it from here" — means moving one concrete message,
verbatim, from where it happened to where the next actor is listening. A
hand-retyped paste is not evidence: the recipient (human or agent) cannot tell
whether the quoted text is what the original author actually wrote. A forward
carries the original's signature, so the copy is a fact the relay and every
client can check rather than a claim the forwarder makes.

## Definitions

- **Original**: the signed event being forwarded.
- **Source channel**: the channel the original was posted in (its `h` tag).
- **Destination channel**: the channel the forward is posted in. DMs are
  ordinary channels with `channel_type = 'dm'`, so channels and DMs are the
  same mechanism throughout this NIP.
- **Forwarder**: the pubkey signing the `kind:40009` event.
- **Note**: the forwarder's optional accompanying message.

## Kinds

| Kind | Name | Signer | Storage | Purpose |
|------|------|--------|---------|---------|
| `40009` | `KIND_STREAM_MESSAGE_FORWARD` | user | regular | A forwarded copy of another message |

`kind:40009` is a regular (stored, append-only) event in the Buzz
stream-messaging family, taking the next free slot after `40008`
`KIND_STREAM_MESSAGE_DIFF`. All kinds are defined in
`crates/buzz-core/src/kind.rs`.

## Event

```jsonc
{
  "kind": 40009,
  "pubkey": "<forwarder_pubkey>",
  "created_at": <unix_seconds>,
  "content": "cc @dana — this is the failure I mentioned",
  "tags": [
    ["h", "9f1c8a20-6d3e-4a51-b7d2-0c5f1e884b31"],
    ["fwd", "{\"id\":\"3b1f…\",\"pubkey\":\"a77c…\",\"created_at\":1780000000,\"kind\":40002,\"tags\":[[\"h\",\"1c2d3e4f-5a6b-7c8d-9e0f-a1b2c3d4e5f6\"],[\"imeta\",\"url https://relay.example/blossom/ab12…\",\"m image/png\",\"x ab12…\"]],\"content\":\"deploy failed: exit 137 on shard 3\",\"sig\":\"9e0d…\"}"],
    ["k", "40002"],
    ["fwd-src", "1c2d3e4f-5a6b-7c8d-9e0f-a1b2c3d4e5f6", "channel"],
    ["q", "3b1f…", "", "a77c…"],
    ["imeta", "url https://relay.example/blossom/ab12…", "m image/png", "x ab12…"],
    ["p", "<dana_pubkey>"]
  ],
  "sig": "…"
}
```

Second example — no note, private source. The `q` tag is absent and the
`fwd-src` type label is `private`:

```jsonc
{
  "kind": 40009,
  "pubkey": "<forwarder_pubkey>",
  "created_at": <unix_seconds>,
  "content": "",
  "tags": [
    ["h", "9f1c8a20-6d3e-4a51-b7d2-0c5f1e884b31"],
    ["fwd", "{\"id\":\"7c4e…\",\"pubkey\":\"b902…\",\"created_at\":1780000500,\"kind\":9,\"tags\":[[\"h\",\"44e9b1c7-2f80-4d13-8a6e-51ba7c093d2a\"]],\"content\":\"rotating the staging token now\",\"sig\":\"1af5…\"}"],
    ["k", "9"],
    ["fwd-src", "44e9b1c7-2f80-4d13-8a6e-51ba7c093d2a", "private"]
  ],
  "sig": "…"
}
```

### `content`

`content` is the forwarder's note in Buzz markdown, or the empty string when
the forwarder adds no note.

`content` SHOULD NOT contain the original's text. The original travels only
inside the `fwd` tag. This is an attribution and search-integrity guideline
rather than a relay-enforceable rule — a relay cannot distinguish quoted text
from a forwarder's own words — but clients SHOULD honour it: the note is
indexed as the forwarder's own words, and full-text search should not
attribute the original author's sentences to the forwarder or return the same
sentence once per forward.

### Tags

| Tag | Cardinality | Meaning |
|-----|-------------|---------|
| `h` | exactly 1 | Destination channel or DM uuid. Standard NIP-29 group scope. MUST be the canonical two-element form `["h", <uuid>]` — trailing elements are rejected. |
| `fwd` | exactly 1 | Stringified JSON of the **complete** original signed event: `id`, `pubkey`, `created_at`, `kind`, `tags`, `content`, `sig`. Maximum 64 KiB. |
| `k` | exactly 1 | The original's kind, stringified. MUST equal the embedded event's `kind` ([NIP-18](18.md) generic-repost convention). |
| `fwd-src` | exactly 1 | `["fwd-src", <source_channel_uuid>, <type>]` where `type` is `channel` (open visibility), `private` (non-open group channel), or `dm` (`channel_type = 'dm'`). The uuid MUST equal the embedded original's own `h` tag, and the embedded original MUST carry exactly one canonical two-element `h` tag. |
| `q` | 0 or 1 | `["q", <original_id>, "", <original_author_pubkey>]` — exactly 4 elements, third empty. Present **only** when `fwd-src` type is `channel`. |
| `imeta` | 0+ | Copied verbatim from the original, preserving attachments ([NIP-92](92.md)). |
| `p` | 0+ | Mentions the forwarder writes in the note, via the standard mention pipeline. |

There is deliberately **no** bare `p` tag for the original author. A forward is
not a mention: p-tagging the original author would fire a false "you were
mentioned" notification on a message they did not participate in and may not be
able to read.

### Forwardable kinds

The original's kind MUST be one of:

| Kind | Name |
|------|------|
| `9` | `KIND_STREAM_MESSAGE` |
| `40002` | `KIND_STREAM_MESSAGE_V2` |
| `45001` | `KIND_FORUM_POST` |
| `45003` | `KIND_FORUM_COMMENT` |

An embedded original of kind `40009` MUST be rejected. Forwarding a forward
flattens client-side: the client forwards the *embedded* original instead, so
forward depth is always exactly 1. This keeps validation cost bounded (no
recursive verification), keeps the 64 KiB cap meaningful (no nesting blowup),
and keeps rendering to a single quoted card.

### Threading

A forward is always a **root** message. Marked `e` tags (`root` / `reply`, per
[NIP-10](10.md)) on a `kind:40009` event MUST be rejected in v1.

A forward may itself become a thread root: replies, reactions, edits of the
note (via the existing `kind:40003` edit), and deletion by its author all work
exactly as on any other message the forwarder owns.

## Relay Behavior

A relay MUST apply a per-kind validator on ingest and **fail closed** — reject
on any doubt. Validation is what upgrades the embedded copy from a claim into a
fact, so a relay that skips it is worse than one that rejects `kind:40009`
outright.

1. **Verifiable, present original.** Exactly one `fwd` tag, parseable as a
   complete event. The relay recomputes the [NIP-01](01.md) event id from
   `(pubkey, created_at, kind, tags, content)` and verifies the Schnorr `sig`,
   reusing the same `buzz-core` verification path as the outer event (and, like
   the outer verify, off the async executor). In addition, the relay MUST
   confirm that the embedded original **exists in this community's event
   store**, stored under the channel named by `fwd-src`. A forward whose
   embedded original is unknown to the relay, or is stored under a different
   channel, MUST be rejected (see Security Considerations → Provenance).
2. **Kind agreement.** The `k` tag equals the embedded `kind`, and that kind is
   in the forwardable allowlist above. Embedded `kind:40009` is rejected.
3. **Source agreement.** The embedded original carries exactly one canonical
   two-element `h` tag (as does the outer event — see Tags), the `fwd-src` uuid
   equals it, the source channel row exists, and the type label
   matches that row's actual `visibility` / `channel_type`. Mismatches are
   rejected — a forwarder MUST NOT be able to label a private source as
   `channel` (or the reverse).
4. **Read access.** The forwarder (`event.pubkey`) MUST be a member of the
   source channel, or the source channel visibility MUST be `open` — the same
   check as the existing channel-membership gate. Without this rule the relay
   would sign off on laundering leaked private content into a public channel:
   the forward would be *verifiably* the original author's words, carrying the
   relay's blessing, published by someone who never had access.
5. **`q` scope.** A `q` tag is allowed only when the `fwd-src` type is
   `channel`; it MUST have exactly four elements with an empty third element,
   its id element MUST equal the embedded original's id, and its pubkey element
   MUST equal the embedded `pubkey`.
6. **Bounds.** The `fwd` tag value MUST be ≤ 64 KiB. Marked `e` tags are
   rejected.
7. **Destination gates are unchanged.** `h` scoping, group token, destination
   membership, archived-channel, ban/timeout, and `imeta` blob verification are
   the existing pipeline. This NIP adds no destination rules; it only requires
   that `kind:40009` flow through them like any other stream message.

Rule 7 is why copied `imeta` tags are safe: the relay re-verifies the
referenced blobs on the forward itself. Blossom blobs are content-addressed and
tenant-scoped, so a same-tenant copy verifies against the same hash without
re-uploading bytes.

## Snapshot Semantics

A forward is a **snapshot** taken at forward time. Later edits
(`kind:40003`) and deletions ([NIP-09](09.md)) of the original do **not**
propagate to any forward of it.

This is deliberate and matches Slack. The embedded event is signed: rewriting
its content on a later edit would invalidate the signature and destroy the
property that makes the copy verifiable, and there is no way to re-sign it
without the original author's key. Propagating deletions would require the
relay to maintain a reverse index from every event to every forward of it
across channel boundaries, and would let a delete reach into channels the
original author never posted in.

Clients SHOULD therefore render the quoted card as of `created_at` of the
forward, and MUST NOT present it as live state. Where a forward's `fwd-src`
type is `channel`, the `q` tag gives a client an optional path to resolve the
current original and surface an "edited since" affordance; this is a
presentation choice, not a protocol requirement.

## Privacy Considerations

**Forwarding out of a private source republishes content.** When the source is
a private channel or a DM, a `kind:40009` event deliberately re-publishes that
content to the destination's members. The relay's rule 4 constrains *who* can
do this (a member of the source) but not *where* — a member may forward into
any channel they can post in. This mirrors Slack, and it mirrors physical
reality: a member of a private channel can already retype anything they read
there. What the protocol adds is attribution integrity, not confinement.
Clients MUST make the consequence visible before the fact: when the source is
private or a DM, the forward dialog shows a one-line notice that forwarding
shares this content with the destination.

**Source identity is not leaked by name, only by uuid.** For non-open sources
the `q` tag is omitted and clients render "Forwarded from a private channel" /
"Forwarded from a direct message" with no link. The embedded original,
however, necessarily carries its own `h` tag — the source channel uuid — because
that tag is covered by the original's signature and cannot be stripped without
breaking verification (rule 1). A uuid is an opaque identifier: it reveals no
channel name, topic, or member list, and it resolves to nothing for a reader
who is not a member. Readers who *are* members of the source learn only that
the message they can already see was forwarded.

**No hidden recipients.** A forward has exactly one destination (`h`). Fan-out
to several places is several events, each individually validated and each
individually visible in its destination.

## Security Considerations

**Provenance — the embedded copy MUST be one the relay has seen.** Signature
verification alone proves only that *some* key signed *some* event; it does not
prove the event was ever posted in the channel the forward claims. Without a
store lookup a forwarder could fabricate an event signed by their own key,
label an arbitrary channel as its source, and have the relay bless it. Relays
therefore MUST verify that the embedded original exists in the destination
community's event store, stored in the channel named by `fwd-src`, in addition
to verifying its NIP-01 id and Schnorr signature (relay rule 1). Forwards
referencing an unknown original, or one scoped to a different channel, MUST be
rejected.

**Freshness — authorize against current membership.** The forwarder's read
access to the source channel (relay rule 4) SHOULD be evaluated against current
membership and visibility state, not against a cached or previously issued
grant: a user removed from a private channel must not be able to forward out of
it. A residual race remains between the authorization check and the store write
when visibility or membership changes concurrently; that race is accepted, and
relays MAY narrow it by re-checking access transactionally with the insert.

**Tag cardinality and shape are normative.** The outer event MUST carry exactly
one `h` tag, and the embedded original MUST carry exactly one `h` tag; anything
else is rejected rather than resolved by picking the first. Both `h` tags MUST
additionally be the canonical two-element shape `["h", <uuid>]`: trailing
elements are rejected rather than ignored, because common event libraries
preserve them and a permissive reader of `["h", <uuid>, <extra>]` could be shown
different data than the access check evaluated. The optional `q` tag MUST be
exactly `["q", <original_id>, "", <original_author_pubkey>]` — four elements,
with an empty relay hint in the third position. Ambiguous multi-`h` events would
otherwise let a forwarder present one scope to the access check and another to
storage or to readers.

**Client trust — verification lives at ingest.** Clients rely on relay-side
ingest verification for the "this is a fact, not a claim" property, so a client
talking to an untrusted or unverified relay inherits that relay's honesty.
Clients SHOULD independently verify the embedded event's id and signature where
feasible (they have the complete signed event in the `fwd` tag), and SHOULD
present a forward whose embedded signature they cannot verify distinctly from a
verified one rather than rendering it as an ordinary quoted message.

Client-side display parsers (TypeScript, Dart) perform display-oriented
structural parsing only: they read the tags a row needs to render and are
deliberately more permissive than the rules above — a duplicated `fwd-src`
resolves to its first occurrence rather than being rejected, and `h` cardinality
and shape are not re-checked. The Tauri desktop backend is a send-side builder
rather than a display parser: it emits the canonical two-element outer `h` and
rejects duplicate `fwd`/`k`/`fwd-src` tags at build time, while still deferring
embedded-event validation and provenance to the relay. Ingest-time validation in
the relay is the single normative gate, so a client MUST NOT treat its own parse
or build acceptance as evidence that a forward is well-formed or that its
provenance was verified.

## Relationship to Other NIPs

- [NIP-18](18.md): `kind:40009` follows the generic-repost convention of
  carrying the stringified original plus a `k` tag, but is not a repost.
  NIP-18 reposts are author-attributed amplifications on a global timeline;
  a forward is a new message *authored by the forwarder*, scoped to one
  destination, with its own note, reactions, and thread.
- [NIP-29](29.md): the reason a reference-only design fails. A `q`-tag-only
  quote (or an `e` tag) is resolvable only by a reader who can read the
  referenced event, and Buzz reads are scoped per `h`: members of the
  destination generally cannot read the source channel, so a bare reference
  renders as a dead link for exactly the audience the forward is for. Embedding
  the signed original is what makes the content readable in the destination
  while keeping it verifiable.
- [NIP-10](10.md): marked `e` tags are rejected on `kind:40009` (see
  Threading). A forward is a root; NIP-10 threading applies to its replies.
- [NIP-92](92.md): `imeta` tags are copied verbatim so attachments survive the
  forward.
- [NIP-09](09.md): deleting a forward deletes the forward only, never the
  original; deleting the original does not affect existing forwards.

## Out of Scope (v1)

- **Threads as destinations.** `h` names a channel; forwarding into a specific
  thread would need a destination thread reference and conflicts with the
  "forward is always a root" rule.
- **Multiple destinations per forward.** One `h` per event; a client wanting
  three destinations publishes three events.
- **Scheduled forwards.** `kind:40006` scheduled messages are a separate
  mechanism and are not composed with forwarding here.
- **External [NIP-17](17.md) gift-wrapped sources or destinations.** A
  gift-wrapped rumor is unsigned by construction, so it cannot satisfy rule 1 —
  there is nothing to verify, and an unverifiable embedded copy is exactly the
  hand-retyped paste this NIP replaces.
- **Forward to a new channel.** The "forward, creating a channel for it"
  variant discussed in
  [#3264](https://github.com/block/buzz/issues/3264) is a channel-creation
  flow layered on top of this event, not a change to it.
