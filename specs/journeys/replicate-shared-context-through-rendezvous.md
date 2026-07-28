---
id: replicate-shared-context-through-rendezvous
type: journey
refs:
  persona: specs/personas/sovereign-node-operator.md
---

# Journey: Replicate a shared context through a rendezvous

## Actor

The sovereign node operator maintaining one member's custody of a shared
context. The rendezvous is a custodian and transport intermediary, not an owner.

Source Persona: `specs/personas/sovereign-node-operator.md`

## Trigger

Two sovereign participants want their relationship history to remain available
across intermittent connectivity while retaining independent nodes and keys.

## Preconditions

- The participants share one stable NIP-29 context ID.
- Events belonging to the context carry its `h` tag.
- A matched event-stream agreement selects the context events and required metadata.
- The destination transport key has any required rendezvous read grant.

## Flow

### 1. Open the shared context

- **User intent**: Use a human-visible space as the relationship record.
- **System response**: Both participants address the same NIP-29 context ID and
  author events with their own application identities.
- **Next**: Bind the context to an event stream.

### 2. Select context events

- **User intent**: Replicate the relationship without exporting unrelated history.
- **System response**: The stream selection includes events carrying the shared
  `h` tag and the metadata needed to render that context. The stream ID remains
  distinct from the context ID.
- **Next**: Export to the rendezvous.

### 3. Custody selected events

- **User intent**: Keep events reachable while either participant is offline.
- **System response**: The rendezvous accepts exact signed event envelopes from
  an authenticated admitted source and exposes only the configured export.
- **Next**: Let the destination drain its authorized view.
- → `POST /replication`

### 4. Drain and ingest the stream

- **User intent**: Advance local custody without gaps or duplicate effects.
- **System response**: The destination transport principal reads ordered pages,
  ingests them into its local relay, and advances its cursor only after
  checkpoint-safe receipts. Original event authors and signatures are preserved.
- **Next**: Open the shared context locally.
- → `POST /replication/read`, `POST /replication`

### 5. Browse the relationship locally

- **User intent**: Read the shared history with an ordinary Buzz client.
- **System response**: The local relay serves the replicated `h`-tagged events
  as one channel-like context without requiring the client to understand
  replication declarations.
- **Next**: Continue authoring locally or wait for the next synchronization.

## Outcomes

- **Success**: Each node holds a verifiable local copy of the selected shared
  context, and the rendezvous has custody but no authorship or policy authority.
- **Failure modes**:
  - the stream ID is mistaken for the shared `h` context ID;
  - the selection omits metadata required to render the context;
  - unrelated events escape the filter;
  - a cursor advances before destination acceptance is checkpoint-safe;
  - replication rewrites event authorship;
  - the rendezvous serves the stream to an ungranted reader.

## Related Stories

- `specs/stories/sovereign-sync/replicate-shared-context-events.md`

## E2E Coverage

- Planned: `crates/buzz-test-client/tests/e2e_sovereign_sync.rs`
