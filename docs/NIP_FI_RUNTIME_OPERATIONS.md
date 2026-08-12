# NIP-FI runtime operations

This guide covers lifecycle, recovery, restore, and rollback for a conforming Buzz NIP-FI implementation. The normative [specification](nips/NIP-FI.md) and [formal model](nips/NIP-FI-MODEL.md) control when this guide differs.

## Availability boundary

This documentation revision supplies no NIP-FI runtime parser or operator tooling. The proposed future `BUZZ_NIP_FI_V1_CONFIG_JSON` contract and operating modes are documented in the [identity configuration contract](CORPORATE_IDENTITY.md), but are not claims about the current relay binary. Once an implementation exists, its own reviewed tooling defines lifecycle command names and request schemas; the [recovery-plan example](examples/nip-fi-recovery-plan.json.example) remains a review template, not a runtime request.

Do not simulate an operator transition with direct database edits. Every authority change uses a separately authorized, audited transition with fresh target-key proof when a new key is introduced.

## Operating principles

- Keep ordinary authorization separate from privileged lifecycle authority.
- Resolve the domain, operation, identity, current version, and target key from one sealed operator request.
- Check both sides of the active binding and every lifecycle selector under serialization.
- Preserve retired pairs, revoked keys, disabled identities, pending lineage, typed history, and immutable provenance.
- Treat preparation and denial as read-only with respect to authoritative state; emit denial observations only through the separately bounded non-authoritative channel.
- Commit final authority state, receipt, replay claim, and required audit evidence atomically.
- Atomically commit current authority versions and a durable invalidation intent. Fence new and reused authority on those versions immediately, then close affected cached leases after commit within the documented detection bound.
- Use compensating transitions for repair; never rewind authority tables.
- Fail closed when current policy, state, receipt, replay, audit, or invalidation dependencies are unreadable.
- Never create an authorization receipt for a denied operation or make denial depend on an observation write.

## Routine readiness

At startup and continuously, verify:

- one current policy lineage covers every protected ingress;
- the canonical verifier can read an authentic current key snapshot within its hard bound;
- binding and lifecycle stores are readable and constraints are healthy;
- replay, receipt, audit, relationship, policy, resource, and invalidation stores are available;
- clock synchronization stays inside configured bounds;
- trusted-proxy ingress is isolated and current HMAC versions are known;
- queues, replay retention, audit capacity, and JWKS refresh age remain inside limits; and
- the deployed artifact, adapter, policy, and route inventory still match the recorded conformance tuple.

Readiness for protected operations fails when a required dependency fails. Liveness may remain available for diagnosis but cannot become a bypass route.

## Lifecycle transition table

| Transition | Required current state and authority | Committed result | Required evidence |
|---|---|---|---|
| Provision | Provisioned enrollment mode; no conflicting active pair or pending lineage; eligible identity and key; privileged authority; fresh target-key proof; configured issuer attestation when required | Fresh active binding with provisioned provenance and new version | Sealed request, target proof, policy and state witnesses, receipt, typed history, and lease invalidation |
| Retire pair | Exact active identity/key pair and expected version | Binding removed, exact pair added to retired state, exact pending lineage recorded | Old and new versions, selector snapshots, receipt, history, and affected lease closure |
| Disable identity | Current enabled identity; exact active version when bound | Identity disabled; active pair retired and pending lineage recorded when present | Identity selector, optional pair/version, receipt, history, and direct/delegated invalidation |
| Revoke key | Domain-scoped key, active or inactive | Key remains revoked; active pair is removed, retired, and recorded as pending when present | Key selector, idempotent-repeat result, optional pair/version, receipt, history, and invalidation |
| Rotate | Exact active old binding/version, eligible new key, no pending lineage, fresh new-key proof | Old pair retired; new binding created at a fresh version; no pending lineage remains | Privileged transition authority, exact old binding/version, fresh new-key proof, selectors, provenance decision, receipt, history, and owner/delegate lease effects |
| Recover | Enabled identity with one exact pending lineage and no active binding; eligible new key; fresh new-key proof | Pending lineage consumed once; new binding created; old retired pair preserved | Pending version, target proof, selector snapshots, receipt, history, and invalidation |
| Re-enable identity | Disabled identity and either no prior lineage or one exact pending lineage; eligible key; fresh key proof | Disabled state cleared; present lineage consumed once; new binding created | Disabled and lineage selectors, target proof, receipt, history, and invalidation |
| Set or clear administrative expiry | Exact active binding/version and privileged authority | Administrative bound and version updated without retiring the pair or freeing either coordinate | Old/new bound, controlled-clock checks, receipt, history, and lease deadline change |

