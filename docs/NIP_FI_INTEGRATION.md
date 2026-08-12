# NIP-FI integration contract

This guide describes how a Buzz implementation integrates the normative [NIP-FI specification](nips/NIP-FI.md), [formal model](nips/NIP-FI-MODEL.md), and [behavioral evidence matrix](nips/NIP-FI-CONFORMANCE.md). It is non-normative and does not weaken those documents.

## Current status

This documentation revision does not add a runtime adapter, activate enforcement, or establish conformance. A later implementation stack must supply exact-head behavioral adapters for all applicable `FI-TRACE-*` identifiers. NIP-FI discovery and enforcement remain off until one immutable implementation and deployment tuple passes the release gate.

Source review, type presence, configuration parsing, route registration, and prose checks are useful review inputs. None proves runtime behavior.

### Delegation availability

NIP-FI delegation is an optional protocol capability, not an implied Buzz
feature. The current public production path has no reviewed delegated-owner
implementation and MUST NOT advertise delegation. Discovery reports
`"delegation": false` or omits the field, and operators keep delegation
disabled. A downstream or legacy configuration switch cannot create the
missing issuance, owner-resolution, expiry, invalidation, reconnect, and
protected-transport behavior.

This documentation-only revision does not change runtime code or configuration
defaults. Operators must verify the effective setting in their deployment;
public wording alone is not evidence that the runtime or its defaults satisfy
this boundary.

The specification, model, sealed types, and conformance vectors may describe
future delegation work without claiming that a route is live. Support can be
advertised only after one reviewed implementation supplies the separate
delegated-owner capability and passes every applicable delegation and session
trace across every advertised transport. Until then, any partially wired
delegation-shaped request must fail closed.

## Integration boundary

NIP-FI is an additional admission authority above NIP-42 or NIP-98 proof of key control. It does not replace Nostr signatures or application authorization. A protected operation proceeds only when all applicable gates agree:

```text
trusted route and domain
  + accepted assertion transport or delegated evidence
  + fresh Nostr proof
  + current binding and lifecycle state
  + current verifier, policy, and resource state
  + final application admission
  = committed authorization
```

A configured deployment uses one canonical current NIP-FI authority for every protected ingress in a domain. It cannot run a legacy identity authority beside NIP-FI, select an older policy lineage for one route, or leave a protected route Nostr-only.

## Protected-ingress inventory

The implementation stack owns an executable inventory of every protected operation. At minimum, reviewers classify and exercise:

- WebSocket connection authentication and each protected message or event path;
- HTTP event submission, query, and count operations;
- media reads, writes, and metadata operations;
- Git smart HTTP and policy hooks;
- audio and real-time session operations;
- invite and moderation operations; and
- lifecycle or operator operations.

The inventory records the server-selected domain, operation, resource, transport profile, policy identity, and final-admission adapter for each route. Unknown protected routes and incompatible policy lineages fail closed. `FI-TRACE-AUTHORITY-UNIFORM` executes the inventory rather than inspecting route source.

## Server-owned context

Trusted listener and route configuration resolves the target context before identity evidence can influence a decision. The Nostr proof then seals the actor key into the request context.

The implementation must not accept a client domain, forwarded authority, assertion claim, tag, or query parameter as a domain selector. If a trusted edge rewrites authority or path, it completes the rewrite before producing configured profile evidence. HMAC-v2 computes its MAC only after that rewrite, and the relay reconstructs the same canonical post-routing values.

Multi-domain deployments use the same authenticated domain lookup for authorization and discovery. An unconfigured domain receives no inherited NIP-FI profile.

## Canonical assertion verifier

Every transport adapter produces one closed normalized verified assertion result and reaches the same final-admission authority. The stock adapters call one provider-neutral compact-JWS verifier. A registered adapter either calls that verifier or authenticates its closed upstream assertion and authorization claim set before producing the same result. Ordinary forwarded headers and adapter-local authorization cannot enter it. Operators separately establish that each configured issuer's subject is stable and non-reassignable; one request cannot prove that property.

