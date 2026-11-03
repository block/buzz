# Community NIP-FI upgrade operator contract

BUZZ-510 adds durable policy acceptance. BUZZ-511 supplies per-community
request enforcement and discovery; BUZZ-512 supplies existing-session activation.
Do not expose upgrade calls in a deployment until those releases and the serving
fleet baseline have been verified. This change alone cannot establish enforcement.
It never reports `active` and never substitutes a timer for activation evidence.

The operator HTTP plane is separate from tenant event ingest. These management
operations extend the existing provisioning API; community clients cannot invoke
them with ordinary membership credentials. Both routes require an allowlisted
operator's NIP-98 signature bound to the configured operator API origin, exact
method, URL (including query), and POST body hash. Each attempt needs a fresh
signature; operation idempotency does not bypass transport replay protection.
Responses use `Cache-Control: no-store`.

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
and activation separately. BUZZ-510 returns `pending` while the target's policy,
lifecycle and issuer binding still match, and `unavailable` if they do not or
trusted issuer configuration is no longer applicable. A missing database response
is HTTP 503, never a fabricated receipt. A receipt is not proof of current service
health. No response in this version reports `active`.

`pending_reasons` is extensible. Clients must preserve pending on unknown values.
`registry_baseline_unverified` prevents completion until BUZZ-512 verifies fleet
membership. That ticket may add `boot_application_pending`, `boot_drain_pending`
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
| 503 | `authorization_unavailable` | Trusted config or authoritative DB unavailable; retry/read back same UUID |

Malformed GET queries use Axum's existing query-extractor rejection. Transport
signature replay uses the existing operator replay error independently of this table.

## Schema and rollout

Migration 0081 is additive: legacy rows keep `requires_nip_fi=false` and generation
zero. It adds no trigger, function, procedure, rule or foreign key and rewrites no
existing rows. Lock and statement budgets are 5 and 30 seconds. Existing global
`enforce` remains an additional floor; this ticket changes no data-plane gate.
Do not infer requirements from issuer rows, backfill free communities, or remove
the enterprise enforce floor as part of this migration.

Apply schema before new code. Desired-state and migration definitions match;
pgschema callers must also run the reconciliation script to register the new
operator-global receipt table. Receipts intentionally survive community purge:
the target ID/host/owner tuple prevents reusing a lost-response UUID for another
community, including after deletion. They are operator-only recovery evidence.
No tenant serving writer owns this table and no fence trigger is attached.

The active-community primary accessor is
`Db::community_auth_policy(CommunityId) -> Result<Option<CommunityAuthPolicy>>`.
`None` means no active target; errors must fail closed. BUZZ-511 must combine
its stored requirement with global enforce and honor repair denial first.
