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
