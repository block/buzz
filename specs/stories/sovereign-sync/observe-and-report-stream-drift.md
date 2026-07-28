---
id: observe-and-report-stream-drift
type: story
refs:
  journey: specs/journeys/detect-and-reconcile-stream-drift.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [1, 2, 3, 4]
---

# Story: Observe and report stream drift

## Narrative

As a sovereign node operator,
I want a narrowly delegated steward to report agreement drift,
So that relationship changes become visible without delegating configuration authority.

## Acceptance Criteria

- [ ] The steward requires an active mandate naming its own signing key.
- [ ] `observe` power is required before inspecting declaration state.
- [ ] `report` power is required before publishing a report.
- [ ] A revoked, missing, or observe-only mandate cannot publish.
- [ ] The steward evaluates governance independently for each node and domain.
- [ ] Findings distinguish open, unoffered, unpinned, stale, revoked, and transport-unhealthy states.
- [ ] Reports carry the shared-context `h` tag named by the reporting mandate.
- [ ] Unchanged findings are not reposted.
- [ ] The steward never receives declaration-writing authority or private transport keys.

## Notes

The steward reports derived observations. Its output is not configuration and
does not confer trust.
