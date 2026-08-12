NIP-FI
======

Federated identity authorization
--------------------------------

`draft` `optional` `relay`

**Protocol dependencies**: NIP-01, plus NIP-42 for WebSocket authorization or NIP-98 for HTTP authorization. **Optional composition**: NIP-11 discovery and a separately validated delegation protocol such as NIP-OA.

## Abstract

This NIP defines how a relay or Nostr-adjacent HTTP service authorizes a Nostr key only when a valid federated identity assertion, fresh Nostr proof, current identity-to-key binding state, and the requested operation's local admission policy all agree. It defines cryptographically bound assertion transport, assertion and proof validation, read-only authorization preparation, final admission, enrollment, lifecycle state, bounded sessions, delegation, rejection behavior, discovery, and privacy.

The identity provider never becomes a Nostr signing authority. A bearer assertion never substitutes for Nostr proof of key control. Binding lifetime is independent of assertion lifetime: a fresh assertion can authorize an existing eligible binding after an earlier assertion expires, while every authorization lease remains bounded by the assertion used to create it.

## Motivation

Organizations may need relay access tied to an external identity system while preserving Nostr key ownership. NIP-42 proves control of a key on a relay connection, and NIP-98 proves control of a key for an HTTP request, but neither binds that key to an issuer-qualified external principal. Without a shared contract, deployments can disagree about assertion transport, key rotation, enrollment, lifecycle denial, and the point at which authorization may mutate state.

This NIP defines a provider-neutral contract. It does not standardize an identity vendor, database schema, operator API, public identity projection, or application-specific membership policy.

## Definitions

- **assertion** (`A`): authoritative federated identity input presented as independent evidence alongside Nostr proof. Both stock profiles carry a compact-JWS JWT. A registered profile may use another private authenticated input only under its closed profile contract and normalized-result obligations.
- **federated identity** (`i`): the exact tuple `(iss, sub)` from a validated assertion. `iss` is the exact accepted issuer identifier. `sub` is the exact non-empty subject string. A username, email address, display name, employee number, mutable profile field, or bare `sub` is not a federated identity.
- **authorization domain** (`D`): a boundary selected from authenticated server routing and configuration. A client-supplied domain, forwarded host value, assertion claim, or unsigned header cannot select `D`.
- **target context** (`R_t`): the server-resolved method, authority, path and query, body digest, transport, operation, and resource for the request being admitted.
- **request context** (`R`): `R_t` sealed with the acting key returned by Nostr-proof validation. Client input cannot supply or replace that key.
- **verifier-contract fingerprint**: a versioned, implementation-supplied digest of every compiled assertion-acceptance rule that is not otherwise represented in configured policy, including protected-header handling, algorithm/key compatibility, claim normalization, and size behavior. A binary that changes any such rule MUST change this fingerprint.
- **verifier policy identity** (`policy_id`): a stable digest of assertion semantics, including the issuer or authenticated upstream-policy source, audience where applicable, allowed algorithms where applicable, authenticated key or policy-source identity, identity/key/claim mapping and normalization rules, time and size bounds, and the verifier-contract fingerprint. It MUST change when those semantics change and MUST NOT include transport, rotating key or upstream-policy contents, snapshot order, cache timestamps, or a snapshot generation.
- **normalized verified assertion result**: the closed, transport-neutral result consumed by direct authorization. It contains an issuer-qualified principal, an optional asserted key, current authorization claims or capabilities, at least one finite authority deadline, `policy_id`, the transport profile, the transport-contract revision, the profile-contract digest, and revalidation dependencies. It contains no unchecked forwarded field.
- **authorization projection**: the shared normalized identity, optional asserted key, and authorization claims or capabilities consumed by local admission. Profile identity, evidence deadlines, `policy_id`, and revalidation dependencies remain profile-specific evidence metadata. `FI-TRACE-VERIFIER-PARITY` validates that metadata for each profile, and `FI-TRACE-PREPARED-STALE` validates changed dependencies.
- **profile-contract digest**: a stable digest of the selected transport profile's authoritative fields, provenance rules, protected request components, replay semantics, and assertion-policy adapter identity. Assertion semantic changes advance `policy_id`; transport or provenance changes advance the profile-contract digest. Evidence from another digest is not interchangeable.
- **JWKS generation** (`g`): an opaque identifier for one effective verification-key snapshot. It MUST change whenever the accepted key identifiers or key material change.
- **binding**: a durable, versioned record associating one identity with one 32-byte Nostr public key in `D`. Its immutable provenance is `attested-key`, `provisioned`, or `tofu`. It MAY carry a separately authorized administrative `binding_not_after` bound. It MUST NOT derive that bound from assertion `exp` or `iat`.
- **retired pair**: a durable denial fact for one exact `(D, i, k)` pair. Ordinary authorization can never recreate that pair.
- **disabled identity**: a durable denial fact that prevents an identity from authorizing or enrolling a key.
- **revoked key**: a durable denial fact that prevents a key from authorizing or binding to any identity in `D`.
- **pending replacement**: durable lineage identifying an old key and binding version that a separately authorized recovery or re-enablement transition may consume once.
- **Nostr proof**: a valid NIP-42 AUTH event or NIP-98 event proving control of a key for the current connection or request.
- **prepared authorization**: an immutable, non-authoritative, read-only result that seals verified evidence, server-owned context, state and policy witnesses, a possible enrollment proposal, and every expiry and invalidation dependency. Producing this result creates no binding, lifecycle fact, replay claim, receipt, audit event, publication, lease, or application mutation.
- **committed authorization**: the result of revalidating a prepared authorization at final admission and atomically committing any allowed enrollment, replay claim, receipt, and required authorization audit evidence.
- **authoritative state**: state used to decide, prove, replay, or apply authorization, including bindings, lifecycle facts, replay claims, leases, capabilities, authorization receipts, required authorization audit evidence, and application state.
- **denial observation**: a capacity-bounded, non-authoritative record that a denial occurred. Its presence, absence, loss, or truncation cannot change an authorization decision or become authorization policy.
- **lease**: a cached committed decision for one actor, domain, operation set, and exact dependency versions. A lease is never a binding and cannot extend one.

