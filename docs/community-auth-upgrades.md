# Community NIP-FI upgrade operator contract

The acceptance layer records durable policy. The per-request enforcement layer
adds community checks and discovery; the activation layer updates existing sessions.
Do not expose upgrade calls in a deployment until those releases and the serving
fleet baseline have been verified. This change alone cannot establish enforcement.
It never reports `active` and never substitutes a timer for activation evidence.

The operator HTTP plane is separate from tenant event ingest. These management
operations extend the existing provisioning API; community clients cannot invoke
them with ordinary membership credentials. Both routes require an allowlisted
operator's NIP-98 signature bound to the configured operator API origin, exact
method and URL, including the query string on GET, and the POST body hash.
Operator POST verification does not bind query strings. Each attempt needs a fresh
signature; operation idempotency does not bypass transport replay protection.
Responses use `Cache-Control: no-store`.

`BUZZ_NIP_FI_UPGRADES_ENABLED` defaults to `false`. While disabled, new
acceptances return 503 `upgrades_disabled` with no policy mutation or
receipt. Authorized GET and identical POST receipt recovery remain available;
principal checks and payload conflicts are unchanged. Enabling this deployment
switch is an explicit operator action after the enforcement and activation fleet
baseline is verified; caller release discipline alone cannot enable acceptance.

## Generation storage

`communities.policy_generation` is `BIGINT NOT NULL DEFAULT 0`, without a
nonnegative database CHECK. Supported writers retain the current generation
when the community already requires FI, or increment it by one on acceptance.
Starting from zero, those writes preserve the nonnegative invariant. Direct SQL
can insert a negative community generation and is outside that writer contract.
Positive-generation CHECKs on acceptance receipts remain in place; activation
and receipt matching still require the exact accepted generation.

The community CHECK is omitted to avoid validating existing rows while adding
the column. The current schema planner folds a column and its CHECK together,
and can rebuild and validate a CHECK when its validation metadata differs.
Migration, fresh desired-state creation and later reconciliation therefore all
omit this community CHECK. No bootstrap or schema-tooling workaround is required.

## Enable

`POST /operator/communities/nip-fi-upgrade`

```json
{
  "operation_id": "c2ffcc20-9505-48fd-b6c7-446aa833bcd1",
  "community_id": "c9acf9a6-37d5-4af1-8df9-92a165637b01",
  "expected_host": "team.communities.example",
  "actor_pubkey": "<owner's registered 64-character hex public key>",
  "expected_owner_pubkey": "<expected current owner's hex public key>",
  "expected_issuer": "https://identity.example/api/goose"
}
```

The trusted operator mediates the authenticated actor. Kgoose must resolve that
actor's usable registered key from its session; it must never accept a user-supplied
actor key as identity. The actor must equal the expected and current owner.
`actor_pubkey` records the mediator's claimed authenticated actor for audit and
idempotency; it is not an independently verified end-user credential. The relay
authenticates the operator and checks the stored current owner.
Host and keys normalize before comparison; issuer bytes match exactly. UUIDs
must be non-nil. Unknown JSON fields are rejected.

The deployment selects the expected issuer from the **single** authorized issuer
in `BUZZ_NIP_FI_PROVISION_BINDING`. An absent or ambiguous selection is unavailable.
The target must be bindable under that policy and the issuer must have a configured
`BUZZ_NIP_FI_ISSUERS` policy. The same issuer must already be bound to the community
in `community_nip_fi_issuers`. The call never adds or broadens trust.

A transaction serializes UUID reuse across targets, then community lifecycle,
host and owner checks against transfer/archive/delete. It locks the exact issuer
row against unbind. Only a first optional-to-required transition increments
`policy_generation`; concurrent distinct operations converge on that generation.
Acceptance and its receipt commit together. No timeout or retry clears the policy.

## Readback and response

`GET /operator/communities/nip-fi-upgrade?operation_id=<UUID>`

Only the original authenticated operator can recover the receipt. Readback
survives owner transfer, archive and deletion; the recorded owner is historical
acceptance evidence, not authority for a fresh enable.

Enable and readback return HTTP 200 for an accepted receipt:

