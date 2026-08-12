# Scope

This model defines the state and transitions required by [NIP-FI](NIP-FI.md). It covers direct authorization, enrollment, lifecycle changes, leases, and delegation. It does not define an identity provider, storage schema, operator API, public identity projection, or application-specific admission policy.

The model is transport-neutral except where transport is part of the authorization evidence. NIP-42 and NIP-98 prove control of a Nostr key. A federated assertion does not.

# Terms

- `D`: an authorization domain selected from authenticated server routing and configuration.
- `i = (iss, sub)`: an issuer-qualified federated identity returned by assertion validation.
- `k`: a 32-byte Nostr public key returned by Nostr-proof validation.
- `A`: the exact authoritative assertion input. For both stock profiles it is the compact-JWS bytes; a registered profile closes its private input in its profile contract and its result mapping in its assertion policy.
- `N`: a fresh Nostr proof for the current connection or HTTP request.
- `R_t`: the server-owned target context `(method, authority, path_and_query, body_digest, transport, operation, resource)`.
- `R`: `R_t` sealed with the actor key returned by Nostr-proof validation.
- `P_t`: the transport tuple `(profile, transport_contract_revision, profile_contract_digest)` derived from the same trusted configuration as `R_t`.
- `E`: the closed normalized verified assertion result returned by the configured profile.
- `C`: local admission policy and resource state for `(D, R)`.
- `now`: verifier time.

Client fields, forwarded routing fields, assertion claims, and Nostr tags cannot select `D`, `R`, `C`, or the operation being admitted.

# Persistent state

For each domain `D`, the service maintains:

```text
B_D : active binding relation
T_D : set of retired (identity, key) pairs
X_D : set of disabled identities
Y_D : set of revoked keys
Q_D : pending-replacement lineage
H_D : immutable lifecycle history
V_D : monotonic binding and lifecycle versions
```

These are authoritative stores. A deployment may also maintain `O_D`, a separately capacity-bounded denial-observation channel. `O_D` is not an authorization witness: reads, writes, loss, truncation, or exhaustion of `O_D` cannot change a decision, receipt, lease, capability, replay claim, lifecycle transition, or application effect.

An active binding is:

```text
Binding = (
  domain,
  identity,
  key,
  version,
  provenance,             // attested-key | provisioned | tofu
  created_at,
  binding_not_after?      // optional administrative bound
)
```

`binding_not_after` is absent unless a separately authorized administrative action sets it. Assertion `exp`, `iat`, or maximum age never creates, renews, or extends this bound. Binding provenance is immutable.

Pending replacement lineage identifies an exact old binding version and old pair. A recovery or re-enablement transition can consume it once. Time passage alone does not create lifecycle state.

# Binding and lifecycle invariants

The following labels are stable conformance references.

**`FI-INV-01 — partial bijection.`** Active bindings are one-to-one within a domain:

```text
forall i, k1, k2:
  (i, k1) in B_D and (i, k2) in B_D implies k1 = k2

forall i1, i2, k:
  (i1, k) in B_D and (i2, k) in B_D implies i1 = i2
```

**`FI-INV-02 — durable binding.`** Assertion expiry does not remove, retire, or expire a binding. A fresh eligible assertion may authorize the same binding after an earlier assertion expires.

**`FI-INV-03 — tombstone monotonicity.`** Ordinary authorization never removes an element from `T_D`, `X_D`, or `Y_D`, consumes `Q_D`, or recreates a retired pair.

**`FI-INV-04 — server-owned context.`** Every allowed operation uses one server-resolved `D`, `R`, operation, resource, and actor. Unauthenticated input cannot replace any of them.

**`FI-INV-05 — independent evidence.`** Direct authorization requires both a currently valid assertion and a fresh Nostr proof. The asserted key, when present, equals the proven key.

**`FI-INV-06 — stable verifier policy.`** Verifier-policy identity changes when accepted assertion semantics change, but not when authenticated key or upstream-policy snapshot contents rotate. Snapshot rotation changes the applicable generation.

**`FI-INV-07 — current-policy verification.`** A prepared result or lease cannot survive removal of the key or upstream policy that authenticated it. A generation change requires revalidation against the current authenticated snapshot before use.