Repeated authorized revocation may be idempotent but cannot erase lineage. A retry of any transition uses its request-bound receipt and must not create a second history fact or binding version.

## Recovery decision

Use the lifecycle state, not the operator's desired outcome, to choose the transition:

- An enabled identity with exact pending replacement lineage uses **Recover**.
- A disabled identity uses **Re-enable identity**, whether or not pending lineage exists.
- An active binding moving directly to a new key uses **Rotate**.
- An identity with no active binding and no lineage uses **Provision** when policy allows it.
- A binding blocked only by administrative expiry uses the separately authorized administrative-expiry transition.

Re-enable identity always requires an eligible target key and fresh proof and
creates the new binding in the same transition that clears disabled state. A
clear-only operation would create a resurrection window: provisioned mode
would have no target to match, while TOFU could let the next ordinary
admission capture first use. To re-enable now and provision later, leave the
identity disabled until the target and proof are available.

If selectors are contradictory, versions are unknown, or history and active state disagree, stop. Preserve the evidence, fail closed, and investigate before authorizing a forward repair.

## Recovery procedure

1. Open an incident or change record and identify the server-selected domain.
2. Read the active binding from both identity and key directions plus `T`, `X`, `Y`, `Q`, `H`, and `V`.
3. Select Recover, Re-enable, Rotate, Provision, or administrative-expiry change using the decision above.
4. Obtain separately authorized operator approval and fresh proof from every new target key.
5. Prepare the transition without mutation and record its exact versions and policy witnesses.
6. At final admission, reread the applicable selectors and atomically commit the transition, receipt, history, audit evidence, current authority versions, and durable invalidation intent.
7. Fence authorization on the committed versions immediately and close affected cached leases after commit within the documented detection bound.
8. Verify the intended postconditions and that old pairs, revoked keys, and disabled or pending selectors changed only as defined.
9. Exercise old-key, old-delegation, replay, cross-domain, and new-key cases through production-equivalent ingress.
10. Retain privacy-safe evidence and close the change only after state and lease checks pass.

The review-only [recovery-plan example](examples/nip-fi-recovery-plan.json.example) helps record this plan without containing real identity or secret values.

## JWKS rotation and outage

On a new key generation:

1. authenticate and validate the complete snapshot;
2. retain the stable verifier-policy identity when accepted semantics did not change;
3. publish the new generation witness atomically;
4. revalidate prepared evidence and direct leases that observe the generation change; and
5. invalidate evidence whose signing key was removed.

A retained key may continue after successful revalidation. A key absent from the current authenticated snapshot cannot. Once a snapshot reaches its hard-validity deadline, unreadable current state denies even if a cache contains the old key.

Alert on refresh failure, increasing snapshot age, unexpected key-set regression, incompatible algorithms, or invalid metadata. The base profile has no durable anti-rollback oracle: if an authenticated source republishes an old set, its keys may become current again. Deployments that require rollback prevention must add and monitor a separately authenticated monotonic version or durable key floor. Never fix an outage by extending hard validity without a reviewed policy change and a new exact-head claim.

## Trusted-proxy secret rotation

Create a new random HMAC version in the secret store and distribute it to the edge and verifier through authenticated channels. If overlap is necessary, bound it explicitly. Keep one replay namespace for the trusted-proxy domain and profile regardless of which active secret verified a nonce; retain the matched version only as private audit metadata. Exercise baseline, old-version, new-version, the same nonce re-signed across versions, cross-request, and direct-origin cases.