```json
{
  "operation_id": "c2ffcc20-9505-48fd-b6c7-446aa833bcd1",
  "community_id": "c9acf9a6-37d5-4af1-8df9-92a165637b01",
  "expected_host": "team.communities.example",
  "actor_pubkey": "<owner at acceptance>",
  "expected_owner_pubkey": "<expected owner at acceptance>",
  "expected_issuer": "https://identity.example/api/goose",
  "operator_pubkey": "<authenticated operator>",
  "policy_generation": 1,
  "requires_nip_fi": true,
  "state": "pending",
  "pending_reasons": ["registry_baseline_unverified"]
}
```

`requires_nip_fi: true` records durable acceptance. `state` reports availability
and activation separately. The acceptance layer returns `pending` while the target's policy,
lifecycle and issuer binding still match, and `unavailable` if they do not or
trusted issuer configuration is no longer applicable. A missing database response
is HTTP 503, never a fabricated receipt. A receipt is not proof of current service
health. No response in this version reports `active`.

`pending_reasons` applies only while `state` is `pending`; clients must ignore
those obligations when state is `unavailable`, which always takes precedence.
`pending_reasons` is extensible. Clients must preserve pending on unknown values.
`registry_baseline_unverified` prevents completion until the activation layer verifies fleet
membership. That layer may add `boot_application_pending`, `boot_drain_pending`
and `boot_unreachable`, and must bind activation evidence to the exact operation
and generation. Baseline invalidation must prevent `active`.

The UUID binds the normalized community/host/actor/owner/issuer/operator tuple.
Identical retries recover that tuple and generation before rechecking mutable
preconditions. Changed payload or principal conflicts. After a lost POST response,
read by UUID or retry the same payload with a fresh NIP-98 signature. Do not generate
a new UUID to hide an ambiguous response. No rollback endpoint exists.

| HTTP | `code` | Meaning |
|---|---|---|
| 400 | `invalid_request` | Invalid enable JSON, UUID, host or key |
| 401/403 | Existing operator auth errors | Missing/invalid credentials or operator not allowlisted |
| 404 | `community_not_found` | No target for a first acceptance |
| 404 | `operation_not_found` | No receipt owned by this operator |
| 409 | `idempotency_conflict` | UUID already bound to a different tuple/principal |
| 409 | `host_conflict` | ID does not map to expected host |
| 409 | `owner_conflict` | Actor/expected owner/current owner disagree |
| 409 | `lifecycle_conflict` | Archive/deletion or durable deletion intent |
| 409 | `issuer_conflict` | Expected issuer/host not approved, or exact stored binding absent |
| 503 | `upgrades_disabled` | New acceptances disabled; wait for operator enablement, preserving the same UUID |
| 503 | `authorization_unavailable` | Trusted config or authoritative DB unavailable; retry/read back same UUID |

Malformed GET queries use Axum's existing query-extractor rejection. Transport
signature replay uses the existing operator replay error independently of this table.

## Schema and rollout

Migration 0082 is additive: legacy rows keep `requires_nip_fi=false` and generation
zero. It adds no trigger, function, procedure, rule or foreign key and rewrites no
existing rows. Lock and statement budgets are 5 and 30 seconds. This migration
changes no data-plane gate. Relay mode determines whether stored requirements
are enforced, as described below.
Do not infer requirements from issuer rows, backfill free communities, or remove
the enterprise enforce floor as part of this migration.

Apply schema before new code. Desired-state and migration definitions match;
The registry entry is static lint metadata, so it is present in the migration
and `schema/seeds/`, not the pgschema reconciliation script (database.md Rule 3).
This follows the other operator-global registry entries. Receipts intentionally survive community purge:
the target ID/host/owner tuple prevents reusing a lost-response UUID for another
community, including after deletion. They are operator-only recovery evidence.
No tenant serving writer owns this table and no fence trigger is attached.

The active-community primary accessor is
`Db::community_auth_policy(CommunityId) -> Result<Option<CommunityAuthPolicy>>`.
`None` means no active target. Repair mode denies protected traffic first. In
`off`, no community requires FI, including a stored-required community; FI lookup
or dependency failures must not deny ordinary admission. In `shadow`, enforce
stored requirements and observe optional communities. In `enforce`, require FI
for every community. Evaluating policy lookups fail closed on errors. Mode
changes do not clear the stored requirement, generation, issuer trust or receipts.

## Mistaken acceptance and containment

