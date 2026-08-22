NIP-FI
======

Federated identity authorization — core
----------------------------------------

`draft` `optional` `relay`

**Protocol dependencies**: NIP-01 and either NIP-42 or NIP-98. Optional
profiles are defined by NIP-FI-EDGE, NIP-FI-LIFECYCLE, NIP-FI-DELEG, and
NIP-FI-CONF.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", and
"MAY" in this document are to be interpreted as described in BCP 14 (RFC 2119
and RFC 8174) when, and only when, they appear in all capitals.

## Abstract

NIP-FI authorizes a Nostr key only when four independent facts agree: a valid
issuer-qualified identity assertion, fresh proof of that Nostr key, current
identity-to-key binding state, and current local policy for the exact operation.
The identity provider never signs Nostr events, and an assertion never replaces
Nostr proof.

Bindings outlive individual assertions. Assertions and authorization leases do
not outlive their evidence. This core defines the portable client-attached
assertion transport, direct enrollment, atomic final admission, bounded
sessions, privacy-preserving denial responses, and the smallest useful binding
lifecycle. Companion profiles add trusted edges, extended lifecycle operations,
and delegation without changing the core admission rule.

This NIP does not define an identity provider, database schema, operator API,
public identity projection, application membership policy, or user interface.

## Terms and identifier classes

- **domain** (`D`): an authorization boundary selected only by authenticated
  server routing and configuration.
- **identity** (`i`): the exact tuple `(iss, sub)` returned by assertion
  validation. Email, display name, employee number, and a bare `sub` are not
  identities.
- **target context** (`R_t`): the server-resolved method, authority, path and
  query, body semantics, transport, operation, and resource.
- **actor** (`k`): the 32-byte public key returned by Nostr-proof validation.
- **request context** (`R`): `R_t` sealed with `k`.
- **binding**: a durable, versioned association `(D, i, k)` with immutable
  provenance `attested-key`, `tofu`, or `provisioned`.
- **retired pair**: a durable denial fact for an exact `(D, i, k)`.
- **revoked key**: a durable denial fact for `(D, k)`.
- **prepared authorization**: immutable, read-only evidence and witnesses for a
  possible admission.
- **committed authorization**: authority returned only after final revalidation
  and atomic commit.
- **lease**: a cached committed decision for one actor and bounded operation
  set. A lease is not a binding.

Identity and authorization-state comparisons preserve every tuple component.
Equal `sub` values under different `iss` values are distinct identities; equal
`(i, k)` pairs under different domains are distinct bindings, retired pairs,
and authorization state. [FI-TRACE-CROSS-DOMAIN-COLLISION]

Every identifier is either **interoperability-critical** or
**deployment-local**. Header names, public response bytes, token type values,
and trace identifiers are interoperability-critical and fixed here.
`assertion_policy_id`, `transport_contract_id`, domain IDs, snapshot versions,
binding versions, policy versions, and correlation IDs are deployment-local;
their values are opaque outside a deployment, while their stability and
invalidation behavior are normative.

## Core security invariants

These labels are the normative home of the NIP-FI invariants. Companion
profiles may add witnesses and bounds but cannot weaken them.

1. **`FI-INV-01 — partial bijection.`** Active bindings are one-to-one within a
   domain: one identity has at most one active key and one key has at most one
   active identity. [FI-TRACE-BINDING-CONFLICT]
2. **`FI-INV-02 — durable binding.`** Assertion expiry removes neither a
   binding nor its provenance. Fresh eligible evidence may authorize the same
   binding later. [FI-TRACE-ASSERTION-REFRESH]
3. **`FI-INV-03 — tombstone monotonicity.`** Ordinary authorization never
   removes a retired-pair or revoked-key fact and never recreates a retired
   pair. [FI-TRACE-TOMBSTONE-REPLAY]
4. **`FI-INV-04 — server-owned context.`** Every admitted operation uses one
   server-resolved domain, target, resource, operation, and proven actor.
   Unauthenticated input cannot replace them. [FI-TRACE-DOMAIN-SPOOF]
5. **`FI-INV-05 — independent evidence.`** Direct authorization requires a
   current assertion and fresh Nostr proof. If the assertion names a key, it
   equals the proven actor. [FI-TRACE-ASSERTION-KEY-MISMATCH]
6. **`FI-INV-06 — stable assertion policy.`** Assertion-policy identity changes
   when accepted assertion semantics change, but not when only authenticated
   key or status snapshot contents rotate. [FI-TRACE-VERIFIER-PARITY]