The stable verifier policy identity deterministically covers every configured assertion-semantic input—including identity/key/claim mapping, bounds, normalization, authenticated key or upstream-policy source identity, and applicable algorithm rules—plus a versioned fingerprint of compiled acceptance rules. It excludes transport and rotating key or upstream-policy snapshot contents. Prepared and direct-lease evidence retains the current profile and assertion-policy dependencies plus confidential material needed to revalidate the authoritative assertion input.

For a JWT policy, a separate JWKS generation identifies the effective key snapshot. On a generation change, final admission and lease reuse revalidate the original assertion against the current snapshot. A retained key can continue to authorize. A key absent from the current snapshot, an unreadable current snapshot, or a hard-expired snapshot denies. The base JWT contract has no durable anti-rollback oracle, so republication of an old authenticated set can make its keys current again. A non-JWT registered adapter defines equivalent authenticated current-policy snapshot dependencies and revalidation. `FI-TRACE-VERIFIER-PARITY` proves the common result; applicable JWT deployments also run `FI-TRACE-JWKS-ADD` and `FI-TRACE-JWKS-REMOVE`.

## Assertion transport

The selected profile, transport-contract revision, and profile-contract digest are part of trusted target context. Server-owned listener, route, and authorization-domain configuration selects exactly one profile before accepting protected traffic. The implementation supports only complete profiles and never falls back after mixed, missing, or rejected evidence. Clients cannot select, negotiate, or downgrade the profile.

### Client attached

The `client-attached` adapter accepts exactly one `Nostr-Federated-Identity: Bearer <JWT>` field and rejects assertion-provenance fields. The same assertion field is used for WebSocket upgrades and HTTP requests. `Authorization` remains reserved for Nostr proof where required and never carries the federated assertion. HTTP evidence and NIP-98 proof arrive on the same request. WebSocket assertion evidence arrives on the authenticated upgrade and is retained confidentially only for the resulting admission or lease.

### Trusted proxy

The `trusted-proxy-hmac-v2` adapter implements the exact envelope, canonical request bytes, time bounds, nonce size, HMAC, client-peer field, and replay retention in NIP-FI. The trusted edge removes inbound assertion, provenance, and client-peer fields before setting its own. v1 envelopes are rejected.

HMAC-v2 is the portable stock trusted-edge profile, not a mandatory deployment choice. When selected, every applicable request requires valid HMAC-v2 evidence and failure never falls back to `client-attached` or another adapter.

The deployment also proves:

- untrusted clients cannot reach verifier ingress;
- direct origin requests deny;
- mixed profiles deny without fallback;
- client header injection denies;
- nonce replay commits at most once; and
- changing the domain, proof transport, authenticated client peer, or any other request-bound field denies.

Local unit tests cannot prove network isolation. The deployment bundle must retain live negative evidence for `FI-TRACE-PROXY-SPOOF`.

### Registered trusted edge

A deployment may install one private registered profile matching `x-<operator>-<profile>-v<N>`. Its contract identifies either request-bound evidence or an authenticated-edge assertion adapter and closes every authoritative field, provenance rule, deadline, protected request component, replay semantic, and assertion-policy adapter identity. The selected assertion policy separately closes the normalized-result semantics. Registered identifiers and private mechanism details remain outside NIP-11 and public examples.

An authenticated-edge adapter must prove cryptographic immediate-caller authentication, accepting-origin isolation, full integrity for authorization-relevant request components, inbound-field stripping, and validated upstream policy projection together. It records mechanism-specific spoof, replay, cross-request, and verifier-parity evidence at the exact implementation, adapter, deployment, policy, transport-contract revision, and profile-contract digest.

If a JWT-based adapter permits bearer reuse, reusing an unexpired JWT with a fresh request-appropriate Nostr proof is not itself a proxy-replay failure. Expired assertions, replayed or transplanted Nostr proofs, presentation outside the configured edge, and every mechanism-specific captured-artifact violation still deny.

## Binding and lifecycle state

Storage represents the formal model's effective state:

- `B`: active durable bindings and immutable provenance;
- `T`: retired exact identity/key pairs;
- `X`: disabled identities;
- `Y`: domain-scoped revoked keys;
- `Q`: pending-replacement lineage;
- `H`: immutable typed lifecycle history; and
- `V`: monotonic binding and lifecycle versions.

