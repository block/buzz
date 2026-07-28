---
id: retrieve-referenced-stream-artifacts
type: story
refs:
  journey: specs/journeys/fetch-referenced-stream-artifacts.md
  persona: specs/personas/sovereign-node-operator.md
  steps: [1, 2, 3, 4, 5]
---

# Story: Retrieve artifacts referenced by an authorized stream

## Narrative

As a sovereign node operator,
I want artifact access to follow accepted event references,
So that blobs move with their manifests without creating a broader artifact ACL.

## Acceptance Criteria

- [ ] Artifact identity is the SHA-256 digest in an accepted event `x` tag.
- [ ] A requester must hold read authority for a stream containing a reference to the digest.
- [ ] A read grant does not reveal unreferenced artifacts.
- [ ] References in unauthorized streams do not reveal artifact existence or bytes.
- [ ] Received bytes are hashed before storage and rejected on mismatch.
- [ ] Storing bytes already present under the digest is idempotent.
- [ ] Authorization denial, missing custody, and hash mismatch are distinguishable outcomes.

## Notes

Events are the artifact manifest. Artifact transfer is not a second event
stream and does not need an independent cursor.