7. **`FI-INV-07 — current-snapshot verification.`** Evidence cannot survive
   removal of the key or policy snapshot that authenticated it; a changed
   snapshot requires revalidation. [FI-TRACE-JWKS-REMOVE]
8. **`FI-INV-08 — read-only preparation.`** Preparation creates no binding,
   tombstone, replay claim, receipt, lease, publication, last-seen value, audit
   authority, or application mutation. [FI-TRACE-FINAL-DENIAL-NO-MUTATION]
9. **`FI-INV-09 — atomic final admission.`** Enrollment, replay claims,
   receipts, and required authorization evidence commit only after complete
   final revalidation, all or none. [FI-TRACE-PREPARED-STALE]
10. **`FI-INV-10 — explicit lifecycle authority.`** Retirement, revocation,
    rotation, and profile-defined lifecycle changes occur only through their
    separately authorized transition. [FI-TRACE-LIFECYCLE-AUTHORITY]
11. **`FI-INV-11 — evidence-bounded leases.`** A lease ends no later than every
    evidence, snapshot, proof, binding, local-policy, and implementation bound
    on which it depends. [FI-TRACE-LEASE-BOUND]
12. **`FI-INV-12 — current-owner delegation.`** When NIP-FI-DELEG is claimed,
    delegation requires the exact current eligible owner binding, fresh
    delegate proof, capability intersection, and a positive finite deadline.
    [FI-DELEG-OWNER-CURRENT]
13. **`FI-INV-13 — privacy-safe denial.`** Public rejection is many-to-one and
    reveals no identity, key, claim, binding, tombstone, enrollment mode, key
    identifier, or private policy fact. [FI-TRACE-DENIAL-ORACLE]
14. **`FI-INV-14 — fail closed.`** Unreadable, ambiguous, stale beyond policy,
    or inconsistent evidence or authoritative state cannot produce authority.
    [FI-TRACE-DEPENDENCY-FAIL-CLOSED]
15. **`FI-INV-15 — uniform authority.`** Every protected ingress in a domain
    uses the same current domain policy and final-admission authority. An
    uncovered or competing path is unavailable. [FI-TRACE-AUTHORITY-UNIFORM]
16. **`FI-INV-16 — canonical verifier.`** Assertion transports feed one closed,
    provider-neutral normalized-result contract and cannot fork final
    admission. [FI-TRACE-VERIFIER-PARITY]

## Client-attached transport

Server configuration selects `client-attached` before protected traffic is
accepted. Request fields cannot select, negotiate, or downgrade transport.
Failure never falls back to another transport. [FI-TRACE-TRANSPORT-CLOSED]

The client sends exactly one field on the request or WebSocket upgrade:

```text
Nostr-Federated-Identity: Bearer <compact-JWS>
```

`Authorization` remains reserved for NIP-98. Assertion and provenance fields
from any other profile are absent. Missing, repeated, comma-combined, empty,
malformed, non-Bearer, or mixed-profile fields deny. Assertions never appear in
URLs, query parameters, Nostr events, tags, filters, application history, or
public identity projections. [FI-TRACE-TRANSPORT-CLOSED]

The core transport contract has deployment-local identity
`transport_contract_id`. It deterministically identifies the exact field,
parsing, request-attachment, no-fallback, and context-preservation semantics.
Changing any of those semantics changes the ID; changing request data does not.
[FI-TRACE-CONTRACT-IDENTITIES]

## Assertion validation

A configured assertion policy accepts exactly one bounded compact JWS and
returns this closed result:

```text
VerifiedAssertion = (
  identity = (iss, sub),
  asserted_key?,
  claims_or_capabilities,
  authority_deadlines,          // non-empty
  assertion_policy_id,
  transport_contract_id,
  revalidation_dependencies
)
```

The verifier rejects ambiguous protected-header or claim members, unknown
critical headers, `alg=none`, symmetric algorithms, algorithm/key mismatch,
incompatible JWK usage, ambiguous key selection, and signatures not valid
under exactly one accepted asymmetric key. It bounds the assertion, headers,
claims, subject, key identifiers, and authenticated key set before lookup or
logging. [FI-TRACE-ASSERTION-VALIDATION]

