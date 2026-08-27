# NIP-FI-LIFECYCLE: Binding Lifecycle Profile

`draft` `optional`

## Abstract

This profile extends NIP-FI with provisioned enrollment, identity disablement,
re-enablement, and an administrative binding-expiry gate. It is for deployments
whose binding changes require separately authorized operator or enterprise
workflows. It does not change NIP-FI assertion validation, Nostr proof, final
admission, or public denial semantics.

The key words **MUST**, **MUST NOT**, **REQUIRED**, **SHALL**, **SHALL NOT**,
**SHOULD**, **SHOULD NOT**, **RECOMMENDED**, **MAY**, and **OPTIONAL** in this
document are to be interpreted as described in BCP 14 when, and only when, they
appear in all capitals as shown here.

## Dependencies and claim

An implementation of this profile implements NIP-FI Core and advertises only
the boolean `"lifecycle": true` inside its NIP-11 `federated_identity` object.
This boolean claims support for this profile; it deliberately reveals neither an
enrollment mode nor lifecycle state. For a fixed set of claimed profiles, the
complete discovery output MUST be byte-identical whether enrollment is
attested-key, TOFU, or provisioned and whether lifecycle facts exist. A server
MUST NOT advertise the claim until every protected ingress in the advertised
authorization domain applies this profile through the same final-admission
authority (`FI-LC-CLAIM`).

This profile contributes lifecycle dependencies and deadlines to the core
prepared decision and lease. They compose with core dependencies by set union;
the earliest applicable deadline wins. This profile cannot weaken, replace, or
bypass a core check.

## Additional state

For authorization domain `D`, this profile adds:

```text
X_D : set of disabled identities
Q_D : identity -> pending lineage

PendingLineage = (
  identity,
  old_key,
  old_binding_version
)
```

It also permits a core binding to carry `binding_not_after`, an optional
administrative deadline. The pending lineage names one exact retired pair and
binding version. There is at most one pending lineage per identity.

`binding_not_after` bounds the grant represented by **one binding**; it is not a
bound on the identity. Rotation continues the same grant: the replacement binding
preserves the carried bound, and only expiry authority changes it. Retirement,
revocation, and disablement end the existing grant. Re-enablement, provisioning,
and ordinary enrollment establish a *new* grant that carries no prior bound — so
retirement of a bound pair followed by ordinary enrollment under `attested-key`
or `tofu` policy yields an unbounded binding, and that is conformant. A deadline that must survive the end
of a grant — an identity-scoped access bound — belongs in the capability
projection of authoritative local-policy state, which core already requires for
any projection whose removal must close authority within a declared bound and
which is reread at preparation, final admission, and every protected lease use.
`binding_not_after` is not that mechanism and cannot substitute for it: it does
not survive the end of the grant that carries it, so a deployment relying on it
for identity-scoped expiry cannot claim a revocation bound for that expiry
(`FI-LC-ADMIN-EXPIRY`).

A binding carrying a reached bound is **active** for every core relation and
eligibility test in this profile and in core, including the core partial
bijection and `TargetEligible` below. It is ineligible for authorization, not
absent from the binding relation.

`X_D`, `Q_D`, and `binding_not_after` are deployment-local state. Their versions
are revalidation dependencies, not contract identities. A change invalidates a
prepared decision and every dependent lease unless complete final-admission
recomputation produces the required current result.

Ordinary authorization MUST deny when its identity is disabled, when pending
lineage exists for that identity, or when `now >= binding_not_after`; it MUST
NOT clear, consume, or alter any of those facts (`FI-LC-ORDINARY-GATES`). An
absent `binding_not_after` has no administrative expiry. Assertion `exp`,
`iat`, refresh, or maximum age never creates, renews, extends, or clears it.
Time passage alone creates no tombstone, lineage, or history.

## Private denial conditions

This profile defines exactly these private condition identifiers and owning
public classes for NIP-FI-CONF enumeration agreement:

| Private condition identifier | Public class |
|---|---|
| `identity_disabled` | `authorization_denied` |
| `explicit_replacement_required` | `authorization_denied` |
| `binding_expired` | `authorization_denied` |

The identifiers are fixture names, not wire values. Adding, removing, renaming,
or reclassifying one requires the same change in NIP-FI-CONF's denial-fixture
table.