An implementation may use different names or tables, but the behavioral selectors remain distinct. A history timestamp is not an active selector. Assertion expiry never becomes binding expiry. Optional administrative expiry is separately authorized and versioned.

Hot-path authorization reads both sides of the active relation and every applicable selector. It does not rely only on writer invariants. Unreadable or contradictory state denies.

## Prepared and committed authorization

Preparation is read-only. It creates no authoritative state, publication, or last-seen value. A denied preparation may attempt the separately bounded, non-authoritative denial observation only after the decision is fixed.

Prepared evidence uses one of two dependency sets:

- `DirectPrepared`: assertion, optional proxy provenance, actor binding and lifecycle witnesses, and enrollment-mode witness; or
- `DelegatedPrepared`: delegation, current owner binding and lifecycle witnesses, and relationship witness.

Both include exact request context, fresh Nostr proof, local policy and resource witnesses, deadlines, and invalidation dependencies.

Final admission rereads only applicable witnesses inside the authorization transaction. Unreadable state denies. Changed state requires a complete recomputation and may commit only a semantically equivalent current result. Identical concurrent enrollment may converge on the same binding version; a conflicting result denies.

Replay claims, eligible enrollment, request-bound receipt, and required authorization audit evidence commit together. If the application effect uses another transaction, it consumes a request-bound idempotent receipt so retry cannot duplicate the effect.

A denied operation creates no authorization receipt. It attempts a stable reason code and correlation identifier through the non-authoritative denial channel. That channel has a finite capacity separate from required authorization audit evidence; an unavailable or exhausted channel cannot weaken, delay, retry, or reverse the denial.

## Enrollment modes

- `attested-key` creates a first binding only when the assertion's key claim equals the proven key.
- `provisioned` never creates a binding from ordinary authorization. A privileged transition still requires fresh target-key proof and any configured issuer attestation.
- `tofu` is risk-labelled. It may bind an attacker's key when the attacker has a stolen assertion for a never-enrolled identity.

Mode changes affect future creation only. They do not rewrite existing bindings or downgrade provenance. A matching key claim in TOFU records `attested-key`, not `tofu`.

## Lifecycle operations

Provision, retire, disable, revoke, rotate, recover, re-enable, and administrative-expiry changes are separately authorized transitions. Each binds authority to the domain, operation, request, identity, old version when present, and target key when present.

Every new target key proves control. Replacement provenance reflects the evidence used for that key. A privileged transition without matching issuer key attestation records provisioned provenance; it never inherits TOFU or attested provenance from another key.

Rotation starts from an active binding and leaves no pending lineage. Recovery consumes exact pending lineage for an enabled identity. A disabled identity uses re-enablement, which requires an eligible target key with fresh proof and creates the binding while clearing disabled state. There is no clear-only transition: without a target, provisioned mode has nothing to match and TOFU could expose a first-use resurrection window. Retired pairs and revoked keys remain durable.

See [runtime operations](NIP_FI_RUNTIME_OPERATIONS.md) for preconditions, postconditions, recovery, and rollback rules.

## Sessions and delegation

HTTP authorization applies to one request. A WebSocket lease is per key, domain, capability, resource, normalized result, and exact dependency set. Its deadline is the earliest normalized authority deadline and every applicable transport, proof, binding, delegation, policy, and implementation bound. JWT profiles include assertion and key-snapshot deadlines; every profile supplies at least one finite authority deadline. Equality is expired.

