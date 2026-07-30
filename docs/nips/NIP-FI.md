NIP-FI
======

Federated Identity Authorization
--------------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-42 (Authentication of Clients to Relays). **Composes with**: NIP-98 (HTTP Auth), NIP-11 (Relay Information Document), and optionally the draft NIP-OA (Owner Attestation) as one delegation-proof format.

## Abstract

This NIP defines how a relay or Nostr-adjacent HTTP service authorizes an already-authenticated Nostr key only when a valid federated identity assertion resolves to the same principal and key. It specifies assertion transport, validation, an identity-to-key binding lifecycle (enroll, conflict, retire, disable, revoke, recover, rotate, and re-enable), session semantics, and failure behavior. A separately validated delegation MAY derive narrower authority from a bound owner as described below; that exception does not turn the delegate into the federated principal.

The identity provider never becomes a Nostr signing authority, and the assertion never substitutes for Nostr proof of key control. This NIP is an authorization layer above NIP-42 and NIP-98, not a replacement for either.

## Motivation

Organizations deploying Nostr internally need relay access tied to their workforce identity system: an employee's relay privileges should follow their corporate identity, survive Nostr key rotation, and end at offboarding. Existing primitives each solve part of this:

- NIP-42 proves control of a Nostr key to a connection but carries no external identity.
- NIP-05 maps an organization-controlled identifier to a pubkey, but by public DNS/HTTPS polling, not by a credential presented on the request being authorized.
- NIP-46 lets a signer demand out-of-band authentication (`auth_url`) but does not bind the resulting external subject to a key at the relay.

Without a standard, each deployment invents an incompatible binding scheme, and the first large deployment's configuration becomes an accidental protocol. This NIP defines the contract so that any relay behind any OIDC-capable identity provider or generic OAuth2 reverse proxy (Okta, Auth0, Keycloak, oauth2-proxy, etc.) can interoperate with any conforming client.

## Definitions

- **assertion**: a JWT issued by a configured identity provider, presented alongside (never instead of) Nostr authentication.
- **federated identity** (`i`): the tuple `(iss, sub)` from a validated assertion. The `iss` value MUST be the exact validated issuer identifier and `sub` the exact non-empty subject string. A username, email, display name, or bare `sub` MUST NOT be used as a federated identity.
- **authorization domain** (`D`): the scope within which bindings apply, resolved by the verifier from authenticated server routing or configuration (an entire relay, or one tenant of a multi-tenant relay). An assertion, proof, header value, or other untrusted request input MUST NOT select or rewrite `D`, and bindings MUST NOT cross domains implicitly.
- **binding**: an active record associating exactly one federated identity with exactly one 32-byte Nostr public key within a domain. A binding MAY carry an authoritative expiry.
- **retired pair**: a durable denial selector recording that one exact `(identity, key)` pair MUST NOT be recreated by ordinary authorization.
- **disabled identity**: a durable denial selector preventing an identity from authorizing or enrolling any key.
- **revoked key**: a durable denial selector preventing a key from authorizing or binding to any identity.
- **pending replacement**: lifecycle state recording that an identity whose prior key was retired MUST use a separately authorized recovery transition, or re-enablement when disabled, before another key can become active.
- **enrollment mode**: the domain's policy for creating bindings — `attested-key`, `provisioned`, or `tofu` (defined below).
- **Nostr proof**: a valid NIP-42 AUTH event (WebSocket) or NIP-98 event (HTTP) proving control of a key on the current connection or request.
- **direct lease**: a cached direct authorization decision for one `(domain, identity, key)`, bounded by the assertion's expiry and every shorter known binding, policy, or implementation limit.
- **delegated lease**: a cached delegated authorization decision for a delegate key, dependent on an active owner binding and bounded by a mandatory finite configured implementation limit and every shorter known owner-binding, delegation, or policy limit. It has no independent assertion-expiry bound unless a stronger deployment policy requires a current owner assertion or direct lease.

## Assertion transport

An assertion reaches the verifier in an HTTP header on the request being authorized: the WebSocket upgrade request for relay connections, or each individual request for NIP-98-authenticated HTTP endpoints. Two transport profiles are defined; a service MUST document which it accepts.