Stop the initiating caller and set `BUZZ_NIP_FI_UPGRADES_ENABLED=false` to prevent
new acceptances. Keep authenticated status/readback available. Record the exact
operation, target and generation for operator-led incident assessment and recovery.
Preserve the durable requirement, issuer trust and receipts while restoring a
working FI path or containing the affected community through the established
archive/lifecycle controls. Coordinate any necessary issuer/configuration repair
through its existing operator procedure; never broaden trust to make a probe pass.

Disabling calls or rolling back code does not undo acceptance. In Shadow, never
restore a serving binary that ignores the durable requirement; retain effective enforcement
or contain the community while repairing the FI path. There is no
clear-policy API or routine SQL reset. A timeout or mistaken call does not
authorize clearing the requirement or discarding operation evidence.

An explicit operator change to `off` intentionally suspends FI enforcement for
all communities while preserving durable requirements and receipts. This is a
relay-wide mode choice, not an undo of acceptance or a binary rollback. Readback
must distinguish stored policy from effective enforcement and must not report
active enforcement in Off. Returning to `shadow` resumes enforcement for stored
required communities; `enforce` requires FI everywhere. Off suspends FI for
every community; it is not per-community containment. Choose relay-wide Off
suspension or archive/lifecycle containment explicitly; neither a fleet-wide
drain nor a deployment is an automatic fallback.

Last-issuer removal is refused once the community requires NIP-FI, including
when the operator binding path requests `allow_last`. Bind a replacement before
removing an issuer, or use archive/lifecycle controls for containment. Removing
the issuer recorded by a receipt while another remains makes that receipt
`unavailable`; it never clears the requirement or destroys the receipt.
Bulk backfill does not recreate missing bindings for required communities;
its unbound report still exposes them for explicit operator repair.

## Request enforcement and discovery (BUZZ-511)

Relay mode controls the effective requirement without changing durable storage:
`off` disables FI for all communities, including stored `requires_nip_fi=true`;
`shadow` enforces FI only for stored required communities and observes optional
communities; `enforce` requires FI everywhere. `deny_protected` remains an
unconditional repair refusal. Off ignores missing FI policy, verifier, deny map
and issuer dependencies; ordinary tenant routing and Nostr authorization remain.
Outside Off, protected requests resolve their normalized Host's active policy
through a bounded primary-read lease cache, with a 500 ms budget on misses.
Existing Nostr proof, key pairing, deny map, membership and resource checks remain.
Discovery, pairing, enrollment and operator exceptions keep their own authentication.

The router shares one policy snapshot with HTTP handlers for that request. Git
also scopes its extractor to one policy and tenant snapshot, retaining its
existing header, signed-URL and denial ordering. Routing and policy share one
host cache capped at 10,000 entries. A serving entry carries immutable tenant ID,
requirement, generation and original read time from the same row. It is usable
for strictly less than one second from before the read's connection checkout;
neither a slow read nor a cache hit renews its lifetime. Concurrent fills for a
host share a successful read. Misses, errors and timeouts are never cached. A late result
after invalidation is usable once by its in-flight request but never published,
and an older generation cannot replace a newer
observed generation for the same community. Off retains ordinary routing-only
cache population and does no FI-policy fills.

Cache fills take a shared transaction-scoped advisory lock keyed by normalized
host in a dedicated namespace. Lock acquisition and the serving-row SELECT are
separate statements in an explicit READ COMMITTED transaction. A contended fill
refuses immediately; a statement has a 400 ms server-side timeout inside the
500 ms overall budget. Waiters on the per-host fill share its successful result;
when no result was cached, each waiter makes at most one lease read of its own
inside that same budget, with no retry loop. Unmapped hosts remain 404.
Each cold policy fill uses six database statements: BEGIN, isolation setup,
statement-timeout setup, try-lock, row SELECT and COMMIT. Ordinary routing keeps
its single plain serving-row query and a routing-only snapshot in the same
bounded host cache. That snapshot cannot supply FI policy. A routing refresh
preserves a fresh policy for the same tenant without extending its lease.

Acceptance takes the shared deletion lock and locks the target community row
`FOR NO KEY UPDATE`. It checks the row's host against `expected_host`, then takes
the exclusive advisory lock derived from that locked host. An optional row waits
1.1 seconds before the remaining owner/issuer checks and commit. The row stays
locked throughout, preventing single or double host remaps during the drain.
This also expires leases issued under any former host before row-lock acquisition.
An initially wrong host precondition is refused before waiting.

