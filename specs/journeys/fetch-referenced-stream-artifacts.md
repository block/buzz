---
id: fetch-referenced-stream-artifacts
type: journey
refs:
  persona: specs/personas/sovereign-node-operator.md
---

# Journey: Fetch artifacts referenced by a replicated stream

## Actor

The sovereign node operator receiving events whose manifests reference
content-addressed blobs.

Source Persona: `specs/personas/sovereign-node-operator.md`

## Trigger

A newly replicated event contains an `x` tag for an artifact that is absent
from the destination's local artifact store.

## Preconditions

- The destination has already accepted the referencing event.
- The artifact is named by its SHA-256 digest.
- The custodian can determine which exported stream contains the reference.
- The requesting transport principal holds an active read grant for that stream.

## Flow

### 1. Discover a missing reference

- **User intent**: Materialize the complete content represented by accepted events.
- **System response**: The destination walks accepted event `x` tags and detects
  which digests are not available locally.
- **Next**: Prove stream-scoped access.

### 2. Authorize through the event reference

- **User intent**: Fetch only blobs reachable through an authorized stream.
- **System response**: The custodian verifies that an event in a stream readable
  by the requester references the requested digest. A read grant alone does not
  reveal unreferenced blobs.
- **Next**: Transfer the blob.
- → `HEAD /artifacts/{sha256}`

### 3. Fetch the immutable bytes

- **User intent**: Retrieve the missing artifact without mutable names.
- **System response**: The custodian returns bytes for the exact digest.
- **Next**: Verify before storage.
- → `GET /artifacts/{sha256}`

### 4. Verify and store idempotently

- **User intent**: Trust local possession without trusting the custodian's claim.
- **System response**: The destination hashes the received bytes, rejects any
  mismatch, and stores matching content under the digest. Existing matching
  content is a successful no-op.
- **Next**: Resolve another missing reference or finish.

### 5. Surface unavailable content

- **User intent**: Distinguish authorization, absence, and corruption.
- **System response**: The destination reports separately that access is denied,
  no authorized event references the digest, bytes are unavailable, or the
  returned hash is invalid.
- **Next**: Correct policy, restore custody, or investigate corruption.

## Outcomes

- **Success**: The destination possesses hash-verified artifact bytes reachable
  from an event it is authorized to read.
- **Failure modes**:
  - artifact access is granted without an event reference;
  - a reference in an unauthorized stream leaks blob existence;
  - corrupt bytes are stored;
  - mutable filenames are treated as artifact identity;
  - a retry creates duplicate logical artifacts.

## Related Stories

- `specs/stories/sovereign-sync/retrieve-referenced-stream-artifacts.md`

## E2E Coverage

- Planned: `crates/buzz-test-client/tests/e2e_sovereign_artifacts.rs`