**`FI-INV-08 — read-only preparation.`** Preparation creates no binding, lifecycle fact, replay claim, receipt, lease, authorization audit evidence, publication, last-seen value, or application mutation. A denied preparation may attempt only the non-authoritative bounded observation defined below.

**`FI-INV-09 — atomic final admission.`** Enrollment, replay claims, receipts, and authorization audit evidence commit only after complete final revalidation. A denied or failed final admission leaves no authority mutation and creates no authorization receipt; attempting a non-authoritative denial observation is outside this commit.

**`FI-INV-10 — explicit lifecycle authority.`** Provisioning, retirement, disablement, revocation, rotation, recovery, re-enablement, and administrative-expiry changes occur only through their separately authorized transition.

**`FI-INV-11 — evidence-bounded leases.`** A lease ends no later than every normalized authority, authenticated policy snapshot, proof, transport-provenance, delegation, local-policy, binding-administrative, and implementation bound on which it depends.

**`FI-INV-12 — current-owner delegation.`** Delegation requires an authorization-eligible owner binding at its exact current version, a fresh delegate proof, capability intersection, and a positive finite deadline.

**`FI-INV-13 — privacy-safe denial.`** Public rejection is a many-to-one class. It does not reveal an identity, key, claim, binding, tombstone, enrollment mode, key identifier, or private policy fact.

**`FI-INV-14 — fail closed.`** Unreadable, ambiguous, stale beyond policy, or inconsistent assertion, key, binding, lifecycle, replay, policy, resource, receipt, or audit state cannot produce authority.

**`FI-INV-15 — uniform authority.`** Every protected ingress in a domain uses the same current domain policy and final-admission authority. An uncovered route, competing authority, or different policy lineage makes enforcement unavailable and fails closed.

**`FI-INV-16 — canonical verifier.`** Assertion acceptance semantics have one provider-neutral normalized-result contract. Transport adapters cannot weaken or fork that contract or final admission.

# Assertion-policy model

A JWT verifier policy has stable identity `policy_id` and contains at least:

```text
VerifierPolicy = (
  exact_issuer,
  accepted_audiences,
  allowed_asymmetric_algorithms,
  authenticated_key_source_identity,
  subject_rules,
  optional_key_claim_rules,
  optional_authorization_claim_or_capability_rules,
  time_and_skew_rules,
  normalization_and_size_rules,
  verifier_contract_fingerprint
)
```

The implementation supplies a versioned `verifier_contract_fingerprint` over compiled acceptance rules not otherwise represented in configured policy. `policy_id` is a deterministic digest of every field above. It excludes transport, JWKS bytes, key identifiers, key order, cache metadata, retrieval time, and JWKS generation. Any configured or compiled change to accepted assertion semantics changes `policy_id`; a key-only rotation does not.

A registered non-JWT assertion policy analogously identifies the exact authenticated upstream-policy source, identity/key/claim mapping, time and size bounds, normalization rules, snapshot semantics, and compiled contract fingerprint. Any semantic change advances `policy_id`; rotating only an authenticated upstream-policy snapshot does not.

Each accepted key snapshot has an opaque generation `g`. Effective addition, removal, or replacement of a verification key changes `g`. Generation order need not be meaningful outside one verifier instance; equality is sufficient for witness comparison.

The stock profiles call `ValidateAssertion(A, D, now)`. A registered profile either calls the same verifier or validates its closed upstream assertion and authorization claim set before returning the same normalized result. Ordinary forwarded headers and unchecked adapter-local fields cannot enter `E`.

`ValidateAssertion(A, D, now)` returns:

```text
JwtAssertionEvidence = (
  identity,
  asserted_key?,
  current_authorization_claims_or_capabilities,
  deadline,
  policy_id,
  jwks_generation,
  verification_key_identity,
  key_snapshot_hard_deadline,
  assertion_digest,
  confidential_revalidation_handle  // yields the exact compact-JWS bytes
)
```

Validation succeeds only when the checks in NIP-FI all pass. The input is exactly one bounded compact JWS with unambiguous protected headers and claims. The algorithm is allowed and asymmetric, and key selection produces exactly one compatible key. Issuer, audience, time, bounded non-empty subject, and optional asserted-key checks then pass under the same verifier contract.

