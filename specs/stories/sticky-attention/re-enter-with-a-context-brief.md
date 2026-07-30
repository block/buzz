---
id: re-enter-with-a-context-brief
type: story
refs:
  journey: specs/journeys/re-enter-a-bounded-context.md
  persona: specs/personas/sovereign-builder.md
  steps: [1, 2, 3]
---

# Story: Re-enter with a context brief

## Narrative

As a sovereign builder,
I want re-entry into a bounded context to produce an orientation projected
from its events,
So that I resume work in under a minute on any surface without recalling
addresses or replaying transcripts.

## Acceptance Criteria

- [ ] Bounded contexts are listed by warmth — newest last-touch first — with title, last-touch time, and touching identity; selection requires recognition, not slug recall.
- [ ] The attention cursor is derived, not stored: the newest session record in the context authored by one of the builder's own attested agents.
- [ ] Records authored by steward or counterpart identities never move the cursor.
- [ ] The brief renders four registers: current understanding (heads), live thread (recent residue in causal order), open loops (undischarged obligations), and changes since the cursor.
- [ ] Changes since the cursor list foreign contributions first.
- [ ] The brief is a deterministic fold: the same context events produce the same brief on every surface — CLI, desktop, or any agent harness.
- [ ] The brief is an orientation, not a transcript: raw history remains one step away but is never the default rendering.
- [ ] Projection reads only the local sovereign relay; no other node is consulted.

## Notes

The brief is the attention twin of the steward's report: one deterministic
reduction over an event union, pointed at orientation instead of governance.