The exact `iss` selects an authenticated policy and key source; `iss` and at
least one `aud` value exactly match configured values. `sub` is a non-empty
bounded string. Each policy configures a non-negative finite `skew`, a positive
finite `maximum_assertion_age`, and, for `current-status`, a positive finite
`maximum_status_age`; a missing value denies. `exp` and `iat` are finite
NumericDate values satisfying `now < exp`, `iat <= now + skew`, and
`now < iat + maximum_assertion_age`. Optional `nbf` satisfies
`nbf <= now + skew`. Arithmetic is overflow-safe and equality at an expiry is
expired. [FI-TRACE-ASSERTION-VALIDATION]

The Nostr-key claim is named `nostr_pubkey`. When present it MUST be a
lowercase hexadecimal encoding of exactly one 32-byte Nostr public key; other
encodings and aliases deny. In `attested-key` enrollment policy and wherever
current matching issuer attestation is required, this exact claim MUST be
present and equal the proven actor; authorization claims or capabilities use a
closed bounded input set and deterministic canonical encoding. Unchecked claims
never enter the result. [FI-TRACE-VERIFIER-PARITY]

### Token class

Policy selects exactly one token class before parsing claims:

- **`at+jwt` access token**: a Buzz-resource access token whose protected
  `typ` is exactly `at+jwt` and whose `aud` contains the configured Buzz
  resource audience. This class selects tokens carrying the RFC 9068 `at+jwt`
  type but validates them under this document's claim contract; it does not
  implement the full RFC 9068 validation profile, and the long-form media type
  `application/at+jwt` is not accepted;
- **dedicated Buzz assertion**: a separately minted assertion whose protected
  `typ` is exactly `nip-fi+jwt`;
- **named compatibility access token**: absent or generic protected `typ=JWT`
  only under an explicit issuer policy whose required and forbidden claims,
  audience, issuer, key source, and validation rules are mutually exclusive
  with every accepted ID-token and other JWT class.

OIDC ID Tokens always deny, even when `iss`, `aud`, and `sub` match. A generic
or absent type has no stock fallback. Failure under one class never triggers
validation under another. An `at+jwt` access token MUST contain one non-empty
bounded `client_id`. Issuer policy MUST distinguish a resource-owner token from a token
whose subject represents the OAuth client, including a client-credentials token,
using authenticated claim semantics and mutually exclusive validation rules. A
token that admits both interpretations denies. If client-subject tokens are
accepted, the issuer MUST guarantee that their `(iss, sub)` coordinates cannot
collide with resource-owner coordinates; otherwise that token class is
ineligible. Token class and every class-specific validation rule are inputs to
`assertion_policy_id`. [FI-TRACE-TOKEN-CLASS]

### Policy identity and snapshots

Core has exactly two semantic contract identities:

```text
assertion_policy_id  = H(canonical assertion-policy contract)
transport_contract_id = H(canonical transport contract)
```

Each uses one implementation-defined but deterministic, versioned encoding and
collision-resistant hash within a deployment. `assertion_policy_id` covers the
canonical issuer, audience, token class, allowed algorithms, authenticated
key/status-source contracts, identity/key/claim mapping, time and size rules,
normalization, freshness class, and compiled verifier behavior. The verifier
fingerprint is an input, not a third identity. `transport_contract_id` covers
the client-attached field, parsing, attachment, context preservation, and
no-fallback semantics; a companion transport may define its own canonical
contract under that same identity slot. A semantic change changes exactly its
owning ID. [FI-TRACE-CONTRACT-IDENTITIES]

Mutable contents and deployment state are not contract identities. They remain
in `revalidation_dependencies`: authenticated assertion-snapshot version,
verification-key identity, key-snapshot hard deadline, optional status
source/version/deadline, binding/lifecycle/local-policy/resource versions,
proof and replay witnesses, and a confidential handle to the exact compact JWS.
Adding, removing, or replacing an accepted key changes the snapshot version,
not `assertion_policy_id`. Changed dependencies require revalidation under
current state; a retained key may continue, while an absent key denies.
Unknown-key refresh is bounded and coalesced and has no attacker-triggered
stale-key fallback. [FI-TRACE-JWKS-ADD] [FI-TRACE-JWKS-REMOVE]

The base contract compares the current authenticated snapshot and makes no
anti-rollback promise. A deployment claiming rollback prevention records a
separately authenticated monotonic floor and tests it. [deployment artifact:
assertion-policy review]

### Freshness class

Each policy declares exactly one server-owned freshness class, included in
`assertion_policy_id`:

- **`offline-jwt`** validates the JWT and authenticated key snapshot only.
  `upstream_authority_deadline` is the minimum of `exp`,
  `iat + maximum_assertion_age`, and the key-snapshot hard deadline. Token age
  bounds assertions minted before revocation; it cannot bound an issuer that
  continues minting accepted assertions afterward. Enabling this class therefore
  requires deployment evidence that revocation stops new accepted issuance, and
  discovery reports the unconditional residual bound as unknown (`null`). It
  MUST NOT advertise a finite unconditional residual bound. [deployment
  artifact: issuer revocation review]
- **`current-status`** additionally requires an authenticated witness
  `(iss, sub, token_or_session_id?, active=true, observed_at, valid_until,
  status_version, authenticated_source_id)`. Issuer, subject, and optional
  session identifier exactly match the assertion. Ambiguous, unauthenticated,
  inactive, or expired status denies. `valid_until` is finite and no later than
  `observed_at + maximum_status_age`. The upstream deadline is the minimum of
  the offline assertion deadlines and `valid_until`. Source outage cannot mint
  or extend a witness; an already verified witness remains usable only until
  its existing `valid_until`. [FI-TRACE-CURRENT-STATUS-STALE]

A current-status deployment advertises a tested positive
`maximum_residual_upstream_revocation_seconds`. Prepared evidence and leases
close within that value after upstream revocation, including a revocation racing
final admission. Poll/cache age, event-delivery and processing delay, and
enforcement delay all fit within the advertised value. A push implementation
may close authority sooner but cannot claim a value below its tested worst case.
[FI-TRACE-CURRENT-STATUS-REVOKED]

An external capability projection whose removal is required to close authority
within a declared revocation bound MUST enter authoritative local-policy state,
not `claims_or_capabilities` from the assertion. That state is reread during
preparation, final admission, and protected lease use. A deployment that carries
such a projection only in assertions cannot claim a revocation bound for its
changes. [FI-TRACE-CAPABILITY-REVOCATION]

Before enabling an issuer, the operator records authoritative evidence that
`sub` is stable for the account lifetime, never reassigned, and not intentionally
derived from mutable profile data. An issuer that cannot provide this property
is ineligible. [deployment artifact: issuer subject-stability review]

## Nostr proof and body semantics

The actor is always returned by fresh Nostr-proof validation, never by an
assertion or unsigned field. NIP-42 binds its AUTH event to the current
challenge, relay URL, connection, and freshness window. NIP-98 binds its event
to the exact server-resolved URL, method, and freshness window. All evidence
agrees with the same `D` and `R_t`. [FI-TRACE-DOMAIN-SPOOF]

Each protected HTTP operation declares in server policy whether its body is
authorization-relevant; clients cannot select the declaration.

For a relevant body, the NIP-98 event contains exactly one `payload` tag equal
to lowercase hexadecimal SHA-256 of the **body bytes**: the complete content
after transfer decoding and before any content decoding. Absence, duplication,
mismatch, validation of only a prefix, or substitution of the body bytes after
validation denies. For an irrelevant body, no
authorization decision, target, capability, or effect selector derives from a
body field not bound by NIP-98. A `payload` tag present on an operation whose
body is declared authorization-irrelevant is validated identically against the
body bytes; duplication or mismatch denies. [FI-TRACE-BODY-BINDING]

Every operation has finite body and spool bounds. A known oversized body is
rejected before hashing; a stream is rejected at octet `limit + 1`; admission
waits for EOF. Before EOF there is no application effect, replay mutation,
receipt, or partial digest authority. Quota failure cleans up staged bytes and
denies. [FI-TRACE-BODY-BOUNDS]

## Direct preparation

The following is normative pseudocode; every read is from authoritative state.

```text
PrepareDirect(request, assertion, proof):
  (D, R_t, operation, resource) := ResolveTargetContext(request) or DENY
  e := ValidateClientAttached(assertion, D, R_t) or DENY
  k := ValidateNostrProof(proof, D, R_t) or DENY
  R := SealActor(R_t, k)
  i := e.identity

  if e.asserted_key exists and e.asserted_key != k: DENY(key_mismatch)
  atomically read B_D(i), B_D(k), T_D(i,k), Y_D(k), enrollment policy,
                      local policy, resource, and all dependency versions
  if k in Y_D: DENY(key_revoked)
  if (i,k) in T_D: DENY(pair_retired)

  if B_D(i) = B_D(k) = binding(i,k):
      proposal := existing(binding.version, binding.provenance)
  else if B_D(i) exists or B_D(k) exists:
      DENY(binding_conflict)
  else if enrollment policy = attested-key:
      if e.asserted_key != k: DENY(attestation_required)
      proposal := enroll(i, k, attested-key)
  else if enrollment policy = tofu:
      proposal := enroll(i, k, e.asserted_key = k ? attested-key : tofu)
  else:
      DENY(binding_required)

  EvaluateLocalPolicy(D, R, operation, resource, k,
                      e.claims_or_capabilities) or DENY
  return PreparedAuthorization(evidence, proposal, witnesses, deadlines)
```

