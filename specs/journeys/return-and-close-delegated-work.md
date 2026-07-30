---
id: return-and-close-delegated-work
type: journey
refs:
  persona: specs/personas/sovereign-node-operator.md
---

# Journey: Return and close delegated work

## Actor

The claimant agent principal returning finished work, then the delegating
owner verifying and closing it. An independent third node may verify result
artifacts without either party's cooperation.

Persona: `specs/personas/sovereign-node-operator.md`

## Trigger

The claimant has finished (or definitively failed) the delegated work and
wants the outcome, its evidence, and its custody to become part of the
durable lifecycle.

## Preconditions

- The handoff has exactly one accepted claim and is not in `CONFLICT`.
- Result artifacts, if any, exist as content-addressed blobs.
- The rendezvous custodian is reachable for artifact custody checks.
- The delegating owner (or an attested signer for that owner) is available to
  verify and close.

## Flow

### 1. Prepare verifiable evidence

- **User intent**: Return claims that a verifier can recompute, not prose to
  be believed.
- **System response**: Evidence names exact commits and content-addressed
  result artifacts. `buzz-ctx announce` uploads bytes to both the sovereign
  and rendezvous stores, posts the manifest, synchronizes it, and re-reads
  the bytes through the rendezvous to confirm the advertised SHA-256.
- **Next**: Publish the return.

### 2. Publish the return

- **User intent**: Bind the outcome to the exact claim that authorized it.
- **System response**: A signed kind-1 event carries `t=handoff:return`, links
  the open with `e/root` and the accepted claim with `e/claim`, restates the
  `claim_id` in content, posts strictly after the claim, is signed by the
  exact cryptographic claimant, and carries status `done` or `failed`.
  Before the return may advertise an `x` tag, the manifest must exist at the
  rendezvous, the bytes must be readable through the authenticated artifact
  path, and the fetched bytes must hash to the advertised digest.
- **Next**: Verify independently.
- → `POST /events`

### 3. Verify result artifacts from an independent node

- **User intent**: Trust custody, not the returner's account of custody.
- **System response**: `buzz-ctx handoff verify-artifacts <return-event-id>`
  reads the exact return from the rendezvous, requires its content artifact
  list to equal its `x` tags, fetches every blob through the verifying
  node's own authenticated reader identity, and byte-compares every
  advertised SHA-256.
- **Next**: Judge the work against the acceptance contract.

### 4. Verify against the acceptance contract

- **User intent**: Decide whether the returned work satisfies what the open
  asked for.
- **System response**: In v0.1 this judgment belongs solely to the opener's
  effective owner. Where the acceptance contract references executable
  checks in the repository at the anchored commit, the judgment reduces to
  re-running them; where it is prose, the close records a single-party
  decision.
- **Next**: Publish the close.

### 5. Publish the close

- **User intent**: Terminate the lifecycle over the exact verified return.
- **System response**: A signed close links the open with `e/root` and the
  verified return with `e/return`, restates the `return_id` in content,
  posts after that return, and is signed directly by the opener owner or by
  a signer carrying that owner's valid attestation. Only a close for the
  newest valid return produces `CLOSED`; a close for an earlier return
  cannot suppress a later one, and a legacy close that identifies only the
  open is not causally valid.
- **Next**: Let observers confirm quiescence.
- → `POST /events`

### 6. Observe the settled lifecycle

- **User intent**: Know that stewards and counterparties see the same
  terminal state.
- **System response**: The mandated steward's next cycle reduces the same
  event union and reports the handoff closed rather than open, claimed, or
  invalid. Steward findings are observations; they never mutate lifecycle
  state.

## Outcomes

- **Success**: The lifecycle reads `open -> claim -> return -> close` from
  signed events alone; every advertised artifact was independently fetched
  and byte-verified; the steward reports quiescence.
- **Failure modes**:
  - a return is signed by an agent of the claimant's owner rather than the
    exact claimant key, and is ignored;
  - an `x` tag advertises bytes the rendezvous cannot serve, so the return
    is refused before posting;
  - a close pins a superseded return and fails to terminate the lifecycle;
  - the opener's owner is the only permitted judge, so acceptance verdicts
    are single-party where the contract is prose — a v0.2 pressure toward
    executable acceptance;
  - a `failed` return closes the delegation record but the underlying
    intent silently evaporates instead of producing a revised open.

## Related Stories

- `specs/stories/journal-handoff/return-verifiable-work-evidence.md`
- `specs/stories/journal-handoff/close-verified-handoff.md`

## E2E Coverage

- Implemented: reducer self-tests in
  `crates/buzz-local-relay/src/bin/buzz-handoff-state.rs`
- Planned: `crates/buzz-test-client/tests/e2e_journal_handoff.rs`
