---
id: re-enter-a-bounded-context
type: journey
refs:
  persona: specs/personas/sovereign-builder.md
---

# Journey: Re-enter a bounded context

## Actor

The sovereign builder, working through any surface on their own node — CLI,
desktop app, or an agent session (Claude Code, Codex, or another harness).
Agents participate as owner-attested principals; a mandated steward and
admitted counterpart identities may also have written into the context.

Persona: `specs/personas/sovereign-builder.md`

## Trigger

The builder returns to a piece of work — after an hour, a night, or a week —
and wants to resume with full orientation instead of reconstructing state
from memory or transcripts.

## Preconditions

- The local sovereign relay is running; no other node is required.
- The work lives in one bounded context: a single NIP-29 `h` identifier that
  scopes its heads, session records, notes, lifecycle events, and artifact
  references alike. Heads outside any `h` boundary are legacy state awaiting
  migration, not members of a context.
- The builder's agents sign with bound keys carrying owner attestations;
  every durable record in the context is signed by someone.
- Distribution of software (relay, CLI, skills) between machines is ordinary
  package/repository distribution and out of scope here; so are node
  relationships and cross-node context mapping.

## Flow

### 1. Name or find the boundary

- **User intent**: Return to "the portable relay work," not to a slug.
- **System response**: The surface lists bounded contexts ordered by warmth —
  most recently touched first — each with its title, last-touch time, and
  the identity that touched it. Choosing one requires recognition, not
  recall.
- **Next**: Locate the attention cursor.

### 2. Locate the attention cursor

- **User intent**: Establish "when I was last here" so change has a
  reference point.
- **System response**: The cursor is derived, never stored: the newest
  session record in this context authored by any of the builder's own
  attested agents. Records by the steward or counterpart identities do not
  move it.
- **Next**: Project the brief.

### 3. Project the context brief

- **User intent**: Be oriented, not replayed.
- **System response**: A deterministic fold over the context's events,
  rendered in four registers: current understanding (the context's
  addressable heads), the live thread (recent residue in causal order),
  open loops (undischarged obligations — claimed handoffs, unanswered
  offers, standing questions), and changes since the cursor (everything
  newer than the builder's last touch, foreign contributions first). The
  same fold on the same events yields the same brief on every surface.
- **Next**: Read provenance alongside content.
- → `POST /query`

### 4. Attribute every element

- **User intent**: Weigh each statement by who stands behind it.
- **System response**: Every brief element names its author identity and,
  where labels exist, agent and machine. A valid owner attestation renders
  as verified; an expired or missing attestation renders as unverified;
  counterpart and steward identities are visibly distinct from the
  builder's own agents. Provenance is displayed, never silently blended.
- **Next**: Resume the work.

### 5. Work, leaving residue without ceremony

- **User intent**: Think about the work, not about memory upkeep.
- **System response**: Agents append session records at meaningful moments
  during the work — decisions, landings, reversals — as a side effect of
  acting. The residue accretes in the context's history the moment it is
  written; the brief reflects it on the next projection.
- **Next**: Distill when understanding changes.
- → `POST /events`

### 6. Distill the head from accumulated residue

- **User intent**: Update current understanding without composing a
  summary from scratch.
- **System response**: Distillation replaces the context's head cell with a
  synthesis of the residue since the previous head — state, open threads,
  and rationale — proposed by the working agent and accepted by the
  builder. A context whose residue is newer than its head reads as
  accreting; a long-idle gap reads as stale. The head is a named
  addressable cell in the same `h` boundary, so the next re-entry's
  "current understanding" is exactly this distillation.
- **Next**: Leave; the room keeps its shape.
- → `POST /events`

## Outcomes

- **Success**: Cold start to first meaningful action in under a minute, on
  any surface, with every claim in the brief attributable and the head
  current with the residue.
- **Failure modes**:
  - heads and residue live under different boundary mechanisms (slug
    namespace vs `h` tag), so the projection cannot see the whole context;
  - the brief degrades into a transcript dump and re-entry cost returns;
  - the cursor is computed from another identity's records and "changes
    since" lies;
  - unattributed or unverified content renders indistinguishably from
    attested content;
  - distillation stays manual-only ceremony, heads go stale, and one head
    silently absorbs many efforts;
  - navigation by question, cross-context warmth ranking, and cross-node
    context mapping are attempted before single-node continuity is solid —
    all three are deliberately out of scope (v2).

## Related Stories

- `specs/stories/sticky-attention/re-enter-with-a-context-brief.md`
- `specs/stories/sticky-attention/accrete-attention-residue.md`
- `specs/stories/sticky-attention/distill-a-context-head.md`
- `specs/stories/sticky-attention/attribute-context-contributions.md`

## E2E Coverage

- Planned: `crates/buzz-test-client/tests/e2e_context_brief.rs`