TOFU is optional private deployment posture and is not self-advertised. It
accepts that a stolen assertion for a never-enrolled identity can bind an
attacker's proven key; deployments enabling it retain a passing
FI-TRACE-TOFU-THEFT artifact. Binding provenance is immutable. A policy change
affects only future creation. [deployment artifact: TOFU risk review]

Preparation, including first-use enrollment, is read-only and produces no
authoritative mutation. [FI-TRACE-FINAL-DENIAL-NO-MUTATION]

## Final admission

A prepared value is consumed at most once. Final admission first requires an
exact domain, context, operation, resource, actor, and transport match; both
contract IDs unchanged; and every bound live. A changed dependency is reread
and re-evaluated from authoritative evidence. [FI-TRACE-PREPARED-STALE]

Two verified assertion results are **equivalent** when:

1. identity-class fields are byte-equal: `iss`, `sub`, asserted-key presence and
   value, canonical claims/capabilities, `assertion_policy_id`, and
   `transport_contract_id`;
2. each bounds-class deadline — every `authority_deadlines` member, the
   key-snapshot hard deadline, and any status deadline — is live now and is no
   later than its prepared value; and
3. provenance-class fields — snapshot version, verification-key identity,
   status source and version, binding, lifecycle, local-policy, and resource
   versions, proof and replay witnesses, the confidential JWS handle, cache
   metadata, ordering, and retrieval time — are ignored after successful
   current revalidation.

Every `revalidation_dependencies` member is bounds-class if it is a deadline
and provenance-class otherwise. Any new or unclassified assertion-content field
belongs to the identity class. A fresher assertion cannot silently extend a
prepared decision. [FI-TRACE-PREPARED-STALE]

Final admission atomically rereads binding, tombstone, revocation, enrollment,
policy, resource, status, replay, receipt, and invalidation witnesses;
recomputes the complete decision; claims applicable proof replay identities;
creates an eligible proposed binding; and appends its request-bound receipt and
required authorization evidence. All commit or none. A concurrent identical
enrollment may recompute as the same `existing` binding; conflicting enrollment
commits at most one winner. [FI-TRACE-CONCURRENT-ENROLLMENT]

A failed admission rolls back all authority mutation. The application operation
runs only after committed authorization. If it cannot share the transaction, a
request-bound idempotent receipt prevents the same proof from creating a second
effect. [FI-TRACE-FINAL-DENIAL-NO-MUTATION]

## Base lifecycle

Retirement, revocation, and rotation require separate privileged authority
bound to the exact domain, transition, identity, old binding version when
present, target key when present, and request. Each atomically rechecks current
state, appends immutable lifecycle history, and invalidates dependent leases
after commit. Every new target key supplies fresh target-bound Nostr proof and
any policy-required current matching issuer attestation. [FI-TRACE-LIFECYCLE-AUTHORITY]

- **RetirePair** removes one exact active binding and durably retires its pair.
- **RevokeKey** records the key as revoked even if inactive; if active, it also
  removes the binding and retires that pair. Repeating the same authorized
  revocation is idempotent.
- **Rotate** replaces one exact active binding with one unused, unrevoked,
  non-retired target key, retires the old pair, and creates a fresh binding
  version. The replacement provenance is `attested-key` when current matching
  issuer attestation was validated and `provisioned` otherwise. Rotation does
  not globally revoke the old key. Rotation continues one grant onto a new key
  rather than establishing a new one: the replacement preserves every
  profile-defined administrative bound carried by the binding it replaces, as an
  opaque field core neither interprets nor clears. Only the authority that set
  such a bound can change it. [FI-TRACE-LIFECYCLE-AUTHORITY]

Failure or stale state causes no partial mutation. Ordinary authorization cannot
perform or undo these transitions. Extended disablement, re-enablement,
provisioning, and administrative expiry are defined only by NIP-FI-LIFECYCLE.

## Request and session bounds

HTTP authority covers one exact request and is never reusable.

