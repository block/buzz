NIP-FI
======

Federated identity authorization — stateless core
---------------------------------------------------

`draft` `optional` `relay`

**Protocol dependencies**: NIP-01, NIP-42.

The key words "MUST", "MUST NOT", "REQUIRED", "SHOULD", "SHOULD NOT", and
"MAY" in this document are to be interpreted as described in BCP 14 (RFC 2119
and RFC 8174) when, and only when, they appear in all capitals.

## Abstract

NIP-FI authorizes a Nostr key when two independent facts agree: a valid
issuer-qualified identity assertion that names the key, and fresh NIP-42 proof
of possession of that key.  No relay-side identity state is required.  The
relay verifies the assertion offline against configured per-issuer JWKS
snapshots; every identity decision beyond key verification is the deployment
adapter's responsibility.

This NIP defines the assertion contract, the offline verification procedure,
session lifetime policy, and an authenticated adapter→relay disconnect API.
Enrollment, rotation, revocation decisions, identity↔key registry, one-identity
one-key enforcement, audit, and SCIM are adapter concerns outside this spec.

## Terms

- **identity** (`i`): the exact tuple `(iss, sub)` returned by assertion
  validation.  Email, display name, employee number, and a bare `sub` are not
  identities.  Equal `sub` values under different `iss` values are distinct
  identities.  [FI-TRACE-CROSS-DOMAIN-COLLISION]
- **actor** (`k`): the 32-byte public key returned by NIP-42 proof validation.
- **assertion**: a compact JWS minted by the deployment adapter, binding `i`
  to `k`.
- **adapter**: the deployment-specific identity authority (e.g. an Okta OIDC
  integration) that authenticates employees and mints assertions.  The relay
  trusts only the adapter's assertion; it does not contact the IdP directly.

## Assertion contract

The assertion is a compact JWS carrying the following claims.

### Required claims

| Claim | Type | Semantics |
|---|---|---|
| `iss` | string | Exact issuer URI.  The relay selects an issuer policy by exact match; no normalization is applied. |
| `sub` | string | Opaque, stable, non-reassignable subject identifier for the account lifetime.  Never an email address or display name. |
| `nostr_pubkey` | string | Lowercase hexadecimal encoding of exactly one 32-byte Nostr public key.  Other encodings deny. |
| `iat` | NumericDate | Issuance time. |
| `exp` | NumericDate | Expiry time.  MUST be finite.  The deployment MUST configure a positive finite maximum TTL; the relay enforces both the token `exp` and the configured `maximum_assertion_age`. |

### Optional claims

| Claim | Type | Semantics |
|---|---|---|
| `aud` | string or array | Required when configured; the relay requires an exact match to the configured audience value. |
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
ledger; the assertion is the binding claim, and it is the adapter's
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

The client sends exactly one field on the WebSocket upgrade request:

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
here.  The `require_attested_key` flag in `IssuerPolicy` is the per-issuer
enforcement primitive for the unconditional `nostr_pubkey` requirement in this
section; conformance to NIP-FI v2 requires startup validation that forces this
flag true for every configured issuer.  That integration is a follow-on code
change outside this PR.

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
5. Admit the connection.  The session's authority deadline is the minimum of all
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
re-mint riding an existing adapter/IdP session is an adapter implementation
detail; the relay never sees anything other than a new upgrade request.

### Reconnect after expiry

A client whose session expired due to normal TTL expiry may reconnect
immediately provided the adapter can issue a fresh assertion.  Session expiry
does not imply key revocation or identity loss; that is the adapter's domain.

## Admin disconnect API

The adapter can terminate live relay sessions for a specific public key via an
authenticated `disconnect` call.

### Semantics (session-only)

A disconnect call causes the relay to close all live WebSocket connections
whose proven `k` equals the target pubkey.  This is a **session-only**
operation: it closes existing connections but does not prevent the key from
reconnecting.  After disconnection, a client holding a still-valid JWT can
reconnect immediately.

