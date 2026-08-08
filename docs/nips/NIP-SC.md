NIP-SC
======

Surface Cards
-------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format, filters), NIP-10 (thread markers), NIP-29 (relay-based groups)

## Abstract

This NIP defines the **surface card**: a channel-scoped event whose content is a
small, versioned, **data-only** JSON document that clients render as a native
card. A surface describes *what* to show — headings, text, badges, key/value
lists, stat grids, tables, progress bars — never *how* to lay it out, and never
executable markup. The author (human via CLI, or an agent) controls content;
the client owns presentation.

Because a surface is an ordinary signed event, it works the same whether the
author is a frontier model, a tiny local model, or a person, and it inherits the
existing NIP-29 scoping, membership auth, and audit trail. Live updates reuse
the existing edit kind (`kind:40003`) with full-spec replacement, so a single
card can move through states — healthy → incident → recovered — inside one
message row.

## Motivation

Agents increasingly need to present *structured* status inside a conversation:
a deploy dashboard, an incident summary, a checklist, a metrics panel. Two
existing options are both wrong:

- **Markdown in a `kind:9` message.** Markdown is a presentation language with
  links, images, and (via some renderers) HTML. Letting an agent emit markdown
  to describe a status panel reopens exactly the link/media/script surface a
  data-only model is trying to close, and it renders inconsistently across
  clients.
- **A general "generative UI" payload** (arbitrary component trees, embedded
  scripts, or a third-party UI runtime). This couples a durable signed event to
  a fast-moving vendor format and hands the author control over layout and
  behavior — the opposite of what a shared, auditable workspace wants.

A surface card is the narrow middle: a fixed, versioned vocabulary of display
nodes carrying only data. Clients that do not implement this NIP ignore the
kind entirely (zero breaking changes); clients that do render a native card, and
on any parse failure fall back to a plain-text summary the author always
provides.

## Non-Goals

- **No interactivity in v1.** Buttons, forms, and inputs are out of scope. A
  future revision MAY add an `actions` node whose only effect is to prefill a
  client composer with a proposed reply; it MUST never execute code, carry a
  URL, or auto-send.
- **No layout control.** The author cannot set widths, colors (beyond the fixed
  semantic tone set), positions, or fonts. Rendering is entirely the client's.
- **No streaming.** Events arrive whole; there is no partial-spec/patch wire
  form. Updates are full-spec replacements (§Updates).
- **No new transport.** A surface is a normal event on the existing submit and
  query surfaces. This NIP adds no endpoint and no envelope.
- **Not searchable.** Surface content is machine-generated JSON; relays SHOULD
  exclude the surface kind from full-text search so spec keys do not pollute the
  index. (A future revision MAY index `fallbackText` only.)

## Terminology

This document uses MUST, MUST NOT, SHOULD, MAY, and RECOMMENDED as defined in
RFC 2119.

- **surface event**: A stored, signed event of the surface kind whose `content`
  is a canonical `SurfaceSpec` JSON document.
- **spec**: The `SurfaceSpec` document — the parsed `content`.
- **node**: One entry in the spec's `nodes` array; a single display element.
- **tone**: A semantic status enum applied to some nodes:
  `default | success | warning | danger | info`. Tone selects a client theme
  token; it is never a raw color.
- **scalar**: A cell/value primitive — a JSON string or a finite JSON number.
  Booleans, `null`, objects, and arrays are not scalars.

## Event

- **kind**: `40110` (proposed; the maintainer assigns the final number). Regular,
  stored, non-replaceable — the same storage class as a stream message.
- **content**: canonical `SurfaceSpec` JSON (§Spec).
- **tags**:
  - `["h", "<channel-uuid>"]` — REQUIRED. The channel, per NIP-29.
  - NIP-10 thread markers — OPTIONAL. A surface MAY be a thread root or a reply.
  - `["p", "<pubkey-hex>"]` — OPTIONAL explicit mentions.

A surface is conversational content: relays and clients MUST treat it like a
`kind:9` message for unread state, home/activity feeds, and mention matching.

## Spec

```jsonc
{
  "version": 1,
  "fallbackText": "Deploy v2.4.1: 2/2 pods running, rollout 100%",
  "title": "Deployment — api-gateway",
  "nodes": [ /* 1..32 nodes, rendered in order */ ]
}
```

- `version` — REQUIRED integer, MUST be `1`.
- `fallbackText` — REQUIRED, non-empty plain text, ≤ 512 characters. This is what
  non-rendering clients and every failure path display. It is the author's
  contract that the card degrades to a meaningful sentence.
- `title` — OPTIONAL plain text, ≤ 256 characters.
- `nodes` — REQUIRED array, 1–32 entries.

Character counts are **Unicode scalar values** (code points), not UTF-16 code
units or bytes, so the same string is judged identically by a Rust relay and a
JavaScript client.

### Node catalog (v1)

```jsonc
{"type": "heading",  "text": "Section heading"}
{"type": "text",     "text": "Free-form paragraph (plain text, never markdown)"}
{"type": "badge",    "text": "2/2 RUNNING", "tone": "success"}
{"type": "keyValue", "items": [{"label": "Version", "value": "v1.2.3", "tone": "info"}]}
{"type": "statGrid", "stats": [{"label": "Pods", "value": 2, "delta": "+1", "tone": "success"}]}
{"type": "table",    "columns": ["Pod", "Status"], "rows": [["web-7d9f", "Running"]]}
{"type": "progress", "label": "Rollout", "value": 80}
```