Subject stability and non-reassignment are issuer trust and deployment assumptions, not mechanically verifiable assertion properties. The operator records authoritative evidence for those properties before enabling an issuer. If the issuer can reassign a subject, the same identity coordinate can inherit lifecycle or recovery authority and the policy remains disabled until separately authorized remediation establishes a non-reassignable coordinate.

Unknown, duplicate, incompatible, or absent-from-current-snapshot keys fail closed. Retrieval and refresh work is bounded and coalesced. A stale known key can be accepted only inside an explicit finite stale-known-key policy and never after the hard key-cache bound.

For a JWT policy, final admission denies when the current verifier policy identity differs from the prepared identity. If the current generation differs from the generation in prepared evidence or a lease, the verifier revalidates the original assertion under the current snapshot. Revalidation must reproduce the same identity, asserted key, claims or capabilities, policy identity, and live time bounds. A key addition can therefore preserve valid evidence when the old key remains accepted; key removal denies evidence signed by the removed key while it remains absent from the current snapshot.

The base model compares against the currently authenticated snapshot and defines no durable JWKS anti-rollback state. Republishing a previously removed key set can make those keys current again. A deployment claiming rollback prevention adds a separately authenticated monotonic version or equivalent durable key floor and tests that extension explicitly.

# Assertion transport model

Trusted listener, route, and authorization-domain configuration selects exactly one `P_t` before protected traffic is accepted. A stock profile is exactly `client-attached` or `trusted-proxy-hmac-v2`. A deployment may instead select one private registered trusted-edge profile whose identifier matches `x-<operator>-<profile>-v<N>`. The verifier does not infer a profile from attacker-controlled fields and does not fall back between profiles after missing, mixed, or rejected evidence.

Every profile returns the same closed result:

```text
NormalizedAssertionEvidence = (
  identity,
  asserted_key?,
  current_authorization_claims_or_capabilities,
  authority_deadlines,  // non-empty; every member is finite
  policy_id,
  transport_profile,
  transport_contract_revision,
  profile_contract_digest,
  revalidation_dependencies
)
```

For a stock profile, `NormalizeJwtEvidence(JwtAssertionEvidence, P_t)` supplies this result and preserves the JWT evidence as revalidation dependencies. A registered adapter authenticates its closed upstream assertion and policy snapshot and returns the same shape. In either case, `policy_id` owns assertion-to-result semantics and the profile-contract digest owns transport and provenance semantics.

The profile preserves the exact server-owned domain and request context, requires independent fresh Nostr proof, and feeds the same binding, lifecycle, invalidation, lease, local-policy, and final-admission rules. `authority_deadlines` is non-empty; comparison is overflow safe and equality is expired. The normalized result is closed; it is not a generic trusted-header map.

The `client-attached` profile carries exactly one `Nostr-Federated-Identity: Bearer <JWT>` field and no assertion-provenance field. Missing, repeated, combined, malformed, or mixed-profile fields deny.

## Trusted-edge provenance

Every trusted-edge profile strips inbound copies of its authority-bearing fields, cryptographically authenticates the immediate edge, denies requests not attributable to that edge, protects each authorization-relevant request component, and imposes a positive finite provenance acceptance bound that expires at equality. It records one of two constructions in its profile contract:

- request-bound signature or MAC evidence, with bounded atomic replay consumption for any single-use claim; or
- an authenticated-edge adapter with validated upstream policy, accepting-origin isolation, inbound-field stripping, and full integrity for authorization-relevant request components.

The second construction does not inherit HMAC-v2's application-verified request seal and therefore owes stronger deployment-boundary evidence. Reusable edge identity is not nonce replay protection. A JWT-based registered adapter may permit reuse of an unexpired JWT when each admission has a fresh request-appropriate Nostr proof; it cannot claim single-use JWT semantics without defining and proving them.

A registered identifier and its private fields are never advertised in NIP-11. Registration cannot change the normalized-result shape or final-admission rules.

### Stock HMAC-v2 provenance