> **Non-normative note — open product question for Will/Tyler:**
>
> The session-only model means a revoked employee retains access until the
> relay closes their connection and they can no longer obtain a fresh
> assertion.  There are two distinct residual sub-windows:
>
> - **Existing session:** a live session that received a disconnect call
>   continues until the relay closes it (bounded by whatever session deadline
>   applies — at most `max_connection_lifetime_seconds` from connection time).
> - **Immediate reconnect:** after being disconnected, a client holding a
>   still-valid assertion can reconnect immediately.  That new session is
>   bounded by `min(remaining assertion TTL, max_connection_lifetime_seconds)`.
>
> These windows are sequential, not additive.  The worst-case window after a
> disconnect call is `max_connection_lifetime_seconds` (for the live session)
> followed immediately by a new session bounded by `min(remaining assertion TTL,
> max_connection_lifetime_seconds)`.  The reconnect window closes when the
> adapter stops issuing new assertions.
>
> The alternative is a **deny-until-TTL** model: the relay holds a
> memory-resident deny-list entry for the pubkey keyed to the adapter's stated
> TTL, and any reconnect attempt for that key is denied `authorization_denied`
> until the entry expires.  This closes the reconnect window at the cost of
> relay in-memory state and a TTL-propagation contract between adapter and relay.
>
> This document intentionally leaves that decision unresolved.  The current
> normative text describes session-only.  If deny-until-TTL is chosen, Section 6
> must be revised to add: the TTL parameter on the disconnect call, the
> deny-list data structure (keyed by pubkey, value = absolute expiry), the
> deny-list check at admission (step 4), and the expiry/eviction rule.

### Transport

The disconnect endpoint is an authenticated adapter→relay API, not a public
Nostr protocol.

### Request binding

Authentication MUST use a short-lived signed command JWT.  The adapter mints
a compact JWS verified by the relay against the same configured per-issuer JWKS
snapshot it uses for client assertions — no additional key material or Nostr
key is required.

The command JWT MUST carry the following claims:

| Claim | Requirement |
|---|---|
| `iss` | Exact issuer URI matching an authorized adapter issuer in the registry. |
| `sub` | Adapter principal identifier.  The relay checks this is an authorized adapter principal. |
| `aud` | Audience matching the relay's configured audience value for this issuer. |
| `iat` | Issuance time.  MUST satisfy `now < iat + command_ttl` and `iat <= now + skew`. |
| `exp` | Expiry time.  MUST be finite; relay enforces `now < exp`. |
| `jti` | Unique, non-guessable identifier for this command.  The relay rejects any command whose `jti` has already been seen within its expiry window (replay denial). |
| `cmd` | Exactly `"disconnect"` (literal string). |
| `target_pubkey` | Lowercase hexadecimal encoding of the target 32-byte Nostr public key — the same encoding required for the assertion `nostr_pubkey` claim. |

The relay verifies the command JWT using `VerifyAssertion` (selecting the
adapter issuer policy), then asserts:

1. `cmd == "disconnect"` — any other value denies.
2. `target_pubkey` matches the pubkey in the request body — mismatch denies `403`.
3. `jti` has not been seen within its expiry window — replay denies `403`.
4. Caller identity is an authorized adapter principal — unauthorized caller denies `403`.

Any failure is fail-closed: the relay takes no action and returns the
appropriate error.  The command TTL MUST be short (deployment policy governs;
60 seconds is a reasonable upper bound).

### Request

```text
POST /api/nip-fi/disconnect HTTP/1.1
Nostr-Federated-Identity: Bearer <compact-command-JWS>
Content-Type: application/json

{"pubkey": "<lowercase-hex-32-byte-pubkey>"}
```