## Common transition contract

Each transition below requires privileged authority distinct from an ordinary
federated assertion and Nostr proof. That authority MUST be bound to the exact
`D`, transition name, identity, request, old binding version when present, and
target key when present (`FI-LC-AUTHORITY`). The deployment defines how that
authority is obtained; role names, approval count, and operator APIs are out of
scope.

A transition MUST, in one atomic commit:

1. validate that privileged authority and fresh target-key evidence;
2. read and recheck the applicable core binding relation, retired pairs,
   revoked keys, `X_D`, `Q_D`, policy, and dependency versions;
3. apply exactly the state changes specified below;
4. append immutable lifecycle history identifying the transition and versions;
   and
5. advance lifecycle state so dependent prepared decisions and leases cannot
   authorize after commit.

A stale precondition, denied transition, unreadable dependency, or failed commit
MUST leave all authoritative state unchanged (`FI-LC-ATOMIC`). Lease
invalidation MAY be delivered asynchronously, but authorization use after the
commit MUST recheck the advanced dependency before allowing an operation.

Every transition that creates a binding MUST state whether it **continues** an
existing grant, and therefore preserves that grant's administrative bound, or
**establishes** a new grant carrying no prior bound. The two cases partition the
binding-creating transitions with no remainder: core rotation continues, and
provisioning, re-enablement, and ordinary enrollment establish. A profile that
adds a binding-creating transition without this declaration cannot claim
conformance (`FI-LC-CLAIM`).

`TargetEligible(i, k, allow_disabled)` means that `k` is not revoked, `(i, k)`
is not retired, neither `i` nor `k` has an active binding, and `i` is not
disabled unless `allow_disabled` is true. Every new target key requires fresh,
request-bound Nostr proof by that key. If domain policy requires issuer key
attestation, the transition also requires a current assertion for `i` whose key
claim equals `k`. Supplied stale, absent, wrong-identity, or mismatched required
attestation denies; it is never ignored as optional evidence
(`FI-LC-TARGET-PROOF`).

A replacement binding records `attested-key` provenance only when current
matching issuer attestation was validated; otherwise it records `provisioned`.
TOFU provenance can arise only from the core ordinary first-use extension and
is never inherited by a replacement.

## Privileged transitions

### Provision binding

```text
ProvisionBinding(i, k):
  require domain enrollment policy = provisioned
  require TargetEligible(i, k, false)
  require Q_D(i) is absent
  require fresh target-key evidence
  create Binding(i, k, new_version, provisioned)
```

The transition creates no authorization lease. Later use requires a current
assertion, fresh Nostr proof, and ordinary final admission. Ordinary
request-time authorization under `provisioned` policy MUST NOT create a binding
(`FI-LC-PROVISION`).

### Disable identity

```text
DisableIdentity(i):
  add i to X_D
  if Binding(i, k, old_version) exists:
    remove Binding(i, k, old_version)
    add (i, k) to the core retired-pair set
    set Q_D(i) = (i, k, old_version)
```

Applying an authorized disablement repeatedly is idempotent. It MUST NOT erase
or replace existing lineage. If `i` has no active binding, disablement creates
no lineage (`FI-LC-DISABLE`).

### Re-enable identity

```text
ReenableIdentity(i, expected_lineage?, k_new):
  require i is in X_D
  require Q_D(i) is absent when expected_lineage is absent,
          otherwise require Q_D(i) = expected_lineage
  require TargetEligible(i, k_new, true)
  require fresh target-key evidence
  remove i from X_D
  consume expected_lineage when present
  create Binding(i, k_new, new_version, ReplacementProvenance(evidence))
```

Clearing disabled state and creating the target binding are inseparable. There
is no clear-only transition: it would permit a later ordinary enrollment to
capture the identity. An operator that intends to provision later leaves the
identity disabled until the target and fresh proof are available
(`FI-LC-REENABLE`).

### Set administrative expiry

```text
SetAdministrativeExpiry(i, k, old_version, binding_not_after?):
  require exact current Binding(i, k, old_version)
  require separate privileged expiry authority
  replace it with Binding(i, k, new_version,
                          same_provenance, binding_not_after?)
```

