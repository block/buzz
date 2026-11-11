NIP-FI
======

Federated identity authorization — stateless core
---------------------------------------------------

`draft` `optional` `relay`

The optional [Agent authorization withdrawal](#agent-authorization-withdrawal)
profile adds authorization withdrawal without retiring an agent key.
The stateless core remains unchanged for deployments not selecting it.

**Protocol dependencies**: NIP-01, NIP-42, NIP-98.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", and
"MAY" in this document are to be interpreted as described in BCP 14 (RFC 2119
and RFC 8174) when, and only when, they appear in all capitals.

## Abstract

NIP-FI authorizes a Nostr key when two independent facts agree: a valid
issuer-qualified identity assertion that names the key, and fresh NIP-42 proof
of possession of that key.  No relay-side identity state is required.  The
relay verifies the assertion offline against configured per-issuer JWKS
snapshots and enforces the resolved community's audience and issuer
authorization policy; other identity decisions remain the assertion issuer's
responsibility.

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
- **community**: the tenant resolved from a connection's `Host`.  A community
  has one configured canonical host URI, which is its assertion audience, and
  an explicit set of issuer URIs authorized to mint assertions for it.

## Assertion contract

The assertion is a compact JWS carrying the following claims.

### Required claims

| Claim | Type | Semantics |
|---|---|---|
| `iss` | string | Exact issuer URI.  The relay selects an issuer policy by exact match; no normalization is applied. |
| `sub` | string | Opaque, stable, non-reassignable subject identifier for the account lifetime.  Never an email address or display name. |
| `nostr_pubkey` | string | Lowercase hexadecimal encoding of exactly one 32-byte Nostr public key.  Other encodings deny. |
| `aud` | string or array | Audience.  MUST be present.  The configured audience value for each community MUST be that community's canonical host URI, following the resource-indicator practice in RFC 8707.  One community MUST map to one canonical audience value; distinct communities MUST NOT share an audience value.  The relay MUST require the assertion audience to exactly match the expected audience of the community resolved from the connection's `Host`; issuer policy selection by `iss` MUST NOT broaden this community binding. |
| `iat` | NumericDate | Issuance time. |
| `exp` | NumericDate | Expiry time.  MUST be finite.  The deployment MUST configure a positive finite maximum TTL; the relay enforces both the token `exp` and the configured `maximum_assertion_age`. |

### Optional claims

| Claim | Type | Semantics |
|---|---|---|
| `nbf` | NumericDate | Not-before time.  When present, the relay enforces `nbf <= now + skew`. |

### Token type

Policy selects exactly one token class before parsing claims:

- **Dedicated NIP-FI assertion**: a dedicated assertion whose protected `typ`
  is exactly `nip-fi+jwt` (a human assertion) or exactly `nip-fi-agent+jwt`
  (an agent assertion; see "Agent keys").  The `typ` value, not any claim,
  determines which of the two an assertion is.
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
  - The access token MUST NOT carry `nostr_owner_pubkey`; its presence, with
    any value, denies.

OIDC ID tokens always deny, even when `iss`, `aud`, and `sub` match.  A
generic or absent `typ` has no accepted class.  `nip-fi-agent+jwt` is accepted
only under the dedicated NIP-FI class.  Failure under one class never triggers
validation under another.  [FI-TRACE-TOKEN-CLASS]

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

Community resolution MUST precede assertion verification.  The relay MUST resolve the
connection's `Host` to exactly one community, its expected audience, and its
configured issuer allowlist; an unknown or unresolvable host, or unavailable
community configuration, MUST fail closed with `authorization_unavailable`.
After bounded decoding looks up the exact issuer policy by `iss`, the relay MUST
immediately require that issuer to be authorized for the resolved community
before touching any issuer-specific dependency, including JWKS.  An untrusted
`iss` is used only to select or reject a candidate policy; it grants no
authority.  Signature verification remains mandatory before acceptance.
`VerifyAssertion` MUST compare `aud` with the resolved community's expected
audience; issuer policy selection by `iss` MUST NOT broaden either binding.

### Agent keys

An issuer MAY issue an agent assertion: an assertion whose `nostr_pubkey`
names an agent key acting for an owner key.  An agent assertion MUST use the
protected `typ` `nip-fi-agent+jwt` and MUST carry `nostr_owner_pubkey`: the
lowercase hexadecimal encoding of one 32-byte public key, distinct from
`nostr_pubkey`.  An issuer MUST NOT issue an assertion of any other type for a
key it has issued an agent assertion for, and MUST NOT issue an agent assertion
while the owner is not authorized by the issuer.

A relay MUST verify `nostr_owner_pubkey` as part of every `nip-fi-agent+jwt`
assertion.  An absent or malformed value, or one equal to `nostr_pubkey`, MUST
deny `evidence_rejected`.  A `nip-fi+jwt` assertion carrying
`nostr_owner_pubkey`, with any value, MUST deny `evidence_rejected`.  The relay
MUST record the value as the
owner of the session or request the assertion admits.  If the relay already
associates a different owner with the asserted key in the resolved community,
it MUST deny `authorization_denied`.
For every agent assertion, regardless of whether the asserted key is itself
authorized in the community, the relay MUST deny when the community bans the
asserted key or the owner, or when the issuer has denied either key.  Only
then, when the asserted key is not itself authorized in the community, the
relay MUST authorize it if and only if the issuer is trusted for the community
and the owner is authorized in the community when access is decided.
[FI-TRACE-AGENT-OWNER]

### Unverified claims

Relays MUST ignore claims they do not verify.  An unverified claim — including
`sid`, `jti`, or issuer-private blocks such as `enterprise_identity` — grants no
authority and MUST NOT be logged or exposed in discovery or error responses.
Issuers SHOULD minimize claims in assertions visible to relays to reduce
unnecessary disclosure.

### Policy identity

```text
AssertionPolicyId = H(canonical assertion-policy contract)
TransportContractId = H(canonical transport contract)
```

`AssertionPolicyId` covers the canonical issuer, community issuer
association, audience, token class, allowed algorithms, key-source contract,
identity/key/claim mapping, time and size rules, and compiled verifier
behavior.  JWKS key rotation changes the snapshot, not the policy ID.
The key-source contract includes the JWKS transport. A mesh transport tag is
encoded only when `jwks_transport` is `mesh`; `public_https` is represented by
the absence of that tag.
`TransportContractId` covers the client-attached field, parsing, attachment,
and no-fallback semantics.

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

### Community issuer authorization

Each configured community MUST declare an explicit allowlist of issuer URIs
that may authorize identities for that community.  An issuer policy selected by
exact `iss` matching is usable only when that issuer is present in the resolved
community's allowlist.  A global `IssuerRegistry` MAY hold the shared policy and
JWKS configuration, but it MUST NOT by itself authorize an issuer for every
community.  Deployments MUST reject a community configuration that has no
canonical host URI, has an ambiguous Host mapping, or grants an issuer access
without an explicit community association.

The community allowlist and its canonical audience are deployment policy, not
claims supplied by the requester.  The relay MUST resolve them before assertion
verification and MUST fail closed with `authorization_unavailable` when the
community configuration is missing or unavailable.

### Multi-issuer registry

The global `IssuerRegistry` remains
shared for issuer policy and JWKS lookup; community authorization is a separate
deployment mapping from canonical host URI to
`(expected_aud, authorized_issuers)`.  `VerifyAssertion` consumes the
Host-resolved mapping, so a globally known issuer is not implicitly trusted by
every community.

The `nostr_pubkey` claim is unconditionally required — absence rejects
regardless of issuer policy.

### JWKS snapshot

Each issuer policy configures:

- `jwks_uri`: selects the authenticated key source. With the default
  `public_https` transport, it MUST use HTTPS and contain no credentials or
  fragment. DNS address checks and pinning are specified under
  [Security considerations](#security-considerations).
- `jwks_transport`: optional per-issuer transport, defaulting to
  `public_https`. The explicit value `mesh` permits an HTTP URI only when it
  uses a DNS hostname rather than an IP literal and has no userinfo, query,
  fragment, or backslash. The only accepted IPv4 ranges are RFC 1918 (`10/8`,
  `172.16/12`, and `192.168/16`), CGNAT (`100.64/10`), reserved `240/4` except
  `255.255.255.255`, and loopback `127/8`. The only accepted IPv6 ranges or
  addresses are ULA (`fc00::/7`) and `::1`. Every other IPv4 or IPv6 class is
  rejected. IPv4-mapped, IPv4-compatible, NAT64, 6to4, and SIIT-translated
  IPv6 addresses are also rejected; embedded IPv4 does not inherit eligibility.
  Every DNS answer is checked and the first allowed address is pinned. Empty or
  mixed answers deny the fetch. Loopback is accepted for isolated local use;
  in production it can reach services on the relay host, so operators MUST
  constrain endpoint selection and egress accordingly. Mesh transport does not
  authenticate itself: selecting it is an operator assertion that the
  deployment enforces mutual TLS peer authentication and integrity protection,
  endpoint authorization, and egress restrictions. The relay cannot verify
  those properties. Unauthenticated HTTP permits JWKS key substitution and
  issuer impersonation. There is no public-network fallback.
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
VerifyAssertion(token, community, D, R_t):
  // `community` is the immutable result of Host resolution by the caller;
  // it contains `expected_aud` and the authorized issuer URI allowlist.
  AssertCommunityConfig(community) or DENY(authorization_unavailable)

  // 1. Select issuer policy and authorize it for this community
  (header, claims) := BoundedJwsDecode(token) or DENY(evidence_rejected)
  policy := IssuerRegistry[claims.iss] or DENY(evidence_rejected)
  AssertIssuerAuthorized(policy.iss, community.authorized_issuers) or DENY(evidence_rejected)

  // 2. Validate token class, typ, and algorithm
  ValidateTokenClass(policy, header) or DENY(evidence_rejected)
  AssertAsymmetricAlgorithm(header.alg) or DENY(evidence_rejected)

  // 3. Validate signature against current authenticated JWKS
  snapshot := policy.key_source.get_snapshot() or DENY(authorization_unavailable)
  key := snapshot.find(header.kid) or DENY(evidence_rejected)
  VerifySignature(token, key) or DENY(evidence_rejected)

  // 4. Validate remaining claims against the resolved community and issuer policy
  AssertExactIss(claims.iss, policy.iss) or DENY(evidence_rejected)
  AssertAudienceMatch(claims.aud, community.expected_aud) or DENY(evidence_rejected)
  AssertTimeBounds(claims, policy) or DENY(evidence_rejected)  // [FI-TRACE-ASSERTION-VALIDATION]
  k_claimed := ParseHexKey(claims.nostr_pubkey) or DENY(evidence_rejected)
  owner := absent
  if header.typ == "nip-fi-agent+jwt":
    owner := ParseHexKey(claims.nostr_owner_pubkey) or DENY(evidence_rejected)
    AssertNotEqual(owner, k_claimed) or DENY(evidence_rejected)
  else:
    AssertAbsent(claims.nostr_owner_pubkey) or DENY(evidence_rejected)

  return VerifiedAssertion(identity=(claims.iss, claims.sub), asserted_key=k_claimed,
                           owner=owner,
                           authority_deadlines=ComputeDeadlines(claims, snapshot))
```

The verifier is **fail-closed**: any unreadable, missing, ambiguous, or
expired input denies.  A missing or unavailable JWKS snapshot or community
configuration denies with `authorization_unavailable`; all other failures deny
with `evidence_rejected`.
[FI-TRACE-DEPENDENCY-FAIL-CLOSED]

### Admission at connection

On WebSocket upgrade:

1. Resolve the connection's `Host` to `community`, including its canonical
   `expected_aud` and configured issuer allowlist; unknown, ambiguous, or
   unavailable community resolution → deny `authorization_unavailable`.
2. Extract the compact JWS `token` from `Nostr-Federated-Identity`; missing or
   malformed → deny `missing_evidence` or `evidence_rejected`.
3. Call `VerifyAssertion(token, community, D, R_t)`; any error → deny per the
   rejection table.
4. Complete NIP-42 handshake; validate AUTH event, extract `k`.
5. Assert `verified.asserted_key == k`; mismatch → deny `authorization_denied`.
   [FI-TRACE-ASSERTION-KEY-MISMATCH]
6. Register the session's proven `k` and admitting issuer `iss` in the
   relay's session table, making it visible to the disconnect close scan.
   Registration includes the session's owner, if any.
   Registration MUST occur before the deny-set check in step 7.  This ordering
   ensures any connection that straddles a concurrent disconnect is caught by
   one side or the other: either the close scan sees the registered session, or
   the deny-set check (step 7) sees the inserted entry.  Both checks use the
   same keys: `(iss, k)` and, for a session with an owner,
   `(iss, verified.owner)`.  A concurrent disconnect close scan MAY cancel
   this registered connection before step 8; cancellation is terminal, so a
   cancelled connection MUST NOT be admitted.
7. Check the deny set for `(iss, k)` and, if `verified.owner` is present, for
   `(iss, verified.owner)`.  Either active entry (`now < until`) denies
   `authorization_denied`.
   [FI-TRACE-DENY-SET]
8. Admit the connection only if it remains live and uncancelled.  The session's
   authority deadline is the minimum of all `authority_deadlines`; see Session
   policy.

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
Clients SHOULD proactively obtain a fresh assertion before expiry and reconnect
with it.  Nothing in this section forbids early re-exchange; renewal remains
out of band rather than an in-band session operation.

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
2. Close all live WebSocket connections registered under the command's issuer
   whose proven `k` equals the target pubkey or whose recorded owner equals
   the target pubkey, whether or not admission has completed, synchronously.
   Disconnect cancellation is terminal: a cancelled in-progress connection
   MUST NOT be subsequently admitted.  Connections
   registered under another issuer are not affected.

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
Key-level revocation is intentionally not community-partial: a key revoked
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
  // capacity failure, making the 503 contract unimplementable.  Capacity
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
retry the same signed command.  On success, the relay MUST close every live connection
registered under the command's issuer (`claims.iss`) whose proven key equals
`CommandResult.target_pubkey` or whose recorded owner equals
`CommandResult.target_pubkey`, whether or not admission has completed.
Disconnect cancellation is terminal: a cancelled in-progress connection MUST NOT
be subsequently admitted.  This close scan matches the deny
entry's `target_pubkey` against each connection's proven key and recorded
owner; connections registered under
another issuer are not affected.  The `until` expiry is taken exclusively from
the signed command JWT claim; the request body carries no `until` field.  An
unknown or unprovable pubkey is not an error; the relay responds `200`
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
receives only credential metadata and never sees request bodies, so a body
hash cannot exist in the signed event. This limitation is architectural to
Git's credential protocol.

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
these three endpoints. It is not a precedent for any other surface.

### Media possession-proof exception (Blossom, kind 24242)

Kind `24242` Blossom auth events are accepted as the NIP-FI pairing possession
proof for media routes **only**.  This is a media-only, operation-specific
alternative possession format — not a general "signed Nostr event" escape hatch
and not precedent for any other surface.  Any future alternative proof format
requires an explicit axis-by-axis security review covering: payload/resource
binding, method/operation scope, audience/tenant scope, freshness, signature/key
pairing, transport cardinality, and cross-endpoint replay.

#### Scope fence

Kind-24242 proofs are valid **only** on the following routes and operations:

| Proof type (`t` tag) | Valid route | Method |
|---|---|---|
| `upload` | `PUT /upload`, `PUT /media/upload` | PUT |
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
deny-map enforcement (see Admission procedure, steps 1–6).

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

1. Resolve the connection's `Host` to `community`, including its canonical
   `expected_aud` and configured issuer allowlist; unknown, ambiguous, or
   unavailable community resolution → deny `authorization_unavailable`.
2. Extract the compact JWS `token` from `Nostr-Federated-Identity`; missing or
   malformed → deny `missing_evidence` or `evidence_rejected`.
3. Call `VerifyAssertion(token, community, D, R_t)`; any error → deny per the
   rejection table.
4. Validate the NIP-98 `Authorization` event per NIP-98; extract the proven
   pubkey `k` from the event.  An absent, malformed, or invalid NIP-98 event
   → deny `missing_evidence` or `evidence_rejected` as appropriate.
   For requests with an authorization-relevant body, the NIP-98 event MUST
   contain exactly one `payload` tag whose value is the lowercase hexadecimal
   SHA-256 hash of the exact consumed request body bytes.  An absent,
   duplicate, or mismatched `payload` tag on such a request → deny
   `evidence_rejected`.  [FI-TRACE-HTTP-INGRESS]
5. Assert `verified.asserted_key == k`; mismatch → deny `authorization_denied`.
   [FI-TRACE-ASSERTION-KEY-MISMATCH]
6. Check the deny set for `(iss, k)` and, if `verified.owner` is present, for
   `(iss, verified.owner)`.  Either active entry (`now < until`) denies
   `authorization_denied`.
   [FI-TRACE-DENY-SET]
7. Admit the request.

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
exception and reveals only that a required dependency or community/Host mapping
is unavailable.

| Private condition | Public class | Nostr text | HTTP response |
|---|---|---|---|
| assertion or proof absent | `missing_evidence` | `auth-required: authentication required` | `401`; `WWW-Authenticate: Nostr`; `Content-Type: text/plain; charset=utf-8`; body `authentication required\n` |
| unknown or community-unauthorized issuer; malformed, invalid, or expired evidence | `evidence_rejected` | `restricted: evidence rejected` | `403`; `Content-Type: text/plain; charset=utf-8`; body `evidence rejected\n` |
| assertion–key mismatch; unauthorized issuer principal, signed-target–body mismatch, or replayed `jti` on a signed command; active deny-set entry for pubkey | `authorization_denied` | `restricted: authorization denied` | `403`; `Content-Type: text/plain; charset=utf-8`; body `authorization denied\n` |
| required JWKS snapshot or community/Host resolution unavailable | `authorization_unavailable` | `restricted: authorization unavailable` | `503`; `Content-Type: text/plain; charset=utf-8`; body `authorization unavailable\n` |

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
- Per-session (`sid`-level) revocation: issuer-side.  The relay's revocation
  primitive is pubkey-scoped and community-wide, keyed `(iss, target_pubkey)`
  (see Admin disconnect API).  If an issuer revokes an entire identity, it
  disconnects each key it has asserted for that identity.  A disconnect of an
  owner key also covers that owner's agent keys.
  Individual session/device logout is handled by assertion expiry within the
  configured TTL window.
- Revocation signaling to the issuer/IdP: issuer-side; the issuer stops
  issuing assertions, which closes the relay window within assertion TTL.
- Directory integration and account-offboarding automation: issuer-side.
- Audit logging beyond what the relay operator chooses to retain: issuer-side.
- Delegation other than "Agent keys": out of scope.

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

A multi-community relay MAY additionally publish a host-specific top-level
`requires_nip_fi` boolean. This expresses mandatory admission evidence, separately
from support for the protocol. In Buzz it is the stored community requirement OR
global enforce; capability is advertised when global mode evaluates OR that
requirement is true. Required hosts therefore retain both existing capability
fields and the descriptor above. Optional shadow hosts still advertise capability.
Root NIP-11 and `/info` MUST agree and use `Cache-Control: no-store`. An unknown or
non-serving Host returns generic 404; unavailable host policy returns 503.

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
| `FI-TRACE-ASSERTION-VALIDATION` | Valid boundary input passes; each signature, key-selection, issuer, audience, time, size, and missing-configuration negative denies; a missing or unavailable community configuration denies `authorization_unavailable`; a known-but-community-unauthorized issuer denies `evidence_rejected` without touching its key source, indistinguishably from an unknown issuer, including when that issuer's snapshot is unavailable; an issuer authorized only for community A cannot authorize community B merely by signing an assertion with community B's audience; an assertion for community A presented on a `Host` resolved to community B denies on the community binding; an unknown, ambiguous, or unresolvable `Host` denies `authorization_unavailable`. |
| `FI-TRACE-TOKEN-CLASS` | `at+jwt`, `nip-fi+jwt`, and `nip-fi-agent+jwt` pass only their selected class; ID tokens, wrong or generic types, `nip-fi-agent+jwt` under an `at+jwt` policy, and cross-class fallback deny. |
| `FI-TRACE-AGENT-OWNER` | A `nip-fi-agent+jwt` with a valid `nostr_owner_pubkey` distinct from `nostr_pubkey` passes verification and records that owner; a `nip-fi-agent+jwt` whose `nostr_owner_pubkey` is absent, malformed, or equal to `nostr_pubkey` denies `evidence_rejected`; a `nip-fi+jwt` or `at+jwt` carrying `nostr_owner_pubkey` with any value denies `evidence_rejected`; an agent assertion denies when the community bans the asserted key or the owner, or the issuer has denied either key, even when the asserted key is itself authorized; an asserted key already associated with a different owner in the community denies `authorization_denied`; an agent whose key is not itself authorized is admitted only while its owner is authorized in the community. |
| `FI-TRACE-ASSERTION-KEY-MISMATCH` | Mismatch between `nostr_pubkey` and the NIP-42 proven key denies with the private-state response. |
| `FI-TRACE-JWKS-ADD` | A key added to the JWKS is accepted after the next snapshot refresh. |
| `FI-TRACE-JWKS-REMOVE` | Connections verified under a removed key deny on next revalidation or reconnect. |
| `FI-TRACE-DEPENDENCY-FAIL-CLOSED` | An unreadable JWKS snapshot denies `authorization_unavailable`; no degraded Nostr-only access. |
| `FI-TRACE-LEASE-BOUND` | A session closes at its earliest deadline; equality at any deadline is expired. |
| `FI-TRACE-DENY-SET` | A pubkey in the deny set is denied `authorization_denied` on admission until `now >= until`; an expired or absent entry does not deny; a past-`until` command closes sessions — absent an active same-key entry it creates no future denial, while an active entry remains unchanged under the merge rule; a deny-set-full command is rejected `503` without closing sessions and without removing any existing entry; capacity is evaluated per issuer — one issuer's capacity exhaustion MUST NOT reject another issuer's command; a connection that passes the deny-set check before a concurrent deny-entry insertion but completes admission after MUST still be terminated (the session's proven `k` and admitting issuer are registered before the deny-set check, ensuring the close scan catches it); a registered-but-not-admitted connection caught by the close scan MUST be cancelled terminally and MUST NOT later be admitted; a disconnect command from one issuer MUST NOT close a same-key connection registered under a different issuer, whether or not admission has completed; two overlapping commands for the same `(iss, pubkey)` in either delivery order result in `until = max(until_A, until_B)` — delivery order does not shorten the longer deny; a past-`until` command arriving over an active entry leaves the active entry's `until` unchanged; a successful disconnect responds `{"disconnected": true}` regardless of how many sessions were closed; the deny entry applies across all communities served by the relay under that issuer. |
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
pubkey with expiry `until` and then closes all matching connections synchronously.
Here, matching means connections registered under that issuer whose proven `k`
equals that target pubkey or whose recorded owner equals the target pubkey,
whether or not admission has completed; disconnect
cancellation is terminal, so a cancelled in-progress connection MUST NOT be
subsequently admitted.  Connections registered under another issuer are not
affected.  Any subsequent admission attempt for that pubkey under the same issuer
is denied `authorization_denied`
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

**JWKS SSRF.** `public_https` MUST validate an HTTPS URI, reject every
resolved address on its IP deny-list, pin the checked address to prevent DNS
rebinding, and deny redirects. The IP deny-list covers the complete IANA
Special-Purpose address registry. The `mesh` address allow-list and operator
security boundary are defined in the [JWKS snapshot contract](#jwks-snapshot).
Both modes MUST resolve once, validate every answer before pinning the first
permitted address, disable proxies, deny redirects, enforce a 10-second
whole-fetch deadline, cap the streamed response at 512 KiB, reject non-2xx
responses, and provide no fallback network path.

**Issuer compromise.** A compromised assertion issuer can impersonate any
identity but cannot prove possession of the assertion-named Nostr key.  The NIP-42
proof remains an independent control.

**Algorithm confusion.** The verifier enforces asymmetric algorithms only;
`alg=none` and symmetric algorithms deny.  The exact `kid`-based key selection
is bounded before any attacker-controlled lookup.

## Agent authorization withdrawal

This optional profile, `agent-authorization-withdrawal-v1`, adds withdrawal of
earlier agent assertions while preserving the asserted key. It does not authorize a workspace
transfer, restore grants, or reverse key retirement. Core deployments retain
the semantics above; where this profile is selected, the rules below supplement
Verification, Session policy, Admin disconnect API, HTTP ingress, Discovery,
and the core's stateless and revocation-scope statements.

### Scope and signed evidence

The profile uses the existing signed `iat` of `nip-fi-agent+jwt`; it adds no
assertion claim or token type. Relays MUST retain each admitted agent session's
verified assertion `iat`, issuer, asserted key, and owner. A timestamp supplied
by the client outside the verified assertion, or the relay's receipt time,
MUST NOT substitute for `iat`. Core assertion signature, audience, owner,
proof-of-possession, time bounds, and authorization checks remain mandatory.

Withdrawal state is an `issued_before` cutoff `C`, keyed by exact
`(iss, target_pubkey)`, across all communities served under that issuer by
the receiving deployment. It matches only agent assertions whose
`nostr_pubkey == target_pubkey` and whose signed `iat < C`. Equality does not
match. It MUST NOT match other issuers, human assertions, or assertions merely
naming the target as their owner. Owner-wide denial remains a core disconnect.

### Withdrawal command

The authenticated administrative endpoint and request body remain
`POST /api/nip-fi/disconnect` and `{"pubkey":"<target>"}`. Command authentication
uses `nip-fi-command+jwt` and all core signature, issuer-principal, audience,
method/path binding, size, command-age, and replay checks. The signed command
has these operation-specific claims:

| Claim | Requirement |
|---|---|
| `cmd` | Exactly `withdraw_agent_authorization`. This is a distinct operation, not an optional modifier of `disconnect`. |
| `target_pubkey` | Same lowercase-hex target encoding and signed-target/body agreement as the core command. |
| `issued_before` | A non-negative integer NumericDate, at most `9007199254740991`, interpreted with the strict comparison above. |
| `until` | MUST be absent. Withdrawal is a persistent cutoff, not a deny-until lease. |

For this operation, verification replaces the core `cmd == disconnect` and
`until` checks with the table above. Missing, malformed, fractional, out-of-range,
or duplicate operation-specific claims deny `evidence_rejected`. Unknown
operations MUST deny without effects; they MUST NOT be treated as `disconnect`.
Core `disconnect` retains its existing schema and MUST reject an
`issued_before` claim rather than silently treating it as a scope modifier.

After all pure checks, the relay MUST atomically reserve `(iss, jti)` through
the command's effective expiry and durably merge
`C := max(existing_C, command.issued_before)`. A replay denies
`authorization_denied` without changing state. A fresh command identity with
the same or an older cutoff is an idempotent semantic retry and MUST NOT
lower state. Expiry of the command JWT or replay record MUST NOT expire the
cutoff. A command that fails pure verification MUST NOT close sessions or
change state. Storage/capacity failure before commit MUST return `503` without
reserving the replay identity or claiming completion.

There is no independent `until` to merge with the cutoff. Implementations
MUST NOT independently maximize cutoff and deadline fields from separate
commands to construct a wider time-bounded scope.

### Admission, live sessions, and completion

Every protected agent admission, including HTTP, MUST deny
`authorization_denied` when `iat < max(C, F)`, where absent per-key `C` is zero
and `F` is the durable issuer-wide agent floor defined below. Unavailable enforcement state MUST deny
`authorization_unavailable`; an unreadable state store is not an absent cutoff.
These checks supplement, and never replace, core key-wide and owner denial.

A withdrawal MUST cancel all matching registered WebSocket sessions, including
those registered but not yet admitted. Registration and cutoff checking MUST
interlock with withdrawal so that either the close scan catches the session or
admission observes the cutoff; cancellation is terminal. A session with
`iat >= C` MUST NOT be closed by this withdrawal, including during delayed or
duplicate inter-process delivery. All inter-process messages MUST preserve the
verified operation and cutoff; they MUST NOT become key-wide close requests.

Before completion, matching sessions MUST lose the ability to dispatch work or
receive protected output. Matching protected HTTP work admitted before the
cutoff MUST finish, be cancelled, or be fenced against further protected effects
before completion is acknowledged. Already-performed effects are not undone.
Implementations MUST NOT acknowledge completion merely because a socket close
has been queued or a pub/sub message sent.

The receiving deployment MUST have an authenticated, operator-configured
authority for the complete set of processes and ingress paths that can serve
the issuer, including HTTP, WebSocket, and relevant long-lived transports.
Its completion coordinator MUST confirm that each serving process has applied
the effective cutoff and fenced matching work, or that the process is fenced
from all serving paths. Joining/restarted processes MUST restore enforcement
state before serving. A partitioned process MUST be fenced or prevent a
completion acknowledgement; an unreachable process is not an acknowledgement.
The deployment MUST preserve these conditions as membership changes.

Only after those conditions hold MAY the coordinator return HTTP `200` with
`{"withdrawn":true,"issued_before":C_requested}` over the authenticated
administrative connection. This attests at least the requested cutoff across
the entire configured deployment; it does not expose a newer stored cutoff.
The issuer trusts this acknowledgement through the authenticated administrative
connection to its configured coordinator; it is not a signed, transferable
receipt and MUST NOT be accepted from an untrusted intermediary.
The receiver MUST use a finite configured completion timeout. An incomplete
barrier returns `503` with `authorization unavailable\n`, not core
`{"disconnected":true}`. Partial application may already have committed and
MUST NOT be rolled back. The issuer retries using a fresh `jti`.

The issuer MUST maintain a trusted inventory of every relay deployment to
which affected assertions were issued. Discovery and a single deployment's
acknowledgement MUST NOT stand in for that inventory. Destination issuance
relying on completed withdrawal MUST wait for completion from every such
deployment. An incomplete or unknown inventory MUST take the fallback below.

### Persistence and precedence

Profile deployments MUST persist cutoffs and recover them before admission;
RAM-only storage plus eventual re-push does not meet this profile. Cutoffs
MUST NOT decrease on retry, restart, failover, policy reload, or restore from
backup. Equivalent or stronger enforcement MAY replace a per-key cutoff only
under the compaction procedure below. If state rollback cannot be excluded, affected issuer admissions MUST stay
closed until reconciled with authoritative issuer state. Fresh keys may have
no cutoff only when the authoritative store confirms that absence.

Profile deployments MUST also persist accepted core key-wide deny entries and
restore them before serving, retaining their original expiry and maximum merge
rule. This deliberately strengthens core restart handling for participating
deployments. An active key-wide deny of the agent or owner always wins,
regardless of `iat` or cutoff; a withdrawal MUST NOT shorten or clear it.
Permanent retirement remains an issuer decision: the issuer MUST NOT issue
fresh assertions for a retired key. This profile defines no unretirement.

Cutoff storage MUST be bounded per issuer. At capacity, new entries MUST fail
`503`, without evicting enforcement state or consuming a replay identity.
There is one cutoff per `(iss, target_pubkey)`: repeated transfers of that key
update its entry by maximum; storage grows with distinct withdrawn keys, not
with transfer count. The following compaction permits bounded reclamation.

### Compaction and rollback gate

Each participating issuer MUST have one durable, non-decreasing agent assertion
floor `F`, initially zero. Every agent admission and live-session enforcement
MUST reject signed `iat < F`, independent of key, assertion-age policy or wall
clock. The floor applies to agents only, and MUST NOT override core key/owner
denies. It is enforcement state and has the same restore-before-serving and
deployment-completion requirements as per-key cutoffs.

For a cutoff `C`, define the conservative compaction horizon
`H = C + S + A`, using overflow-safe arithmetic. `A` is the greatest positive
finite `maximum_assertion_age` used by any serving process or applicable policy
version since this cutoff was first stored; `S` is the greatest configured skew
over that interval. An increase before compaction MUST recompute `H`, or MUST
be blocked until compaction is complete. A decrease MUST NOT shorten `H`.
The coordinator MUST establish that every serving process's trusted current
time is at least `H` and that no agent assertion below `C` retains live or
in-flight authority on any protected surface. Unknown clock bounds, legacy
sessions without expiry, or incomplete ingress coverage prevent compaction.
This time check protects availability for other keys whose old assertions
would be covered by the broader floor; time alone never permits deletion.

Reclamation MUST first durably raise `F := max(F, C)`, complete its enforcement
across the deployment, and only then delete a per-key entry whose current
value is at most `F`. Raising `F` and concurrent withdrawal MUST serialize so
a newly increased per-key cutoff cannot be lost. A crash between raising the
floor and deletion retains both, which is safe. Any affected live sessions
MUST be fenced before floor completion is acknowledged. If the issuer cannot
meet these conditions, it MUST retain the per-key entry or independently fence
the affected agents; capacity pressure is not permission to weaken denial.

After compaction, `iat < C` remains rejected by `F` even if clocks move
backward, assertion-age limits are later raised, or old commands are replayed.
`F` MUST NOT be reset or reclaimed on expiry, policy change, restart, or restore.
An issuer clock behind `F` MUST wait rather than mint a fabricated future `iat`.
The issuer MUST learn the authenticated current floor before fresh issuance;
it MUST NOT expose that floor in public discovery or errors.

A profile-unaware binary MUST NOT serve the issuer's agents unless an
independent enforcement layer retains the effective predicate
`iat >= max(F, C_key)` and all applicable key/owner denies on every protected
surface, and fences pre-existing sessions/work before serving. Otherwise agent
ingress MUST remain closed. Even `now >= max(C_key, F) + S + A` alone MUST NOT
authorize rollback: a later clock rollback or age-limit increase could revive
old assertions in a binary that lost the predicates. The rollback test is to
replay a withdrawn assertion after compaction and downgrade, then repeat after
winding the clock back and increasing age limits: each MUST deny or remain
fenced. If that test cannot be satisfied, rollback MUST retain profile-aware
enforcement or keep agent ingress closed.

### Issuer ordering and fresh authorization

An issuer using this profile MUST serialize source assertion finalization,
authorization withdrawal, and destination activation for the affected agent.
At each successful assertion finalization it MUST durably record the per-key
maximum finalized `iat` under that serialization. It MUST durably end source
issuance and choose a cutoff `T` strictly greater
than every `iat` finalized under the withdrawn authority, including assertions
issued ahead of wall time within permitted skew. Each later transfer MUST
choose `T` strictly greater than the preceding transfer's cutoff. The issuer
MUST retain these high-water marks across clock rollback, restart and failover.

The issuer MUST NOT fabricate a future `iat` to bypass withdrawal. It MUST
wait until its trusted wall clock reaches `T`, revalidate fresh destination
authority, and use an actual issuance time `iat >= T`. If clock correctness or
the high-water mark is uncertain, issuance MUST stop until reconciled. A new
grant or another approving session MUST NOT resume source issuance.

Before destination issuance, the issuer MUST obtain all required completion
acknowledgements and fence every stale authorization-loss check, queued retry,
temporary-deny producer and issuance result against its originating approval.
A check based on withdrawn authority MUST NOT enqueue or extend a key-wide
deny over fresh authority. Existing legitimate security denies MUST NOT be
discarded. Already queued/in-flight legacy key-wide transfer disconnects MUST
be reconciled; the scoped command does not neutralize them. If their possible
future effects cannot be excluded, the issuer MUST use the fallback and MUST
NOT promise uninterrupted destination access.

These requirements permit a pause for clock, delivery, and runtime barriers.
They do not promise zero interruption. Destination resource access, compute,
credentials, billing and retained data remain issuer/application concerns.

### Compatibility, discovery, and fallback

A supporting deployment MAY advertise
`"authorization_withdrawal":"agent-authorization-withdrawal-v1"` inside the existing NIP-11
`federated_identity` object. This conveys capability only; it is not an
acknowledgement, an issuer inventory, or evidence that every serving process
has completed a particular withdrawal. It MUST contain no private identifiers.
Profile selection MUST be operator-controlled and covered by policy identity.

The assertion format is unchanged, so an older relay can still accept an old
source assertion. A legacy peer rejects the new signed operation; this alone
does not withdraw source authority. On unsupported, rejected, timed-out, or
unprovable completion at any affected deployment, the issuer MUST stop
source/destination issuance for the transition, send existing key-wide core
disconnect commands to all affected deployments, and durably retry failed
delivery. It MUST NOT report completed withdrawal based on a rejection or
local `200` from the core disconnect endpoint.

As a fallback, destination execution MUST remain disconnected until all source
assertions and already-authenticated source session authority have demonstrably
expired or been fenced across the inventory, and all relevant key-wide deny
windows have elapsed. The issuer MUST establish that each serving version
actually terminates session authority at the applicable deadline; admission
checks alone are insufficient. Late legacy close delivery can still disrupt
fresh sessions and MUST be reconciled or disclosed as a fallback limitation.
The verified-drain fallback MUST NOT rely on off/shadow mode or legacy sessions
without enforced expiry. Such sessions and ingress MUST first be fenced or
demonstrably drained. Completion MUST cover every access surface in the claimed
scope, including bridge, media and git HTTP routes and long-lived work they
start; evidence for WebSocket/audio expiry alone is insufficient. Implementers
MUST validate each surface before relying on this fallback.
If these bounds, clock assumptions, or serving inventory cannot be established,
the agent MUST remain disconnected pending repair; a timeout is not approval.

Deployments enabling the profile MUST first close or drain pre-existing agent
sessions whose verified `iat` is unavailable. They MUST prevent admission via
older processes, shadow/off mode, or alternate ingress that omits enforcement.
A rolling upgrade or rollback MUST meet the compaction and rollback gate above,
including retention of `F` after per-key reclamation. The issuer MUST NOT
retry the new operation as core `disconnect` and then treat its response as
equivalent scoped completion. Core-only deployments retain their existing
bounded-expiry behavior and make no new durability promise.

### Behavioral oracles for the profile

| ID | Required outcome |
|---|---|
| `FI-TRACE-WITHDRAW-SCOPE` | At cutoff T, signed agent `iat < T` denies on HTTP and WebSocket and matching sessions close; equality and later assertions survive this operation; another issuer, human, or agent merely owned by the target is unaffected. |
| `FI-TRACE-WITHDRAW-ORDER` | Concurrent admission is caught by scan or cutoff check; delayed T1 after T2 retains max(T1,T2) and never closes assertions at/after that maximum; exact-command replay has no side effects; a semantic retry with fresh jti converges. |
| `FI-TRACE-WITHDRAW-DURABLE` | Restart, failover, policy reload, or rollback cannot restore withdrawn authority; unavailable state fails closed; active agent/owner key-wide denial wins and survives restart; per-issuer capacity failure does not evict state or consume jti. |
| `FI-TRACE-WITHDRAW-BARRIER` | A process without applied enforcement is fenced or prevents 200; registering/restarted processes cannot serve stale state; in-flight work cannot exercise old authority after completion; local pub/sub success is insufficient. |
| `FI-TRACE-WITHDRAW-ISSUANCE` | Source finalization racing withdrawal cannot issue past the fence; skewed source iat and repeated same-second transfers force a sufficient cutoff; clock rollback stops issuance; stale confirmed-loss checks cannot extend a key-wide deny over destination authority. |
| `FI-TRACE-WITHDRAW-COMPAT` | An older peer rejects the distinct operation without a key-wide side effect; issuer fallback sends core denies and withholds destination execution through the verified drain; missing capability, incomplete inventory, and partial completion never count as scoped success. |
| `FI-TRACE-WITHDRAW-COMPACTION` | Repeated transfers update one key entry; distinct keys consume distinct entries. At H, persist/apply F before deleting C. Replay below C after deletion, clock rollback and age-limit increase always denies; age/skew increases before compaction extend H. Concurrent withdrawal cannot lose an increased C; a crash before deletion retains safe state; profile-unaware rollback without independent enforcement leaves agent ingress closed. |

## Sources

- NIP-42 authentication: <https://github.com/nostr-protocol/nips/blob/6d2979b3f503a8539c983efbcdcf901bbcf9ed23/42.md>
- NIP-98 HTTP authorization: <https://github.com/nostr-protocol/nips/blob/ae0fd96907d0767f07fb54ca1de9f197c600cb27/98.md>
- JWT BCP: <https://www.rfc-editor.org/rfc/rfc8725>
- JWT access-token profile: <https://www.rfc-editor.org/rfc/rfc9068>
- Resource Indicators for OAuth 2.0: <https://www.rfc-editor.org/rfc/rfc8707>
- DPoP: <https://www.rfc-editor.org/rfc/rfc9449>