`tone` is OPTIONAL everywhere it appears and defaults to `default`.

### Structural limits

| Field | Limit |
|---|---|
| canonical `content` | ≤ 32 KiB |
| `nodes` | 1–32 |
| `title` | ≤ 256 chars |
| `fallbackText` | 1–512 chars |
| `text` node body | ≤ 4096 chars |
| any label / value / cell / delta | ≤ 512 chars |
| `keyValue.items`, `statGrid.stats` | ≤ 32 each |
| `table` | ≤ 12 columns, ≤ 100 rows |
| cell / value type | string or finite number only |
| `progress.value` | finite number; clients clamp to 0–100 |

The 32 KiB content cap exists because a Nostr relay's frame limit bounds the
whole event (tags + signature included); 32 KiB leaves generous headroom for
data-heavy tables while keeping worst-case render work bounded.

## Canonicalization

Producers MUST serialize the spec **canonically** before signing: stable field
order and no insignificant whitespace, so byte-identical specs produce
byte-identical content (and event ids). Producers SHOULD accept and normalize
known field aliases (e.g. a `table` node's `fields`/`headers` → `columns`)
*before* canonicalizing, so authored convenience never reaches the wire.

## Relay behavior (strict gate)

For a surface event, and for any `kind:40003` edit whose target is a surface,
the relay MUST:

1. Apply the same auth as a stream message: messages-write scope, channel
   membership, and the required `h` scope.
2. Parse `content` as a `SurfaceSpec` and reject on any of: unknown top-level
   or node field, unknown `type`, unknown `tone`, a non-scalar or non-finite
   cell/value, a table row whose length ≠ the column count, `version` ≠ 1, any
   structural-limit violation, or content exceeding 32 KiB.

Rejection MUST carry a field-specific reason (e.g.
`nodes[3].table: 14 columns exceeds max 12`) so a producer — human or agent —
can repair the payload without a round-trip.

The relay is the strict gate. Tolerance (below) lives only in clients, for
historical or foreign events signed by other implementations.

## Updates (edit semantics)

A live update is a `kind:40003` edit event (NIP-29 message edit) whose `e` tag
targets the surface and whose `content` is the **full replacement spec**. There
are no partial patches.

- The edit's `content` MUST itself validate as a `SurfaceSpec` (relays enforce
  this; see §Relay behavior).
- Only the surface's author — or, for an agent-authored surface, the agent's
  owning human — may edit it.
- The edit's channel MUST match the target surface's channel.
- Latest edit wins. Because `created_at` has one-second resolution, clients MUST
  order edits by `created_at`, breaking ties on the **lexicographically
  smallest** event id, so two same-second edits resolve identically on every
  client.

  Smallest-id is not arbitrary: a relay that orders results
  `created_at DESC, id ASC` returns the winner as the first row, so a reader
  can resolve current state with a one-row lookup per target instead of
  fetching an unbounded edit history. The consequence is worth stating plainly:
  a burst of edits inside a single second has no defined "last" — every reader
  converges on the same one, but which one is arbitrary. Authors who need a
  specific final state should let a second elapse.

## Client behavior (tolerant)

A rendering client parses `content` and applies this fallback matrix:

| Condition | Render |
|---|---|
| valid v1 spec | native card |
| `version` ≠ 1 / unknown version | `fallbackText` as plain text |
| some nodes invalid | drop the invalid nodes, render the survivors |
| unknown `tone` | coerce to `default`, keep the node |
| numeric cell/value | render the number as text |
| zero valid nodes | `fallbackText` as plain text |
| JSON unparseable, or `fallbackText` missing/blank | escaped raw `content` as plain text |

A client MUST NOT route any failure path through a markdown renderer, and MUST
NOT render a blank row or an error row. When a client can salvage a usable
`fallbackText` from an otherwise-unparseable envelope (e.g. a future spec
version whose node shapes it does not understand), it SHOULD prefer that plain
text over raw JSON.

Clients that do not implement this NIP simply do not match the surface kind in
their timeline filters; the event is invisible to them. This is the intended
degradation.

## Rationale

**Why a Buzz-owned schema rather than an existing "generative UI" format.**
A signed event is durable protocol: it lives forever and must render the same in
a Rust relay, a React desktop client, and a future Flutter client. That argues
for a tiny, stable, trivially-reimplementable vocabulary, not a fast-moving
third-party runtime format. Generation-side token cost (the usual argument for
richer formats) is decoupled from the wire: an agent MAY generate in any format
and transpile to canonical `SurfaceSpec` before signing.

**Why data-only.** The workspace's whole security model is "signed, inspectable,
no scripts." Layout/behavior control would hand that back to the author. Fixing
the vocabulary keeps every card auditable and consistent, and lets clients
enforce accessibility (tone is never color-alone) and zoom behavior uniformly.

**Why edit-based updates.** Reusing `kind:40003` means a card and its whole
history live in one message row with one audit trail, instead of spawning a new
row per update that clients would have to de-duplicate.
