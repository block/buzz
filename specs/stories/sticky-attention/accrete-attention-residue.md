---
id: accrete-attention-residue
type: story
refs:
  journey: specs/journeys/re-enter-a-bounded-context.md
  persona: specs/personas/sovereign-builder.md
  steps: [5]
---

# Story: Accrete attention residue

## Narrative

As a sovereign builder,
I want attention to leave signed traces as a side effect of working,
So that the context accumulates memory continuously instead of depending on
a closing ceremony I may skip.

## Acceptance Criteria

- [ ] Working agents append session records at meaningful moments — decisions, landings, reversals — during the work, not only at session end.
- [ ] Every residue record carries the context's `h` boundary; residue without an `h` boundary belongs to no context.
- [ ] Residue records are signed by the writing agent's own bound key with its owner attestation attached.
- [ ] Residue is durable the moment it is accepted by the local relay and appears in the next brief projection without further action.
- [ ] A session that ends abruptly loses at most the residue not yet written, never the context's prior memory.
- [ ] No step of accretion requires the builder to name a scope, compose a summary, or run a save command.

## Notes

The existing session log discipline already demonstrates this: agent-signed
one-line records written mid-work. The story binds that discipline to the
`h` boundary and removes the remaining ceremony.