Within a domain, active bindings form a partial bijection: one identity has at most one active key, and one key has at most one active identity.

## Assertion transport

An assertion is captured on the request being authorized: the WebSocket upgrade for NIP-42 connections or the same HTTP request as its NIP-98 proof. Assertions MUST NOT appear in URLs, query parameters, Nostr events, tags, filters, application history, or public identity projections.

Two stock transport profiles are defined: `client-attached` and `trusted-proxy-hmac-v2`. A deployment MAY also install a private registered trusted-edge profile whose identifier matches `x-<operator>-<profile>-v<N>`. Server-owned listener, route, and authorization-domain configuration selects exactly one profile before accepting requests. Client input cannot select, negotiate, or downgrade it.

This document defines transport-contract revision `2`, represented as the unsigned integer `2`. A change to the common transport-profile obligations or evidence meaning requires a new revision. Each profile contract is an immutable UTF-8 artifact whose exact bytes identify its authoritative fields, provenance rules, protected request components, replay semantics, deadlines, and assertion-policy adapter identity. Its digest is `sha256:` followed by the lowercase hexadecimal SHA-256 of those exact bytes; implementations MUST NOT apply implicit reserialization or newline normalization while computing it. A profile-specific contract change produces a new artifact and digest.

### Transport profile obligations

Every configured transport profile MUST:

1. Produce one normalized verified assertion result under a closed profile contract. The result identifies an issuer-qualified principal, an optional asserted key, current authorization claims or capabilities, at least one finite authority deadline, `policy_id`, the selected profile, the transport-contract revision, the profile-contract digest, and revalidation dependencies. Deadline comparison MUST be overflow safe and equality is expired.
2. Feed the same Nostr-proof, binding, lifecycle, invalidation, lease, local-policy, and final-admission authority. An adapter cannot weaken or replace those checks.
3. Preserve the exact server-resolved domain, operation, resource, method, authority, path and query, body semantics, proof transport, and actor key used by final admission.
4. Reject absent, mixed, malformed, or profile-inconsistent evidence without falling back to or from another profile.
5. Require independent fresh Nostr proof for the current request or connection.
6. Expose every assertion and provenance deadline to lease construction, including the required finite authority deadline.
7. Keep credentials, signatures, MACs, and unredacted identity or authorization claims out of public output and observability.
8. Bind conformance evidence to one exact implementation, adapter, deployment, policy, transport-contract revision, and profile-contract digest.

The normalized result is a security boundary, not a generic map of trusted headers. A profile contract closes its fields, types, authority, deadlines, provenance rules, and assertion-policy adapter identity. The selected assertion policy closes identity, key, claim or capability, deadline, and normalization semantics. A deployment MUST NOT treat header presence, source address, private-network location, hostname, or reachability alone as trusted-edge provenance.

### Client-attached profile

This profile's discovery identifier is `client-attached`. The client sends exactly one `Nostr-Federated-Identity: Bearer <JWT>` field and no assertion-provenance field. The same field is used on WebSocket upgrades and NIP-98 HTTP requests; `Authorization` remains reserved for the Nostr proof where that protocol requires it. Missing, repeated, comma-combined, malformed, empty, non-Bearer, or mixed-profile assertion fields are rejected.

### Trusted-edge profile obligations

Every stock or registered trusted-edge profile MUST strip inbound copies of each assertion, identity, authorization, and capability field before setting its own values. It MUST cryptographically authenticate the immediate trusted edge, deny requests that the origin cannot attribute to that edge, protect every request component used for authorization, and impose a positive finite provenance acceptance bound that expires at equality. It MUST document the accepting origin, direct-origin controls, field-stripping point, caller authentication, protected components, upstream validation, Nostr-proof path, compromise impact, and evidence location.

A trusted-edge profile uses one of these constructions:

- **Request-bound evidence:** a signature or MAC binds the authoritative assertion and authorization-relevant request components. A single-use replay claim also requires bounded atomic replay consumption.
- **Authenticated-edge assertion adapter:** a trusted platform validates upstream identity and policy, the immediate edge is cryptographically authenticated, the accepting origin is isolated, inbound authority-bearing fields are stripped, and the complete authorized request is integrity protected. The adapter maps only its closed validated claim set into the normalized result.

Request-bound HMAC-v2 gives Buzz application-verified proof of the exact request and relies less on deployment-only assurances for spoof, replay, and cross-request resistance. An authenticated-edge adapter omits that application-level seal and therefore MUST demonstrate immediate-caller authentication, accepting-origin isolation, full integrity for authorization-relevant request components, inbound-field stripping, and validated upstream policy projection together.

