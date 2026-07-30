---
id: claim-delegated-work-exclusively
type: story
refs:
  journey: specs/journeys/delegate-work-through-journal-handoff.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [4, 5, 6]
---

# Story: Claim delegated work exclusively

## Narrative

As an accountable agent principal,
I want my claim on offered work to be authorized by signature and reduced
deterministically against competing claims,
So that exactly one principal is answerable for the work at any time.

## Acceptance Criteria

- [ ] The claim links its open with `["e", "<open>", "", "root"]` and posts strictly after the open.
- [ ] The claim is signed by a key named in an open `p` tag; unauthorized signers are ignored.
- [ ] Runner, host, and agent labels in content are presentation metadata and confer no authority.
- [ ] Duplicate claims from one signer reduce to that signer's earliest `(created_at, id)` claim.
- [ ] Claims from more than one authorized signer reduce deterministically to `CONFLICT`, and no return is accepted while the conflict stands.
- [ ] The lifecycle is reduced from the union of the sovereign and rendezvous views, not from either node alone.
- [ ] Unattended execution stays disabled until the relay supplies an exclusive claim primitive, because query-then-post is not compare-and-set.

## Notes

Deterministic conflict is a reporting guarantee, not mutual exclusion. The
missing relay-enforced claim transaction is the single gate on autonomous
execution and is the highest-leverage v0.2 primitive.