This drains every optional one-second lease across all pods BEFORE commit,
without relying on Redis delivery or synchronized wall clocks. The extra 100 ms
is a clock-rate margin. Lease duration and acceptance wait are compile-time fleet
protocol constants; increasing the lease requires a coordinated writer rollout
first. Existing receipt recovery skips this wait. Already-required targets need
no lease drain. Fills take only a shared advisory host lock and never lock the
community row, preserving the deletion-shared → row → host-lock ordering.

Policy resolutions that start after acceptance commits therefore cannot use an
optional lease for that community, even on a pod that missed invalidation.
Admissions that resolved policy before commit can still be in flight, exactly
as with primary reads; their completion and cancellation belong to BUZZ-512's
generation-registration and drain fence. Cache expiry is not activation evidence.
The acceptance drain blocks new policy fills and other acceptances for the
host. Its community row lock also blocks archive/unarchive, deletion admission
and fences, ownership transfer/convergence and issuer-binding writers for that
community during the 1.1-second wait. These are operator actions; ordinary
serving reads and writes do not take a conflicting row lock. Foreign-key
`FOR KEY SHARE` locks do not conflict with `FOR NO KEY UPDATE`, and the shared
deletion lock remains compatible with ordinary tenant writers. Cold policy
requests during the transition may return 503; acceptance is rare and this
avoids allowing stale optional leases after commit.

Relay acceptance, provisioning and archive/unarchive writes drop the local host
snapshot and publish a best-effort Redis invalidation to other pods. An epoch
fences fills that began before a local or received drop. Direct administrative
database writes and missed notifications retain the one-second routing bound.
Host remap, archive and deletion do not drain leases: cached tenant binding can
lag those changes by up to one second, as ordinary host routing already does.
Policy and tenant data always use the same cached row, including NIP-11
descriptors and decorations. Issuer-binding and JWKS freshness keep their own
existing bounds and checks; cached status never substitutes for those checks.

`buzz_nip_fi_policy_cache_hits_total` and `buzz_nip_fi_policy_cache_misses_total`
measure cache use; `buzz_nip_fi_policy_reads_total` counts fill attempts and
`buzz_nip_fi_policy_read_seconds` measures completed source reads. A warm ordinary
protected request or discovery request performs no policy writer read. Root and
audio upgrades intentionally keep a final uncached authoritative tenant/policy
binding after the early evidence gate: one read on a cache hit, two on a cold
successful upgrade. Its optional-to-required recheck preserves the unarchive and
host-remap race fence, as well as early refusal and deadline ordering.

Issuer-binding/JWKS caches keep their existing bounds; unavailable dependencies
refuse required admission, rather than treating it as optional.

WebSocket upgrades also bind tenant identity and its policy from the same primary
row. An earlier missing or optional policy result cannot authorize a socket for a
required row that becomes serving during lookup (for example, on unarchive).
Before upgrade, enforce that bound row's requirement even when the early check
ran in Shadow. Off never promotes a stored requirement. This changes no discovery fields.

Database-sourced deployments load the FI binding snapshot and command components
only in evaluating modes. Off does not depend on that snapshot and cannot report
required-admission readiness. Configure finite
`BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS`, issuer JWKS and command configuration
before enforcing any row in Shadow or Enforce. Missing required components return
503 only when FI is enabled. An optional row
still follows global off/shadow behavior, including signed and unsigned legacy
proofs where the endpoint permits them.

Both `GET /info` and root NIP-11 negotiation return `Cache-Control: no-store`.
An unknown or non-serving Host returns generic 404. Outside Off, policy lookup
failure returns 503. Off uses ordinary tenant routing and returns generic 404 on
a tenant lookup failure, without consulting FI policy. The top-level `requires_nip_fi` is a host-specific boolean. Existing
`limitation.federated_identity` and `federated_identity` descriptor fields retain
capability semantics: advertise when global mode evaluates OR the host requires
FI. Shadow optional hosts still advertise capability. Off hosts advertise neither
FI capability nor an effective requirement, even when stored policy is required. No issuer, audience or identity
provider address is added to discovery. Existing clients can keep reading their
current fields; new clients must use the requirement field to distinguish optional
capability from mandatory evidence.

This is admission enforcement only. Existing requests and sockets may predate the
new policy. Only BUZZ-512's application and drain evidence may establish completed
activation; the operator receipt remains pending until that evidence is recorded.

