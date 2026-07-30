---
id: distill-a-context-head
type: story
refs:
  journey: specs/journeys/re-enter-a-bounded-context.md
  persona: specs/personas/sovereign-builder.md
  steps: [6]
---

# Story: Distill a context head

## Narrative

As a sovereign builder,
I want current understanding to be distilled from accumulated residue and
held in a named head cell,
So that the next re-entry starts from synthesis rather than from raw
history.

## Acceptance Criteria

- [ ] The head is an addressable event whose replacement semantics keep exactly one current cell per context; it carries the same `h` boundary as the residue it distills.
- [ ] Distillation is proposed by the working agent from residue accumulated since the previous head, and accepted by the builder; it is a synthesis of what already accreted, not a composition from scratch.
- [ ] The head records state, open threads, and rationale — enough to orient, not to replay.
- [ ] A context whose residue is newer than its head reads as accreting; a head far behind its residue reads as stale and is surfaced, not hidden.
- [ ] Replacing the head never destroys residue; history remains replayable beneath every distillation.
- [ ] One head per bounded context: a head that silently absorbs several efforts indicates a boundary that should split.

## Notes

Freshness is a relation between two things the relay already stores: the
head cell's timestamp and the newest residue in the same boundary. No new
state is required to detect staleness.