The trusted proxy removes every inbound assertion, provenance, and client-peer field and supplies exactly one `Nostr-Federated-Identity: Bearer <JWT>` field, one `Nostr-Federated-Identity-Provenance` field, and one `Nostr-Federated-Identity-Client-Peer` field. The provenance value is exactly `v2.<timestamp>.<nonce>.<mac>`; v1 is rejected. `timestamp` is canonical unsigned decimal without leading zeroes except `0`. `nonce` and `mac` are canonical unpadded base64url, decoding to at least 16 bytes and exactly 32 bytes respectively. Each nonce has at least 128 bits from a cryptographically secure random source. The client-peer value is a canonical IPv4 or RFC 5952 IPv6 address, with IPv4-mapped IPv6 encoded as IPv4. Configured finite field, client-peer, and nonce maxima apply before decoding, lookup, or replay storage.

Let `LP(x)` be the eight-byte unsigned big-endian length of byte string `x`, followed by `x`. The stock MAC input is:

```text
"NIP-FI-PROXY-2" ||
LP(timestamp) || LP(nonce) || LP(SHA256(A)) ||
LP(D.opaque_16_byte_id) ||
LP(R_t.method) || LP(R_t.authority) || LP(R_t.path_and_query) ||
LP(R_t.body_digest) || LP(R_t.proof_transport_code) || LP(client_peer)
```

The secret has at least 256 bits. The parsed timestamp is an eight-byte unsigned big-endian Unix-seconds value in the MAC. The nonce uses its decoded bytes. The assertion and body digests are raw SHA-256 output. The domain identifier is the exact server-selected 16-byte value. Method is the exact uppercase ASCII endpoint method. Authority is the server-configured lowercase ASCII host, explicit effective port, and bracketed IPv6 when applicable. Path and query are the exact post-routing ASCII origin-form, with `/` for an empty path, the leading `?` on a query, preserved percent-encoding and parameter order, and no fragment. The proof-transport code is one byte: `0x01` NIP-42, `0x02` NIP-98, `0x03` Git smart-HTTP session, or `0x04` Blossom. Client peer is the exact canonical ASCII field. Ambiguous or non-canonical values deny.

The profile accepts time only when `timestamp <= now + future_skew` and `now < timestamp + maximum_provenance_age`, with finite configured bounds and overflow-safe comparisons. Equality at the age bound is expired. The verifier reconstructs every request component from authenticated server state, verifies the MAC in constant time against a finite active-secret set, and rejects a committed nonce in the domain/profile replay namespace regardless of which secret matched. Header presence or network location is not provenance. An assertion on direct ingress or without a valid MAC is denied.

A direct lease under this profile ends no later than `min(assertion deadline, timestamp + maximum_provenance_age)` and remains subject to every other applicable lease bound.

The nonce is only claimed during final admission and retained through at least `timestamp + maximum_provenance_age`. An applicable proof replay identity is retained through its entire acceptance window. Preparation reserves neither and cannot cause later requests to fail.

# Nostr-proof model

`ValidateNostrProof(N, D, R_t, now)` returns `k` only when signature, event identity, freshness, and exact target binding pass:

- NIP-42 binds the proof to the current challenge, relay URL, connection, and freshness window.
- NIP-98 binds the proof to the exact server-resolved URL, method, payload digest when required, and freshness window.

The key used for authorization is always `k`, never an assertion claim or unsigned input. Applicable proof replay identity is claimed only during final admission.

# Prepared authorization

A prepared result is immutable evidence, not authority:

```text
PreparedAuthorization = (
  exact_context,                  // D, R_t, R, operation, resource, actor
  nostr_proof_evidence,
  path_dependencies,              // direct: DirectPrepared |
                                  // delegated: DelegatedPrepared
  policy_and_resource_witness,
  proposal,                       // existing | enroll | delegated
  all_deadlines,
  invalidation_dependencies
)

DirectPrepared = (
  assertion_evidence,
  transport_evidence,
  actor_binding_and_lifecycle_witness,
  enrollment_mode_witness
)

DelegatedPrepared = (
  delegation_evidence,
  owner_binding_and_lifecycle_witness,
  relationship_witness
)
```

For direct authorization, preparation is equivalent to:

