# NIP-FI threat model

This document records the security model for Buzz implementations of the normative [NIP-FI specification](nips/NIP-FI.md) and [formal model](nips/NIP-FI-MODEL.md). It is non-normative. The [conformance evidence matrix](nips/NIP-FI-CONFORMANCE.md) defines the behavioral release gate.

## Current status

This documentation revision contains no NIP-FI runtime adapter and establishes no conformance claim. The threats and mitigations below become implementation claims only when a later exact-head implementation and deployment tuple passes every applicable `FI-TRACE-*` behavior.

## Security objectives

NIP-FI adds issuer-qualified identity to fresh Nostr key proof without making either one sufficient by itself. Its security objectives are:

- bind authority to one server-selected domain, route, operation, resource, and actor key;
- accept assertion input only under one provider-neutral assertion policy and current authenticated key or upstream-policy snapshot;
- require every trusted-edge profile to prove its provenance, protected request components, and declared replay semantics;
- keep durable identity/key bindings independent of assertion lifetime;
- preserve retirement, disablement, revocation, replacement, and provenance across ordinary authorization;
- keep preparation read-only and make final admission the only authority commit point;
- bound sessions and delegation by every current dependency;
- deny on unreadable, ambiguous, stale, or contradictory state; and
- prevent identity material and detailed decision reasons from becoming public or attacker-observable while retaining bounded private denial signals.

## Protected assets

| Asset | Required protection |
|---|---|
| Assertions and confidential revalidation handles | Confidentiality, integrity, bounded retention, and no public projection |
| Fresh Nostr proofs and proven actor keys | Request binding, freshness, and separation between keys on one connection |
| Trusted-proxy HMAC keys, timestamps, and nonces | Secret delivery, canonical request binding, replay retention, and rotation |
| Registered-profile contracts and deployment evidence | Integrity of authoritative field mapping, caller authentication, origin controls, request protection, deadlines, and profile-specific replay claims |
| Server-selected domain and request context | Integrity against host, forwarded-field, path, query, and body substitution |
| Assertion policy and authenticated snapshot generations | Stable semantic identity, authentic refresh, hard expiry, and current-key or upstream-policy revalidation |
| Binding and lifecycle state `B`, `T`, `X`, `Y`, `Q`, `H`, and `V` | Serialization, durability, monotonic lineage, and backup consistency |
| Prepared evidence, receipts, replay claims, and audit evidence | Integrity, idempotence, privacy, and atomic final admission |
| Denial-observation channel | Separate finite capacity, minimal attacker-controlled payload, drop visibility, and no authorization effect |
| Leases, delegation, and invalidation state | Finite bounds and current owner, key, policy, and resource dependencies |
| Conformance reports and artifacts | Exact-revision provenance, digest integrity, completeness, and reproducibility |

## Actors and trust boundaries

- **Client:** controls all request fields and may hold a valid assertion, a Nostr key, both, or neither.
- **Trusted edge:** terminates the external request, strips inbound authority-bearing fields, and produces the configured profile evidence. For `trusted-proxy-hmac-v2`, it creates the request-bound HMAC envelope. A registered authenticated-edge adapter instead depends on the reviewed caller-authentication, origin-isolation, request-integrity, and upstream-policy boundary.
- **Buzz ingress:** selects the domain and route from trusted configuration, verifies transport evidence, and invokes one final-admission authority.
- **Issuer or upstream-policy authority:** authenticates identity and publishes verification keys or policy snapshots. It is trusted only within configured policy and freshness bounds.
- **State services:** store binding, lifecycle, replay, receipt, audit, policy, relationship, and application state.
- **Operator:** controls deployment policy and privileged lifecycle transitions. Operator authority is not ordinary user authority.
- **Evidence runner:** drives production-equivalent behavior and retains privacy-safe conformance artifacts for one immutable claim tuple.

The external client-to-edge, edge-to-origin, origin-to-issuer, process-to-storage, operator-to-lifecycle, and test-runner-to-deployment boundaries are independent. Trust at one boundary does not replace authentication at another.

## Attacker capabilities

The model assumes an attacker can:

- send arbitrary HTTP and WebSocket traffic, headers, authority values, paths, query strings, bodies, events, and Nostr keys;
- reach any listener exposed by deployment, replay captured traffic, race enrollment and lifecycle operations, and hold connections across rotations;
- steal a bearer assertion without stealing its user's Nostr key, or steal a Nostr key without obtaining a valid assertion;
- choose equal subjects across issuers and collide inputs across domains;
- trigger issuer or upstream-policy, authenticated snapshot, database, replay, policy, receipt, audit, and network outages; and
- observe public responses and externally exported logs, metrics, and traces.

The model does not assume that source IP, header presence, a private subnet, or a function name proves trusted provenance or authorization.

