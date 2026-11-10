# Operator NIP-FI policy client contract

Use signed POST and GET `/operator/communities/nip-fi-policy`. The former
`/operator/communities/nip-fi-upgrade` route is removed and returns 404 for GET
and POST; it no longer performs operator authentication. Deployment proxies
may independently reject a request before it reaches the relay.

For a new intent, read GET `?community_id=<UUID>&expected_host=<host>`. Copy its
`expected_policy_generation` and `expected_requires_nip_fi` into the request.
These are the only current-state field pair; the response also includes
`community_id`, `expected_host`, `mode` and `effective_required`.

Durably save the complete immutable request before its first possible dispatch:
operation UUID, community identity, expected host, authenticated actor and owner,
desired `requires_nip_fi`, both CAS preconditions, and the trusted issuer for an
enable. Disable omits the issuer. After a crash, timeout or lost response, read
GET `?operation_id=<UUID>` or replay that exact saved request with a fresh NIP-98
signature. Never reread current policy and overwrite its saved CAS fields under
the same UUID. A `policy_conflict` requires a new explicitly authorized intent
with a new UUID and freshly observed preconditions.

POST and operation-status GET return `{ "receipt": { ... }, "mode": "...",
"effective_required": true }`. Validate the authenticated operator, saved tuple,
desired action, CAS preconditions and positive accepted generation inside
`receipt`. Its `current_policy_generation` and `current_requires_nip_fi` describe
the current target, not the original intent. They are null after target deletion.
`pending`, `applied`, `superseded` and `unavailable` have the meanings in the
[operator contract](community-auth-upgrades.md#readback-and-response).
Historical acceptance never certifies current policy or fleet-wide socket closure.
No response reports `active`.

Previously accepted enable receipts remain readable through operation-status
GET, with desired=true and null CAS preconditions. Recover them as history; do
not invent preconditions or convert them into a new write. Reusing their UUID
for a new fenced request returns `idempotency_conflict`, even after deletion.
Treat only the structured 404 `operation_not_found` from the supported endpoint
as an absent receipt. A 404 from the removed route is not evidence that an
operation was never accepted.

Ship migrated callers with mutation dispatch paused before rolling out the
route removal. Close the old gate on all old binaries and keep the new gate
closed on new binaries. Apply additive schema, complete the whole-fleet rollout,
then open `BUZZ_NIP_FI_CHANGES` and resume supported callers. See
[rollout](community-auth-upgrades.md#schema-and-rollout). An uncertain operation
from the old caller needs explicit reconciliation; a missing receipt is not
permission to synthesize a replacement policy write.