1. **Trusted proxy**: an authenticating reverse proxy (for example oauth2-proxy or an SSO-aware ingress) injects the assertion after authenticating the user. The injected header name is deployment configuration. This profile is conforming only if untrusted clients cannot reach the verifier directly and the proxy strips every inbound copy of that header before setting it. This is the recommended profile for browser-based clients, which cannot attach arbitrary WebSocket upgrade headers.
2. **Client-attached**: the client sends the assertion itself in `Nostr-Federated-Identity: Bearer <JWT>`. A verifier MAY additionally accept another documented header on WebSocket upgrades, including `Authorization: Bearer`; HTTP requests using NIP-98 MUST use `Nostr-Federated-Identity` because their `Authorization` header carries the `Nostr` proof.

Assertion acquisition and interactive OIDC login are outside this NIP. A client-attached assertion value MUST use the `Bearer` scheme; after removing that scheme, the value MUST contain exactly one JWT and no comma-separated alternatives.

Normal browser WebSocket APIs cannot attach the client-attached header. Browser deployments therefore require the trusted-proxy profile or a separately standardized assertion transport. Bearer assertions MUST NOT be placed in WebSocket URLs or query strings.

On a WebSocket connection, the assertion captured at upgrade is evaluated when a key performs NIP-42 AUTH — each authenticating key is authorized against that assertion independently. On HTTP, the assertion and the NIP-98 proof MUST arrive on the same request they authorize.

Assertions MUST NOT be carried inside Nostr events, event tags, or subscription filters, and MUST NOT be written to relay-visible event history.

## Assertion validation

The verifier is configured, per accepted issuer, with: the issuer identifier, a signing-key source (a JWKS endpoint, discoverable via OIDC `/.well-known/openid-configuration`), accepted audience values, and optional Nostr-key and display-name claim mappings. Validation MUST enforce all of the following; any failure MUST reject the assertion:

1. The JWT signature verifies under a currently trusted key for an explicitly allowed **asymmetric** algorithm. Symmetric (HS*) and `none` algorithms MUST be rejected before any key lookup.
2. `iss` exactly equals the configured issuer identifier used to select the verification key.
3. At least one `aud` value exactly equals a configured audience.
4. `exp` is present and in the future; `nbf` and `iat`, when present, are no later than verifier time plus a bounded, configured clock skew. An `iat` beyond that allowed skew is future-dated and fails validation.
5. The JWT `sub` claim is present, a non-empty string, and unambiguously a single value. Base V1 always defines `i = (iss, sub)`; mapping another claim into a local principal is a deployment extension and MUST NOT be advertised as base V1 conformance.
6. If a key claim is configured and present, it parses to exactly one 32-byte Nostr public key. Lowercase hex is the canonical encoding; a bare NIP-19 `npub` using Bech32 (not Bech32m) MAY be accepted as a documented input normalization. That optional decoder accepts only a bare NIP-19 Bech32 — not Bech32m — `npub` that is all lowercase or all uppercase, whose lowercased prefix is exactly `npub`, with a valid checksum and exactly 32 decoded bytes. It rejects mixed case, Bech32m, another prefix, a TLV form, invalid checksum, or another payload length, and converts either permitted case to the same canonical lowercase hex before comparison, so one claim has only one key interpretation.

Display-name, email, and similar profile claims MAY be extracted as mutable metadata. They MUST NOT participate in any authorization decision.

Signing-key retrieval failures MUST fail closed. Verifiers SHOULD cache the key set with a bounded lifetime and SHOULD NOT refetch it in response to an unknown `kid` that was absent from a freshly fetched set, so that forged tokens cannot drive request floods to the identity provider.

## Nostr proof

The key being authorized is always the key returned by Nostr proof validation — a valid NIP-42 AUTH for the current WebSocket connection, or a valid NIP-98 event for the current HTTP request. It is never taken from an assertion claim, an unsigned request field, or client metadata. A bearer assertion alone MUST NOT authenticate a Nostr key.

## Authorization

Given a validated assertion yielding identity `i`, optional asserted key `k_a`, and expiry `exp`, and a Nostr proof yielding key `k`, the verifier evaluates one atomic decision in the trusted server-resolved domain `D`:

```text
Authorize(D, i, k_a?, k):
  if k_a exists and k_a != k:            DENY (key mismatch)

  atomically read:
    b_i := active binding for i in D, if any
    b_k := active binding for k in D, if any
    p   := whether (i, k) is a retired pair in D
    x   := whether i is disabled in D
    y   := whether k is revoked in D
    q   := whether i is pending explicit replacement in D

  if b_i = (i, k) and b_k = (i, k)
     and not (p or x or y or q):           preserve source; ALLOW (existing binding)
  if b_i exists or b_k exists:           DENY (binding conflict)
  if x:                                   DENY (identity disabled)
  if y:                                   DENY (key revoked)
  if p:                                   DENY (pair retired)
  if q:                                   DENY (explicit replacement required)

  # no active binding or applicable lifecycle gate: first enrollment
  attested-key:  k_a required, else DENY; create (i, k, source=attested-key); ALLOW
  provisioned:   DENY (binding must be pre-created by an operator)
  tofu:          create (i, k, source=(k_a exists ? attested-key : tofu)); ALLOW
```

The active-binding and lifecycle-gate reads, and any insertion, MUST be one linearizable transition for `(D, i)` and `(D, k)`. They MUST serialize with provisioning and every lifecycle transition affecting those selectors, including pair retirement, identity disablement, key revocation, rotation, recovery, and re-enablement. Under concurrent first use of the same identity or key, at most one binding is created and every other attempt observes it (allow on exact match, deny on conflict). Missing lifecycle state, storage failure, or a race whose committed result cannot be read MUST deny — never fall back to an unchecked allow.

### Enrollment modes

- **`attested-key`**: the identity provider carries the user's Nostr public key in the configured key claim. First use binds only when the asserted key equals the proven key. This is the strongest mode and SHOULD be used when the identity provider can carry custom claims.
- **`provisioned`**: bindings are created only through an out-of-band administrative process; requests never create bindings.
- **`tofu`** (trust on first use): first use of an unbound identity with an unbound key creates the binding. A stolen assertion for a never-enrolled identity can bind an attacker's key in this mode; services offering it MUST document this risk. When an assertion in `tofu` mode carries a valid key claim, the binding SHOULD record the stronger `attested-key` provenance, and a binding's recorded provenance MUST NOT be downgraded by later requests.