## Threats and required evidence

| Threat | Security effect | Required control | Behavioral evidence |
|---|---|---|---|
| Forged or unsigned forwarded identity | Assertion accepted without the configured trusted edge | Require the complete selected profile; reject direct, missing, mixed, and client-injected provenance without fallback | `FI-TRACE-PROXY-SPOOF` |
| Proxy nonce replay | Duplicate authorization or application effect | Retain nonce state through final admission and commit replay claim with receipt and authority state | `FI-TRACE-PROXY-REPLAY`, `FI-TRACE-FINAL-DENIAL-NO-MUTATION` |
| Cross-request HMAC reuse | Identity transplanted to another domain, peer, proof transport, or operation | MAC the timestamp, nonce, assertion digest, authorization domain, proof transport, authenticated client peer, method, authority, path/query, and body digest using exact canonical bytes | `FI-TRACE-PROXY-CROSS-REQUEST` |
| Inadequate registered-edge contract | Unchecked fields, a spoofed caller, origin bypass, or request tampering becomes authority | Close the assertion-policy mapping and profile contract; authenticate the immediate caller; isolate the accepting origin; strip inbound fields; protect every authorization-relevant component; validate upstream policy; and test the profile's actual replay surface | `FI-TRACE-PROXY-SPOOF`, `FI-TRACE-PROXY-REPLAY`, `FI-TRACE-PROXY-CROSS-REQUEST`, `FI-TRACE-VERIFIER-PARITY` |
| Domain or route confusion | Authority crosses tenants or bypasses policy | Resolve context from trusted listener and route state; reject uncovered or different-lineage authorities | `FI-TRACE-DOMAIN-SPOOF`, `FI-TRACE-AUTHORITY-UNIFORM`, `FI-TRACE-CROSS-DOMAIN-COLLISION` |
| Transport-specific verifier drift | A weaker ingress accepts assertion input or claims rejected elsewhere | Require one closed normalized-result and final-admission contract and run one corpus through every transport adapter | `FI-TRACE-VERIFIER-PARITY` |
| Verifier policy aliasing | A semantic change reuses an old policy identity | Digest every configured semantic input and a versioned compiled verifier-contract fingerprint | `FI-TRACE-VERIFIER-PARITY` policy vectors |
| Reassignable issuer subject | A different account inherits identity-scoped lifecycle or recovery authority | Treat stable non-reassignment as an issuer trust and deployment prerequisite; disable the policy when it cannot be established | Issuer review record; `FI-TRACE-CROSS-DOMAIN-COLLISION` |
| JWKS rotation race or stale key use | A key absent from the current snapshot continues to authorize, or a retained key fails unpredictably | Separate policy identity from JWKS generation and revalidate prepared evidence and leases on generation change | `FI-TRACE-JWKS-ADD`, `FI-TRACE-JWKS-REMOVE` |
| JWKS source rollback | Republishing an old authenticated set reauthorizes removed keys | Treat this as residual issuer risk unless the deployment adds an authenticated monotonic version or durable key floor | A→B→A `FI-TRACE-JWKS-REMOVE` evidence |
| Assertion/key substitution | Issuer identity attaches to an unproven key | Require fresh Nostr proof and equality with any asserted key claim | `FI-TRACE-ASSERTION-KEY-MISMATCH` |
| Binding takeover or concurrent first use | One identity or key silently replaces another | Serialize both sides of the partial bijection and commit at most one conflicting enrollment | `FI-TRACE-BINDING-CONFLICT`, `FI-TRACE-CONCURRENT-ENROLLMENT` |
| Tombstone bypass | Retired, disabled, revoked, or pending lineage reappears | Check every active lifecycle selector before enrollment and preserve lineage durably | `FI-TRACE-TOMBSTONE-REPLAY` |
| Assertion expiry confused with binding expiry | A durable binding disappears or is recreated with different provenance | Keep binding lifetime independent; treat administrative expiry as a separate privileged field | `FI-TRACE-ASSERTION-REFRESH`, `FI-TRACE-ADMIN-EXPIRY` |
| Stale prepared decision or time-of-check/time-of-use race | Changed request or authority state commits under old evidence | Keep preparation read-only; seal witnesses; reread applicable dependencies at final admission | `FI-TRACE-PREPARED-STALE`, `FI-TRACE-FINAL-DENIAL-NO-MUTATION` |
| Stolen assertion first use | Attacker enrolls its key for a new identity | Prefer attested or provisioned enrollment; expose TOFU only as explicit risk-labelled policy | `FI-TRACE-TOFU-THEFT` |
| Delegation survives owner change or expiry | Former or expired owner authority persists | Bind delegation to exact current owner version and a positive finite deadline | `FI-TRACE-DELEGATE-OWNER-ROTATED`, `FI-TRACE-DELEGATION-EXPIRED` |
| Cross-key session confusion | One key's lease authorizes another key | Key leases by actor, domain, capability, resource, and dependency set | `FI-TRACE-MULTI-KEY-SESSION` |
| Public decision oracle | Identity or private policy can be enumerated | Map private reasons to stable many-to-one public denial classes | `FI-TRACE-DENIAL-ORACLE` |
| Denial-observation exhaustion | Attacker-reachable denials consume finite observation capacity or interfere with admission | Use a separately bounded non-authoritative channel; keep denials effective when writes fail; alert on deny rate, drops, and saturation | `FI-TRACE-FINAL-DENIAL-NO-MUTATION`, `FI-TRACE-PRIVACY-NONPUBLIC` |
| Identity leakage | Assertions or claims enter public events or observability | Minimize private state and scan protocol output plus every configured sink with canaries | `FI-TRACE-PRIVACY-NONPUBLIC` |
| Dependency outage or contradictory state | Availability failure becomes an authorization bypass | Bound work, alert, and fail closed at preparation, final admission, and lease reuse | `FI-TRACE-DEPENDENCY-FAIL-CLOSED` |
| Evidence substitution | A report from another revision or transport contract is used to activate enforcement | Bind reports to one immutable claim tuple, transport-contract revision, and profile-contract digest and fail on missing or duplicate traces | Complete exact-head matrix validation |

