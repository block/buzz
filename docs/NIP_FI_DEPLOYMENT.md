# NIP-FI stock deployment

This guide describes a provider-neutral stock deployment for the normative [NIP-FI contract](nips/NIP-FI.md). It does not by itself activate support.

## Availability boundary

This documentation revision does not add a NIP-FI runtime parser, adapter, discovery, or enforcement. The proposed `BUZZ_NIP_FI_V1_CONFIG_JSON` document, operating modes, field bounds, and future startup rejection rules are recorded in the [identity configuration contract](CORPORATE_IDENTITY.md); they are not claims about the current relay binary.

After a later implementation supplies that parser, configuring Enforce is still not conformance. A deployment must not advertise or enforce NIP-FI until it passes the [behavioral evidence matrix](nips/NIP-FI-CONFORMANCE.md) at the deployed revision.

The files under [`docs/examples`](examples/) are review templates. They are not accepted runtime schemas and contain no deployable secrets.

## Stock topology

```text
external client
  -> TLS listener or trusted edge
  -> Buzz verifier ingress
  -> one canonical NIP-FI normalized-result and final-admission authority
       |-> binding/lifecycle, replay, policy, receipt, authorization-audit, and application stores
       |-> separately bounded, non-authoritative denial observations
       `-> configured assertion-policy source over authenticated transport