The relay verifies the command JWT, confirms `target_pubkey` in the JWT matches
the body `pubkey` field, confirms the caller is an authorized adapter principal,
then closes all matching live connections.  An unknown or unprovable pubkey is
not an error; the relay responds `200` with `{"disconnected": 0}`.

### Response

| Condition | Status | Body |
|---|---|---|
| Authorized; action taken or no-op | `200` | `{"disconnected": <n>}` where `n` is the count of sessions closed |
| Missing or invalid command JWT | `401` / `403` | Per the rejection table |
| Malformed request body | `400` | `bad request\n` |

## Rejection and privacy

Public class is a function only of evidence the requester supplied, never of
private per-principal server state; `authorization_unavailable` is the sole
exception and reveals only that a required dependency is unreadable.

| Private condition | Public class | Nostr text | HTTP response |
|---|---|---|---|
| assertion or proof absent | `missing_evidence` | `auth-required: authentication required` | `401`; `WWW-Authenticate: Nostr`; `Content-Type: text/plain; charset=utf-8`; body `authentication required\n` |
| malformed, invalid, or expired evidence | `evidence_rejected` | `restricted: evidence rejected` | `403`; `Content-Type: text/plain; charset=utf-8`; body `evidence rejected\n` |
| assertion–key mismatch; local policy denial; adapter-initiated disconnect (session-only model) | `authorization_denied` | `restricted: authorization denied` | `403`; `Content-Type: text/plain; charset=utf-8`; body `authorization denied\n` |
| required JWKS snapshot unreadable | `authorization_unavailable` | `restricted: authorization unavailable` | `503`; `Content-Type: text/plain; charset=utf-8`; body `authorization unavailable\n` |

A denial decided on a WebSocket upgrade is the HTTP response in place of `101`.
A denial decided after the connection is established is the Nostr text.
Responses contain no free text, reason code, issuer, subject, key, claim, or
timing hint.  [FI-TRACE-DENIAL-ORACLE]

NIP-FI defines no public identity projection.  Raw assertions, `iss`, `sub`,
email, display name, and private claims MUST NOT appear in public events, tags,
filters, discovery, logs, metrics, or traces.  [FI-TRACE-PRIVACY-NONPUBLIC]

## Out of scope

The following are adapter and deployment concerns.  This spec defines no
normative behavior for them:

- Identity↔key registry, key ownership records, and the one-identity one-key
  constraint: adapter-side.
- Key rotation, re-enrollment after device loss: adapter-side.
- Revocation signaling to the adapter/IdP: adapter-side; the adapter stops
  issuing assertions, which closes the relay window within assertion TTL.
- SCIM, HR system integration, employee offboarding automation: adapter-side.
- Audit logging beyond what the relay operator chooses to retain: adapter-side.
- Delegation: out of scope.
- Companion profiles (NIP-FI-EDGE, NIP-FI-LIFECYCLE, NIP-FI-DELEG, NIP-FI-CONF): removed.

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
deployment adapter MUST configure a `max_connection_lifetime_seconds` and
assertion TTL consistent with the organization's acceptable revocation latency.

For upstream revocation without an explicit disconnect call (adapter stops
issuing assertions; no active session termination), the residual window is
`max(existing session remaining lifetime, assertion TTL)`: whichever is
longer governs when access finally ceases.

For the session-only disconnect model (adapter issues a disconnect call that
closes the live session), the residual window has two sequential parts: the
live session closes within `max_connection_lifetime_seconds`, after which a
reconnect is bounded by `min(remaining assertion TTL, max_connection_lifetime_seconds)`.
The reconnect window closes when the adapter stops issuing new assertions.
See the non-normative note in the Admin disconnect section for the open product
question on the deny-until-TTL alternative.

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
- JWT BCP: <https://www.rfc-editor.org/rfc/rfc8725>
- JWT access-token profile: <https://www.rfc-editor.org/rfc/rfc9068>
- DPoP: <https://www.rfc-editor.org/rfc/rfc9449>
