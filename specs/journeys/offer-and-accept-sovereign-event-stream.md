---
id: offer-and-accept-sovereign-event-stream
type: journey
refs:
  persona: specs/personas/sovereign-node-operator.md
---

# Journey: Offer and accept a sovereign event stream

## Actor

The sovereign node operator acting first as a source operator. A destination
operator is the secondary participant and independently controls admission.

Source Persona: `specs/personas/sovereign-node-operator.md`

## Trigger

The source operator wants another sovereign node to receive a bounded selection
of signed events without granting it ownership of the source node or its event
authors.

## Preconditions

- Each node has an application owner identity for signing declarations.
- Each node has a stable node principal and one or more transport verification keys.
- Both parties know which owner identity represents the counterparty.
- The source can express the intended event selection as a filter, mirror, or upstream source.

## Flow

### 1. Identify the parties and key roles

- **User intent**: Know which identities own policy and which keys only move bytes.
- **System response**: The relationship names the source owner, destination
  owner, source and destination node principals, and transport verification
  keys without treating any one identity as interchangeable with another.
- **Next**: Define the stream.

### 2. Define an immutable stream selection

- **User intent**: Describe exactly which signed events may cross the boundary.
- **System response**: The source assigns a stream ID to one explicit selection:
  a NIP-01 filter set, a whole-journal mirror, or an upstream source. Changing
  the selection requires a new stream ID.
- **Next**: Publish the offer.

### 3. Publish the source offer

- **User intent**: Make the proposal durable and attributable.
- **System response**: The source owner signs an active `export/<stream-id>`
  declaration. Its `p` tags name destination owner identities allowed to answer
  the offer; its content records the immutable event selection.
- **Next**: Let the destination assess it.
- → `POST /events`

### 4. Assess destination admission

- **User intent**: Decide whether the destination will accept this stream and
  which authenticated transport path may deliver it.
- **System response**: The destination verifies the current export head,
  selection, source owner, retention expectation, and intended transport keys.
- **Next**: Publish the destination half.

### 5. Publish a pinned admit

- **User intent**: Accept exactly the reviewed offer, not a future replacement.
- **System response**: The destination owner signs an active
  `admit/<stream-id>` declaration whose `e` tag pins the current export event
  ID and whose `p` tags name transport verification keys accepted by the
  destination sink.
- **Next**: Authorize the transport direction.
- → `POST /events`

### 6. Authorize stream reading when required

- **User intent**: Permit the designated puller to drain the exported stream.
- **System response**: For pull or custodian-mediated delivery, the source or
  custodian owner signs a `read/<stream-id>` declaration pinned to the export;
  its `p` tags name transport keys allowed to read. Push-only paths may omit a
  read grant.
- **Next**: Evaluate agreement and transport readiness.
- → `POST /events`

### 7. Evaluate the current heads

- **User intent**: Distinguish mutual intent from operational connectivity.
- **System response**: The system reports separately whether the export and
  admit form a current matched pair, whether any required read grant exists,
  and whether authenticated transport is ready. A transport success never
  manufactures an agreement.
- **Next**: Replicate events or resolve the reported mismatch.

## Outcomes

- **Success**: Both owners have signed compatible current heads, the admit pins
  the reviewed export, and the required transport principals are explicitly
  authorized.
- **Failure modes**:
  - an owner identity is confused with a transport key;
  - an admit is active but unpinned;
  - the export does not offer to the admit author;
  - the stream IDs differ;
  - a required read grant is absent or names the wrong transport key;
  - a valid transport proof is incorrectly treated as policy consent.

## Related Stories

- `specs/stories/sovereign-sync/offer-sovereign-event-stream.md`
- `specs/stories/sovereign-sync/accept-sovereign-event-stream.md`
- `specs/stories/sovereign-sync/authorize-sovereign-stream-transport.md`

## E2E Coverage

- Planned: `crates/buzz-test-client/tests/e2e_sovereign_sync.rs`