A WebSocket lease is scoped to one actor, domain, operation set, binding
version, normalized result, policy/resource versions, and invalidation
witnesses. Its deadline is the earliest assertion, upstream-authority,
key-snapshot, proof/connection, local-policy, and implementation deadline.
Arithmetic is overflow-safe and equality is expired. [FI-TRACE-LEASE-BOUND]

Before each protected use, the service checks actor, domain, operation,
resource, deadline, binding version, contract IDs, snapshot/status versions,
policy versions, and invalidation state. Changed dependencies require current
revalidation to an equivalent result; unreadable or ineligible state denies.
A lease for one key never authorizes another key on the same connection.
[FI-TRACE-MULTI-KEY-SESSION]

Expiry ends the lease, not the binding. Renewal requires a new connection with
a fresh assertion attached to its WebSocket upgrade, fresh Nostr proof,
preparation, and final admission; there is no in-band renewal path. Confidential
assertion revalidation material is destroyed on expiry, close, or invalidation.

## Rejection and privacy

Public class is a function only of evidence the requester supplied, never of
private per-principal server state; `authorization_unavailable` is the sole
exception and reveals only that a required authoritative dependency is
unreadable, never any per-principal fact. Replay status is a function of
committed per-principal server state, not of the supplied evidence alone;
replayed evidence is therefore classed `authorization_denied`, indistinguishable
from any other private-state denial, so that resubmitting captured evidence
reveals nothing about whether the original request committed. Under the
private-posture rule, even `key_mismatch` joins the private-state anonymity set.

| Private condition | Public class | Nostr prefix and exact text | HTTP response |
|---|---|---|---|
| assertion/proof absent | `missing_evidence` | `auth-required: authentication required` | `401`; `WWW-Authenticate: Nostr`; `Content-Type: text/plain; charset=utf-8`; `authentication required\n` |
| malformed, invalid, or expired evidence | `evidence_rejected` | `restricted: evidence rejected` | `403`; `Content-Type: text/plain; charset=utf-8`; `evidence rejected\n` |
| replayed evidence; key mismatch; attestation required; binding conflict; retired pair; revoked key; lifecycle gate; binding required/expired; local policy denial | `authorization_denied` | `restricted: authorization denied` | `403`; `Content-Type: text/plain; charset=utf-8`; `authorization denied\n` |
| required current dependency unreadable | `authorization_unavailable` | `restricted: authorization unavailable` | `503`; `Content-Type: text/plain; charset=utf-8`; `authorization unavailable\n` |

Nostr text is the exact UTF-8 text after an applicable NIP-42/NIP-01 prefix.
A denial decided on a WebSocket upgrade request, before any NIP-42 proof
exists, is the HTTP response in the table, sent instead of `101`; a denial
decided after the connection is established is the Nostr text. For HTTP, the
compared denial contract is closed over the
status, complete body,
and exact values of only the header fields named in the table; header order and
other fields are outside that contract and their values cannot depend on the
private condition. The body is the shown UTF-8 bytes with one LF and no other
bytes. The `Nostr` challenge satisfies RFC 9110 Section 15.5.2.
Responses contain no free text, reason code, request ID, issuer, subject, key,
claim, binding state, enrollment posture, token material, or timing hint. All
private conditions in `authorization_denied` produce byte-identical responses.
[FI-TRACE-DENIAL-ORACLE]

NIP-FI defines no public identity projection. Public events, tags, filters,
discovery, responses, logs, metrics, and traces contain no raw assertions or
unredacted `iss`, `sub`, email, display name, or private claim. Access-controlled
authoritative stores retain only what enforcement and investigation require.
A separate presentation protocol cannot confer NIP-FI authority.
[FI-TRACE-PRIVACY-NONPUBLIC]

## Discovery

A relay SHOULD advertise core support in NIP-11 as:

```json
{
  "limitation": { "federated_identity": true },
  "federated_identity": {
    "core": "client-attached",
    "assertion_freshness": {
      "class": "offline-jwt",
      "maximum_residual_upstream_revocation_seconds": null
    }
  }
}
```

NIP-FI-EDGE owns the optional `edge_transports` member and its exact type,
placement, and value semantics. For `current-status`, the final value is a tested
positive integer. Discovery never states enrollment mode or TOFU posture and
never exposes issuer URLs, audiences, claim names, tenant IDs, or
deployment-local identifiers. For a fixed
set of claimed profiles, the complete public discovery output is byte-identical
for every enrollment policy, including `attested-key`, private `tofu`, and any
companion profile mode: no field, flag, value, omission, ordering, or object shape
may distinguish the configured mode. Profile documents own only non-enrollment
public claims.
[FI-TRACE-DISCOVERY-PRIVATE]