```text
PrepareDirect(request, assertion_input, N):
  (D, R_t, operation, resource) := ResolveTargetContext(request) or DENY
  (e, transport_evidence) :=
      ValidateConfiguredTransport(D, R_t, assertion_input, P_t, now) or DENY
  k := ValidateNostrProof(N, D, R_t, now) or DENY
  R := SealActor(R_t, k)
  i := e.identity

  if e.asserted_key exists and e.asserted_key != k:
      DENY(key_mismatch)

  atomically read B_D(i), B_D(k), T_D(i,k), X_D(i), Y_D(k),
                      Q_D(i), mode(D), C, and all versions

  if i in X_D:       DENY(identity_disabled)
  if k in Y_D:       DENY(key_revoked)
  if (i,k) in T_D:   DENY(pair_retired)
  if Q_D(i) exists:  DENY(explicit_replacement_required)

  if B_D(i) = B_D(k) = b(i,k):
      if b.binding_not_after exists and now >= b.binding_not_after:
          DENY(binding_expired)
      proposal := existing(b.version, b.provenance)
  else if B_D(i) exists or B_D(k) exists:
      DENY(binding_conflict)
  else if mode(D) = attested-key:
      require e.asserted_key = k
      proposal := enroll(i, k, attested-key)
  else if mode(D) = provisioned:
      DENY(binding_required)
  else if mode(D) = tofu:
      provenance := e.asserted_key = k ? attested-key : tofu
      proposal := enroll(i, k, provenance)

  EvaluateEveryLocalAdmissionPolicy(
      D, R, operation, resource, k,
      e.current_authorization_claims_or_capabilities
  ) or DENY
  return PreparedAuthorization(evidence, proposal, witnesses, deadlines)
```

Preparation is read-only for an existing binding and every enrollment mode. It creates no authoritative state. A denied preparation may attempt a non-authoritative denial observation only after the decision is fixed; observation failure cannot change or delay the denial.

# Final admission

`CommitAdmission(prepared, current_request)` is equivalent to:

```text
require ExactContextMatch(prepared, current_request)
require every evidence and policy deadline is live

if prepared.path_dependencies is DirectPrepared:
  require CurrentTransportContract(D, prepared.R_t) =
          (prepared.direct.assertion_evidence.transport_profile,
           prepared.direct.assertion_evidence.transport_contract_revision,
           prepared.direct.assertion_evidence.profile_contract_digest)
  require CurrentAssertionPolicyIdentity(
            D, prepared.direct.assertion_evidence.identity.iss
          ) = prepared.direct.assertion_evidence.policy_id
  if any prepared profile or assertion-policy revalidation dependency changed:
      revalidate the authoritative assertion input under current dependencies
      require the normalized result, including identity, asserted key,
              claims or capabilities, policy identity, and live deadlines,
              is equivalent
else:
  require prepared.path_dependencies is DelegatedPrepared
  revalidate its delegation, relationship, owner, target, and policy witnesses

atomically:
  reread every applicable binding, lifecycle, enrollment-mode, policy, resource,
         replay, receipt, and invalidation witness
  recompute the complete decision from current state
  require the current result is equivalent and eligible
  require every applicable transport and proof replay identity is unclaimed,
          including the HMAC-v2 nonce when that profile is selected
  claim applicable replay identities
  create the proposed binding only if it remains eligible
  append the request-bound authorization receipt and required audit evidence

return CommittedAuthorization(
  exact_actor,
  binding_dependencies,
  capabilities,
  dependencies,
  deadline
)
```

The atomic section either commits all authority mutations or none. Unreadable state denies. Changed state requires complete recomputation and may commit only a semantically equivalent current decision. A concurrent identical enrollment may be reread and recomputed as `existing`; a conflicting enrollment denies. A missing or unreadable committed result does not fall back to allow.

A denied or failed final admission creates no authorization receipt. After rollback, it may attempt a denial observation in `O_D`; that attempt is not part of the authoritative transaction.

The application operation runs only after committed authorization. When it cannot share the authorization transaction, a request-bound idempotent receipt or equivalent staging prevents the same proof from creating a second effect.

# Enrollment modes

- `attested-key`: a first binding requires `asserted_key = proven_key`; provenance is `attested-key`.
- `provisioned`: ordinary authorization never creates a binding; only `ProvisionBinding` can.
- `tofu`: eligible first use may bind a proven key without issuer key attestation. This mode is explicitly risk-labelled because a stolen assertion for a never-enrolled identity can bind an attacker's key. A matching claim records `attested-key` provenance.

A mode change affects future creation only. It cannot rewrite an existing binding or its provenance.

# Lifecycle transitions