A registered profile is private deployment policy. Its identifier, fields, caller identity, issuer, and mechanism MUST NOT appear in NIP-11 or public examples. Registration does not weaken verifier parity or final admission. If a JWT-based registered profile permits bearer reuse, reuse of an unexpired JWT with a fresh request-appropriate Nostr proof is not itself a proxy-replay violation. The profile still rejects expired assertions, replayed or transplanted Nostr proofs, and presentation outside its configured edge; it cannot claim single-use JWT semantics unless it defines and proves them.

### Trusted-proxy HMAC profile

This profile's discovery identifier is `trusted-proxy-hmac-v2`. The trusted proxy strips every inbound copy of all assertion, provenance, and client-peer fields, then inserts exactly one `Nostr-Federated-Identity: Bearer <JWT>` field, exactly one `Nostr-Federated-Identity-Provenance` field, and exactly one `Nostr-Federated-Identity-Client-Peer` field. Header presence, source IP, or network topology alone is not trusted-proxy provenance. Unsigned forwarded identity MUST be rejected.

The provenance field has this exact ASCII form:

```text
v2.<timestamp>.<nonce>.<mac>
```

`timestamp` is canonical unsigned decimal without leading zeroes, except that zero is `0`. `nonce` and `mac` are canonical unpadded base64url. The trusted proxy generates each nonce with at least 128 bits from a cryptographically secure random source. A decoded nonce contains at least 16 bytes, and a decoded MAC contains exactly 32 bytes. The verifier applies configured finite maximum provenance-field and nonce sizes before decoding, lookup, or replay storage. Missing, repeated, comma-combined, oversized, non-canonical, or extra components are malformed.

The client-peer field is the proxy-authenticated end-client IP address in canonical ASCII. IPv4 uses dotted decimal without leading zeroes. IPv6 uses lowercase RFC 5952 text; an IPv4-mapped IPv6 address is encoded as canonical IPv4. The field is at most 64 bytes. Empty, repeated, comma-combined, whitespace-padded, non-IP, or non-canonical values are rejected. After verification, an implementation MAY replace the address with a domain-separated keyed digest for bounded private admission state; the raw address is not an identity claim and MUST NOT enter public output.

The stock profile uses HMAC-SHA-256 with a deployment secret of at least 256 bits. Let `LP(x)` be the eight-byte unsigned big-endian length of byte string `x`, followed by `x`. The MAC input is:

```text
"NIP-FI-PROXY-2" ||
LP(timestamp) || LP(nonce) || LP(assertion_digest) ||
LP(authorization_domain_id) ||
LP(method) || LP(authority) || LP(path_and_query) || LP(body_digest) ||
LP(proof_transport_code) || LP(client_peer)
```

For the MAC, parsed `timestamp` is encoded as an eight-byte unsigned big-endian value. `nonce`, `assertion_digest`, `body_digest`, and `mac` are their decoded bytes. `assertion_digest` is SHA-256 over the exact JWT octets after the Bearer scheme. `authorization_domain_id` is the exact 16-byte opaque identifier selected by authenticated server routing and configuration. `method` is the exact uppercase ASCII method token accepted by the endpoint. `authority` is the server-configured lowercase ASCII host, with an explicit decimal effective port and brackets around IPv6. `path_and_query` is the exact ASCII origin-form received after trusted routing: an empty path becomes `/`, the query includes its leading `?`, and percent-encoding, parameter order, and repeated parameters are preserved. It contains no fragment. A proxy rewrite is complete before these values are computed. Ambiguous or non-canonical values are rejected. `body_digest` is SHA-256 over the exact request body, including the empty body used by a WebSocket upgrade. `proof_transport_code` is exactly one byte: `0x01` for NIP-42, `0x02` for NIP-98, `0x03` for a Git smart-HTTP session, or `0x04` for Blossom. `client_peer` is the exact canonical ASCII field value. The verifier compares the MAC in constant time.

The profile configures a positive finite `maximum_provenance_age` and a non-negative finite `future_skew`. It accepts time only when `timestamp <= now + future_skew` and `now < timestamp + maximum_provenance_age`, using overflow-safe comparisons. Equality at the age bound is expired.

For direct authorization through this profile, the lease deadline is no later than `min(assertion deadline, timestamp + maximum_provenance_age)`, in addition to every other applicable lease bound.

The verifier MUST reject an absent, repeated, malformed, stale, future-dated, wrong-key, or mismatched provenance value. It MUST reject v1 envelopes and any absent or invalid client-peer field. It MUST reject a committed nonce. A committed nonce is retained through at least `timestamp + maximum_provenance_age`; replay uniqueness is scoped to the authorization domain and `trusted-proxy-hmac-v2` profile and is independent of which active secret verified the MAC. An applicable Nostr-proof replay identity is retained through its entire acceptance window. The nonce and proof replay identity become consumed only during final admission. The MAC therefore cannot be replayed across an assertion, authorization domain, proof transport, client peer, method, authority, path, query, or body. Secret selection and rotation may try only a configured finite set of active secrets and fail closed when none verifies.

The proxy-to-verifier hop still requires confidentiality and integrity. Trusted listener and route configuration selects the profile in `R_t`. Direct ingress to a listener configured for this profile MUST reject assertion-bearing requests that lack valid provenance and MUST NOT fall back to `client-attached` after missing or rejected provenance.