### Staging and containment

Outside Off, this changes every serving host, including free hosts: protected
routes and discovery require a valid primary policy lease. Off retains ordinary routing. Unmapped discovery previously could
return a generic document; it now returns 404. Policy failures return 503.
Database-source startup loads its FI binding snapshot only in evaluating modes;
Off does not load it. A failed load still fails startup in Shadow or Enforce. The local dependency gauge
`buzz_nip_fi_required_admission_ready` must be 1 before accepting any upgrade;
it covers verifier, deny map and finite lifetime, not fleet baseline verification.
Binding and JWKS readiness must also be verified through a signed request.

Keep `BUZZ_NIP_FI_UPGRADES_ENABLED=false` throughout a mixed-version rollout
until EVERY acceptance path implements the host-lock lease-drain protocol.
Uncached serving pods can coexist with caching pods, but an old acceptance
writer cannot safely coexist with cached optional leases. Before rolling an
acceptance writer back or removing this protocol, disable new acceptance on
every writer and verify that disablement; keep the lease-aware writers until
in-flight acceptance has finished. Disabling new calls does not undo existing
requirements. Existing required-community enforcement and BUZZ-512 activation
and revocation prerequisites still apply. Never enable an older acceptance path
while any caching pod serves optional communities.
Older pods may log an unknown `CommunityHost` cache-invalidation message during
the mixed-version rollout. They keep serving through their existing uncached
policy reads; missing that best-effort notification does not weaken the lease
protocol. The acceptance-writer gate above still applies.

Keep `BUZZ_NIP_FI_UPGRADES_ENABLED=false` during the initial staging rollout.
Start with zero required rows and record a 30-minute baseline using representative
free signed/unsigned HTTP, root/audio WS and discovery traffic. Compare identical
traffic before and after this change. Record request rate, policy-read p95/p99,
writer pool acquisition p99 (`buzz_db_pool_acquire_duration_seconds`,
`pool_role=writer`, `operation=authorization`), policy timeout/error count, HTTP
503 rate and startup/restart success. A local PostgreSQL 16 probe measured 100
serial reads in 13.36 ms; that is not a production capacity result.

Initial staging abort criteria (validate before increasing traffic): any policy
timeout, policy-induced 503, unexpected free-admission denial or startup failure;
policy-read p99 above 50 ms; writer acquisition p99 above 20 ms or more than twice
its pre-change baseline; total 503 rate more than 0.1 percentage point above
baseline over five minutes. These are conservative rollout limits, not measured
production guarantees. Do not expand until the full 30-minute run stays within
them and the operator has reviewed measured results. Verify unknown-host 404 and
dependency-failure 503 with no-store explicitly.

Then test a disposable required community with complete FI dependencies, in both
Shadow and Enforce: unsigned/missing evidence denies, signed valid FI succeeds,
root/audio upgrades agree, discovery agrees, and a pending deletion request does
not weaken the requirement. Before production acceptance, complete BUZZ-512's
session application/drain and the production baseline-verifier gate. No such
production verification is established by this PR.

Before any required row exists, an operator may stop the rollout and restore the
previous binary after verifying the primary contains zero required rows. Once a
requirement has been accepted, a pre-511 binary in Shadow is unsafe: it ignores
that row. Off explicitly suspends FI; global Enforce still requires all hosts.
Stop new upgrade calls, retain policy and receipt records, and keep an enforcing
binary while repairing dependencies. If admission cannot be restored, contain the
affected serving hosts with an operator-approved repair-mode/traffic action;
Off explicitly disables FI; use it only as an operator mode decision and never
clear accepted policy. Returning to Shadow restores stored requirements. No automatic fleet
drain or deployment is prescribed here. Existing receipt recovery remains usable
while new acceptances are disabled.

Required sessions admitted under global Shadow are real enforced FI sessions.
The existing global-Shadow admin disconnect path still only observes would-close;
BUZZ-512 must close these enforced sessions independently of global rollout mode,
while preserving the separate optional-shadow registry. Before enabling upgrade
acceptance for any serving community in Shadow, deploy and verify BUZZ-512's local
and cross-pod disconnect behavior, including real Redis propagation. Keep upgrade
acceptance disabled until that prerequisite is met; admission tests alone do not
establish it. Production also requires the baseline-verifier gate above.