## Worked example (non-normative)

A protected HTTP POST under `client-attached` with NIP-98 proof and an
authorization-relevant body. Credentials are elided; the NIP-FI-CONF exit
fixture pins the complete request compared objects.

```text
POST /media HTTP/1.1
Host: relay.example
Nostr-Federated-Identity: Bearer eyJhbGciOiJFUzI1NiIsInR5cCI6ImF0K2p3dCIs...
Authorization: Nostr eyJpZCI6IjE1ZTI3ZDc0Li4uIiwicHVia2V5IjoiOTljNzQ4Li4u...
Content-Type: application/octet-stream
Content-Length: 4

abcd
```

The bearer JWS validates under the configured assertion policy: exact `iss`
and `aud`, token class `at+jwt`, live time claims, and `nostr_pubkey` equal to
the NIP-98 event's `pubkey`. The NIP-98 event binds the server-resolved method
and URL, and its single `payload` tag equals the SHA-256 of the four body
bytes. Admission then follows Direct preparation and Final admission; success
returns the application response, and every failure class returns exactly the
bytes fixed in the Rejection table. On a WebSocket upgrade the same header
attaches to the upgrade request and NIP-42 supplies the proof after connect.

## Core behavioral oracles

A core claim covers every applicable oracle below at one implementation and
policy revision. NIP-FI-CONF defines evidence and mutation-adequacy rules.

| ID | Required outcome |
|---|---|
| `FI-TRACE-TRANSPORT-CLOSED` | Exact one-header input succeeds; missing, repeated, combined, malformed, mixed, URL, and fallback variants deny. |
| `FI-TRACE-ASSERTION-VALIDATION` | Valid boundary input passes; each signature, key-selection, issuer, audience, time, size, ambiguity, and missing-configuration negative denies. |
| `FI-TRACE-TOKEN-CLASS` | An `at+jwt` access token and a dedicated `nip-fi+jwt` assertion pass only their selected class. ID tokens, wrong/generic types outside a named compatibility policy, client-only audiences, absent or ambiguous `client_id`, resource-owner/client-subject ambiguity, and every attempted cross-class fallback deny. |
| `FI-TRACE-CONTRACT-IDENTITIES` | Mutate each assertion semantic, transport semantic, and mutable dependency independently: semantic mutations change only their owning contract ID; snapshot/binding/lifecycle/policy/resource/status mutations change neither ID but force current revalidation. |
| `FI-TRACE-VERIFIER-PARITY` | Equal authoritative input and policy produce the same canonical normalized result. |
| `FI-TRACE-JWKS-ADD` | Retained-key rotation revalidates successfully under the changed snapshot version. |
| `FI-TRACE-JWKS-REMOVE` | Evidence and leases under a removed key deny after snapshot change. |
| `FI-TRACE-CURRENT-STATUS-REVOKED` | Revocation, including one racing final admission, closes authority within the advertised tested bound. |
| `FI-TRACE-CURRENT-STATUS-STALE` | Inactive/ambiguous status denies; an issuer, subject, or session-identifier mismatch denies; expiry equality, outage, delayed events, and changed status versions cannot mint or extend a witness. |
| `FI-TRACE-CAPABILITY-REVOCATION` | Removal of a revocation-bounded external capability projection from authoritative local policy closes prepared evidence and lease use within the declared bound; assertion-only projection cannot satisfy this oracle. |
| `FI-TRACE-BODY-BINDING` | Exact complete relevant body passes; absent/duplicate/mutated/partial/substituted payload variants deny without effects; a payload tag on an irrelevant-body operation validates identically and denies on duplication or mismatch. |
| `FI-TRACE-BODY-BOUNDS` | Oversized, over-quota, and pre-EOF variants deny with bounded work, cleanup, and no effects. |
| `FI-TRACE-DOMAIN-SPOOF` | Client routing and forwarded authority cannot replace server-owned context. |
| `FI-TRACE-ASSERTION-KEY-MISMATCH` | Mismatch denies with no mutation and the private-state response. |
| `FI-TRACE-BINDING-CONFLICT` | A binding conflict denies without replacing either existing binding. |
| `FI-TRACE-TOMBSTONE-REPLAY` | Fresh eligible evidence for a retired pair or revoked key denies without recreation. |
| `FI-TRACE-ASSERTION-REFRESH` | Fresh evidence reuses the same eligible durable binding after prior assertion expiry. |
| `FI-TRACE-PREPARED-STALE` | Changed identity-class witnesses or extended bounds deny; provenance-only rotation revalidates. |
| `FI-TRACE-CONCURRENT-ENROLLMENT` | Identical first use converges; conflicting first use commits at most one winner. |
| `FI-TRACE-FINAL-DENIAL-NO-MUTATION` | Every failed phase leaves all authoritative stores and effects unchanged. |
| `FI-TRACE-LIFECYCLE-AUTHORITY` | Unprivileged/stale transitions deny; authorized retirement/revocation/rotation is atomic. |
| `FI-TRACE-LEASE-BOUND` | A lease ends at its earliest bound; equality at any bound is expired. |
| `FI-TRACE-MULTI-KEY-SESSION` | One actor's lease never authorizes another key on the same connection. |
| `FI-TRACE-DENIAL-ORACLE` | Each private row produces its exact fixed bytes on every surface where its condition can be decided — HTTP, a WebSocket upgrade, or after connect; all private-state rows compare byte-identical. |
| `FI-TRACE-DEPENDENCY-FAIL-CLOSED` | Each unreadable authoritative dependency denies. |
| `FI-TRACE-AUTHORITY-UNIFORM` | Every protected ingress reaches one current final-admission authority. |
| `FI-TRACE-CROSS-DOMAIN-COLLISION` | Equal subjects across issuers and equal pairs across domains remain distinct. |
| `FI-TRACE-PRIVACY-NONPUBLIC` | Private identity does not enter public surfaces. |
| `FI-TRACE-DISCOVERY-PRIVATE` | Complete discovery bytes remain identical across attested-key, TOFU, and companion enrollment modes. |
| `FI-TRACE-TOFU-THEFT` | Stolen-assertion first use denies unless private TOFU is enabled and the attacker also proves its chosen key. |

