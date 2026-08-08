# Scope

This model specifies a relay or HTTP service authorizing a Nostr principal only when a valid federated identity assertion and a valid Nostr proof resolve to the same active identity-to-key binding. It models authorization, enrollment, revocation, and key rotation. It does not publish the federated identity on Nostr and does not make the identity provider a Nostr signing authority.

The model is transport-independent. A concrete NIP must separately define how an assertion reaches a verifier and how support is advertised. NIP-42 and NIP-98 remain the mechanisms for proving control of a Nostr key; a bearer assertion alone is never a Nostr proof.

# Terms and domains

- `D`: authorization domain chosen by the service (for example one relay tenant). Bindings never cross domains implicitly.
- `I`: federated principal, the tuple `(iss, sub)`. `iss` is the assertion's exact validated issuer identifier and `sub` is its exact non-empty subject string. A username, email, display name, or bare `sub` is not an identity key.
- `K`: 32-byte Nostr public key.
- `A`: federated assertion.
- `P`: Nostr proof authenticating key `k`, such as a valid NIP-42 AUTH event or NIP-98 event.
- `now`: verifier time.
- `B_D`: active binding relation in domain `D`, a partial bijection between `I` and `K`.
- `R_D`: durable history of revoked bindings.
- `mode(D)`: enrollment policy, either `attested-key`, `provisioned`, or `tofu`.

A binding record is:

```text
Binding = (domain, identity, key, source, created_at, revoked_at?)
source  = attested-key | provisioned | tofu
```

`display_name`, email, and similar values may be stored as mutable metadata but are never part of binding identity or an authorization decision.

# Trust assumptions

1. The verifier has an authenticated configuration for each accepted issuer: issuer identifier, allowed signing algorithms, key source, accepted audience(s), and claim mapping.
2. TLS and/or a trusted ingress boundary prevents attackers from injecting or replacing assertions. A reverse-proxy assertion header is trusted only when untrusted clients cannot reach the verifier directly and all inbound copies of that header are stripped before the trusted proxy sets it.
3. The issuer protects its signing keys and assigns stable, non-reassignable `sub` values within an issuer. If an issuer reassigns a subject, the model cannot distinguish the people.
4. The Nostr signature primitive is unforgeable and the concrete Nostr proof is fresh and bound to the target relay or HTTP request.
5. Binding-state transactions are serializable with respect to the same domain, identity, or key. The implementation may realize this with locks and unique constraints.
6. The verifier's clock is sufficiently accurate for assertion and proof freshness checks.

Compromise of an accepted issuer or trusted ingress can impersonate federated principals. It still cannot satisfy Nostr proof for an already-bound uncompromised key, and in `attested-key` mode it cannot bind an arbitrary key unless the compromised issuer also attests that key. Theft of an assertion alone cannot authorize an already-bound identity without control of the bound Nostr key.

# Assertion validity

Let `ValidateAssertion(A, C, now)` return either `(i, k_a?, exp)` or failure under issuer configuration `C`.

It succeeds only if all of the following hold:

1. the signature validates under a currently trusted key and an explicitly allowed asymmetric algorithm;
2. `A.iss` exactly equals the configured issuer identifier used to select that key;
3. at least one `A.aud` value exactly equals an audience configured for this service;
4. `exp` exists and `now < exp`, allowing only a bounded configured clock skew;
5. if present, `nbf <= now` and `iat` is not unreasonably in the future;
6. the configured subject claim is a non-empty string;
7. `i = (A.iss, A.subject)`; and
8. if a configured Nostr-key claim is present, it parses to exactly one 32-byte key `k_a` (hex on the wire; bech32 may be accepted only as an explicitly documented input normalization).

Unknown issuers, key IDs, algorithms, claims, and validation failures fail closed. Key retrieval failure also fails closed. A verifier must bound key-cache lifetime and refresh behavior; it must not accept a token merely because parsing succeeded.

# Nostr-proof validity

`ValidateProof(P, target, now) = k` only when the applicable Nostr standard verifies the event ID and Schnorr signature, freshness, and target binding:

- NIP-42: kind, challenge, relay URL, and timestamp are valid; or
- NIP-98: kind, absolute request URL, HTTP method, timestamp, and payload hash when required are valid.