Retire the old version only after its maximum timestamp skew, request lifetime, replay retention, and in-flight processing bounds have passed. Remove it from both edge and verifier and retain no secret value in logs or evidence.

## Dependency incident

When verifier, JWKS, binding, lifecycle, replay, policy, receipt, audit, relationship, resource, or invalidation state is unavailable or contradictory:

1. keep the affected protected operations fail closed;
2. stop new leases and close leases whose current dependencies cannot be checked;
3. preserve aggregate availability signals without recording raw identity material;
4. restore the dependency from a consistent known-good point;
5. reconcile versions, receipts, replay retention, and application effects before reopening; and
6. rerun `FI-TRACE-DEPENDENCY-FAIL-CLOSED` plus affected state and privacy traces.

Do not queue an authorization for later implicit approval. A client retry starts a new request with fresh evidence.

## Backup and restore

Back up a consistent authority set:

- active bindings and immutable provenance;
- retired pairs, disabled identities, revoked keys, and pending lineage;
- typed history and monotonic versions;
- verifier policies, JWKS generation metadata, and domain-policy lineage;
- delegation and relationship revisions;
- request-bound receipts and required authorization audit evidence;
- replay state for every still-acceptable request window; and
- application idempotency state needed to reconcile receipts and effects.

Keep secret material in the secret system's own protected backup process. Record secret versions, not values, with the authority backup.

After restore, hold protected traffic closed. Verify referential and selector invariants, policy and key-generation freshness, replay coverage, receipt/application reconciliation, audit continuity, lease invalidation, and route inventory. Rerun the complete exact-head conformance matrix before advertising enforcement from a materially changed restore topology.

Restoring an older database snapshot must not resurrect removed keys, deleted policy generations, retired pairs, disabled identities, revoked keys, or consumed pending lineage. Apply verified forward records or a privileged compensating transition before reopening.

## Rollback

An application rollback is safe only when the prior artifact understands the current authority schema, policy lineage, stable labels, and transition semantics. Pin the compatible artifact and verify its digest before rollout.

If no compatible conformant artifact exists, disable discovery and fail the protected operation closed while repairing forward. Do not restore unsigned forwarded identity, a legacy corporate authority, Nostr-only access to an NIP-FI-protected route, an old JWKS key, or a prior database snapshot as an authorization shortcut.

Any authority correction uses a new privileged transition with new receipt and history. Preserve the erroneous record and incident evidence.

## Monitoring and privacy

Monitor dependency readiness, public denial class, private stable denial reason, denial-observation drop and saturation rate, final-admission conflict, replay pressure, JWKS age, lease invalidation lag, lifecycle transition rate, receipt/application reconciliation, authorization-audit capacity, and route-inventory drift.

Never use raw assertions, issuer-qualified identities, Nostr keys linked to private identity, email, display name, HMAC value, or private decision reason as a metric label or public trace field. Denial observations contain only a stable reason code, correlation identifier, timestamp, transport class, and bounded or keyed-hashed source coordinates; never raw tokens or verbatim unverified claims. Access-controlled investigation records retain only the minimum needed under a documented retention period.

The denial-observation channel has a finite capacity independent of required authorization audit evidence. When it is unavailable or exhausted, drop or truncate the observation, preserve aggregate health signals where possible, and keep the denial effective without blocking or retrying admission. Do not use this best-effort history for authorization, lockout, or rate-limit decisions, and do not infer from a missing record that no denial occurred.

Run privacy canaries through allowed and denied flows after adding an observability sink or changing redaction. Finding a canary in protocol output, public history, logs, metrics, or traces is an incident and a conformance failure.

## Periodic exercises

Rerun the full matrix for every immutable release tuple and after a material deployment, issuer, policy, proxy, storage, restore, or observability change. Periodically exercise concurrency, JWKS add/remove, proxy bypass/replay, dependency outage, recovery, restore, and rollback even when application code did not change.
