NIP-FI
======

Federated identity authorization — stateless core
---------------------------------------------------

`draft` `optional` `relay`

**Protocol dependencies**: NIP-01, NIP-42, NIP-98.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", and
"MAY" in this document are to be interpreted as described in BCP 14 (RFC 2119
and RFC 8174) when, and only when, they appear in all capitals.

## Abstract

NIP-FI authorizes a Nostr key when two independent facts agree: a valid
issuer-qualified identity assertion that names the key, and fresh NIP-42 proof
of possession of that key.  No relay-side identity state is required.  The
relay verifies the assertion offline against configured per-issuer JWKS
snapshots; every identity decision beyond key verification is the assertion
issuer's responsibility.

This NIP defines the assertion contract, the offline verification procedure,
session lifetime policy, and an authenticated issuer→relay disconnect API.
Enrollment, rotation, revocation decisions, identity↔key registry, one-identity
one-key enforcement, audit, and directory integration are issuer concerns outside
this spec.

## Terms

- **identity** (`i`): the exact tuple `(iss, sub)` returned by assertion
  validation.  Email, display name, opaque user ID, and a bare `sub` are not
  identities.  Equal `sub` values under different `iss` values are distinct
  identities.  [FI-TRACE-CROSS-DOMAIN-COLLISION]
- **actor** (`k`): the 32-byte public key returned by NIP-42 proof validation.
- **assertion**: a compact JWS minted by the assertion issuer, binding `i`
  to `k`.
- **assertion issuer**: the deployment-specific identity authority (e.g. an
  OIDC identity provider integration) that authenticates users and mints
  assertions.  The relay trusts only the issuer's assertion; it does not
  contact the IdP directly.

## Assertion contract

The assertion is a compact JWS carrying the following claims.

### Required claims

| Claim | Type | Semantics |
|---|---|---|
| `iss` | string | Exact issuer URI.  The relay selects an issuer policy by exact match; no normalization is applied. |
| `sub` | string | Opaque, stable, non-reassignable subject identifier for the account lifetime.  Never an email address or display name. |
| `nostr_pubkey` | string | Lowercase hexadecimal encoding of exactly one 32-byte Nostr public key.  Other encodings deny. |
| `aud` | string or array | Audience.  MUST be present.  The relay requires an exact match to the configured audience value for this issuer. |
| `iat` | NumericDate | Issuance time. |
| `exp` | NumericDate | Expiry time.  MUST be finite.  The deployment MUST configure a positive finite maximum TTL; the relay enforces both the token `exp` and the configured `maximum_assertion_age`. |

### Optional claims

| Claim | Type | Semantics |
|---|---|---|
| `nbf` | NumericDate | Not-before time.  When present, the relay enforces `nbf <= now + skew`. |

### Token type

Policy selects exactly one token class before parsing claims:

- **`nip-fi+jwt`**: a dedicated assertion whose protected `typ` is exactly
  `nip-fi+jwt`.
- **`at+jwt` access token**: a resource access token whose protected `typ` is
  exactly `at+jwt`.  When this class is selected:
  - The assertion MUST contain a non-empty `client_id` claim.
  - The issuer policy MUST name exactly one authenticated marker claim and two
    non-empty, disjoint value sets: one for resource-owner subjects and one for
    client-subject tokens.  A token whose marker value matches neither set, both
    sets, or whose marker claim is absent is ambiguous and denies.
  - When client-subject tokens are admitted, the issuer policy MUST record the
    non-collision posture: the issuer MUST guarantee that resource-owner and
    client-subject `(iss, sub)` coordinates are disjoint.
  - Absent, unknown, or ambiguous classification always denies; no fallback to
    the other class is attempted.

OIDC ID tokens always deny, even when `iss`, `aud`, and `sub` match.  A
generic or absent `typ` has no accepted class.  Failure under one class never
triggers validation under another.  [FI-TRACE-TOKEN-CLASS]

### Time bounds

**Required claims:** `iat` and `exp` MUST be present; absence denies.

**Policy knobs:** the relay enforces the following rules.  `maximum_assertion_age`
is a required positive finite configuration; a missing or non-positive
configuration denies.  `skew` is a non-negative finite maximum with default `0`;
it narrows acceptable bounds and cannot be omitted to mean "unchecked".

- `now < exp` — equality at expiry is expired
- `iat <= now + skew` — issuance is not in the future beyond allowable skew
- `now < iat + maximum_assertion_age` — caps total assertion age independent of `exp`
- `nbf <= now + skew` — when `nbf` is present (optional claim; absence is not an error)

[FI-TRACE-ASSERTION-VALIDATION]

### Assertion–key binding

`nostr_pubkey` MUST name the exact key the client proves via NIP-42.  The relay
denies any token whose `nostr_pubkey` does not match the NIP-42 `pubkey`.
[FI-TRACE-ASSERTION-KEY-MISMATCH]

This is the entire identity-to-key binding.  There is no relay-side binding
ledger; the assertion is the binding claim, and it is the assertion issuer's
responsibility to ensure the assertion names the correct key.

### Policy identity

```text
AssertionPolicyId = H(canonical assertion-policy contract)
TransportContractId = H(canonical transport contract)
```

`AssertionPolicyId` covers the canonical issuer, audience, token class,
allowed algorithms, key-source contract, identity/key/claim mapping, time and
size rules, and compiled verifier behavior.  JWKS key rotation changes the
snapshot, not the policy ID.  `TransportContractId` covers the client-attached
field, parsing, attachment, and no-fallback semantics.

## Client-attached transport

The client sends exactly one `Nostr-Federated-Identity` field on the
WebSocket upgrade request or protected HTTP request (see HTTP ingress):

```text
Nostr-Federated-Identity: Bearer <compact-JWS>
```

`Authorization` remains reserved for NIP-98.  Missing, repeated,
comma-combined, empty, malformed, non-Bearer, or mixed-profile fields deny.
Assertions MUST NOT appear in URLs, query parameters, Nostr events, tags, or
filters.  [FI-TRACE-TRANSPORT-CLOSED]