This section defines the contract for future delegation support; the current
[delegation availability](#delegation-availability) statement remains
controlling for Buzz deployments.

Direct lease reuse rechecks current binding and lifecycle versions plus every profile and assertion-policy dependency; changed dependencies require equivalent normalized-result revalidation. JWT dependencies include the key-snapshot hard deadline and JWKS generation. Delegated lease reuse rechecks the current exact owner binding and relationship revision. A lease for one key never covers another key on the connection.

Delegation is optional and separate from federated assertion transport. The delegate supplies its own fresh Nostr proof and no assertion or assertion-provenance field. The owner must remain current and eligible at preparation and final admission. Rotation does not transfer delegation to the new owner key.

## Discovery and privacy

Discovery is a claim, not a feature flag. The service advertises only implemented stock profiles and optional behavior that passed the evidence matrix at the running implementation and deployment revision. Registered profile identifiers and private mechanism details never appear in NIP-11. An implementation without a complete stock adapter omits NIP-FI discovery.

NIP-FI defines no public identity projection. Assertions, issuer-qualified identities, profile claims, HMAC correlation values, and private policy state stay out of protocol output and public history. Access-controlled enforcement state retains only what lifecycle, audit, and incident response need.

Public denials use the stable classes from NIP-FI. They do not reveal whether an identity, key, binding, tombstone, claim, enrollment mode, or private policy exists.

Private denial observations contain no raw tokens or verbatim unverified claims and use only bounded or keyed-hashed source coordinates. Their absence does not prove that no denial occurred, and authorization, lockout, or rate-limit policy does not depend on them.

## Implementation-stack handoff

The later implementation stack must deliver, at one exact head:

1. Recheck binding version, lifecycle version, invalidation state, every
   authenticated policy-snapshot hard deadline, and current snapshot
   generation before every protected WebSocket use. JWT evidence revalidates
   the retained assertion when JWKS generation changes.
2. Install exactly one server-selected transport for each bound route and
   domain, with no request negotiation or fallback.
3. Preserve the HMAC-v2 envelope, canonicalization, time bounds, nonce ledger,
   replay behavior, secret rotation, and fixtures unchanged when that stock
   profile is selected.
4. Install a registered adapter only after its closed profile contract,
   normalized-result mapping, deployment evidence, and profile-contract digest
   are reviewed. Permission in these documents does not mean the current
   runtime implements such an adapter.
5. Bound every lease by all assertion, provenance, authenticated policy
   snapshot, proof, binding, local-policy, and implementation deadlines.
6. Register every issued lease with the invalidation registry, meaning the
   implementation's generic lease-cancellation owner, or an equivalent owner
   that supplies immediate version fencing and bounded post-commit closure.
7. Preserve evidence for the exact request, actor, proof transport, admission,
   adapter, deployment, policy, transport-contract revision, and
   profile-contract digest.
8. Demonstrate the selected profile's spoof, replay, cross-request,
   verifier-parity, and final-admission properties before claiming
   conformance.

The same handoff also includes the complete supporting deliverables:

- a route-adapter manifest for every protected ingress;
- one canonical normalized-result contract and shared authorization corpus;
- serialized lifecycle storage with selector-conflict fixtures;
- read-only preparation and atomic final admission;
- direct and delegated lease dependency revalidation;
- an executable adapter mapping for every applicable `FI-TRACE-*` label; and
- a conformance report whose revisions, profile contracts, and artifact
  digests match the exact deployment.

The configuration migration is explicit: add
`configuration_contract_revision`; replace the flat issuer shape with
`domains[]` and nested `domains[].issuers[]`; add `denial_observation`; remove
the earlier client-presentation-capacity object and restore map; and replace
the earlier nonstandard enrollment value with the `attested-key`,
`provisioned`, and `tofu` enum. Parser behavior, validation, fixtures, policy
digests, and adapter evidence move to that shape together. The implementation
must reject the obsolete shape rather than treat it as an alias.

These are implementation obligations, not claims about present runtime
behavior. The implementation stack may remain HMAC-v2-only until a deployment
chooses and implements a registered profile. This documentation change does
not require deleting the HMAC envelope, secrets, nonce ledger,
canonicalization, replay machinery, or fixtures. The new configuration and
trace contracts do require a revised implementation plan and evidence.

The implementation handoff also lists exact commands, required services,
fault-injection controls, and cleanup steps. A source grep, documentation link,
or claim that code paths are wired is not a substitute.

## Related guidance

- [Threat model](NIP_FI_THREAT_MODEL.md)
- [Stock deployment](NIP_FI_DEPLOYMENT.md)
- [Runtime operations](NIP_FI_RUNTIME_OPERATIONS.md)
- [Contributor guide](NIP_FI_CONTRIBUTING.md)
- [Legacy corporate identity migration](CORPORATE_IDENTITY.md)
