---
id: reconcile-stream-agreement-drift
type: story
refs:
  journey: specs/journeys/detect-and-reconcile-stream-drift.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [5, 6]
---

# Story: Reconcile stream agreement drift

## Narrative

As a sovereign node operator,
I want each drift condition to require an explicit owner decision,
So that recovery cannot silently broaden or resurrect trust.

## Acceptance Criteria

- [ ] An unchanged offer may be re-pinned only by the destination owner.
- [ ] A changed event selection requires a new stream ID rather than reusing the old agreement.
- [ ] Transport-key rotation replaces the relevant grants without changing owner identity.
- [ ] Revocation remains effective and never falls back to stale file or environment trust.
- [ ] Historical declaration events remain durable after replacement or revocation.
- [ ] Current agreement state is recomputed from effective heads after reconciliation.
- [ ] Replication resumes only when required agreement and transport checks pass.

## Notes

Reconciliation changes owner-signed declarations. A steward may recommend an
action but cannot perform it under observe/report powers.