```

The stock profiles are provider free. They identify issuers by exact configured values and retrieve keys under bounded policy. They do not require a provider-specific sidecar, claim dialect, SDK, or forwarded-identity convention.

## Activation prerequisites

Do not publish NIP-FI discovery or enable enforcement until all of these are true for one immutable claim tuple:

1. The running artifact supplies a canonical normalized-result contract, final-admission authority, lifecycle state, and executable adapter manifest.
2. Every protected WebSocket and HTTP ingress appears in the executed route inventory.
3. One current domain policy and policy lineage covers every protected ingress.
4. Assertion-policy source, identity/key/claim mapping, time, size, authenticated snapshot, and enrollment policy is explicit for each domain; JWT policies also define issuer, audience, algorithm, and JWKS behavior.
5. Binding, lifecycle, replay, receipt, authorization-audit, invalidation, and application storage meets serialization and durability requirements; denial observations use a separate finite-capacity non-authoritative channel.
6. Secrets arrive from an access-controlled secret store and never from repository examples.
7. Backup, restore, key rotation, dependency outage, recovery, and rollback exercises pass.
8. All applicable `FI-TRACE-*` rows pass at the exact implementation, adapter, artifact, deployment, and policy digests.

A source scan, config parse, rendered manifest, healthy process, or documentation link closes none of these gates.

## Domain policy

Use one reviewed policy object per server-selected domain. The review-only [stock domain example](examples/nip-fi-stock-domain.json.example) records the required decisions without claiming a runtime schema.

The runtime document and its fixed verifier semantics together represent:

- exact domain identity and trusted listener or route mapping;
- accepted issuer and audience values;
- allowed compact-JWS algorithms and compatible key types;
- subject and optional Nostr-key claim rules, plus the operator evidence that the issuer subject is stable and non-reassignable;
- assertion, clock-skew, header, and body bounds;
- authenticated policy-snapshot refresh, stale-on-error, and hard-validity bounds, including JWKS behavior for JWT policies;
- exactly one enrollment mode;
- exactly one server-selected assertion transport per bound route and domain, with its transport-contract revision and profile-contract digest;
- delegation support and positive finite maximum, when enabled;
- stable public denial mapping;
- bounded private denial observation with stable reason and correlation identifiers, minimal attacker-controlled payload, and no authorization effect; and
- current policy identity and lineage, including the authenticated key or upstream-policy source identity and compiled verifier-contract fingerprint.

Rotating authenticated key or upstream-policy snapshot contents changes the applicable generation, not the stable verifier-policy identity. Changing accepted assertion semantics creates a new policy identity. For JWT policies, verification-key rotation changes the JWKS generation.

The stock contract compares against the current authenticated JWKS snapshot and does not promise durable anti-rollback. If an old set is republished, its keys may become current again. A deployment that needs rollback prevention adds an authenticated monotonic version or durable key floor and corresponding evidence.

## Client-attached profile

Expose `client-attached` only where the client can send exactly one `Nostr-Federated-Identity: Bearer <JWT>` field and fresh Nostr proof through one protected request flow. Use that assertion field for both WebSocket upgrades and HTTP requests. Never accept the federated assertion in `Authorization`. Reject assertion-provenance fields on this profile and never fall back to it after trusted-proxy evidence is missing, mixed, or invalid.

Treat assertions as confidential bearer material. Do not log headers, echo them in errors, retain them in public history, or forward them beyond the canonical verifier.

## Trusted-proxy profile

The Buzz stock trusted-proxy profile is `trusted-proxy-hmac-v2`; v1 is rejected. A deployment is not required to implement or advertise it. When selected, every applicable request requires valid HMAC-v2 evidence and cannot fall back to another profile. The edge:

1. removes every inbound assertion, provenance, and client-peer field, including attacker-supplied duplicates;
2. completes trusted routing and canonical path rewriting;
3. computes the assertion digest and request body digest;
4. creates a fresh timestamp and nonce;
5. canonicalizes the authenticated end-client IP and selects the authorization domain and proof-transport code;
6. MACs the exact timestamp, nonce, assertion digest, authorization domain, method, authority, path/query, body digest, proof transport, and client peer; and
7. sends exactly one `Nostr-Federated-Identity: Bearer <JWT>` field, one `Nostr-Federated-Identity-Provenance` field, and one `Nostr-Federated-Identity-Client-Peer` field to verifier ingress.

The HMAC secret is random, access controlled, versioned, and delivered independently of application configuration. Rotation uses a short, explicit overlap. Replay uniqueness is scoped to the trusted-proxy domain, profile, and nonce and is independent of which active secret verified the MAC. A nonce re-signed with another active secret remains a replay. The matched secret version may appear only in private audit metadata. Remove the old version after all requests and replay windows expire.

Network controls ensure that only the trusted edge can reach verifier ingress. A separate health or administrative listener cannot proxy protected operations. The deployment test runs direct-origin, bypass, client-header injection, mixed-profile, replay, and cross-request mutations from both sides of the boundary. A mocked listener test cannot satisfy `FI-TRACE-PROXY-SPOOF`.

## Registered trusted-edge profile

A deployment may install one private registered trusted-edge profile whose identifier matches `x-<operator>-<profile>-v<N>`. The identifier and its mechanism, fields, caller identity, issuer, and topology stay out of NIP-11 and public examples. Clients cannot request or infer it.

The reviewed profile contract identifies either request-bound evidence or an authenticated-edge assertion adapter. It closes every authoritative field, provenance rule, positive finite provenance bound, protected request component, replay semantic, and assertion-policy adapter identity. Equality is expired. The selected assertion policy separately closes normalized-result semantics. An authenticated-edge adapter proves cryptographic immediate-caller authentication, accepting-origin isolation, full integrity for authorization-relevant request components, inbound-field stripping, and validated upstream policy projection together.

The deployment record identifies the trusted edge, accepting origin, direct-origin controls, field-stripping point, caller authentication, protected components, upstream assertion validation, Nostr-proof path, compromise impact, and evidence location. It binds all proxy and verifier-parity evidence to the exact transport-contract revision and profile-contract digest.

If a JWT-based profile permits bearer reuse, an unexpired JWT may be presented again only with a fresh request-appropriate Nostr proof and current binding, lifecycle, policy, and final-admission state. The deployment does not claim single-use JWT semantics unless the profile defines and proves them.

## Storage and transactions

Production-equivalent storage must provide:

- serialized uniqueness for both sides of each domain's active binding relation;
- durable lifecycle selectors and immutable typed history;
- monotonic binding and lifecycle versions;
- atomic enrollment, replay claim, request-bound receipt, and required authorization audit evidence;
- idempotent application consumption when the application effect uses another transaction;
- bounded replay retention that outlives the accepted provenance window;
- a separately capacity-bounded denial-observation channel whose failure never changes or delays denial and never creates an authorization receipt;
- current policy, authenticated snapshot generation, resource, and relationship witnesses; and
- backup and restore consistency across authority state.

Caching may improve reads but cannot authorize from state older than its witnessed invalidation bound. An unavailable cache origin, database, replay store, receipt store, or required authorization-audit sink denies. An unavailable or exhausted denial-observation channel drops or truncates observation while the denial stands and authoritative stores remain unchanged.

## Rollout

### 1. Inventory

List every protected ingress and its current authorization path. Include WebSocket operations, HTTP event/query/count, media, Git, audio, invite, moderation, and operator paths. Remove or fail startup on uncovered and competing authorities.

### 2. Install without discovery

After the implementation stack supplies the proposed configuration contract, deploy its exact artifact with NIP-FI discovery and enforcement off using the implementation's reviewed fail-closed mechanism. Load reviewed policy and secret references. Validate assertion-policy source connectivity and authenticity, including JWKS for JWT policies, plus state migrations, backup, observability redaction, and route inventory without creating production bindings.

### 3. Run isolated behavior

Run every applicable adapter in an isolated namespace using synthetic issuers, identities, keys, and domains. Retain commands, results, state snapshots, wire captures, sink scans, and artifact digests.

### 4. Exercise the deployed boundary

Run production-equivalent trusted-edge negative tests, dependency fault injection, restore, key rotation, concurrency, lifecycle, recovery, and rollback exercises. Do not infer these outcomes from isolated unit tests.

### 5. Activate atomically

Only after the exact claim tuple passes, publish discovery and enable one canonical authority for the complete protected-ingress set. Do not canary by leaving some protected routes under an older identity authority. Canary domains or isolated deployments instead.

### 6. Observe without identity leakage

Monitor aggregate allow/deny classes, stable private denial reasons, denial-observation drops and saturation, dependency readiness, refresh age, replay pressure, final-admission conflicts, lease invalidations, and authorization-audit capacity. Private reason details stay access controlled. Raw assertions and identity claims are never metric labels or trace attributes, and denial records never contain raw tokens or verbatim unverified claims.

## Rollback

Rollback means returning to a previously conformant artifact and compatible policy/state lineage, or disabling NIP-FI discovery and failing the protected operation closed. It does not mean accepting unsigned identity, restoring a removed verification key, deleting tombstones, rewinding lifecycle tables, bypassing audit, or running a legacy authority beside NIP-FI.

Before rollout, record compatible artifact, policy, migration, and storage checkpoints. If a schema or semantic change is not backward compatible, use a reviewed forward repair or compensating privileged transition. Follow the [runtime operations guide](NIP_FI_RUNTIME_OPERATIONS.md) for authority-state recovery.

## Compose and Helm

The current [Compose](../deploy/compose/README.md) and [Helm](../deploy/charts/buzz/README.md) bundles contain no NIP-FI adapter wiring. Their NIP-FI sections are readiness statements, not activation instructions.

A later bundle must:

- pin the exact implementation artifact and expose its adapter/config version;
- mount domain policy and secret references without putting secrets in values or examples;
- isolate trusted edge and verifier ingress when the proxy profile is enabled;
- provide readiness that covers every fail-closed dependency;
- name the backup, restore, migration, and rollback procedures; and
- link the matching immutable conformance report.

## Deployment record

For each enforcing environment, retain the immutable claim tuple, policy and artifact digests, route inventory, accepted profiles, transport-contract revision, immutable profile-contract artifacts and digests, enrollment mode, delegation posture, secret versions without secret values, storage topology, executed adapter report, restore exercise, activation time, and rollback target.
