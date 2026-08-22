NIP-FI-DELEG
============

Delegated agent authorization profile
--------------------------------------

`draft` `optional` `relay`

**Protocol dependency**: NIP-FI core.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", and
"MAY" in this document are to be interpreted as described in BCP 14 (RFC 2119
and RFC 8174) when, and only when, they appear in all capitals.

## Scope

This profile authorizes a delegate key from separately validated delegation
evidence rooted in a currently eligible NIP-FI owner binding. The delegate
proves its own key. It does not present a federated assertion and never receives
or inherits the owner's binding. Because a trusted edge inserts assertion and
provenance fields on every request it forwards, and `FI-DELEG-PATH-SEPARATION`
denies any such field on a delegated request, delegated requests cannot traverse
a route that requires edge provenance; they use ingress on which NIP-FI-EDGE is
not required.

This profile defines the normalized delegation result and its additional
preparation, final-admission, and lease witnesses. It does not define a wire
format for creating delegation relationships; NIP-OA or another protocol may
supply the evidence if it satisfies this contract.

## Delegation evidence

A validator returns this closed result:

```text
DelegationEvidence = (
  domain,
  owner_key,
  delegate_key,
  relationship_id,
  relationship_revision,
  audience,
  operations,
  conditions,
  resource_or_target,
  not_before?,
  mandatory_expiry
)
```

`relationship_id` and `relationship_revision` are deployment-local dependency
identifiers. All other fields are interoperability-critical in meaning even
when their concrete encoding belongs to the supplying delegation protocol.

The evidence authenticates every field, has one unambiguous owner and delegate,
matches the server-owned domain and exact request or target, and has a finite
expiry satisfying `now < mandatory_expiry`; equality at an expiry is expired.
Optional `not_before` satisfies `not_before <= now + skew`, using the
configured delegated `skew`; arithmetic is overflow-safe. A missing
configured `skew` denies. The proven actor equals `delegate_key`.
[FI-DELEG-EVIDENCE-CLOSED]

A delegated request carries fresh request-appropriate Nostr proof and no
`Nostr-Federated-Identity` or profile provenance field. Mixed direct and
delegated evidence denies rather than selecting a path. [FI-DELEG-PATH-SEPARATION]

## Private denial conditions

This profile defines exactly this private condition identifier and owning public
class for NIP-FI-CONF enumeration agreement:

| Private condition identifier | Public class |
|---|---|
| `delegation_not_current` | `authorization_denied` |

The identifier is a fixture name, not a wire value. Adding, removing, renaming,
or reclassifying it requires the same change in NIP-FI-CONF's denial-fixture
table.

## Preparation

Preparation resolves the exact server-owned domain, target context, operation,
resource, and proven delegate actor before validating delegation evidence. It
then atomically reads:

- the active owner binding and exact binding version;
- every owner tombstone, key-revocation, administrative, and profile lifecycle
  gate applicable to that binding;
- the exact relationship identifier and revision;
- current local policy and resource versions; and
- every invalidation dependency and deadline.

The owner binding is current and authorization-eligible at preparation. A
cached owner lease is not authority. The requested capability is the
intersection of the delegation's operation, audience, conditions, and target
with current local policy; an unsupported operation or empty intersection
denies. [FI-DELEG-OWNER-CURRENT]

Preparation remains read-only under `FI-INV-08`. It cannot create or change an
owner or delegate binding, identity, provenance, lifecycle fact, relationship,
last-seen value, replay claim, receipt, lease, or application effect.
[FI-DELEG-NO-BINDING]

## Final admission

Core final admission additionally requires:

1. the exact delegation evidence and delegate proof remain live;
2. domain, actor, target, audience, operation, resource, and relationship match
   the prepared value;
3. the exact current owner binding and binding version remain eligible;
4. relationship identity and revision remain current;
5. current capability intersection equals the prepared intersection; and
6. changed dependencies are reread and the complete delegated decision is
   recomputed before atomic commit.

Any mismatch, expiry, owner retirement, owner key revocation, owner binding
version change, relationship change, unreadable dependency, or unsupported
capability denies. Rotation makes the former owner key non-current; its
relationships do not transfer to the new key. [FI-DELEG-OWNER-CURRENT]

The delegated path creates no owner or delegate binding and cannot consume an
enrollment opportunity. Its receipt identifies the delegate actor and exact
owner-binding and relationship dependencies without publishing identity
material. [FI-DELEG-NO-BINDING]

## Delegated leases

A deployment configures a positive finite delegated maximum and a non-negative
finite delegated `skew`. The lease deadline is no later than the minimum of:

- delegation expiry;
- delegate proof or connection bound;
- owner binding administrative bound, when applicable;
- current relationship bound;
- local policy bound;
- the lease issue instant plus the configured delegated maximum; and
- any stronger owner-assertion bound the deployment requires.

Missing finite configuration denies. Equality is expired and arithmetic is
overflow-safe. [FI-DELEG-LEASE-BOUND]

Before each protected use, the service checks the delegate actor, owner binding
and version, relationship and revision, capability intersection, target,
resource, local policy, deadline, and invalidation state. It closes or rejects
the lease within the deployment's tested revocation-detection bound after any
owner or relationship dependency becomes ineligible. The claimed bound is no
smaller than measured worst-case detection plus enforcement delay.
[FI-DELEG-INVALIDATION-BOUND]

Owner retirement, revocation, rotation, disablement under NIP-FI-LIFECYCLE, or
binding replacement invalidates dependent delegates on the same effective
schedule as owner authority. A delegate lease never authorizes another delegate
or owner key on the same connection. [FI-DELEG-OWNER-CURRENT]

## Discovery

A relay claiming this profile MAY add `"delegation": true` to the NIP-11
`federated_identity` object only when owner-current resolution, the positive
finite maximum, uniform final admission, and all profile oracles are active. It
does not advertise relationship IDs, owner keys, private delegation protocol
names, or policy detail. [FI-DELEG-DISCOVERY]

## Behavioral oracles

| ID | Required outcome |
|---|---|
| `FI-DELEG-EVIDENCE-CLOSED` | Valid closed evidence passes; unauthenticated, ambiguous, wrong-domain/actor/target/audience, not-yet-valid, and expiry-equality variants deny. |
| `FI-DELEG-PATH-SEPARATION` | Delegation plus any direct assertion/provenance field denies; neither path falls back to the other. |
| `FI-DELEG-OWNER-CURRENT` | Exact current owner succeeds; retirement, revocation, rotation, replacement, stale owner version, stale relationship, and unreadable owner state deny without inheritance. |
| `FI-DELEG-NO-BINDING` | Successful, denied, and concurrent delegated requests create or change no owner/delegate binding or lifecycle state. |
| `FI-DELEG-LEASE-BOUND` | Every authority bound and equality boundary closes the lease; absent finite maximum denies. |
| `FI-DELEG-INVALIDATION-BOUND` | Measured owner/relationship revocation closes prepared evidence and live leases within the claimed detection bound. |
| `FI-DELEG-DISCOVERY` | Discovery is false/absent until the complete active profile passes; public output contains no relationship or owner detail. |

NIP-FI-CONF defines evidence packaging and mutation adequacy. Each uppercase
requirement above names the oracle that detects its violation.

## Security considerations

Delegation expands authority only by intersection and never by copying owner
capabilities. A stolen delegation still requires the delegate key. A stolen
delegate key is bounded by the relationship and finite lease. Owner rotation
cannot silently transfer delegation because the exact owner key and binding
version are dependencies. Implementations should invalidate by dependency index
rather than wait for incidental delegate traffic.
