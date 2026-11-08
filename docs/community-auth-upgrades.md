# Community NIP-FI policy operator contract

The operator plane manages a stored community requirement independently of the
global FI mode. See [operator usage](operator-nip-fi-upgrade.md). Receipts prove
durable acceptance, never fleet-wide socket closure or current authorization.

## Operator boundary

POST/GET `/operator/communities/nip-fi-policy` require an allowlisted operator's
NIP-98 signature bound to `RELAY_OPERATOR_API_ORIGIN`, method and URL (including
the GET query), and POST body hash. Each attempt needs a fresh signature. These
routes extend provisioning outside tenant event ingest; ordinary membership
credentials cannot invoke them. All responses use `Cache-Control: no-store`.

The operator mediates an authenticated actor's registered key. The relay verifies
the operator and stored owner; `actor_pubkey` is the mediator's claimed actor,
not an independent user credential. Never accept a caller-selected actor as identity.

## Generation storage

`communities.requires_nip_fi` defaults false; `policy_generation` defaults zero.
Every new general operation advances the generation, even a same-state intent.
Replay never increments. Generation exhaustion is refused. The historical
`community_auth_upgrades` table stores acceptance receipts for both directions.
Receipts survive target deletion and are not tenant serving data.

## Enable and disable

POST contains `operation_id`, `community_id`, canonical `expected_host`,
`actor_pubkey`, `expected_owner_pubkey`, desired `requires_nip_fi`, and mandatory
`expected_policy_generation` plus `expected_requires_nip_fi`. UUIDs must be non-nil,
keys lowercase hex, generations nonnegative; unknown fields are rejected.
Enable requires `expected_issuer`; disable must omit it.

Read current preconditions with GET `?community_id=<UUID>&expected_host=<host>`.
Both generation and stored state must still match under the writer row lock.
A mismatch returns 409 `policy_conflict` without mutation or receipt. Re-read
and use a new UUID/request for revised intent. On a lost response, preserve the
original complete file/UUID and recover or retry it with a fresh signature.

Acceptance serializes UUID reuse, then locks deletion-shared → community
`FOR NO KEY UPDATE` → exclusive host lease, preserving lifecycle/deletion-intent,
exact host, single owner and actor==owner checks and community write admission.
Acceptance and receipt commit atomically. UUID identity binds the full tuple,
action, both preconditions and authenticated operator. Different
meaning under a committed UUID returns 409 `idempotency_conflict`. Identical
committed replay recovers history before mutable preflight and never writes policy.

Enable requires a single deployment-selected authorized issuer in
`BUZZ_NIP_FI_PROVISION_BINDING`, an applicable registry policy and exact existing
community binding. It locks the binding against unbind and adds no trust.
Disable does not depend on issuer availability and does not alter bindings.
Every optional→required transition, including re-enable, excludes new host leases
and drains prior optional leases for 1.1 seconds before committing.

## Readback and response

GET `?operation_id=<UUID>` returns only the originating operator's receipt.
Mixed query forms are invalid. The response separates `receipt` (original tuple,
accepted generation/action/preconditions and current target generation/flag) from
`mode` and `effective_required`. Current-state reads also print exact expected
`expected_policy_generation` and `expected_requires_nip_fi` fields, without a
duplicate unprefixed pair. Missing/deleted targets have null current fields.

| Receipt state | Meaning | CLI exit |
|---|---|---|
| `applied` | Current stored disable | 0 |
| `pending` | Current accepted enable; closure uncertified | 10 |
| `superseded` | Later generation or state differs | 11 |
| `unavailable` | Target/lifecycle/host or required issuer trust unavailable | 12 |

`pending_reasons` includes `connection_closure_unverified` for enable. Neither
time, local counters nor a successful sweep certify fleet completion. No response
reports `active`. History is not a current authorization credential; ownership
changes do not destroy the original operator's recovery authority.

