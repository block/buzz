---
id: delegate-work-through-journal-handoff
type: journey
refs:
  persona: specs/personas/sovereign-node-operator.md
---

# Journey: Delegate work through a journal handoff

## Actor

A sovereign node operator acting as the delegating owner. The secondary
participant is a counterparty operator whose owner-attested agent principal
performs the work.

Persona: `specs/personas/sovereign-node-operator.md`

## Trigger

The delegating owner wants a named counterparty agent to perform a bounded
piece of work against a known repository state, with every step of the
delegation durable, signed, and independently re-derivable from the journal.

## Preconditions

- Both parties share a NIP-29 context (`h` tag) admitted on their nodes.
- The delegating owner signs directly or the opening agent carries a valid
  NIP-OA owner attestation.
- Candidate claimant keys are known; each is either a root signer or an agent
  key whose owner attestation the claimant will attach.
- The work is anchored to a canonical, full 40-character Git object ID that
  both parties can resolve.
- The contract for the work exists before the offer: scope, acceptance, and
  embodiment are written down, not implied.

## Flow

### 1. Anchor the work to the repository

- **User intent**: Delegate against one exact repository state, not a branch
  name that may move.
- **System response**: The offer names a canonical lowercase 40-character
  `base_commit`. A runner additionally requires that the commit resolves
  without identity change and is an ancestor of its configured trusted ref.
- **Next**: Write the contract.

### 2. State the contract of the work

- **User intent**: Say what is wanted, how completion will be judged, and
  within which execution boundary the work may embody itself.
- **System response**: The offer carries a scope, an acceptance contract, and
  an embodiment contract (stdin policy, network policy, trust boundary,
  tooling). Contract prose duplicated from repository specifications should
  instead reference them at the anchored commit; the journal carries the
  commitment, the repository carries the contract.
- **Next**: Publish the offer.

### 3. Publish the open

- **User intent**: Make the delegation durable and attributable.
- **System response**: A signed kind-1 event carries `t=handoff:open`, exactly
  one `h` context, one or more `p` tags naming the only keys allowed to
  claim, the `base_commit`, the contracts, and optional referenced artifact
  `x` tags. The opener's effective owner — the author, or the owner recovered
  from exactly one valid event-covering NIP-OA `auth` tag — becomes the only
  owner identity that may later verify and close.
- **Next**: Let a named claimant answer.
- → `POST /events`

### 4. Claim the work

- **User intent**: Commit one accountable principal to the work.
- **System response**: A signed claim links the open with an `e/root` tag, is
  authored by a key named in an open `p` tag, and posts strictly after the
  open. Runner and host labels in content are presentation only; they confer
  no authority and cannot substitute for the signature.
- **Next**: Evaluate claim exclusivity.
- → `POST /events`

### 5. Reduce claims deterministically across both views

- **User intent**: Know who holds the work, even when both nodes were written
  independently.
- **System response**: The lifecycle is reduced from the union of the
  sovereign and rendezvous views. Duplicate claims from one signer reduce to
  that signer's earliest `(created_at, id)` claim. Claims from more than one
  authorized signer reduce to `CONFLICT`, and no return is accepted until the
  conflict is resolved. Unauthorized claim signers are ignored.
- **Next**: Perform the work inside the embodiment boundary.

### 6. Execute within the declared boundary

- **User intent**: Let the claimant work without inheriting the host's
  authority.
- **System response**: Host-mode execution is manual and explicit. The
  sandboxed contract requires a digest-pinned image, read-only root, dropped
  capabilities, resource limits, an ephemeral workspace, and no inherited
  credentials, agent sockets, or private keys. Unattended execution remains
  disabled until the relay enforces claim exclusivity, because
  query-then-post is not compare-and-set.
- **Next**: Return the work (see the return-and-close journey).

## Outcomes

- **Success**: One authorized claimant holds the work; the open's contract,
  anchor commit, and allowed claimants are re-derivable by any party from
  signed events alone.
- **Failure modes**:
  - the `base_commit` is abbreviated, mixed-case, or unresolvable, so the
    open is invalid and every dependent transition is orphaned;
  - contract prose is inlined instead of referencing repository
    specifications, so intent forks from the repo and cannot be amended;
  - two authorized signers claim and the lifecycle parks in `CONFLICT`;
  - a runner label is trusted instead of the claim signature;
  - a stale or mistaken open cannot be retired — v0.1 has no withdraw or
    acknowledgment transition, so invalid opens are re-reported until a
    v0.2 retirement primitive exists.

## Related Stories

- `specs/stories/journal-handoff/open-attributable-work-offer.md`
- `specs/stories/journal-handoff/claim-delegated-work-exclusively.md`

## E2E Coverage

- Implemented: reducer self-tests in
  `crates/buzz-local-relay/src/bin/buzz-handoff-state.rs`
- Planned: `crates/buzz-test-client/tests/e2e_journal_handoff.rs`