In `provisioned` mode, creation requires a separately authorized `ProvisionBinding` transition. That transition uses the operation- and domain-bound lifecycle authority described in [Revocation and rotation](#revocation-and-rotation) and formalized as `LifecycleAuthorization(D, provision, i, k)` in the model. It MUST verify the configured mode, that no active binding exists for `i` or `k`, that `i` is neither disabled nor pending replacement, that `k` is not revoked, and that the exact pair `(i, k)` is not retired. Its transition shape is `ProvisionBinding(D, i, k): atomically(require LifecycleAuthorization(D, provision, i, k); require provisioned mode and every eligibility condition above; create (i, k, provisioned); append provision history)`, followed by no direct lease. A failed requirement, including an unreadable mode, active-binding, or lifecycle-selector read, rolls back the entire shape. A later direct `Authorize` still requires a valid assertion for `i` and fresh Nostr proof by `k`. Delegated authorization remains subject to the separate active-owner, delegation-proof, and deployment-admission rules below.

### Binding invariant

Within a domain, active bindings form a partial bijection: an identity has at most one active key and a key has at most one active identity. An active binding MUST NOT overlap a retired pair, disabled identity, revoked key, or pending-replacement identity. Every state transition in this NIP preserves these invariants.

Pending replacement is identity-scoped and blocks every active binding for that identity until an explicit recovery transition—or re-enablement when the identity is disabled—succeeds. Exact-pair retirement remains pair-scoped: by itself it does not revoke the retired key for a different identity in the same authorization domain.

Base V1 therefore has one active principal key per domain. Multiple devices either share that principal key or use bounded delegation. Supporting multiple simultaneously active principal keys requires a future protocol extension.

Base V1 also treats the verifier's binding and lifecycle state as authoritative. A signed external binding authority is a future extension and requires explicit claim, conflict, rotation, revocation, and migration semantics; a normal assertion under this NIP does not transfer that authority.

## Session semantics

For HTTP requests, the decision applies to that request only.

For a NIP-42 WebSocket connection, the relay MAY cache the decision as a direct lease. Its expiry MUST be no later than the assertion's `exp` and every shorter known binding-expiry, policy, or configured implementation bound. At expiry the relay MUST reject protected operations or close the connection. Renewal requires a new WebSocket connection carrying a fresh assertion on its upgrade request, followed by fresh NIP-42 proof; base V1 defines no in-connection renewal message. When a relay learns that a binding, identity, key, policy decision, or delegation on which a lease depends is no longer valid, it MUST invalidate every matching direct and delegated lease. A relay that detects revocation by polling MUST NOT claim immediate revocation and SHOULD document its maximum detection latency.

When multiple keys authenticate on one connection (NIP-42 permits this), authorization is tracked per key. A lease for one key MUST NOT authorize operations attributed to another.

## Revocation and rotation

Provisioning and lifecycle changes are explicit administrative or policy transitions, never side effects of `Authorize`. Each requires authenticated authority for the named operation in the selected domain. This NIP defines state semantics, not an operator transport, approval policy, retry protocol, or complete audit schema; an implementation MAY wrap the transitions in an idempotent operation-ID interface with actor, reason, correlation, and durable audit evidence.

The storage representation is implementation-defined, but each transition's denial selectors, active-binding changes, and lifecycle-state-history append MUST commit atomically. That history preserves state and replacement lineage; it is not by itself the complete operational audit contract. After a successful commit, the implementation MUST trigger invalidation of affected direct and dependent delegated leases within its documented detection bound; cache invalidation need not share the state transaction:

- **Retire pair**: recheck within the atomic transition that the active pair's identity has no pending-replacement selector; a present or unreadable selector denies without mutation instead of overwriting lineage. Then remove `(i, k)`, retain an exact-pair tombstone, mark `i` pending explicit replacement, and invalidate matching direct and dependent delegated leases after commit.
- **Disable identity**: record the identity selector even when `i` has never enrolled. If `i` has an active binding, recheck within the atomic transition that its pending-replacement selector is absent; a present or unreadable selector denies and rolls back the entire invocation. Then remove the active pair, add an exact-pair tombstone, and mark `i` pending explicit replacement. If no active binding exists and `i` was already pending replacement, preserve that state. After commit, invalidate direct and dependent delegated leases for `i`.
- **Revoke key**: record the key selector even when `k` is not active. If `k` has an active binding, recheck within the atomic transition that its identity has no pending-replacement selector; a present or unreadable selector denies without mutation instead of overwriting lineage. Then remove the active pair, retire it, and mark its identity pending explicit replacement. If no active binding exists, preserve every pending-replacement selector unchanged. After commit, invalidate every direct or delegated lease that depends on `k`.

Every privileged lifecycle transition affecting the same domain, identity, or key MUST be linearizable with every other such transition and with `Authorize`. Its preconditions MUST be rechecked within the atomic transition, so concurrent provision, retire, disable, revoke, rotate, recover, or re-enable operations cannot partially commit or overwrite one another.

A subsequent valid assertion — including one whose key claim matches a retired key — cannot clear these selectors or create a replacement binding. This prevents a replayed, still-valid assertion and a routine login with a different key from silently undoing revocation.

Rotation and recovery are distinct privileged transitions. `Rotate` requires an active `(i, k_old)` binding and an absent pending-replacement selector for `i`; `Recover` requires both `Q(i) = k_old` and the exact retired pair `(i, k_old)`, plus no active binding for `i`. `Rotate` reads and rechecks the selector's absence inside the same atomic transition as its mutation; a present or unreadable selector denies without mutation. `Recover` likewise reads and rechecks both pending lineage and the exact retired-pair selector inside the same atomic transition; missing, mismatched, or unreadable state denies without mutation. It conditionally compare-and-clears `Q(i)` from `k_old` to absent inside that atomic transition; if the comparison loses to a concurrent lifecycle change, the entire invocation denies and rolls back with no binding, selector, or history mutation, lease, or invalidation. Both transitions require that `i` is not disabled, no active binding exists for `k_new`, `k_new` is not revoked, the exact pair `(i, k_new)` is not retired, and fresh target-bound Nostr proof by `k_new` is supplied for the privileged request. A retired pair containing `k_new` and a different identity does not make `k_new` ineligible. Any assertion supplied as key-attestation evidence to either transition MUST currently validate to identity `i` with key claim `k_new`; a stale assertion, different identity, absent key claim, or different key is invalid evidence and cannot be treated as if no attestation was supplied. Where the domain requires issuer attestation, valid matching evidence MUST be supplied.

`Rotate` atomically removes the active old pair from `B`, adds the exact `(i, k_old)` pair to retired-pair selector `P`, and creates `(i, k_new)`. `Recover` preserves the already-retired old pair, clears pending replacement, and creates `(i, k_new)`. Each records `attested-key` provenance when matching attestation evidence is present and `provisioned` provenance otherwise, and appends a distinct rotation or recovery history entry. After commit it invalidates direct and dependent delegated leases for the old pair. Neither transition revokes `k_old` across the authorization domain; key revocation remains the separate `RevokeKey` transition. A routine request presenting a new key is either a binding conflict or `explicit replacement required` and MUST be denied without mutation, including no binding, lifecycle, enrollment, publication, or last-seen update; a redacted security audit record is allowed.

Evidence carried by ordinary `Authorize`, including its Nostr proof, cannot satisfy lifecycle authorization or invoke `Rotate`, `Recover`, or `EnableIdentity`. Replacement-key proof is validated only inside the separately authorized lifecycle transition.

Base V1 recovery uses a replacement key that has no active binding, is not revoked in the authorization domain, and has never formed a retired pair with `i`. A retired pair with another identity does not by itself revoke the key across that domain; only the distinct domain-scoped key-revocation selector does. Same-pair reactivation is an extension and MUST provide an equivalently explicit privileged transition while retaining the original lifecycle history; ordinary `Authorize` can never perform it.

Base V1 has no standalone operation that merely clears a disabled-identity selector. Re-enablement requires a privileged `EnableIdentity` transition that selects an eligible replacement or first key, requires fresh target-bound Nostr proof by that key, and atomically creates its binding, clears the disabled selector, conditionally clears a present pending-replacement selector, and appends identity-enablement history. The transition reads `Q(i)` inside that atomic transition and fails closed if it is unreadable: an absent selector remains absent; when `Q(i) = k_old` is present, the transition rechecks the exact retired pair `(i, k_old)` and conditionally compare-and-clears that exact selector to absent. Missing, mismatched, or unreadable retired-pair state denies without mutation instead of clearing lineage. If the conditional comparison loses to a concurrent lifecycle change, the entire invocation denies and rolls back with no binding, selector, or history mutation, lease, or invalidation. Any assertion supplied as key-attestation evidence to the transition MUST currently validate to identity `i` with a key claim equal to the selected key; a stale assertion, different identity, absent key claim, or different key is invalid evidence and cannot be treated as if no attestation was supplied. Where the domain requires issuer attestation, valid matching evidence MUST be supplied. Matching attestation records `attested-key` provenance; otherwise the transition records `provisioned`. After commit, invalidate any direct and dependent delegated leases for a prior retired pair. For a never-enrolled identity there is no prior pair, but the new binding is still created in the same transaction that clears disablement. A prior key remains pair-retired; revoking it across the authorization domain requires the separate `RevokeKey` transition. Ordinary enrollment therefore never observes a re-enabled identity with neither an active binding nor a lifecycle gate.

## Delegation

Delegation is outside the base primitive but composes with it. A service MAY admit a key that presents no assertion when a separately validated delegation proof (for example a NIP-OA `auth` tag) establishes an owner key that holds an active binding in the domain. A cached owner lease MUST NOT substitute for that active binding. This delegation path MUST NOT create or modify any federated identity binding or lifecycle selector for either the owner or delegate key. The delegated decision retains an explicit dependency on the active owner binding, intersects the delegated operations and conditions, and expires no later than a mandatory finite configured implementation limit and every shorter known owner-binding, delegation, or policy bound. A service without that finite maximum MUST NOT issue delegated leases or advertise delegation support. Because the delegate presents no assertion, its lease has no independent assertion-expiry bound; if a deployment additionally requires a current owner assertion or direct lease, that bound is included too. Revoking or retiring the owner binding invalidates dependent delegated leases on the same detection schedule as the owner's own leases. A deployment MAY require a stronger current-provider admission decision for the owner, but that is an additional authorization layer rather than part of this base binding primitive.

## Rejection semantics

Machine-readable rejection classes and their transport mapping reuse NIP-01/NIP-42 prefixes on `OK` and `CLOSED` messages:

- `auth-required: ` — no assertion was presented, or no applicable Nostr proof has been performed or presented for the protected operation: NIP-42 for WebSocket or NIP-98 for HTTP.
- `restricted: ` — the assertion or proof was presented but failed validation, mismatched, conflicted with an active binding, or the identity's enrollment/binding state does not permit the operation.
- Transport mapping (not a new prefix) — a protected operation on an established WebSocket connection by a key without applicable NIP-42 proof uses `auth-required`; an HTTP protected request without applicable NIP-98 proof uses `auth-required` and status `401`.


HTTP endpoints respond `401` where `auth-required` applies and `403` where `restricted` applies. Rejection bodies MUST NOT echo assertion contents, claim values, or the conflicting party's identity or key.

## Discovery

A relay SHOULD advertise support in its NIP-11 document under `limitation` as `"federated_identity": true`. It MAY additionally include this top-level object:

```json
{
  "federated_identity": {
    "transports": ["trusted-proxy", "client-attached"],
    "enrollment": "attested-key",
    "delegation": true, "delegated_lease_max_seconds": 300
  }
}
```

The value `300` is illustrative; an implementation publishes its actual configured finite maximum.

`transports` contains the supported profile names from this NIP, `enrollment` is exactly one enrollment mode, and `delegation` states whether separately validated delegation may be honored. When `delegation` is `true`, the object MUST also include `delegated_lease_max_seconds` as a positive integer advertising the configured finite upper bound; when `delegation` is `false`, that field SHOULD be omitted. A relay MUST NOT advertise delegation without that finite bound. Unknown fields MUST be ignored. A relay MUST NOT publish issuer-internal detail (tenant URLs, claim names, audiences) that is not already public.

## Privacy

Federated identities are typically personal data (employee identifiers). NIP-FI itself MUST NOT publish `iss`, `sub`, assertion contents, or display-name claims in Nostr events or tags, and a conforming service MUST NOT expose another user's binding state through rejection messages. Access-controlled binding and lifecycle records are service-internal and MAY retain the identifiers needed to enforce and audit the state machine. Operational logs and metrics MUST NOT record raw bearer assertions or unredacted `iss`, `sub`, display-name, email, or other private assertion claims; redacted or pseudonymous security records are allowed.

A separate, opt-in relay-signed projection protocol such as NIP-85 MAY publish an approved label. Such a projection MUST NOT contain `iss`, `sub`, bearer material, or other unapproved private claims, and it MUST NOT be accepted as NIP-FI authorization evidence.

## Security considerations

- **Issuer or proxy compromise** impersonates federated principals, but cannot satisfy Nostr proof for an already-bound uncompromised key, and in `attested-key` mode cannot bind an arbitrary key without also forging the key claim.
- **Assertion theft** cannot authorize an already-bound identity without control of the bound key. Its remaining power — enrolling a never-bound identity — exists only in `tofu` mode, which is why that mode is risk-labeled.
- **Header injection**: the trusted-proxy profile is void if clients can reach the verifier directly or the proxy forwards inbound copies of the assertion header. Deployments MUST verify both properties. Verification evidence identifies the enforced origin-isolation control — for example a network ACL, mutually authenticated proxy-to-verifier channel, or local socket boundary — and records negative tests showing that bypass ingress is unreachable and a client-supplied assertion header is stripped or replaced before trusted injection.
- **Algorithm confusion** is excluded by rejecting symmetric algorithms before key selection.
- **Availability vs. safety**: issuer, key-set, and storage outages deny. Availability MUST NOT override identity safety.
- **Cross-issuer collision**: the same exact literal `sub` value under different issuers denotes distinct identities, which MUST never collide or inherit each other's bindings.

The companion [formal model](NIP-FI-MODEL.md) defines the state machine and safety/liveness properties. The [conformance matrix](NIP-FI-CONFORMANCE.md) supplies stable, reviewable success, denial, concurrency, lifecycle, session, disclosure, and privacy traces.

## Implementation relationship

Buzz PR [#1476](https://github.com/block/buzz/pull/1476), reviewed at revision `1e9822de8dbe0ae91c00c0ce0ed8ff583915692f`, is a disabled partial foundation from which this provider-neutral contract was generalized. Its default identity-claim selection is `sub`, but that revision trims string claims; preserving the exact literal `sub` value, future-`iat` rejection, NIP-11 discovery, and additional lifecycle and lease conformance remain additive implementation work. Its combined replacement helper and domain-scoped old-key revocation also do not implement this draft's distinct rotation, recovery, and pair-retirement semantics. A configured non-`sub` mapping is a deployment extension and cannot claim Base V1 identity conformance. This draft does not require modifying the reviewed revision.