Invalid requests return 400; operator auth uses existing 401/403 errors. Missing
community/operation returns 404. Host, owner, lifecycle, issuer and policy conflicts
return 409 with corresponding `*_conflict` codes; generation exhaustion returns
409 `policy_generation_exhausted`. New mutations behind a closed gate return
503 `upgrades_disabled`; unavailable authoritative DB/trust returns
503 `authorization_unavailable`. Request/transport/response errors exit 5.

## Schema and rollout

Migration 0085 adds desired action and nullable CAS preconditions to existing
receipts. True/default and null preconditions preserve old receipt meaning and
old INSERT compatibility. No data backfill, trigger, function or foreign key is
added. Historical rows with null CAS preconditions remain readable through
policy status, including after target deletion. A new fenced request cannot reuse
such a row's UUID: null historical preconditions differ from the mandatory new
preconditions, so it returns `idempotency_conflict` without mutation.

The old `/operator/communities/nip-fi-upgrade` endpoint and the `nip-fi-upgrade`
admin command have been removed. Its GET and POST requests receive 404; use
`nip-fi-policy` for both changes and receipt recovery. Upgrade existing clients
for the [policy client contract](operator-nip-fi-policy-client.md) before this
relay version reaches their fleet, with their mutations paused until rollout ends.
No client may guess CAS fields or refresh a saved request during recovery.

Before rolling, close `BUZZ_NIP_FI_UPGRADES_ENABLED` on every old relay and keep
`BUZZ_NIP_FI_CHANGES=false` on new relays. Apply schema first, roll the whole
fleet to the supported policy writer, then explicitly enable the new gate and
resume migrated clients. Old binaries with their old gate open can still accept
unfenced enables and reverse a disable; a new-name setting alone cannot close
that old gate. Retire the old setting after the last old binary is gone.
Reads and committed replay remain available on the supported API with its gate
closed. Do not roll back to a writer that exposes unfenced enable after any
policy operation. This removal does not erase receipts or rewrite schema.

## Request enforcement and discovery

`off` ignores stored requirements; `shadow` enforces stored true; `enforce` is a
global requirement floor; `deny_protected` denies protected traffic. Report
`effective_required` alongside stored state so disable under enforce and enable
under off cannot be mistaken for effective changes. Evaluating policy lookups
fail closed on errors; off does not require an FI policy read.

Host policy comes from the writer, bound to community identity and lifecycle.
Optional cache leases last at most one second; enable excludes refills and drains
them. Fail-closed cache ordering may retain old required state for the remaining
lease after disable. It cannot renew that stale lease. `/info` advertises effective
`requires_nip_fi` without exposing operator receipts.

### Existing connections

The periodic revalidator reads stored policy without row-locking transactions.
After disable it stops requesting required-policy closes; a concurrent sweep may
request one last non-FI close from its earlier snapshot. That client reconnects
under optional policy. FI-admitted sockets retain their finite lifetime and then
reconnect under current policy. Disable does not reopen or force-logout sessions.
Re-enable repeats the optional-lease drain; non-FI root/audio sockets close through
existing revalidation. Valid FI-admitted sockets retain existing rules.

## Mistaken acceptance and containment

Stop an uncertain initiating caller and preserve its operation evidence. Read
current policy before deciding whether to issue a new explicit CAS disable.
A timeout is not permission to discard the original request or edit SQL.
Disable is a supported operator action; acceptance is no longer monotonic.
Disable removes the required-policy block on last-issuer removal; the separate
last-binding guard still applies. The operator unbind command keeps the last
issuer for bindable hosts. Re-enable must recheck trust and the exact binding.
Removing trust never implicitly clears stored policy.

Close the mutation gate to stop new changes while keeping recovery available.
Use established archive/lifecycle containment or repair FI trust as appropriate;
never broaden trust to make a probe pass. Global off suspends FI everywhere while
preserving stored policy and receipts; it is not per-community disable. Binary
rollback does not undo acceptance and must not restore unfenced writers. No live
mutation or deployment is implied by this procedure.