A service may define another proof profile only if it has equivalent signer-control, freshness, and target/replay binding. The key used for the authorization decision is the key returned by proof validation, never an unsigned request field or assertion display claim.

# Binding invariant

For every domain `D`, active bindings are one-to-one:

```text
∀ i, k1, k2: (i, k1) ∈ B_D ∧ (i, k2) ∈ B_D ⇒ k1 = k2
∀ i1, i2, k: (i1, k) ∈ B_D ∧ (i2, k) ∈ B_D ⇒ i1 = i2
```

Equivalently, an active identity has at most one key and an active key has at most one identity in a domain.

# Authorization and enrollment transition

Given domain `D`, assertion result `(i, k_a?, exp)`, and proof result `k`, evaluate one atomic transaction:

```text
Authorize(D, i, k_a?, k):
  if k_a exists and k_a != k:
      DENY(key_mismatch)

  b_i := active binding in B_D for i, if any
  b_k := active binding in B_D for k, if any

  if b_i = (i, k) and b_k = (i, k):
      ALLOW(existing)

  if b_i exists or b_k exists:
      DENY(binding_conflict)

  switch mode(D):
    attested-key:
      if k_a is absent: DENY(key_attestation_required)
      atomically insert (i, k, attested-key) into B_D
      ALLOW(created)
    provisioned:
      DENY(binding_required)
    tofu:
      atomically insert (i, k, source = k_a exists ? attested-key : tofu) into B_D
      ALLOW(created)
```

If a concurrent attempt finds the identical committed binding, it allows as `existing`; if the committed outcome cannot be read or storage is unavailable, deny — never fall back to an unchecked allow. The check and possible insertion must be linearizable for `(D, i, k)`.

The resulting authorization lease is:

```text
L = (D, i, k, binding_version, expires_at)
expires_at <= assertion.exp
```

An implementation may impose a shorter maximum lease. A lease authorizes only policy-selected operations in `D`; it does not authorize signing and does not imply that event authors may differ from `k`.

# Session behavior

For a single HTTP request, the assertion, Nostr proof, and authorization decision apply only to that request.

For a NIP-42 WebSocket connection, a relay may cache `L`, but it must not use the lease after `expires_at`. It must reject protected operations or terminate the connection; obtaining a fresh assertion and proof requires a new connection under this transport profile. A relay that learns that the binding or federated session was revoked must invalidate matching leases. Implementations must document their maximum revocation-detection latency; they cannot claim immediate revocation if they only poll.

If multiple keys authenticate on one NIP-42 connection, authorization is tracked independently per key. A lease for one `(i, k)` must not authorize another authenticated key.

# Revocation and rotation

Revocation is an explicit administrative transition:

```text
Revoke(D, i, k):
  require (i, k) ∈ B_D
  atomically remove (i, k) from B_D
  append immutable revocation record to R_D
  invalidate cached leases for the binding as soon as observed
```

An assertion, including one with `k_a = k`, must not silently reactivate the same revoked binding unless the domain's explicit recovery policy authorizes that transition. This prevents replay of a still-valid assertion from undoing revocation.

Key rotation is not an authorization side effect:

```text
Rotate(D, i, k_old, k_new):
  require explicit recovery/admin authorization
  require (i, k_old) ∈ B_D
  require no active binding for k_new
  if issuer-attested rotation is required, require fresh k_a = k_new
  atomically revoke (i, k_old) and create (i, k_new)
  invalidate leases for k_old
```

A normal request that presents `i` with `k_new` while `k_old` is active is a conflict and must not rotate automatically.

# Delegation

Delegation is outside the base identity-binding primitive. A separate delegation standard may allow a bound owner key to authorize a delegate key. If supported, the verifier must first validate the delegation proof and derive the owner key, then require an active, unexpired authorization lease or binding for that owner. It must not create a federated identity binding for the delegate unless explicitly specified. Delegation expiry/revocation and allowed operations remain bounded by both the owner identity authorization and the delegation.

# Safety properties

Under the trust assumptions, for direct (non-delegated) authorization:

1. **Proof possession:** every allowed protected operation is associated with a valid proof of control of its Nostr key.
2. **Federated authenticity:** every allowed protected operation is associated with a currently valid assertion for its issuer-qualified identity.
3. **Agreement:** if the issuer supplies a key claim, the asserted key, proven key, and bound key are equal.
4. **Binding consistency:** no two active identities share a key and no identity has two active keys in one domain.
5. **No implicit rotation:** conflicting assertions or proofs cannot replace an active binding.
6. **Domain separation:** authorization in one domain does not imply authorization in another.
7. **Lease boundedness:** no cached authorization survives assertion expiry; after revocation is observed, no matching cached authorization remains valid.
8. **Fail-closed storage and verification:** validation, key retrieval, or binding-state failures never produce allow.
9. **Privacy:** conforming protocol behavior need not publish `iss`, `sub`, JWTs, email, or display names in Nostr events or relay-visible event history.

# Liveness properties

Assuming the issuer, key source, binding store, and network are available:

1. a valid assertion and matching proof for an existing active binding are eventually authorized;
2. an unbound pair is eventually authorized exactly once when the configured enrollment mode permits it;
3. after an authorized revocation/rotation and bounded cache invalidation, the old key is denied and the new valid binding can be authorized.

Liveness is intentionally not guaranteed during issuer/JWKS/storage outage; availability must not override identity safety.

# Representative attack traces

| Trace | Required result |
|---|---|
| Valid assertion for `i`, attacker proves unbound `k_x`, `i` already bound to `k_v` | Deny `binding_conflict` |
| Valid assertion with key claim `k_v`, attacker proves `k_x` | Deny `key_mismatch` before mutation |
| Stolen assertion for never-enrolled `i`, attacker proves `k_x` | Deny in `attested-key`/`provisioned`; TOFU can bind and explicitly accepts this risk |
| Client injects trusted-proxy header while bypassing proxy | Deployment is non-conforming; verifier must reject direct/untrusted ingress |
| Assertion for issuer `A`, same `sub` as issuer `B` | Distinct identities; never collide or inherit binding |
| Assertion has wrong audience, expired `exp`, unknown algorithm/key, malformed subject/key | Deny without binding mutation |
| Concurrent first use of `(i,k1)` and `(i,k2)` | At most one commits; the other denies conflict |
| Reuse of valid WebSocket authorization after assertion expiry | Deny protected operation or reauthenticate/close |
| Fresh assertion for a revoked pair | Deny unless explicit recovery transition authorizes reactivation |
| New key presented for bound identity | Deny; require explicit rotation |
| Display name/email changes while `(iss,sub)` is stable | May update metadata; binding identity is unchanged |
| One NIP-42 connection authenticates `k1` and `k2`, only `k1` is bound | Only operations attributed to `k1` receive its lease |
| JWT or corporate identifier is accidentally published as event/tag | Non-conforming privacy failure; assertion transport must not enter relay event history |

# Conformance hooks for the NIP

The normative NIP should expose enough information for clients and operators to determine:

- accepted assertion transport profile(s);
- issuer discovery or configured issuer and accepted audience rules without leaking private tenant data;
- whether a key claim is required;
- enrollment mode (`attested-key`, `provisioned`, or explicitly risk-labeled `tofu`);
- authorization lease/re-authentication behavior;
- machine-readable rejection classes using existing NIP-42 `auth-required:` and `restricted:` prefixes where applicable;
- privacy requirements and trusted-proxy deployment requirements.

It should not standardize database schema, lock mechanism, Okta-specific claims, mutable display metadata, or an administration API. Those are implementation choices as long as the invariants and transitions above hold.

# Sources

- NIP-42 authentication: https://github.com/nostr-protocol/nips/blob/8f8444d05a8842c40211ded5d10af3521541f865/42.md
- NIP-98 HTTP auth: https://github.com/nostr-protocol/nips/blob/8f8444d05a8842c40211ded5d10af3521541f865/98.md
- NIP-05 issuer-controlled identifier mapping precedent: https://github.com/nostr-protocol/nips/blob/8f8444d05a8842c40211ded5d10af3521541f865/05.md
- NIP-46 external auth challenge precedent: https://github.com/nostr-protocol/nips/blob/8f8444d05a8842c40211ded5d10af3521541f865/46.md
- Companion protocol specification: [`NIP-FI.md`](NIP-FI.md)
- Buzz implementation semantics reviewed at `bd822f3ea8fc04b449501fd4738097c32d3da950` (PR #1476)