`trusted-proxy-hmac-v2` is a portable stock profile, not a mandatory deployment choice. A service that selects it MUST require valid HMAC-v2 evidence on every applicable request. Selection occurs before listeners accept protected traffic and never changes in response to request evidence. Failure MUST NOT fall back to `client-attached` or a registered profile.

## Assertion validation

Every adapter produces the same closed normalized verified assertion result. The two stock profiles supply compact-JWS bytes to one canonical verifier. A registered profile either uses that verifier or supplies a reviewed adapter that authenticates its closed upstream assertion and authorization claim set before producing the same result. Ordinary forwarded headers, unchecked companion fields, or adapter-local authorization cannot enter it. Claims or capabilities in the result constrain local admission; they never replace the current local policy decision.

For each accepted JWT issuer, the verifier has authenticated configuration for the exact issuer identifier, accepted audiences, allowed asymmetric algorithms, key source, optional Nostr-key claim, optional authorization claim or capability mapping, finite maximum assertion age, and bounded clock skew. Policy construction incorporates the running implementation's verifier-contract fingerprint; it is not a client field or an operator-selected weakening. Transport adapters cannot change this contract. JWT validation enforces all of the following:

1. The input is exactly one bounded compact JWS. Protected-header and claim member names are unambiguous. Unknown critical headers, `none`, symmetric algorithms, algorithm and key-type mismatch, and incompatible JWK `use` or `key_ops` are rejected before signature acceptance.
2. The signature verifies under exactly one currently accepted asymmetric key and explicitly allowed algorithm. A duplicate or ambiguous `kid` fails. A missing `kid` is accepted only when policy deterministically selects exactly one compatible key.
3. `iss` exactly equals the configured issuer used to select the policy and key source.
4. At least one `aud` value exactly equals an accepted audience.
5. `exp` and `iat` are finite numeric dates. The verifier requires `now < exp`, `iat <= now + skew`, and `now < iat + maximum_assertion_age`, using overflow-safe comparisons. An optional `nbf` requires `nbf <= now + skew`. Equality at an expiry or maximum-age bound is expired.
6. `sub` is a non-empty exact string within the configured size bound.
7. If a Nostr-key claim is configured and present, it resolves unambiguously to one 32-byte public key. Lowercase hexadecimal is canonical. Any additional accepted encoding must normalize to that value without ambiguity.
8. Any configured authorization claim or capability mapping accepts only its closed bounded input set and produces one deterministic normalized value. When no mapping is configured, the result contains an empty set.

The verifier bounds assertion, header, claim, subject, key-identifier, and configured key-set sizes before lookup or observability. Attacker-controlled values, including `kid`, are never emitted unsanitized.

Subject stability is an issuer trust and deployment assumption, not a property that a signed request can prove. Before accepting an issuer, the operator MUST record authoritative evidence that its selected subject is opaque, stable for the account lifetime, never reassigned, and not intentionally derived from a profile or personally identifying claim. That record identifies the issuer policy version and review owner. If the issuer can reassign a subject, the same `(iss, sub)` can name a different principal and inherit identity-scoped lifecycle or recovery authority; the issuer policy MUST remain disabled until a separately authorized remediation establishes a new non-reassignable coordinate.

The normalized verified result seals `i`, an optional asserted key `k_a`, current authorization claims or capabilities, every assertion or transport deadline, `policy_id`, the transport profile, transport-contract revision, profile-contract digest, and revalidation dependencies. For JWT profiles those dependencies include JWKS generation `g`, verification-key identity, the key snapshot's hard-validity deadline, and confidential material that can recover the exact compact-JWS bytes. Display names, email addresses, and other unchecked profile claims do not enter this result.

Verifier-policy identity is independent of authenticated snapshot rotation. `policy_id` MUST be derived from a deterministic, versioned encoding of every configured semantic input plus the verifier-contract fingerprint; implementations MUST publish vectors showing that every semantic change advances it while snapshot-only rotation does not. Final admission MUST deny if the current assertion-policy identity differs from the prepared identity. A changed dependency requires revalidation that reproduces the same identity, asserted key, claims or capabilities, policy identity, and live time bounds. For JWT profiles, adding, overlapping, or removing issuer keys changes `g`, not `policy_id`; evidence under `g_old` is revalidated against the current key snapshot, and an absent key, unreadable generation, or failed revalidation denies. A normal overlapping key rotation therefore does not require a new binding or policy lineage.

The base JWT verifier does not define durable JWKS anti-rollback state. If an authenticated key source republishes a previously removed key set, that document is the current snapshot and assertions under those keys may validate again. A deployment that promises rollback prevention MUST add a separately authenticated monotonic version or equivalent durable key floor and corresponding conformance evidence; otherwise key-source rollback remains residual issuer risk.

JWT signing-key retrieval fails closed. Refresh work MUST be bounded and coalesced. An unknown `kid` cannot trigger unbounded per-request retrieval and has no stale-key fallback. A previously known key MAY be used after a soft refresh failure only under a documented finite stale-known-key policy and never after its hard maximum age.

## Nostr proof and server-owned context

The authorized key is always returned by Nostr proof validation, never by an assertion claim or unsigned field.

- NIP-42 validation binds the AUTH event to the current challenge, relay URL, connection, and freshness window.
- NIP-98 validation binds the event to the exact server-resolved absolute request URL, method, payload digest when required, and freshness window.

