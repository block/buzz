---
id: detect-and-reconcile-stream-drift
type: journey
refs:
  persona: specs/personas/sovereign-node-operator.md
---

# Journey: Detect and reconcile sovereign stream drift

## Actor

The sovereign node operator reviewing observations from a delegated steward.
The steward is a secondary actor limited to observation and reporting.

Source Persona: `specs/personas/sovereign-node-operator.md`

## Trigger

A declaration head, transport key, stream selection, or node health changes
after an agreement was proposed or matched.

## Preconditions

- Declaration history is durable and current heads are queryable.
- The steward has an active owner-signed mandate naming its key and powers.
- Any report destination is a named shared context.
- The operator retains exclusive authority to alter declarations and keys.

## Flow

### 1. Verify the steward mandate

- **User intent**: Receive observations only from a currently authorized agent.
- **System response**: The steward verifies an active mandate for its own key,
  requires `observe` before inspecting, and requires `report` before publishing.
- **Next**: Evaluate current state.

### 2. Observe heads and runtime health

- **User intent**: Compare declared intent with current operational state.
- **System response**: The steward reads effective declaration heads, evaluates
  each governed domain independently, and checks node health without receiving
  configuration authority or private transport keys.
- **Next**: Classify findings.

### 3. Classify agreement state

- **User intent**: Understand exactly why a relationship is not current.
- **System response**: The steward distinguishes open offers, unoffered admits,
  missing pins, stale pins, revoked heads, absent read grants, unscoped
  declarations, and transport-health failures.
- **Next**: Report only if authorized and changed.

### 4. Publish a bounded report

- **User intent**: Keep drift visible in the relationship's own history.
- **System response**: With `report` power, the steward signs a report carrying
  the mandated shared-context `h` tag. Without that power it prints locally
  only. Unchanged findings are not reposted.
- **Next**: Operator reviews the finding.
- → `POST /events`

### 5. Choose an explicit reconciliation

- **User intent**: Resolve drift without silently broadening trust.
- **System response**: The operator may re-pin an unchanged offer, revoke a
  declaration, authorize a rotated transport key, or define a new stream ID
  when selection semantics changed.
- **Next**: Publish the chosen owner-signed declaration.

### 6. Re-evaluate current heads

- **User intent**: Confirm that the relationship converged.
- **System response**: Matching and transport-readiness checks run again from
  current heads. Historical disagreement remains durable while only current
  active heads confer authority.
- **Next**: Resume replication or continue investigation.

## Outcomes

- **Success**: Drift is attributable, reported within delegated powers, and
  resolved only through an explicit owner decision.
- **Failure modes**:
  - a revoked or observe-only mandate publishes a report;
  - a steward changes configuration directly;
  - an unpinned admit is reported as matched;
  - transport health is confused with agreement state;
  - changing stream selection reuses the old stream ID;
  - report deduplication hides a materially changed finding.

## Related Stories

- `specs/stories/sovereign-sync/observe-and-report-stream-drift.md`
- `specs/stories/sovereign-sync/reconcile-stream-agreement-drift.md`

## E2E Coverage

- Planned: `crates/buzz-test-client/tests/e2e_sovereign_steward.rs`
