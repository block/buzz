# Remote mention preparation and publication

Owned relay agents can be selected and invited without local runtime custody.
The picker and preparation phase evaluate policy but may admit an owned
nonmember. Publication separately refreshes authorization for the exact eventual
destination, including a newly created DM. Ownership is not membership and
cached picker evidence is never publication authorization.

Selections are intent: a vanished/revoked key must fail visibly, retain the draft
and send nothing, rather than silently dropping a recipient. Captured agent keys
survive composer clearing, media upload, edits and asynchronous preparation.
A failed local inventory read cannot veto authenticated relay identities or
admit stale local runtimes. Locally managed runtime readiness remains a separate
existing path; remote identities never gain synthetic local management records.

Chat offers explicit Invite or reference-only send without inviting. Failed adds,
revoked policy, failed final authorization and cancellation preserve recoverable
drafts. Standalone forum sends report authorization failures, but standalone
forum invitation is a subsequent change reusing this phase contract.

Native discovery prerequisite: `docs/owned-agent-discovery.md` (PR6).
Regression coverage: `agentAutocompleteEligibility.test.mjs`,
`agentMentionRevalidation.test.mjs`, `useMentionSendFlow.helpers.test.mjs`,
`submitMessageEdit.test.mjs`, `mentions.spec.ts`, and
`remote-owned-mentions.spec.ts`. The new remote browser fixtures use a single-word
name deliberately: mention separator behavior belongs to the independent PR1.

NIP-OA establishes ownership, not physical hosting, availability, or lifecycle
control. Final native queries do not provide an atomic relay transaction with
message publication. Independent review of both native and publication boundaries
is required before landing.

## Invitation intent and cancellation

Each chat Invite owns one synchronous pending latch and abort signal, covering
preparation, inventory, adds and the eventual publication continuation. Escape,
navigation/unmount, reference-only selection or a replacement prompt invalidates
that attempt. Clearing the prompt when promoting it to send is not cancellation;
the signal remains live through final validation and queued media completion.
Check cancellation after asynchronous preparation and before subsequent mutations
(including nested local readiness), and again before publication. A completed add
cannot be undone, but its late response cannot revive cancelled message intent.
The pending state includes preparation, disabling both Invite and reference-only
buttons. Cancellation after optimistic clearing restores the captured draft and
exact mention refs without overwriting newer edits.

`useMentionSendFlow.cancellation.test.mjs` drives the actual hooks with React
StrictMode and deferred dependencies; `remote-owned-mentions.spec.ts` covers
visible pending, Escape/retry and route navigation at deferred IPC boundaries.

### Source draft ownership

Invitation intent belongs to one visit of the effective persistence key and
channel, not just the mounted composer or destination channel. Same-channel
thread changes and A → B → A invalidate the original visit. The visible owner gates
optimistic clear, editor recovery and pending state. Storage recovery instead
consults shared authored intent for the source draft key, even after exit,
re-entry or unmount. The draft store retains a revision and authoritative-empty
marker independently of the stored value. Later same-key authored text/deletion
or a new send revokes older recovery and sent-draft cleanup; editing B does not
change A's authority. A visit-specific accessor reads that shared authority while
preserving the separate visible-owner identity. Invalidation makes recovery durable
synchronously, before a later visit can load or edit the source draft; late async
completion cannot revive that recovery or release a newer attempt's latch.

The editor's authored revision distinguishes an intentional edit → clear from an
optimistic empty composer. Programmatic send clear/recovery runs inside the draft
lifecycle's restoration boundary, so it does not mark the source as authoritatively
deleted. Pending clipboard identity verification settles before exact selected
mention refs are captured. The source visit and authored revision are captured
before that wait: an edit, navigation (including A → B → A), or unmount cancels
rather than reading the new draft's maps. Subsequent asynchronous preparation
consumes the captured selections.
Persona preparation similarly consumes captured selections and returns resolved
refs, writing them into the editor only while the original visit/revision remains
current. Normal persona creation/reuse and ordinary destination-bound background
sends retain their existing behavior; accepted membership is never rolled back.

Recovery additionally compares stored content/media/exact refs before replacing
an existing record. Programmatic persistence does not itself change semantic
intent. Explicit inbox deletion and replacement do; scope reset invalidates old
handles. This is same-window authority for live continuations, not cross-window
synchronization or a versioned storage protocol. Reload destroys continuations;
authored deletion has already removed the durable value. Final membership/policy reads are not atomic with send,
and cancellation cannot retract an already dispatched publication. Standalone
forum transport-failure binding recovery and native compatibility remain separate
review/follow-up boundaries.
