# Read-state accessory extension constraints

Design guidance for follow-up work, not implemented wire capabilities.
The current [API contract](buzz-v1-read-state.md) remains authoritative.

Thread summaries are implemented (see `threads` in the API contract). Richer
previews that carry signed-event content should be opt-in and preserve the base sidebar's work/byte budgets;
fetching detail separately remains valid. Reuse canonical classification and
read frontiers, not a second definition of unread. Historical-mention eligibility
or mute-aware attention requires an explicit policy contract, not an unnoticed
change to today's `attention` field.

Channel-content revisions, personal-state revisions and synchronized preferences
are separate from read frontiers. Future revisions must describe the same
snapshot as their payload, and are invalidation tokens, not history cursors or
access grants. Count reuse also needs time/configuration validity because the
unread horizon moves without writes. Mutes and manual-unread overrides must not
be encoded by advancing or rewinding a read frontier.

These are extension constraints, not implemented capabilities: this version does
not advertise revision-based reuse or synchronized overrides.