The service resolves `D`, operation, resource, transport, and authority from trusted server state. All evidence must agree with that same context. Unknown routes, effects, resources, domains, or transport provenance deny before preparation can become authority.

Every protected ingress in a domain MUST use one canonical current domain policy and final-admission authority. A route with no such authority, a competing authority, or an authority at a different policy lineage makes enforcement unavailable and MUST fail closed.

## Read-only preparation and final admission

Authorization uses two phases. Implementations MAY combine the phases inside one transaction, but they MUST preserve the same no-mutation and revalidation properties.

```text
PrepareAuthorization(request, assertion_input?, nostr_proof?, delegation?):
  (D, R_t, operation, resource) := ResolveTargetContext(request) or DENY

  if delegation is present:
      require assertion_input and all profile-specific fields are absent
      ValidateNostrProof(nostr_proof, D, R_t) -> k or DENY
      R := SealActor(R_t, k)
      return PrepareDelegated(D, R, k, delegation)

  ValidateConfiguredTransport(D, R_t, assertion_input) ->
      (verified_assertion, transport_evidence) or DENY
  ValidateNostrProof(nostr_proof, D, R_t) -> k or DENY
  R := SealActor(R_t, k)
  (i, k_a?, claims_or_capabilities, deadlines, policy_id,
   revalidation_dependencies) :=
      verified_assertion
  if k_a exists and k_a != k: DENY(key_mismatch)

  atomically read B(i), B(k), retired(i,k), disabled(i),
                      revoked(k), pending(i), mode(D), and policy state

  if disabled(i):                 DENY(identity_disabled)
  if revoked(k):                  DENY(key_revoked)
  if retired(i,k):                DENY(pair_retired)
  if pending(i):                  DENY(explicit_replacement_required)

  if B(i) = B(k) = binding(i,k):
      if binding.binding_not_after exists and
         now >= binding.binding_not_after: DENY(binding_expired)
      proposal := existing(binding.version, binding.provenance)
  else if B(i) exists or B(k) exists:
      DENY(binding_conflict)
  else switch mode(D):
      attested-key:
          require k_a = k
          proposal := enroll(i, k, attested-key)
      provisioned:
          DENY(binding_required)
      tofu:
          proposal := enroll(i, k, k_a = k ? attested-key : tofu)

  EvaluateEveryLocalAdmissionPolicy(
      D, R, operation, resource, k, claims_or_capabilities
  ) or DENY
  return PreparedAuthorization(all evidence, proposal, witnesses, and bounds)
```

An absent `binding_not_after` has no expiry. Assertion `exp`, `iat`, and maximum age never populate or extend it. Enrollment mode controls creation only; changing the mode does not rewrite or downgrade an existing eligible binding or its provenance.