Server configuration selects `client-attached` before any protected traffic is
accepted.  Request fields cannot select, negotiate, or downgrade the transport.
Failure never falls back to another transport.

## Verification

The relay verifies assertions **offline** against configured per-issuer JWKS
snapshots.  No IdP contact occurs at admission time.

### Multi-issuer registry

The relay maintains one [`IssuerRegistry`](../../crates/buzz-auth/src/nip_fi/config.rs):
a map from exact `iss` strings to issuer policies.  The `iss` carried in the
signed token selects exactly one policy; unknown issuers deny.  A
single-issuer deployment is a registry of length one.  [FI-TRACE-CROSS-DOMAIN-COLLISION]

The existing `FederatedAssertionVerifier<S>` and `ProductionJwksSource<F>`
(merged in PR 3 / `70895b355`) implement the verification procedure described
here.  The `nostr_pubkey` claim is unconditionally required — absence rejects
regardless of issuer policy (NIP-FI v2, PR #7221).

### JWKS snapshot

Each issuer policy configures:

- `jwks_uri`: HTTPS URI selecting the authenticated key source.  SSRF-protected
  at both URI validation and DNS-resolution time; no credentials, fragments,
  or private-IP endpoints accepted.
- `refresh_interval_seconds`: positive, ≤ 1 year, strictly less than
  `key_snapshot_hard_deadline_seconds`.
- `key_snapshot_hard_deadline_seconds`: the outer time bound after which no
  assertion verified under this snapshot can authorize.

The snapshot is re-fetched periodically.  A key added to the JWKS is accepted
after the next fetch; a key removed from the JWKS causes any assertion verified
under that key to deny on next revalidation.  [FI-TRACE-JWKS-ADD]
[FI-TRACE-JWKS-REMOVE]

The snapshot is authenticated: no external consumer can relabel one issuer's
JWKS as another's.  The maximum number of keys per snapshot is bounded before
any attacker-controlled `kid` lookup.

### Verification procedure

```text
VerifyAssertion(token, D, R_t):
  // 1. Select issuer policy
  (header, claims) := BoundedJwsDecode(token) or DENY(evidence_rejected)
  policy := IssuerRegistry[claims.iss] or DENY(evidence_rejected)

  // 2. Validate token class, typ, and algorithm
  ValidateTokenClass(policy, header) or DENY(evidence_rejected)
  AssertAsymmetricAlgorithm(header.alg) or DENY(evidence_rejected)

  // 3. Validate signature against current authenticated JWKS
  snapshot := policy.key_source.get_snapshot() or DENY(authorization_unavailable)
  key := snapshot.find(header.kid) or DENY(evidence_rejected)
  VerifySignature(token, key) or DENY(evidence_rejected)

  // 4. Validate claims
  AssertExactIss(claims.iss, policy.iss) or DENY(evidence_rejected)
  AssertAudienceMatch(claims.aud, policy.aud) or DENY(evidence_rejected)
  AssertTimeBounds(claims, policy) or DENY(evidence_rejected)  // [FI-TRACE-ASSERTION-VALIDATION]
  k_claimed := ParseHexKey(claims.nostr_pubkey) or DENY(evidence_rejected)

  return VerifiedAssertion(identity=(claims.iss, claims.sub), asserted_key=k_claimed,
                           authority_deadlines=ComputeDeadlines(claims, snapshot))
```

The verifier is **fail-closed**: any unreadable, missing, ambiguous, or
expired input denies.  A missing JWKS snapshot denies with
`authorization_unavailable`; all other failures deny with `evidence_rejected`.
[FI-TRACE-DEPENDENCY-FAIL-CLOSED]

### Admission at connection

On WebSocket upgrade:

1. Extract `Nostr-Federated-Identity` header; missing or malformed → deny
   `missing_evidence` or `evidence_rejected`.
2. Call `VerifyAssertion`; any error → deny per the rejection table.
3. Complete NIP-42 handshake; validate AUTH event, extract `k`.
4. Assert `verified.asserted_key == k`; mismatch → deny `authorization_denied`.
   [FI-TRACE-ASSERTION-KEY-MISMATCH]
5. Register the session's proven `k` in the relay's session table, making it
   visible to the disconnect close scan.  Registration MUST occur before the
   deny-set check in step 6.  This ordering ensures any connection that straddles
   a concurrent disconnect is caught by one side or the other: either the close
   scan sees the registered session, or the deny-set check (step 6) sees the
   inserted entry.
6. Check deny set for `(iss, k)`; active entry (`now < until`) → deny
   `authorization_denied`.  [FI-TRACE-DENY-SET]
7. Admit the connection.  The session's authority deadline is the minimum of all
   `authority_deadlines`; see Session policy.

## Session policy

### Maximum connection lifetime

Every NIP-FI deployment MUST configure a positive finite
`max_connection_lifetime_seconds`.  This is a **required deployment knob**;
there is no default that permits an indefinite session.  Operators MUST select
a value; infosec policy governs the specific bound.

A connected session MUST be terminated no later than `connection_time + max_connection_lifetime_seconds`,
regardless of assertion expiry.

The effective session deadline is:

```
session_deadline = min(
    connection_time + max_connection_lifetime_seconds,
    min(authority_deadlines),        // from VerifiedAssertion
    key_snapshot_hard_deadline       // from the issuer policy
)
```

Equality at any deadline is expired.  Arithmetic is overflow-safe.
[FI-TRACE-LEASE-BOUND]

### Re-authentication

There is **no in-band session renewal**.  When a session expires, the relay
closes the WebSocket.  The client must open a new connection with a fresh
assertion on the upgrade request and complete a fresh NIP-42 proof.  A silent
re-mint riding an existing issuer/IdP session is an issuer implementation
detail; the relay never sees anything other than a new upgrade request.

### Reconnect after expiry

A client whose session expired due to normal TTL expiry may reconnect
immediately provided the issuer can supply a fresh assertion.  Session expiry
does not imply key revocation or identity loss; that is the issuer's domain.

## Admin disconnect API

The assertion issuer can terminate live relay sessions for a specific public key via an
authenticated `disconnect` call.

### Semantics (deny-until-TTL)

A disconnect call causes the relay to:

1. Insert a **deny entry** keyed by `(iss, target_pubkey)` into the relay's
   in-memory deny set, with an absolute expiry of `until` (a Unix timestamp
   carried in the signed command JWT; see Command JWT).  If an entry for
   `(iss, target_pubkey)` already exists, the relay MUST retain
   `max(existing_until, command.until)` — an accepted disconnect MUST NOT
   shorten an active deny.  A past-`until` command still closes sessions but
   MUST NOT clear or shorten an independently active entry.  Any subsequent
   connection or admission attempt for that pubkey under the same issuer is
   denied `authorization_denied` until `now >= until`.  If the deny set is at
   capacity and a new entry cannot be inserted, the relay MUST reject the command
   `503`; no sessions are closed and no replay state is consumed.
2. Close all live WebSocket connections whose proven `k` equals the target
   pubkey, synchronously.

The deny set is held **in relay memory only** — no durable storage, no schema
changes.  A relay restart MAY forget active deny entries.  If the issuer stops
issuing assertions and re-push completes before any expired-entry reconnection
attempt, the residual exposure after a restart is bounded by
`max(0, min(exp, iat + maximum_assertion_age) - now)`.  If the issuer continues
issuing or re-push does not complete in time, that formula does not apply and
access may continue beyond it.  [FI-TRACE-DENY-SET]

The relay MUST bound the deny set size **per issuer**.  Capacity exhaustion under
one issuer MUST NOT cause rejection of another issuer's commands; the `503`
capacity check is evaluated against the command's own issuer bound.
Implementations MUST evict only expired entries; when an issuer's partition is
at capacity and all entries are still active, the relay MUST reject the new
command `503` without removing any existing entry.

The `until` timestamp MUST NOT exceed the maximum possible remaining assertion
validity for any assertion the issuer could currently mint.  Because an
assertion accepted at the future-skew boundary (`iat <= now + skew`) remains
valid until `iat + maximum_assertion_age`, the latest possible authority deadline
is `now + skew + maximum_assertion_age`; the relay MUST enforce
`until <= now + skew + maximum_assertion_age` for this issuer policy.  An
`until` that exceeds this ceiling is rejected `400`.  Supplying an
`until` in the past is a no-op disconnect (sessions are still closed, deny entry
is immediately expired); this is not an error.

The deny entry is keyed `(iss, target_pubkey)` and applies to admission
across **all communities** served by the relay under that issuer.
Identity-level revocation is intentionally not community-partial: a key revoked
by its issuer loses access in every community that issuer governs.

In a deployment with multiple relay processes, the deployment MUST propagate
both the session-close and the deny entry to every process serving admissions
for the issuer's communities.  The propagation mechanism is deployment-defined
(for example, the existing inter-process message bus — the same posture as JWKS
snapshot convergence); each process holds its own RAM copy.  Propagation is
asynchronous with no protocol-level completion bound.  The issuer re-push duty
specified below is the recovery path for lost propagation, exactly as for relay
restart.  A success response from the receiving process does not imply
cluster-wide application.

> **Note — adopted position (session-only vs deny-until-TTL):**
>
> The session-only model closes existing connections but places no
> protocol-level bound on reconnection.  For session-only, if the issuer also
> stops issuing new assertions after a disconnect call, cumulative residual
> access is bounded by:
>
> ```
> max(0, min(exp, iat + maximum_assertion_age) - now)
> ```
>
> `max_connection_lifetime_seconds` only partitions that interval into
> individual sessions; it does not shorten the total window.  If the issuer
> continues issuing new assertions, cumulative access extends indefinitely.
>
> The **deny-until-TTL** model closes this reconnect window.  The relay holds a
> memory-resident deny set keyed by `(iss, pubkey)`, with absolute expiry
> carried by the issuer as `until` in the disconnect command.  Any admission
> attempt for that key is denied until the entry expires.  The issuer must boot
> the deny TTL to outlast the longest live assertion it may have already
> issued; otherwise an unexpired assertion lets the client back in the moment
> the entry expires.
>
> The deny set is RAM-cache, not a database — the same operational posture as
> the JWKS snapshot.  A relay restart clears the set; the issuer, as the
> durable system of record for revocations, SHOULD re-push still-active denies
> when it observes a relay restart (same publish/cache pattern as JWKS).  A
> fresh relay MAY consult the issuer before first admissions to close the
> startup race; this is non-normative.
>
> This design was chosen because session-only disconnection must outlast
> the live socket to mean anything as a revocation primitive; a self-expiring
> RAM entry preserves the zero-persistence guarantee while closing the window.

### Transport

The disconnect endpoint is an authenticated issuer→relay API, not a public
Nostr protocol.

### Command JWT

Authentication uses a short-lived signed command JWT with a dedicated token
type.  The relay verifies it with a **dedicated command verifier** that reuses
the same `IssuerRegistry`, bounded JWS parsing, issuer-bound JWKS snapshots,
signature verification, audience, and time-bound primitives as assertion
verification, but operates over a distinct token type and produces a closed
command result.  The `VerifyAssertion` primitive is not used here.

The command JWT protected header MUST carry `"typ": "nip-fi-command+jwt"`.
Any other `typ` value denies before claim parsing.

The command JWT MUST carry the following claims:

| Claim | Requirement |
|---|---|
| `iss` | Exact issuer URI matching an authorized issuer in the registry. |
| `sub` | Issuer principal identifier.  The relay checks this is an authorized issuer principal. |
| `aud` | Audience matching the relay's configured audience value for this issuer. |
| `iat` | Issuance time.  MUST satisfy `iat <= now + skew`. |
| `exp` | Expiry time.  MUST be finite; relay enforces `now < exp`. |
| `jti` | Unique, non-guessable identifier for this command.  Used for replay prevention; see below. |
| `method` | Exactly `"POST"` (uppercase literal).  Binds the command to the HTTP method. |
| `path` | Exactly `"/api/nip-fi/disconnect"` (literal string).  Binds the command to the endpoint. |
| `cmd` | Exactly `"disconnect"` (literal string).  Operation selector. |
| `target_pubkey` | Lowercase hexadecimal encoding of the target 32-byte Nostr public key — the same encoding required for the assertion `nostr_pubkey` claim. |
| `until` | Unix timestamp (NumericDate) at which the deny entry expires.  MUST NOT exceed `now + skew + maximum_assertion_age` for this issuer policy.  The relay validates this ceiling; a value that exceeds it rejects `400`. |

The `maximum_command_age` policy knob is a required positive finite
configuration per authorized issuer, with a normative upper bound of
60 seconds.  The relay enforces `0 < maximum_command_age <= 60` and
`now < iat + maximum_command_age` in addition to `now < exp`.  A missing,
non-positive, or out-of-range configuration denies.

The `VerifyCommandJwt` procedure:

```text
VerifyCommandJwt(token, request_method, request_path, request_body_pubkey):
  // 1. Bounded decode and type check
  (header, claims) := BoundedJwsDecode(token) or DENY(evidence_rejected)
  assert header.typ == "nip-fi-command+jwt" or DENY(evidence_rejected)

  // 2. Select issuer policy; verify signature
  policy := IssuerRegistry[claims.iss] or DENY(evidence_rejected)
  AssertAsymmetricAlgorithm(header.alg) or DENY(evidence_rejected)
  snapshot := policy.key_source.get_snapshot() or DENY(authorization_unavailable)
  key := snapshot.find(header.kid) or DENY(evidence_rejected)
  VerifySignature(token, key) or DENY(evidence_rejected)

  // 3. Validate claims (pure verification — no side effects)
  AssertExactIss(claims.iss, policy.iss) or DENY(evidence_rejected)
  AssertAudienceMatch(claims.aud, policy.aud) or DENY(evidence_rejected)
  AssertCommandTimeBounds(claims, policy) or DENY(evidence_rejected)
      // enforces: now < exp, iat <= now + skew, now < iat + maximum_command_age
  assert claims.method == request_method or DENY(evidence_rejected)
  assert claims.path   == request_path   or DENY(evidence_rejected)
  assert claims.cmd    == "disconnect"   or DENY(evidence_rejected)
  target_k := ParseHexKey(claims.target_pubkey) or DENY(evidence_rejected)
  until := claims.until or DENY(evidence_rejected)

  // 4. Validate until ceiling
  deny_ceiling := now + policy.skew + policy.maximum_assertion_age
  assert until <= deny_ceiling or REJECT(400)  // out-of-range until, not an auth failure

  // 5. Principal authorization (pure check — no side effects)
  AssertAuthorizedIssuerPrincipal(claims.iss, claims.sub) or DENY(authorization_denied)

  // 6. Signed-target / request-body agreement (pure check — no side effects)
  // The body carries the target pubkey so the relay can route the disconnect
  // without parsing the JWS first; the signed claim MUST agree with this
  // independently parsed input.  `until` has no such external routing role —
  // the signed claim is the sole authority and is not repeated in the body.
  assert target_k == request_body_pubkey or DENY(authorization_denied)

  // 7. Atomically reserve jti and insert deny entry — single all-or-nothing
  // admission mutation, after all pure authorization checks and before any
  // session close.  Combining both mutations here ensures a capacity failure
  // leaves neither behind: the jti is not burned, and the caller may safely
  // retry the same signed command.  Performing the jti reservation alone
  // (without the deny-entry insertion) would burn the command identity on a
  // capacity failure, making the new 503 contract unimplementable.  Capacity
  // is checked against the per-issuer bound for claims.iss; a different
  // issuer's capacity exhaustion does not produce a 503 here.
  // On same-key collision: retain max(existing_until, until) — never shorten
  // an active deny.  A past-until command merges as max(existing, past) which
  // preserves any active entry; a fresh entry with a past-until inserts with
  // an already-expired value (immediately inactive for future admissions).
  effective_expiry := min(claims.exp, claims.iat + policy.maximum_command_age)
  AtomicReserveJtiAndDenyEntry(
      iss=claims.iss, jti=claims.jti, effective_expiry=effective_expiry,
      target_pubkey=target_k, until=until
  ) or DENY(authorization_denied)  // replay: jti already reserved
    or REJECT(503)                 // capacity: deny set full; neither mutation applied

  return CommandResult(target_pubkey=target_k, caller=(claims.iss, claims.sub))
```

Any failure at any step is fail-closed: no side effects occur and the relay
returns the appropriate error.

This verifier and the disconnect API endpoint are follow-on code changes
outside this PR.

### Request

```text
POST /api/nip-fi/disconnect HTTP/1.1
Nostr-Federated-Identity: Bearer <compact-command-JWS>
Content-Type: application/json

{"pubkey": "<lowercase-hex-32-byte-pubkey>"}
```

The relay calls `VerifyCommandJwt` passing the request method, path, and
body `pubkey` field; any failure denies per the rejection table.  `VerifyCommandJwt`
performs all pure authorization checks and then, as its single atomic admission
mutation (step 7), simultaneously reserves the `(iss, jti)` replay identity and
inserts the deny entry — both or neither.  A capacity failure at that step rejects
`503`; neither the jti nor the deny entry is recorded, and the caller may safely
retry the same signed command.  On success, the relay closes all live connections
whose proven `k` equals `CommandResult.target_pubkey`.  The `until` expiry is taken
exclusively from the signed command JWT claim; the request body carries no `until`
field.  An unknown or unprovable pubkey is not an error; the relay responds `200`
with `{"disconnected": true}`.  An `until` value in the past is not an error;
sessions are closed.  Absent an active same-key deny entry the past-`until`
creates no future denial; if an active entry already exists it remains unchanged
under the merge rule.

### Response

| Condition | Status | Body |
|---|---|---|
| Authorized; action taken or no-op | `200` | `{"disconnected": true}` |
| Missing or invalid command JWT | `401` / `403` | Per the rejection table |
| Malformed request body or `until` exceeds ceiling | `400` | `bad request\n` |
| Deny set at capacity | `503` | `deny set full\n` |

## HTTP ingress

In enforce mode, every protected HTTP request MUST carry both a NIP-98
authorization event and a NIP-FI assertion bound to the same key, and the
deployment verifies both.  NIP-98 proves key possession; NIP-FI proves
identity.  Without the assertion check, a principal holding an active key
can mint fresh NIP-98 events indefinitely and retain HTTP access for as long
as the key remains accepted — the assertion's TTL provides no bound because
the assertion is never examined.  Pairing introduces the assertion lifetime
bound; an active deny-set entry blocks the next request immediately.

### Protected surfaces

A **protected HTTP surface** is a deployment-configured set of routes for
which the deployment enforces NIP-FI admission.  In enforce mode, the
deployment MUST apply NIP-FI verification to all routes in the protected
set; unprotected routes outside the set are not governed by this spec.  The
protected set MUST be configured fail-closed: a route that cannot be
classified as exempt MUST be treated as protected.  Deployment operators
define the set; this spec assigns no normative route names.

The NIP-FI issuer→relay administrative API (e.g. `/api/nip-fi/disconnect`)
is **not** a protected HTTP surface.  It is a distinct administrative
transport governed exclusively by the command-JWT contract in Admin
disconnect API.  It carries `Nostr-Federated-Identity` for its command JWS,
not for an identity assertion, and MUST NOT be subjected to the HTTP ingress
admission procedure.

> **Non-normative examples of surfaces that may appear in a protected set:**
> HTTP API bridge, invite redemption, media storage, git smart-HTTP.

### Git smart-HTTP NIP-98 credential-helper exemption

Git smart-HTTP endpoints (`info/refs`, `git-upload-pack`, and
`git-receive-pack`) use the git credential-helper proof pattern. The
credential helper signs a single NIP-98 event at credential-fetch time against
the repository-root URL with method `GET`, and Git reuses that event across the
session. For these endpoints, the NIP-98 event is exempt from method binding,
endpoint-URL binding, and the `payload` tag requirement. This exemption applies
to all three requirements on all three endpoints, including both POST
endpoints; it is not a payload-only exemption.

This exemption is required by Git's credential protocol. The credential helper
receives only credential metadata and never sees request bodies
(`crates/git-credential-nostr/src/lib.rs:98-132`), so a body hash cannot exist
in the signed event. This limitation is architectural to Git's credential
protocol.

The NIP-FI assertion and pairing requirement is unchanged and applies per
request on every one of these endpoints. Each request MUST still carry and
pass a verified NIP-FI assertion, satisfy `nostr_pubkey` == the NIP-98 event
pubkey, and pass the deny-map check. These requirements are not satisfied by a
prior request or by reuse of the NIP-98 event. The offboarding guarantee is
therefore fully intact on Git: an offboarded key is denied on its next request.

The following compensating controls are REQUIRED for this proof pattern:

1. NIP-FI assertion verification MUST run on every request.
2. The reused proof's signature and timestamp MUST be re-verified on every
   request.
3. The proof timestamp MUST remain within a `±60s` window for every request.
4. Production deployments MUST use TLS.
5. The NIP-98 event MUST bind to the repository-root URL. This binding is
   repo-scoped, not endpoint-scoped or service-scoped.
6. Git endpoint routing MUST separate clone endpoints from push endpoints.
7. Every push MUST pass pre-receive hook push authorization.

This exemption applies solely to the Git credential-helper proof pattern on
these three endpoints. It is not a precedent for any other surface. A client
change that enables per-request signing supersedes this exemption.

### Media possession-proof exception (Blossom, kind 24242)

Kind `24242` Blossom auth events are accepted as the NIP-FI pairing possession
proof for media routes **only**.  This is a media-only, operation-specific
alternative possession format — not a general "signed Nostr event" escape hatch
and not precedent for any other surface.  Any future alternative proof format
requires an explicit axis-by-axis security review covering: payload/resource
binding, method/operation scope, audience/tenant scope, freshness, signature/key
pairing, transport cardinality, and cross-endpoint replay.  Per-request NIP-98
support supersedes this exception when available.

#### Scope fence

Kind-24242 proofs are valid **only** on the following routes and operations:

| Proof type (`t` tag) | Valid route | Method |
|---|---|---|
| `upload` | `PUT /upload` (and temporary alias `PUT /media/upload` until that alias is removed) | PUT |
| `get` | `GET /media/{hash…}`, `HEAD /media/{hash…}` | GET, HEAD |

No other protected route may accept a kind-24242 proof.  A kind-24242 event
presented on any other route MUST be rejected as `evidence_rejected`.

#### Upload proofs

Upload proofs MUST carry exactly one `x` tag whose value is the lowercase
hexadecimal SHA-256 of the exact consumed request body bytes.  The signed `x`
MUST be verified against the completed body; temporal admission is checked
before the body is consumed.

Upload proofs MUST carry exactly one `server` tag whose value matches the
request's already-resolved tenant host.  An upload proof with an absent or
mismatched `server` tag MUST be rejected as `evidence_rejected`.

#### Read proofs

Read proofs MAY be host-wide: no `x` tag is required for reads, and a valid
read proof authorizes reads of any blob on the one bound tenant host.

Read proofs MUST carry exactly one `server` tag whose value matches the
request's already-resolved tenant host.  A read proof with an absent or
mismatched `server` tag MUST be rejected as `evidence_rejected`.

If an `x` tag is present on a read proof: there MUST be exactly one, and it
MUST match the requested parent blob hash.  A mismatched `x` tag MUST be
rejected as `evidence_rejected`.

> **Named residual:** within a window of at most 60 seconds from minting (plus
> a bounded 5-second future-skew allowance), a captured full header set allows
> reading any media blob on exactly one tenant host.  This access is read-only,
> membership-checked, and revocable via assertion expiry or deny-map
> enforcement.  It is not state-changing and does not cross tenant boundaries.

#### Freshness

The following freshness rules apply to all kind-24242 proofs (upload and read):

- `created_at <= now + 5s` — bounded future skew; a proof dated more than 5
  seconds in the future MUST be rejected as `evidence_rejected`.
- `now - created_at <= 60s` — a proof older than 60 seconds MUST be rejected
  as `evidence_rejected`.
- Exactly one `expiration` tag MUST be present, MUST be valid (strictly in the
  future at admission time), and MUST satisfy `expiration <= created_at + 60s`.
  An absent, duplicate, expired, or out-of-range `expiration` tag MUST be
  rejected as `evidence_rejected`.

#### Transport and cardinality

The following cardinality rules apply to all kind-24242 proofs:

- A missing `Authorization` header field MUST be treated as `missing_evidence`.
  Repeated, comma-combined, empty, malformed, or wrong-scheme `Authorization`
  values MUST be rejected as `evidence_rejected`.
- Exactly one `t` tag MUST be present.  A missing, duplicate, or unrecognized
  `t` value MUST be rejected as `evidence_rejected`.
- Exactly one `expiration` tag MUST be present (see Freshness above).
- Exactly one `server` tag MUST be present on all kind-24242 proofs (see
  Upload proofs and Read proofs above).
- If `x` is present, exactly one instance is permitted (see Upload proofs and
  Read proofs above).
- Malformed, empty, duplicate, or conflicting instances of any of these fields
  MUST be rejected as `evidence_rejected`.

#### Per-request pairing

Every kind-24242 proof MUST be subject to the full NIP-FI per-request pairing
requirement: full assertion verification, exact key equality between the
assertion's `nostr_pubkey` claim and the kind-24242 event's public key, and
deny-map enforcement (see Admission procedure, steps 1–5).

The effectiveness of deny-map enforcement is contingent on the real
issuer-scoped deny map.  Until that map is operational, the stub implementation
constitutes a **known gap** in this section's security guarantees.

#### Compliance note

The implementation as of PR #7264 pairs via a permissive Blossom verifier and
is explicitly non-compliant with this section.  The named gaps are:
multi-tag acceptance, a 3600-second proof window, and an optional `server`
tag.  These are resolved when the bounded hardening task lands.

### Request format

Each protected HTTP request MUST present both of the following:

```text
Authorization: Nostr <base64-NIP-98-event>
Nostr-Federated-Identity: Bearer <compact-JWS>
```

`Authorization` carries the NIP-98 authorization event as specified in NIP-98.
The field MUST be present exactly once, MUST use the `Nostr` scheme with a
single base64-encoded event value, and MUST NOT be repeated, comma-combined,
empty, use an alternative scheme, or carry a fallback credential.  Missing,
repeated, comma-combined, empty, malformed, non-`Nostr`, or wrong-scheme
`Authorization` values deny.

`Nostr-Federated-Identity` carries the compact-JWS assertion, identical in
format to the WebSocket transport field.  The field names are distinct; they
serve different roles and MUST NOT be combined or substituted for each other.
A request presenting only one of the two is denied.  The rules for
`Nostr-Federated-Identity` from Client-attached transport apply unchanged:
missing, repeated, comma-combined, empty, malformed, non-Bearer, or
mixed-profile fields deny.  [FI-TRACE-TRANSPORT-CLOSED]

### Admission procedure

On each protected HTTP request:

1. Extract `Nostr-Federated-Identity`; missing or malformed → deny
   `missing_evidence` or `evidence_rejected`.
2. Call `VerifyAssertion`; any error → deny per the rejection table.
3. Validate the NIP-98 `Authorization` event per NIP-98; extract the proven
   pubkey `k` from the event.  An absent, malformed, or invalid NIP-98 event
   → deny `missing_evidence` or `evidence_rejected` as appropriate.
   For requests with an authorization-relevant body, the NIP-98 event MUST
   contain exactly one `payload` tag whose value is the lowercase hexadecimal
   SHA-256 hash of the exact consumed request body bytes.  An absent,
   duplicate, or mismatched `payload` tag on such a request → deny
   `evidence_rejected`.  [FI-TRACE-HTTP-INGRESS]
4. Assert `verified.asserted_key == k`; mismatch → deny `authorization_denied`.
   [FI-TRACE-ASSERTION-KEY-MISMATCH]
5. Check deny set for `(iss, k)`; active entry (`now < until`) → deny
   `authorization_denied`.  [FI-TRACE-DENY-SET]
6. Admit the request.

A body is **authorization-relevant** whenever any body byte influences the
authorization decision, target resource, requested capability, effect
selector, or state change.  Every state-changing body is authorization-relevant.
A body may be classified non-authorization-relevant only if none of those
properties derive from body fields that are not otherwise bound.  The
classification MUST be fail-closed: a body that cannot be classified as
non-authorization-relevant MUST be treated as authorization-relevant and a
`payload` tag required.

### Per-request verification

HTTP is sessionless.  **Every** protected request re-executes the full
admission procedure above; there is no session lifetime, no cached admission
decision, and no carry-over from a prior request.  The cumulative residual
bound `max(0, min(exp, iat + maximum_assertion_age) - now)` applies per
request — there is no per-connection lifetime partition to shorten it
further.  Issuers SHOULD configure short assertion TTLs consistent with the
deployment's acceptable revocation latency.

### Denial responses

Denial on a protected HTTP request produces an HTTP response (not a Nostr
text frame).  The same public denial classes, status codes, and fixed body
bytes from the Rejection and privacy table apply.  The response contains no
free text, reason code, issuer, subject, key, claim, or timing hint.
[FI-TRACE-DENIAL-ORACLE]

## Rejection and privacy

Public class is a function only of evidence the requester supplied, never of
private per-principal server state; `authorization_unavailable` is the sole
exception and reveals only that a required dependency is unreadable.

| Private condition | Public class | Nostr text | HTTP response |
|---|---|---|---|
| assertion or proof absent | `missing_evidence` | `auth-required: authentication required` | `401`; `WWW-Authenticate: Nostr`; `Content-Type: text/plain; charset=utf-8`; body `authentication required\n` |
| malformed, invalid, or expired evidence | `evidence_rejected` | `restricted: evidence rejected` | `403`; `Content-Type: text/plain; charset=utf-8`; body `evidence rejected\n` |
| assertion–key mismatch; local policy denial; active deny-set entry for pubkey | `authorization_denied` | `restricted: authorization denied` | `403`; `Content-Type: text/plain; charset=utf-8`; body `authorization denied\n` |
| required JWKS snapshot unreadable | `authorization_unavailable` | `restricted: authorization unavailable` | `503`; `Content-Type: text/plain; charset=utf-8`; body `authorization unavailable\n` |

A denial decided on a WebSocket upgrade is the HTTP response in place of `101`.
A denial decided on a protected HTTP request is the HTTP response.
A denial decided after a WebSocket connection is established is the Nostr text.
Responses contain no free text, reason code, issuer, subject, key, claim, or
timing hint.  [FI-TRACE-DENIAL-ORACLE]

NIP-FI defines no public identity projection.  Raw assertions, `iss`, `sub`,
email, display name, and private claims MUST NOT appear in public events, tags,
filters, discovery, logs, metrics, or traces.  [FI-TRACE-PRIVACY-NONPUBLIC]

## Out of scope

The following are issuer and deployment concerns.  This spec defines no
normative behavior for them:

- Identity↔key registry, key ownership records, and the one-identity one-key
  constraint: issuer-side.
- Key rotation, re-enrollment after device loss: issuer-side.
- Revocation signaling to the issuer/IdP: issuer-side; the issuer stops
  issuing assertions, which closes the relay window within assertion TTL.
- Directory integration and account-offboarding automation: issuer-side.
- Audit logging beyond what the relay operator chooses to retain: issuer-side.
- Delegation: out of scope.
- Companion profiles (NIP-FI-EDGE, NIP-FI-LIFECYCLE, NIP-FI-DELEG, NIP-FI-CONF, NIP-FI-MODEL): removed.

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

`maximum_residual_upstream_revocation_seconds` is `null` for the
`offline-jwt` class because this spec provides no unconditional finite
upstream-revocation bound.  The deny-until-TTL mechanism closes the
reconnect window when the issuer issues a well-formed disconnect command
and the relay retains the entry, but neither condition is guaranteed by
the protocol: `until` is an upper ceiling on deny duration, not a lower
bound ensuring denial outlasts all live assertions; a past or short
`until` is valid; and a relay restart MAY forget active entries.  If the
issuer stops issuance after a successful deny, the residual ceiling is
`max(0, min(exp, iat + maximum_assertion_age) - now)` — but this is an
issuer-operational property, not a protocol invariant this field can
advertise unconditionally.

Discovery MUST NOT state issuer URLs, audiences, claim names, tenant IDs, or
deployment-local identifiers.  [FI-TRACE-DISCOVERY-PRIVATE]

## Behavioral oracles

| ID | Required outcome |
|---|---|
| `FI-TRACE-TRANSPORT-CLOSED` | Exact one-header input succeeds; missing, repeated, combined, malformed, and fallback variants deny. |
| `FI-TRACE-ASSERTION-VALIDATION` | Valid boundary input passes; each signature, key-selection, issuer, audience, time, size, and missing-configuration negative denies. |
| `FI-TRACE-TOKEN-CLASS` | `at+jwt` and `nip-fi+jwt` pass only their selected class; ID tokens, wrong or generic types, and cross-class fallback deny. |
| `FI-TRACE-ASSERTION-KEY-MISMATCH` | Mismatch between `nostr_pubkey` and the NIP-42 proven key denies with the private-state response. |
| `FI-TRACE-JWKS-ADD` | A key added to the JWKS is accepted after the next snapshot refresh. |
| `FI-TRACE-JWKS-REMOVE` | Connections verified under a removed key deny on next revalidation or reconnect. |
| `FI-TRACE-DEPENDENCY-FAIL-CLOSED` | An unreadable JWKS snapshot denies `authorization_unavailable`; no degraded Nostr-only access. |
| `FI-TRACE-LEASE-BOUND` | A session closes at its earliest deadline; equality at any deadline is expired. |
| `FI-TRACE-DENY-SET` | A pubkey in the deny set is denied `authorization_denied` on admission until `now >= until`; an expired or absent entry does not deny; a past-`until` command closes sessions — absent an active same-key entry it creates no future denial, while an active entry remains unchanged under the merge rule; a deny-set-full command is rejected `503` without closing sessions and without removing any existing entry; capacity is evaluated per issuer — one issuer's capacity exhaustion MUST NOT reject another issuer's command; a connection that passes the deny-set check before a concurrent deny-entry insertion but completes admission after MUST still be terminated (the session's proven `k` is registered before the deny-set check, ensuring the close scan catches it); two overlapping commands for the same `(iss, pubkey)` in either delivery order result in `until = max(until_A, until_B)` — delivery order does not shorten the longer deny; a past-`until` command arriving over an active entry leaves the active entry's `until` unchanged; a successful disconnect responds `{"disconnected": true}` regardless of how many sessions were closed; the deny entry applies across all communities served by the relay under that issuer. |
| `FI-TRACE-HTTP-INGRESS` | A protected HTTP request with both valid headers and matching pubkeys is admitted; absent, mismatched, or invalid assertion or NIP-98 event denies; a request presenting only one of the two denies; an active deny-set entry denies; a route that cannot be classified as exempt is treated as protected; repeated, comma-combined, wrong-scheme, or alternative-credential `Authorization` fields deny `evidence_rejected`; a missing `Authorization` field denies `missing_evidence`; an authorization-relevant body without exactly one matching `payload` tag denies; the NIP-FI administrative API is not a protected surface.  For kind-24242 (Blossom) proofs on media routes: a `t=upload` proof with valid `x`, `server`, `expiration`, and freshness is admitted on `PUT /upload`; a `t=get` proof with valid `server`, `expiration`, and freshness is admitted on `GET\|HEAD /media/{hash…}`; a kind-24242 proof on any other route denies; an upload proof with absent or mismatched `server` tag denies; a read proof with absent or mismatched `server` tag denies; a proof with a duplicate, missing, or out-of-range `expiration` tag denies; a proof dated more than 5 seconds in the future denies; a proof older than 60 seconds denies; key mismatch between assertion `nostr_pubkey` and the kind-24242 event pubkey denies; an active deny-set entry denies. |
| `FI-TRACE-DENIAL-ORACLE` | Each public-class row produces its exact fixed bytes; all private-state rows compare byte-identical. |
| `FI-TRACE-DISCOVERY-PRIVATE` | Complete discovery bytes do not expose issuer, audience, or deployment-private state. |
| `FI-TRACE-CROSS-DOMAIN-COLLISION` | Equal `sub` values under different `iss` values remain distinct identities. |
| `FI-TRACE-PRIVACY-NONPUBLIC` | Private identity does not enter public surfaces. |

## Security considerations

**Assertion theft.** A stolen assertion cannot authorize without also proving
the named `nostr_pubkey` via NIP-42.  The relay's assertion–key binding check
is the primary control against assertion replay across keys.

**TTL window after revocation.** Offline JWT verification means the relay
cannot observe IdP-side revocation until the current assertion expires.  The
deployment MUST configure a `max_connection_lifetime_seconds` and
assertion TTL consistent with the organization's acceptable revocation latency.

For upstream revocation without an explicit disconnect call (issuer stops
issuing assertions; no active session termination), access persists until the
live session's effective authority deadlines expire.  After the session closes
naturally, a reconnect requires an assertion that remains valid when reverified.
Previously issued assertions that have not yet expired remain valid for
reconnection until `min(exp, iat + maximum_assertion_age)` (subject to possible
earlier termination from a snapshot refresh failure, hard-deadline expiry without
key replacement, or signing-key removal).  Stopping issuance prevents minting
assertions that extend this window; it does not invalidate already-issued
assertions.  If the issuer continues issuing assertions, access continues.

For the deny-until-TTL disconnect model (issuer issues a successful disconnect
call with an `until` timestamp), the relay inserts a deny entry for the target
pubkey with expiry `until` and then closes all matching sessions synchronously.  Any
subsequent admission attempt for that pubkey is denied `authorization_denied`
until `now >= until`.  The `until` ceiling enforced by the relay is
`now + skew + maximum_assertion_age`; this limits how long a deny entry may last —
it does not ensure denial outlasts all live assertions.  A short or past
`until` is valid, and if the issuer continues issuing after the entry
expires, access resumes.  The issuer SHOULD set `until` to outlast the
longest still-live assertion it has issued to ensure no unexpired assertion
slides through the expiry boundary; this is an operational recommendation,
not a protocol invariant.

A relay restart clears the in-memory deny set.  The issuer SHOULD re-push
still-active entries on observed restart, but re-push is not required and
carries no specified completion bound.  If the issuer stops issuing
assertions and re-push completes before any expired-entry reconnection
attempt, the residual exposure after restart is bounded by
`max(0, min(exp, iat + maximum_assertion_age) - now)`.  If the issuer
continues issuing or re-push does not complete in time, that formula does
not apply and access may continue beyond it.

**HTTP ingress bypass.** Without the pairing rule, a principal holding an
active key can mint fresh NIP-98 events indefinitely and retain HTTP access
for as long as the key remains accepted — the assertion TTL provides no
bound when the assertion is never examined.  The pairing rule closes this
gap by requiring assertion verification on every protected HTTP request.
The per-request re-verification model means there is no cached admission
window; a deny-set entry takes effect on the very next request.

**SSRF.** The JWKS fetcher implements SSRF protection: HTTPS-only URI
validation, DNS resolution with IP deny-list enforcement, address pinning to
prevent DNS rebinding TOCTOU, and redirect denial.  The complete IANA
Special-Purpose address deny table is implemented; see `crates/buzz-core/src/network.rs`.

**Issuer compromise.** A compromised assertion issuer can impersonate any
identity but cannot prove possession of the assertion-named Nostr key.  The NIP-42
proof remains an independent control.

**Algorithm confusion.** The verifier enforces asymmetric algorithms only;
`alg=none` and symmetric algorithms deny.  The exact `kid`-based key selection
is bounded before any attacker-controlled lookup.

## Sources

- NIP-42 authentication: <https://github.com/nostr-protocol/nips/blob/6d2979b3f503a8539c983efbcdcf901bbcf9ed23/42.md>
- NIP-98 HTTP authorization: <https://github.com/nostr-protocol/nips/blob/ae0fd96907d0767f07fb54ca1de9f197c600cb27/98.md>
- JWT BCP: <https://www.rfc-editor.org/rfc/rfc8725>
- JWT access-token profile: <https://www.rfc-editor.org/rfc/rfc9068>
- DPoP: <https://www.rfc-editor.org/rfc/rfc9449>
