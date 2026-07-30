---
id: attribute-context-contributions
type: story
refs:
  journey: specs/journeys/re-enter-a-bounded-context.md
  persona: specs/personas/sovereign-builder.md
  steps: [4]
---

# Story: Attribute context contributions

## Narrative

As a sovereign builder,
I want every element of a brief attributed to the identity that signed it,
So that I weigh each statement by who stands behind it — especially where
counterpart and steward identities write into the same context.

## Acceptance Criteria

- [ ] Every brief element names its signing identity; agent and machine labels are shown where present and are presentation metadata, never authority.
- [ ] A contribution whose signer carries a cryptographically valid, current owner attestation renders as verified.
- [ ] An expired or absent attestation renders as unverified; unverified content is displayed, but never indistinguishably from verified content.
- [ ] The builder's own agents, the mandated steward, and counterpart identities render as visibly distinct classes of contributor.
- [ ] Provenance display requires no relationships beyond identities already admitted to the context.
- [ ] Provenance is available for heads as well as residue: a distilled head names the agent that proposed it.

## Notes

Provenance visibility costs nothing to keep because every record is already
signed; it would be expensive to retrofit if signing discipline lapsed.
Display is the deliverable of this story — the cryptography already exists.