Preparation is read-only, including for Attested and TOFU first use. It creates or changes no authoritative state, publication, or last-seen value. A denied preparation or local-policy decision has the same authoritative no-mutation property and MUST NOT create an authorization receipt. After the decision is fixed, the implementation attempts the non-authoritative denial observation defined under [rejection semantics](#rejection-semantics).

Final admission consumes the prepared value exactly once:

```text
CommitAdmission(prepared, current_request):
  require exact D, R, operation, resource, actor, and transport match
  require every assertion, proof, transport, delegation, and policy bound is live
  if prepared is DirectPrepared:
      require CurrentTransportContract(D, R_t) matches the prepared profile,
              transport-contract revision, and profile-contract digest
      require CurrentAssertionPolicyIdentity(D, prepared.direct.i.iss) =
              prepared.direct.policy_id
      if a prepared revalidation dependency changed:
          revalidate the configured profile's authoritative assertion input
          require the normalized result, including claims or capabilities,
                  is equivalent
  else:
      require prepared is DelegatedPrepared
      revalidate its delegation, relationship, owner, target, and policy witnesses

  atomically:
      reread every applicable binding, lifecycle, enrollment-mode, policy, resource,
             replay, and invalidation witness
      unreadable state denies; changed state requires a complete recomputation
      require the current result, including current claims or capabilities,
              is equivalent and eligible
      claim every applicable transport and proof replay identity,
            including the HMAC-v2 nonce when that profile is selected
      create the proposed binding only if enrollment remains eligible
      append the required receipt and privacy-safe authorization audit evidence

  return CommittedAuthorization(exact actor, binding dependencies,
                                capabilities, dependencies, and deadline)
```

No committed authorization can be constructed directly from raw claims, a prepared value, cached policy, or earlier lease. A final-admission failure rolls back every authority mutation. Complete recomputation may accept only a semantically equivalent current result. If another request concurrently creates the identical eligible binding, this request may therefore recompute as `existing`; a conflicting winner denies. Storage failure or an unreadable committed result never falls back to allow.

A denied or failed final admission MUST NOT create an authorization receipt. Its denial observation is outside the authorization transaction and cannot make a rolled-back decision authoritative.

The admitted application operation runs only after committed authorization. If the operation cannot share the authorization transaction, the implementation must use a request-bound idempotent receipt or equivalent staging so a retry cannot create a second effect from the same proof.

## Enrollment modes

- **`attested-key`**: first use requires the assertion's key claim to equal the proven key. The created binding records `attested-key` provenance.
- **`provisioned`**: ordinary requests never create a binding. A separately authorized `ProvisionBinding` transition creates it without creating a lease; later direct use still requires a current assertion and fresh proof.
- **`tofu`**: first eligible use may create a binding without a key claim. This accepts the risk that a stolen assertion for a never-enrolled identity can bind an attacker's key. Deployments MUST label and document that risk. When a matching key claim is present, the binding records `attested-key`, not `tofu`.

Binding provenance is immutable and cannot be downgraded by later requests.

## Lifecycle transitions

Provisioning, retirement, disablement, revocation, rotation, recovery, re-enablement, and administrative-expiry changes are explicit privileged transitions, never side effects of ordinary authorization. Privileged authority is bound to the exact domain, operation, identity, old binding version when present, target key when present, and request. Ordinary assertion and Nostr proof cannot substitute for that authority.

Every transition reads and rechecks the active relation and all applicable retired-pair, disabled-identity, revoked-key, and pending-replacement facts in one atomic transition. It appends immutable lifecycle history and triggers dependent lease invalidation after commit. Failure or stale state causes no partial mutation.

- **Provision binding**: allowed only in `provisioned` mode for an eligible identity and key. It creates a fresh binding version with `provisioned` provenance and no lease.
- **Retire pair**: removes the active binding, records its exact pair as retired, and records pending replacement lineage.
- **Disable identity**: records the identity as disabled. If an active binding exists, it retires that exact pair and records pending lineage.
- **Revoke key**: records the key as revoked even if it is not active. If active, it removes the binding, retires the exact pair, and records pending lineage. Repeating the same authorized revocation is idempotent and cannot erase lineage.
- **Rotate**: replaces one exact active old binding with an eligible new key, retires the old pair, and creates a fresh binding version. Rotation does not globally revoke the old key.
- **Recover**: consumes one exact pending-replacement lineage, preserves the retired old pair, and creates a fresh binding version for an eligible new key. A disabled identity uses Re-enable identity instead of Recover.
- **Re-enable identity**: requires the disabled identity, an eligible target key with fresh proof, and either no prior lineage or one exact pending lineage. It creates the new binding, clears the disabled state, and consumes present lineage exactly once. Clearing disabled state without a target would create a resurrection window: provisioned mode would have no target to match, while TOFU could let the next ordinary admission capture first use. An operator that wants to re-enable now and provision later instead leaves the identity disabled until the target and fresh proof are available.
- **Set administrative expiry**: requires one exact active binding version and sets, replaces, or clears `binding_not_after` under separate privileged policy. It advances the binding version and cannot change the pair or provenance.

Every new target key, including a provisioned key, requires fresh target-bound Nostr proof. When the domain requires issuer attestation for creation or replacement, the transition also requires a current assertion for the same identity with a key claim equal to the target key. Supplied stale, claimless, wrong-identity, or mismatched attestation is rejected; it cannot be treated as absent optional evidence.

An administrative `binding_not_after` is an authorization gate, not an implicit lifecycle transition. At or after the bound, the binding remains durable and occupies both sides of the partial bijection, but it is authorization-ineligible. Time passage alone creates no tombstone, pending lineage, or history. Restoring access requires `SetAdministrativeExpiry` or another applicable privileged lifecycle transition; ordinary authorization cannot renew the bound.

## Delegation

Delegation is a separate evidence path. The delegate presents fresh proof of its own key and no federated assertion. Separately validated delegation evidence seals the owner key, delegate key, relationship identifier and revision, allowed operations and conditions, exact request or target, and mandatory finite expiry.

The service MUST resolve a current authorization-eligible owner binding and exact binding version at preparation and final admission. A cached owner lease is not substitute authority. The delegated operation is the intersection of the sealed delegation and local operation policy. The path creates or changes no owner or delegate binding, lifecycle fact, provenance, or last-seen state.

A delegated lease requires a configured positive finite maximum. Its deadline is no later than every owner-binding, delegation, local-policy, implementation, and optional stronger owner-assertion bound. Missing finite configuration, stale owner state, actor or request mismatch, unsupported capability, unreadable dependency, or expired delegation denies. Owner retirement, disablement, key revocation, binding-version change, or relationship change invalidates dependent leases within the documented detection bound.

## Session semantics

HTTP authorization applies to one exact request. It does not imply a reusable lease.

A WebSocket lease is scoped to one authenticated key, domain, operation set, direct-assertion or delegated-evidence dependencies, current binding and lifecycle versions, policy versions, and invalidation dependencies. A direct lease records its normalized result, profile and policy revalidation dependencies, and confidential material needed to revalidate the authoritative assertion input. For a JWT profile those dependencies include JWKS generation, verification-key identity, key-snapshot hard-validity deadline, and the exact compact-JWS bytes. A delegated lease instead records the exact owner binding version and relationship revision.

The deadline is the earliest authority deadline in the normalized result and every applicable proof, transport-provenance, administrative binding, delegation, local-policy, and configured implementation bound. JWT profiles include assertion `exp`, `iat + maximum_assertion_age`, and the key-snapshot hard-validity deadline. Comparisons are overflow safe and equality is expired.

Assertion expiry ends the lease, not the binding. Renewal requires a new connection carrying a fresh assertion on the upgrade request, followed by fresh NIP-42 proof and a complete new preparation and final admission. If the durable binding remains eligible, expiry of the assertion used for an earlier lease does not prevent the new decision. Exact assertion revalidation material is retained confidentially only through the admission or lease that may need it and is destroyed on expiry, close, or invalidation.

Before each protected use, the service rechecks the binding and lifecycle versions, administrative bound, operation, resource, actor, lease deadline, and direct profile and assertion-policy dependencies. A changed dependency requires revalidation that reproduces the equivalent normalized result. For JWT evidence, the key snapshot must remain readable within its hard-validity deadline and a changed JWKS generation requires revalidation of the original assertion. For a delegated lease, the service rechecks the exact current owner binding and relationship revision. When another dependency changes, the service rejects protected operations or closes the connection within its documented detection bound. A polling implementation cannot claim immediate invalidation. A lease for one key never authorizes an operation attributed to another key on the same connection.

## Rejection semantics

Implementations may retain detailed private decision reasons for audit and conformance, including `key_mismatch`, `binding_conflict`, `pair_retired`, `identity_disabled`, `key_revoked`, `explicit_replacement_required`, and `binding_expired`. Public results map them to four stable, privacy-safe classes:

| Public code | Nostr prefix | HTTP status | Meaning |
|---|---|---:|---|
| `missing_evidence` | `auth-required:` | 401 | Required assertion, proof, or delegation evidence was absent. |
| `evidence_rejected` | `restricted:` | 403 | Presented evidence or transport provenance was rejected. |
| `authorization_denied` | `restricted:` | 403 | Current binding, lifecycle, delegation, or local operation policy denied access. |
| `authorization_unavailable` | `restricted:` | 503 | Required current state could not be verified. |

Responses MUST NOT identify another principal or key, distinguish a conflict from a tombstone, expose issuer or claim details, echo bearer material, or reveal private policy state. An unavailable dependency never becomes an allow.

For every denial, an implementation MUST attempt to emit an access-controlled denial observation with a stable private reason code and correlation identifier. The record contains only that code, the correlation identifier, a timestamp, a transport class, and bounded or keyed-hashed source coordinates. It MUST NOT contain raw tokens, HMAC values, or verbatim unverified claim values; failed `iss` or `sub` values are omitted, truncated, or keyed-hashed.

The denial-observation channel is best-effort and MUST have a finite capacity bound separate from the non-reclaimable lifetime budget for required authorization audit evidence. When the channel is unavailable, full, or truncates a record, the denial still stands, authoritative state remains unchanged, and admission does not block, retry-loop, or latch on the observation write. Missing records therefore do not prove that no denial occurred. Denial observations are for monitoring and investigation only: authorization, lockout, and rate-limit policy MUST NOT derive from them. Implementations MUST monitor and alert on deny rate, dropped observations, and channel saturation.

## Discovery

A relay SHOULD advertise support in its NIP-11 document under `limitation` as `"federated_identity": true`. It MAY include this top-level object:

```json
{
  "federated_identity": {
    "transports": ["trusted-proxy-hmac-v2"],
    "enrollment": "attested-key",
    "delegation": false
  }
}
```

`transports` contains only the exact stock identifiers `client-attached` and `trusted-proxy-hmac-v2` for profiles implemented completely. A service MUST NOT advertise a registered profile identifier. `enrollment` is exactly one configured mode. `delegation` is true only when owner-current resolution and a positive finite delegated maximum are configured. Unknown fields are ignored.

A service MUST NOT enter enforcement or advertise support until every configured protected operation uses the same canonical final-admission authority, unknown protected routes fail closed, and all applicable conformance traces pass at one reviewed revision. Discovery is selected by the same server-owned domain policy as authorization. It MUST NOT expose private issuer URLs, audiences, claim names, tenant identifiers, HMAC key identifiers, or implementation-only policy detail.

## Privacy

NIP-FI defines no public identity projection. Protocol events, tags, filters, discovery, errors, logs, metrics, traces, and denial observations MUST NOT contain raw assertions or unredacted `iss`, `sub`, email, display name, or other private claims. Access-controlled binding, lifecycle, receipt, and authorization-audit state may retain the minimum identifiers required for enforcement and investigation.

Any separate presentation protocol is non-authoritative and cannot create, renew, prove, or revoke NIP-FI authorization. Implementations MUST bound metric and log cardinality and use redacted or pseudonymous correlation.

## Security considerations

- **Issuer compromise** can impersonate principals but cannot prove an uncompromised already-bound Nostr key. In `attested-key` mode it must also forge the matching key claim to enroll an arbitrary key.
- **Assertion theft** cannot use an eligible existing binding without the bound key. TOFU intentionally retains first-use theft risk.
- **Proxy spoofing and replay** are limited by the configured trusted-edge profile's reviewed provenance, protected request components, deadlines, and replay semantics. HMAC-v2 supplies request-bound provenance, bounded time, one-time nonce consumption, exact assertion and body digests, and exact server-resolved routing values. An authenticated-edge adapter instead depends on the complete reviewed boundary-control set above.
- **JWKS rotation** does not change stable policy identity. Final generation revalidation prevents a key absent from the currently authenticated snapshot from authorizing. The base profile has no durable anti-rollback oracle; authenticated key-source republication of an old set is residual issuer risk.
- **Time-of-check/time-of-use races** are limited by read-only preparation and complete witness revalidation in final admission.
- **Lifecycle replay** cannot erase retired-pair, disabled-identity, revoked-key, or pending-replacement facts. Ordinary assertions never reactivate them.
- **Cross-domain and cross-request confusion** are prevented by server-owned context and exact evidence binding.
- **Availability attacks** on issuer, key retrieval, policy, binding, replay, or authorization-audit state fail closed. Attacker-reachable denials can exhaust finite observation capacity, so denial records use a separate bound, never weaken or delay the denial, and expose saturation and drop signals.
- **Delegation confusion** is limited by exact owner and delegate keys, owner binding version, relationship revision, capability intersection, target binding, and finite expiry.

## Stable conformance labels

The companion model and later executable matrix use these stable trace identifiers. A conforming implementation must cover every applicable trace and its boundary and concurrency subcases at one reviewed revision. Every proxy-trace and `FI-TRACE-VERIFIER-PARITY` result records `transport_contract_revision` and `profile_contract_digest`; an older revision or different profile, digest, adapter, deployment, or policy tuple cannot satisfy the claim. Each profile declares the exact spoof, replay, and cross-request artifacts its oracle exercises. The model also defines the stable safety labels `FI-INV-01` through `FI-INV-16`.

| ID | Required property |
|---|---|
| `FI-TRACE-PROXY-SPOOF` | A trusted-edge request without the configured profile's valid provenance, including direct ingress, denies. HMAC-v2 retains its exact field, MAC, and boundary negatives. |
| `FI-TRACE-PROXY-REPLAY` | The configured trusted-edge profile enforces its declared replay semantics. For HMAC-v2, two final admissions using one proxy nonce produce at most one committed authorization and preparation consumes neither. |
| `FI-TRACE-PROXY-CROSS-REQUEST` | Changing a request component protected by the configured profile denies. HMAC-v2 protects the assertion, domain, proof transport, authenticated client peer, method, authority, path/query, and body. |
| `FI-TRACE-AUTHORITY-UNIFORM` | Every protected ingress uses the same current domain policy and final-admission authority. |
| `FI-TRACE-VERIFIER-PARITY` | Equivalent authenticated assertion input and policy produce the same authorization projection and final-admission decision at each controlled time on every transport. The same trace validates each profile's identity, revision, digest, `policy_id`, and deadlines; `FI-TRACE-PREPARED-STALE` validates changed revalidation dependencies. Deterministic vectors prove semantic changes advance `policy_id` while authenticated snapshot-only rotation does not. |
| `FI-TRACE-DOMAIN-SPOOF` | Client-selected domain or forwarded authority cannot replace server-owned context. |
| `FI-TRACE-ASSERTION-KEY-MISMATCH` | An asserted key different from the proven key denies before mutation. |
| `FI-TRACE-BINDING-CONFLICT` | A pair that conflicts with either side of the active relation denies without replacement. |
| `FI-TRACE-TOMBSTONE-REPLAY` | Fresh evidence for a retired pair, disabled identity, revoked key, or pending replacement denies ordinary authorization. |
| `FI-TRACE-ASSERTION-REFRESH` | A fresh assertion can authorize the same eligible durable binding after an earlier assertion expires. |
| `FI-TRACE-ADMIN-EXPIRY` | A fresh assertion after administrative expiry denies; only an explicit privileged transition can restore access. |
| `FI-TRACE-JWKS-ADD` | A generation change with the old key retained revalidates and may authorize the unchanged binding. |
| `FI-TRACE-JWKS-REMOVE` | A generation change that removes the signing key denies prepared evidence and leases while that key is absent from the current authenticated snapshot; an A→B→A sequence proves the deployment's declared rollback behavior. |
| `FI-TRACE-PREPARED-STALE` | Changed request or decision witnesses deny or require a complete recomputation before admission. |
| `FI-TRACE-FINAL-DENIAL-NO-MUTATION` | Denied preparation, local policy, and final admission create no authoritative mutation or authorization receipt. An available denial channel records one bounded observation; an unavailable or exhausted channel leaves the denial and authoritative stores unchanged. |
| `FI-TRACE-CONCURRENT-ENROLLMENT` | Identical eligible first uses converge on one binding version; conflicting first uses commit at most one winner. |
| `FI-TRACE-TOFU-THEFT` | Stolen-assertion first use denies except under explicit risk-labelled TOFU. |
| `FI-TRACE-DELEGATE-OWNER-ROTATED` | Owner rotation makes an old-owner delegation non-current and denies without inheritance. |
| `FI-TRACE-DELEGATION-EXPIRED` | Missing or expired finite delegation bounds deny. |
| `FI-TRACE-DENIAL-ORACLE` | Unknown, conflict, tombstone, and private-policy denials are not publicly distinguishable. |
| `FI-TRACE-DEPENDENCY-FAIL-CLOSED` | An unreadable current verifier, key, state, replay, policy, receipt, or audit dependency denies. |
| `FI-TRACE-MULTI-KEY-SESSION` | A lease for one authenticated key does not authorize another key on the same connection. |
| `FI-TRACE-CROSS-DOMAIN-COLLISION` | Equal subjects across issuers or equal pairs across domains remain distinct. |
| `FI-TRACE-PRIVACY-NONPUBLIC` | Assertion or private identity material in protocol output, public history, or observability is a conformance failure. |

The companion [formal model](NIP-FI-MODEL.md) gives the state machine, safety and liveness properties, and the complete form of these traces.