## Enrollment risk

Attested enrollment resists bearer-assertion theft only when the issuer's key claim is itself trustworthy and equals the independently proven key. Provisioned enrollment moves creation to an explicit privileged transition but still requires target-key proof. TOFU accepts the first proven key for a never-enrolled identity and therefore cannot prevent first-use theft by an attacker holding a valid bearer assertion.

Changing enrollment mode does not repair or reclassify existing bindings. Operators must use an authorized lifecycle transition and preserve the original provenance history.

## Availability and resource exhaustion

Fail-closed behavior deliberately trades availability for authorization safety. Attackers may amplify issuer refresh, signature verification, replay lookup, authorization-audit writes, denial observations, or policy reads. Implementations bound assertion and header sizes, canonicalization work, clock skew, JWKS refresh, replay retention, concurrency, queues, and observability work. A full or unavailable required authorization-audit or replay store denies instead of silently dropping evidence.

The authorization-audit budget is a non-reclaimable installation-lifetime
capacity. Legitimate exhaustion is an accepted, unrecoverable, domain-wide
fail-closed outage within that installation and domain lineage. Sizing is an
irreversible installation-lifetime decision, so operators monitor consumption
and alert with substantial headroom. Successful or authorization-affecting
operations consume the finite budget; denied operations do not. The base
contract defines no prune, export, reset, acknowledgement, or recovery path.

Denial observations have different failure semantics because a denial is already safe. They use finite capacity separate from the non-reclaimable authorization-audit budget. Saturation or write failure drops or truncates the observation, emits aggregate health signals where possible, and leaves the denial and authoritative stores unchanged. Records minimize attacker control: no raw tokens or verbatim failed claims, only stable reason and correlation identifiers, time, transport class, and bounded or keyed-hashed source coordinates. Authorization, lockout, and rate-limit policy does not consume this best-effort channel.

Rate limits cannot replace cryptographic verification, lifecycle selectors, or final-admission serialization.

## Residual risk

The protocol cannot eliminate:

- compromise or malicious behavior by an accepted issuer within its configured claims;
- issuer subject reassignment after operator review;
- authenticated JWKS source rollback when no monotonic anti-rollback extension is deployed;
- compromise of the trusted edge, an active HMAC key, a registered adapter or its authenticated caller, the Buzz process, storage credentials, or privileged operator authority;
- first-use theft in risk-labelled TOFU mode;
- denial of service caused by required dependencies failing closed;
- unrecoverable domain-wide outage after legitimate authorization-audit budget exhaustion;
- correlation visible to systems that legitimately process private identity state; or
- an implementation defect that the exact behavioral matrix does not exercise.

Use short-lived and narrowly scoped secrets, separation of duties, access-controlled audit, independent artifact retention, and periodic reruns to reduce these risks.

## Out of scope

NIP-FI does not define provider setup, account recovery at an issuer, public identity projection, a public profile event, human-resources policy, or a mechanism for trusting ordinary unsigned corporate headers. A registered profile can rely on projected claims only under its complete cryptographically authenticated edge contract; field presence alone never creates authority.

## Review triggers

Review this threat model and rerun affected traces when a change adds an ingress, transport, issuer rule, accepted algorithm, claim, enrollment mode, lifecycle transition, delegation capability, state dependency, cache, lease, proxy hop, observability sink, restore process, or rollback path.