Each lifecycle transition requires separate privileged authority bound to `D`, the transition, identity, old binding version when present, target key when present, and request. It atomically rechecks relevant `B_D`, `T_D`, `X_D`, `Y_D`, `Q_D`, policy, and version state; appends `H_D`; advances `V_D`; and invalidates dependent leases after commit.

`TargetEligible(i, k, allow_disabled)` means `k` is not in `Y_D`, `(i,k)` is not in `T_D`, neither side has an active binding, and `i` is not in `X_D` unless `allow_disabled` is true for ReenableIdentity. Provision and rotation additionally require no pending lineage. Recovery and re-enablement require the exact lineage stated by their transition.

`ReplacementProvenance(evidence)` records the evidence that authorized the new target: `attested-key` only for a current matching issuer key attestation, otherwise `provisioned` for a privileged lifecycle transition. TOFU provenance is created only by ordinary first use in `tofu` mode and is never inherited by a replacement key.

```text
ProvisionBinding(i, k):
  require mode(D) = provisioned
  require TargetEligible(i, k, false) and Q_D(i) is absent
  require fresh target-key proof and any required issuer attestation
  create Binding(i, k, new_version, provisioned)

RetirePair(i, k, old_version):
  require exact current Binding(i, k, old_version)
  remove it from B_D
  add (i,k) to T_D
  record Q_D(i) for old_version

DisableIdentity(i):
  add i to X_D
  if an active binding exists, retire its pair and record Q_D(i)

RevokeKey(k):
  add k to Y_D even when k is inactive
  if an active binding exists, retire its pair and record Q_D(i)
  repeated authorized application is idempotent and preserves lineage

Rotate(i, k_old, old_version, k_new):
  require exact current Binding(i, k_old, old_version)
  require k_new is not revoked and (i,k_new) is not retired
  require k_new has no active binding and Q_D(i) is absent
  require fresh target-key proof and any required issuer attestation
  remove (i,k_old) from B_D and add (i,k_old) to T_D
  create Binding(i, k_new, new_version, ReplacementProvenance(evidence))

Recover(i, pending_version, k_new):
  require exact Q_D(i, pending_version)
  require i is not disabled
  require TargetEligible(i, k_new, false)
  require fresh target-key proof and any required issuer attestation
  consume Q_D(i, pending_version)
  create Binding(i, k_new, new_version, ReplacementProvenance(evidence))

ReenableIdentity(i, pending_version?, k_new):
  require i in X_D
  require exact absent lineage or exact Q_D(i, pending_version)
  require TargetEligible(i, k_new, true)
  require fresh target-key proof and any required issuer attestation
  remove i from X_D
  consume supplied lineage when present
  create Binding(i, k_new, new_version, ReplacementProvenance(evidence))

SetAdministrativeExpiry(i, k, old_version, binding_not_after?):
  require exact current Binding(i, k, old_version)
  require separate privileged expiry authority
  create the same pair and provenance at new_version with the supplied bound
```

`ReenableIdentity` deliberately clears `X_D` only while creating an eligible
target binding under fresh proof. Clearing disabled state without a target
would create a resurrection window: provisioned mode would have no target to
match, while TOFU could let the next ordinary admission capture first use. An
operator that wants to re-enable now and provision later leaves the identity
disabled until the target and proof are available; there is no separate
clear-only transition.

Rotation does not globally revoke `k_old`; revocation does. A retired pair remains retired after rotation, recovery, or re-enablement. Ordinary authorization cannot cross disabled, revoked, retired, pending, or administratively expired state.

# Lease model

HTTP authorization applies to one exact request and has no reusable lease.

A WebSocket lease is:

```text
Lease = (
  D,
  actor_key,
  binding_dependencies,          // direct actor | delegated owner
  lifecycle_versions,
  evidence_dependencies,        // DirectEvidence | DelegatedEvidence
  operations,
  resources,
  deadline,
  invalidation_dependencies
)
```

`DirectEvidence` records the normalized result, profile and assertion-policy revalidation dependencies, and a confidential handle for the authoritative assertion input. JWT dependencies include JWKS generation, verification-key identity, the key-snapshot hard-validity deadline, assertion digest, and exact compact-JWS bytes. `DelegatedEvidence` records the exact owner binding and version, relationship identifier and revision, and delegation expiry. Revalidation material is retained only through the admission or lease that may need it and is destroyed on expiry, close, or invalidation.