This transition changes neither side of the pair nor its provenance. Setting,
replacing, or clearing the bound advances the binding version. At equality the
binding is ineligible but remains durable and occupies both sides of the core
partial bijection. This transition is the only expiry authority: no other
transition in this profile or in core sets, replaces, or clears the bound, and
core rotation carries it onto the replacement binding unchanged. Only this or
another applicable privileged transition can restore access; ordinary
authorization cannot renew the bound (`FI-LC-ADMIN-EXPIRY`).

## One-shot lineage and concurrency

Consumption of `Q_D` and creation of its replacement binding MUST be one
compare-and-commit operation over the exact pending lineage. Of two concurrent
re-enablings presenting the same lineage, at most one can commit. The loser
observes changed state and denies without creating a binding, consuming another
lineage, or changing history (`FI-LC-QD-ONCE`).

A lifecycle transition racing ordinary final admission is ordered by the same
authoritative state transaction or dependency check. If the lifecycle commit
wins, the ordinary operation denies; if final admission wins first, the
lifecycle transition still invalidates subsequent lease use. No ordering
permits authority from a disabled identity, consumed lineage, or expired
binding after the corresponding state change is observed.

## Behavioral oracles

Each oracle is normative. A conforming implementation produces the stated
result at final admission and retains no partial authoritative mutation from a
denied case.

| ID | Setup and required result |
|---|---|
| `FI-LC-CLAIM` | For a fixed profile set, compare complete discovery bytes across attested-key, TOFU, and provisioned configurations and across lifecycle states: they are identical. If one protected ingress omits lifecycle gates or uses a different lifecycle lineage, the domain cannot advertise the profile and the uncovered ingress fails closed. If any claimed profile defines a binding-creating transition that declares neither grant continuation nor grant establishment, the domain cannot advertise that profile. |
| `FI-LC-ORDINARY-GATES` | Fresh assertion and proof for a disabled identity, an identity with pending lineage, and a binding at administrative-expiry equality each deny without changing lifecycle state. |
| `FI-LC-AUTHORITY` | An ordinary assertion plus valid Nostr proof, but no transition-specific authority, cannot perform any transition; mutation of any authority-bound field denies. |
| `FI-LC-ATOMIC` | Inject failure at each transition write boundary; no binding, tombstone, disabled fact, lineage, history entry, or dependency version is partially committed. |
| `FI-LC-TARGET-PROOF` | Missing, stale, wrong-key, wrong-request, or mismatched required attestation for a new target denies without mutation. |
| `FI-LC-PROVISION` | Ordinary first use in provisioned mode denies; authorized provisioning creates one binding and no lease; later current ordinary admission may use it. |
| `FI-LC-DISABLE` | Disabling an active identity atomically disables it, retires its exact pair, records exact lineage, and closes subsequent lease use; replay is idempotent and preserves lineage. |
| `FI-LC-REENABLE` | Re-enablement creates an eligible proven binding in the same commit that clears disabled state; absent or wrong expected lineage and a clear-only attempt deny. |
| `FI-LC-ADMIN-EXPIRY` | Before the bound the binding may authorize; at equality it denies while still occupying the relation, so a target eligibility test for either side of that pair fails. Rotating the expired binding to a new key carries the bound: the replacement denies at the same instant. No non-expiry transition clears it. Only an authorized version-checked update by the expiry authority changes the bound. Conversely, retirement, revocation, or re-enablement of the bound pair followed by an authorized or ordinary new grant produces an unbounded binding, which is the required result and not an escape. |
| `FI-LC-QD-ONCE` | Two concurrent re-enablings consume the same `Q_D` lineage; exactly one commits and the loser leaves every authoritative store unchanged. |
| `FI-LC-RACE` | Race each transition against prepared ordinary admission and lease use; no operation authorizes after observing the advanced lifecycle or binding dependency. |

## Security considerations

Privileged authority compromise can provision or replace enterprise bindings;
deployments should apply controls proportionate to that authority. This profile
makes the authority request-bound and transitions atomic, but does not define
approval UX or key custody.

Disabled identities, retired pairs, revoked keys, and pending lineage serve
different purposes. Re-enablement removes only the exact disabled fact and
optional exact lineage named by its transition. No transition in this profile
removes a core revoked-key or retired-pair fact.

Administrative expiry is local policy, not upstream revocation freshness. It
cannot extend an assertion, status witness, Nostr proof, or lease deadline.