## Relationship to other work (non-normative)

NIP-FI binds an access token to a key the resource server itself verifies, the
goal DPoP (RFC 9449) and mTLS-bound tokens (RFC 8705) reach through a `cnf`
claim. Here the proof is the NIP-42 or NIP-98 event the relay already
validates, so no second proof is defined and the issuer need not attest the
key; `nostr_pubkey` is the optional `cnf` analogue. Unlike those profiles the
binding is durable server state rather than a per-token claim: a stolen
assertion cannot reach an enrolled identity without its key, and revocation is
a local fact rather than a token-lifetime race. One identity, one key per
domain is stricter than WebAuthn's many-credentials-per-account model because
the Nostr key is itself the public identity; additional devices do not create
additional active bindings. Two contract identities plus explicit dependency
versions exist because folding a mutable key snapshot into policy identity would make benign
rotation change policy lineage, while omitting it would let evidence under a
removed key survive. Denial responses deliberately collapse the conditions that
RFC 6750 error codes distinguish. `trusted-proxy-hmac-v2` in NIP-FI-EDGE is a
fixed-component request MAC in the family of HTTP Message Signatures (RFC 9421)
and AWS SigV4, without negotiation and with length-prefixed canonicalization.

## Security considerations

Issuer compromise can impersonate a principal but cannot prove an uncompromised
bound Nostr key. Assertion theft cannot use an existing binding without that
key; private TOFU intentionally retains first-use theft risk. Snapshot
revalidation limits removed-key reuse but the base policy accepts authenticated
key-source rollback as residual issuer risk. Two-phase admission closes the
binding and policy TOCTOU window only when every authoritative witness is reread
atomically. Availability failures deny rather than degrade to Nostr-only access.

## Sources

- NIP-42 authentication: <https://github.com/nostr-protocol/nips/blob/6d2979b3f503a8539c983efbcdcf901bbcf9ed23/42.md>
- NIP-98 HTTP authentication: <https://github.com/nostr-protocol/nips/blob/6d2979b3f503a8539c983efbcdcf901bbcf9ed23/98.md>
- JWT BCP: <https://www.rfc-editor.org/rfc/rfc8725>
- JWT access-token profile: <https://www.rfc-editor.org/rfc/rfc9068>
- DPoP: <https://www.rfc-editor.org/rfc/rfc9449>
- OAuth 2.0 mTLS client certificate-bound tokens: <https://www.rfc-editor.org/rfc/rfc8705>
- HTTP Message Signatures: <https://www.rfc-editor.org/rfc/rfc9421>
- Non-normative composed model: [NIP-FI-MODEL.md](NIP-FI-MODEL.md)