Before each protected use, the service verifies the key, domain, capability, resource, applicable binding and lifecycle versions, administrative bound, and deadline. Direct evidence requires current profile and assertion-policy dependencies; a changed dependency requires revalidation that reproduces the equivalent normalized result. For JWT evidence, the key snapshot must be readable before its hard-validity deadline and a changed JWKS generation requires revalidation of the original assertion. Delegated evidence requires the exact current eligible owner binding and relationship revision. A lease for one key never authorizes another key on the same connection.

For direct authorization:

```text
lease.deadline <= min(
  every_normalized_authority_deadline,
  proof_or_connection_bound,
  transport_provenance_bound_if_present,
  binding_not_after_if_present,
  local_policy_bound,
  implementation_maximum
)
```

For JWT evidence, the normalized authority deadlines include the assertion and key-snapshot hard-validity deadlines. The set is non-empty, comparison is overflow safe, and equality is expired.

Lease expiry removes session authority. It does not remove, renew, or retire the durable binding.

# Delegation model

Delegation is a separate evidence path. The delegate supplies fresh proof of `k_delegate`; both federated-assertion and assertion-provenance fields are absent. Separately validated evidence contains:

```text
DelegationEvidence = (
  D,
  owner_key,
  delegate_key,
  relationship_id,
  relationship_revision,
  audience,
  operations,
  resource_or_target,
  not_before?,
  mandatory_expiry
)
```

Preparation and final admission both require the exact current, authorization-eligible owner binding and version. The proven delegate key must equal `delegate_key`. The admitted capability is the intersection of delegation evidence and current local policy. The path cannot create or change an owner or delegate binding, identity, provenance, lifecycle fact, or last-seen state.

The delegated deadline is bounded by the delegation expiry, delegate proof, current owner administrative bound and lifecycle version, local policy, configured positive finite delegated maximum, and any stronger owner evidence a deployment requires. Rotation makes the former owner key non-current, so its delegations deny and do not transfer to the new key.

# Denial and privacy model

Internal reasons map many-to-one to these public classes:

```text
missing_evidence            -> 401, auth-required:
evidence_rejected           -> 403, restricted:
authorization_denied        -> 403, restricted:
authorization_unavailable   -> 503, restricted:
```

Internal reason, issuer, subject, key, binding existence, lifecycle state, enrollment mode, claim value, and key identifier remain private. Raw bearer material never enters protocol output, public events, logs, metrics, traces, or denial observations.

Every denial attempts one access-controlled observation in `O_D` containing only a stable private reason code, correlation identifier, timestamp, transport class, and bounded or keyed-hashed source coordinates. Verbatim unverified claims are excluded. `O_D` has finite capacity independent of the non-reclaimable authorization-audit budget. If it is unavailable or exhausted, the denial stands without retry, blocking, latching, receipt creation, or authoritative mutation. Missing observations are not evidence of no denial. `O_D` may support monitoring and investigation, but authorization, lockout, and rate-limit decisions do not read it.

# Liveness

Liveness assumes available issuer keys, verifier policy, binding and lifecycle storage, replay storage, local policy, receipt and audit storage, and network:

1. An eligible existing binding with current evidence is eventually admitted.
2. An eligible unbound pair is eventually admitted exactly once when its enrollment mode permits creation.
3. After an authorized lifecycle transition and bounded invalidation, stale authority is denied and an eligible new binding can be admitted.

No liveness promise overrides `FI-INV-14`. Dependency outage may deny otherwise valid work.

# Stable conformance traces

Each trace identifier has the same meaning in NIP-FI, this model, and later executable conformance tests. Proxy and verifier-parity evidence carries the transport-contract revision and profile-contract digest. Evidence from another revision, profile, digest, adapter, deployment, or policy tuple is not transferable. Each profile declares the spoof, replay, and cross-request artifacts its oracle exercises.

| ID | Setup and required result |
|---|---|
| `FI-TRACE-PROXY-SPOOF` | A trusted-edge request without the configured profile's valid provenance, including direct ingress, denies. HMAC-v2 retains its exact field, MAC, and boundary negatives. |
| `FI-TRACE-PROXY-REPLAY` | The configured trusted-edge profile enforces its declared replay semantics. For HMAC-v2, two final admissions using the same proxy nonce produce at most one committed authorization and preparation consumes neither. |
| `FI-TRACE-PROXY-CROSS-REQUEST` | Changing a request component protected by the configured profile denies. HMAC-v2 protects the assertion, domain, proof transport, authenticated client peer, method, authority, path/query, and body. |
| `FI-TRACE-AUTHORITY-UNIFORM` | Every protected ingress uses the same current domain policy and final-admission authority; uncovered, competing, or different-lineage paths fail closed. |
| `FI-TRACE-VERIFIER-PARITY` | Equivalent authenticated assertion input and policy produce the same authorization projection and final-admission decision at each controlled time on every transport. The same trace validates each profile's identity, revision, digest, `policy_id`, and deadlines; `FI-TRACE-PREPARED-STALE` validates changed revalidation dependencies. Policy vectors cover every configured and compiled semantic input and authenticated snapshot-only rotation. |
| `FI-TRACE-DOMAIN-SPOOF` | Client-selected domain or forwarded authority cannot replace server-owned context and denies on mismatch. |
| `FI-TRACE-ASSERTION-KEY-MISMATCH` | An asserted key different from the proven key denies before mutation. |
| `FI-TRACE-BINDING-CONFLICT` | A valid identity and key that conflict with either side of the active relation deny without replacement. |
| `FI-TRACE-TOMBSTONE-REPLAY` | A fresh assertion for a retired pair, disabled identity, revoked key, or pending replacement denies ordinary authorization. |
| `FI-TRACE-ASSERTION-REFRESH` | A fresh assertion can authorize the same eligible durable binding after the assertion used at enrollment expires. |
| `FI-TRACE-ADMIN-EXPIRY` | A fresh assertion after `binding_not_after` denies; only an explicit privileged transition can restore access. |
| `FI-TRACE-JWKS-ADD` | Generation changes, the old signing key remains accepted, revalidation passes, and the unchanged binding may authorize. |
| `FI-TRACE-JWKS-REMOVE` | Generation changes, the signing key is removed, and prepared evidence and leases signed by it deny. |
| `FI-TRACE-PREPARED-STALE` | A request, binding, lifecycle, policy, resource, mode, replay, or invalidation witness changes before final admission and the stale decision denies or is completely recomputed. |
| `FI-TRACE-FINAL-DENIAL-NO-MUTATION` | Denied preparation, local policy, and final admission create no authoritative mutation or authorization receipt. An available denial channel records one bounded observation; an unavailable or exhausted channel leaves the denial and authoritative stores unchanged. |
| `FI-TRACE-CONCURRENT-ENROLLMENT` | Identical eligible first uses converge on one binding version; conflicting first uses commit at most one winner. |
| `FI-TRACE-TOFU-THEFT` | Stolen assertion first use denies in attested and provisioned modes; only explicit risk-labelled TOFU may bind the attacker's proven key. |
| `FI-TRACE-DELEGATE-OWNER-ROTATED` | Owner rotation makes an old-owner delegation non-current and denies without inheritance. |
| `FI-TRACE-DELEGATION-EXPIRED` | Missing or expired finite delegation bounds deny. |
| `FI-TRACE-DENIAL-ORACLE` | Unknown, conflict, tombstone, and private-policy denials are not publicly distinguishable. |
| `FI-TRACE-DEPENDENCY-FAIL-CLOSED` | Unreadable current verifier, key, state, replay, policy, receipt, or audit dependency denies. |
| `FI-TRACE-MULTI-KEY-SESSION` | A lease for one authenticated key does not authorize another key on the same connection. |
| `FI-TRACE-CROSS-DOMAIN-COLLISION` | Equal `sub` values across issuers or equal pairs across domains remain distinct and cannot inherit authority. |
| `FI-TRACE-PRIVACY-NONPUBLIC` | Assertion and private identity material in protocol output, public history, or observability is a conformance failure. |

# Sources

- NIP-42 authentication: <https://github.com/nostr-protocol/nips/blob/8f8444d05a8842c40211ded5d10af3521541f865/42.md>
- NIP-98 HTTP authentication: <https://github.com/nostr-protocol/nips/blob/8f8444d05a8842c40211ded5d10af3521541f865/98.md>
- Companion protocol specification: [NIP-FI.md](NIP-FI.md)
