NIP-FI
======

Federated Identity Authorization
--------------------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-42 (Authentication of Clients to Relays). **Composes with**: NIP-98 (HTTP Auth), NIP-11 (Relay Information Document), NIP-OA (Owner Attestation).

## Abstract

This NIP defines how a relay or Nostr-adjacent HTTP service authorizes an already-authenticated Nostr key only when a valid federated identity assertion resolves to the same principal and key. It specifies assertion transport, validation, an identity-to-key binding lifecycle (enroll, conflict, revoke, rotate), session semantics, and failure behavior. A separately validated delegation MAY derive narrower authority from a bound owner as described below; that exception does not turn the delegate into the federated principal.

The identity provider never becomes a Nostr signing authority, and the assertion never substitutes for Nostr proof of key control. This NIP is an authorization layer above NIP-42 and NIP-98, not a replacement for either.

## Motivation

Organizations deploying Nostr internally need relay access tied to their workforce identity system: an employee's relay privileges should follow their corporate identity, survive Nostr key rotation, and end at offboarding. Existing primitives each solve part of this:

- NIP-42 proves control of a Nostr key to a connection but carries no external identity.
- NIP-05 maps an organization-controlled identifier to a pubkey, but by public DNS/HTTPS polling, not by a credential presented on the request being authorized.
- NIP-46 lets a signer demand out-of-band authentication (`auth_url`) but does not bind the resulting external subject to a key at the relay.

Without a standard, each deployment invents an incompatible binding scheme, and the first large deployment's configuration becomes an accidental protocol. This NIP defines the contract so that any relay behind any OIDC-capable identity provider or generic OAuth2 reverse proxy (Okta, Auth0, Keycloak, oauth2-proxy, etc.) can interoperate with any conforming client.

## Definitions

- **assertion**: a JWT issued by a configured identity provider, presented alongside (never instead of) Nostr authentication.
- **federated identity** (`i`): the tuple `(iss, sub)` from a validated assertion. The `iss` value MUST be the exact validated issuer identifier and `sub` the exact non-empty subject string. A username, email, display name, or bare `sub` MUST NOT be used as a federated identity.
- **authorization domain** (`D`): the scope within which bindings apply, chosen by the service (an entire relay, or one tenant of a multi-tenant relay). Bindings MUST NOT cross domains implicitly.
- **binding**: an active record associating exactly one federated identity with exactly one 32-byte Nostr public key within a domain.
- **enrollment mode**: the domain's policy for creating bindings — `attested-key`, `provisioned`, or `tofu` (defined below).
- **Nostr proof**: a valid NIP-42 AUTH event (WebSocket) or NIP-98 event (HTTP) proving control of a key on the current connection or request.
- **lease**: a cached authorization decision for one `(domain, identity, key)`, bounded by the assertion's expiry.

## Assertion transport

An assertion reaches the verifier in an HTTP header on the request being authorized: the WebSocket upgrade request for relay connections, or each individual request for NIP-98-authenticated HTTP endpoints. Two transport profiles are defined; a service MUST document which it accepts.

1. **Trusted proxy**: an authenticating reverse proxy (for example oauth2-proxy or an SSO-aware ingress) injects the assertion after authenticating the user. The injected header name is deployment configuration. This profile is conforming only if untrusted clients cannot reach the verifier directly and the proxy strips every inbound copy of that header before setting it. This is the recommended profile for browser-based clients, which cannot attach arbitrary WebSocket upgrade headers.
2. **Client-attached**: the client sends the assertion itself in `Nostr-Federated-Identity: Bearer <JWT>`. A verifier MAY additionally accept another documented header on WebSocket upgrades, including `Authorization: Bearer`; HTTP requests using NIP-98 MUST use `Nostr-Federated-Identity` because their `Authorization` header carries the `Nostr` proof.

Assertion acquisition and interactive OIDC login are outside this NIP. A client-attached assertion value MUST use the `Bearer` scheme; after removing that scheme, the value MUST contain exactly one JWT and no comma-separated alternatives.

On a WebSocket connection, the assertion captured at upgrade is evaluated when a key performs NIP-42 AUTH — each authenticating key is authorized against that assertion independently. On HTTP, the assertion and the NIP-98 proof MUST arrive on the same request they authorize.

Assertions MUST NOT be carried inside Nostr events, event tags, or subscription filters, and MUST NOT be written to relay-visible event history.

## Assertion validation

The verifier is configured, per accepted issuer, with: the issuer identifier, a signing-key source (a JWKS endpoint, discoverable via OIDC `/.well-known/openid-configuration`), accepted audience values, and a claim mapping. Validation MUST enforce all of the following; any failure MUST reject the assertion:

1. The JWT signature verifies under a currently trusted key for an explicitly allowed **asymmetric** algorithm. Symmetric (HS*) and `none` algorithms MUST be rejected before any key lookup.
2. `iss` exactly equals the configured issuer identifier used to select the verification key.
3. At least one `aud` value exactly equals a configured audience.
4. `exp` is present and in the future; `nbf` and `iat`, when present, are not in the future — each within a bounded, configured clock skew.
5. The configured subject claim is present and a non-empty string. A configured claim that is absent when required, not of its expected type, or not unambiguously a single value MUST be rejected.
6. If a key claim is configured and present, it parses to exactly one 32-byte Nostr public key. Lowercase hex is the canonical encoding; `npub` bech32 MAY be accepted as a documented input normalization.

A display-name claim MAY be extracted as mutable metadata. It MUST NOT participate in any authorization decision.

Signing-key retrieval failures MUST fail closed. Verifiers SHOULD cache the key set with a bounded lifetime and SHOULD NOT refetch it in response to an unknown `kid` that was absent from a freshly fetched set, so that forged tokens cannot drive request floods to the identity provider.

## Nostr proof

The key being authorized is always the key returned by Nostr proof validation — a valid NIP-42 AUTH for the current WebSocket connection, or a valid NIP-98 event for the current HTTP request. It is never taken from an assertion claim, an unsigned request field, or client metadata. A bearer assertion alone MUST NOT authenticate a Nostr key.

## Authorization

Given a validated assertion yielding identity `i`, optional asserted key `k_a`, and expiry `exp`, and a Nostr proof yielding key `k`, the verifier evaluates one atomic decision in domain `D`:

```text
Authorize(D, i, k_a?, k):
  if k_a exists and k_a != k:            DENY (key mismatch)

  b_i := active binding for i in D, if any
  b_k := active binding for k in D, if any

  if b_i = (i, k) and b_k = (i, k):      ALLOW (existing binding)
  if b_i exists or b_k exists:           DENY (binding conflict)

  # no active binding on either side: enrollment
  attested-key:  k_a required, else DENY; create (i, k); ALLOW
  provisioned:   DENY (binding must be pre-created by an operator)
  tofu:          create (i, k); ALLOW
```

The check and any insertion MUST be atomic for `(D, i, k)`: under concurrent first use of the same identity or key, at most one binding is created and every other attempt observes it (allow on exact match, deny on conflict). Storage failure or a race whose committed result cannot be read MUST deny — never fall back to an unchecked allow.

### Enrollment modes

- **`attested-key`**: the identity provider carries the user's Nostr public key in the configured key claim. First use binds only when the asserted key equals the proven key. This is the strongest mode and SHOULD be used when the identity provider can carry custom claims.
- **`provisioned`**: bindings are created only through an out-of-band administrative process; requests never create bindings.
- **`tofu`** (trust on first use): first use of an unbound identity with an unbound key creates the binding. A stolen assertion for a never-enrolled identity can bind an attacker's key in this mode; services offering it MUST document this risk. When an assertion in `tofu` mode carries a valid key claim, the binding SHOULD record the stronger `attested-key` provenance, and a binding's recorded provenance MUST NOT be downgraded by later requests.

### Binding invariant

Within a domain, active bindings form a partial bijection: an identity has at most one active key and a key has at most one active identity. Every state transition in this NIP preserves this invariant.

## Session semantics

For HTTP requests, the decision applies to that request only.

For a NIP-42 WebSocket connection, the relay MAY cache the decision as a lease. A lease MUST NOT be honored past the assertion's `exp` (implementations MAY enforce a shorter maximum). At expiry the relay MUST reject protected operations or close the connection; a fresh assertion arrives only on a new connection's upgrade request. When a relay learns a binding was revoked, it MUST invalidate matching leases; a relay that detects revocation by polling MUST NOT claim immediate revocation and SHOULD document its detection latency.

When multiple keys authenticate on one connection (NIP-42 permits this), authorization is tracked per key. A lease for one key MUST NOT authorize operations attributed to another.

## Revocation and rotation

Revocation is an explicit administrative or policy transition: the binding is removed from the active set and a durable revocation record is retained. A subsequent valid assertion — including one whose key claim matches the revoked key — MUST NOT reactivate a revoked binding unless the domain's documented recovery policy explicitly authorizes that transition. This prevents a replayed, still-valid assertion from silently undoing revocation.

Key rotation is likewise explicit, never a side effect of authorization: rotating `i` from `k_old` to `k_new` requires administrative or documented recovery authorization, an active `(i, k_old)` binding, no active binding for `k_new`, and — where the domain requires issuer attestation — a fresh assertion whose key claim equals `k_new`. The old binding is revoked and the new one created atomically, and leases for `k_old` are invalidated. A routine request presenting `i` with a new key while `(i, k_old)` is active is a binding conflict and MUST be denied.

## Delegation

Delegation is outside the base primitive but composes with it. A service MAY admit a key that presents no assertion when a separately validated delegation proof (for example a NIP-OA `auth` tag) establishes an owner key that holds an active binding in the domain. The delegate key MUST NOT acquire a federated identity binding of its own through this path, and the delegate's authorization is bounded by both the owner's binding state and the delegation's own conditions. Revoking the owner's binding revokes the delegate's admission on the same schedule as the owner's own leases.

## Rejection semantics

Machine-readable rejections reuse NIP-01/NIP-42 prefixes on `OK` and `CLOSED` messages:

- `auth-required: ` — no assertion was presented, or no NIP-42 proof has been performed.
- `restricted: ` — the assertion or proof was presented but failed validation, mismatched, conflicted with an active binding, or the identity's enrollment/binding state does not permit the operation.

HTTP endpoints respond `401` where `auth-required` applies and `403` where `restricted` applies. Rejection bodies MUST NOT echo assertion contents, claim values, or the conflicting party's identity or key.

## Discovery

A relay SHOULD advertise support in its NIP-11 document under `limitation` as `"federated_identity": true`. It MAY additionally include this top-level object:

```json
{
  "federated_identity": {
    "transports": ["trusted-proxy", "client-attached"],
    "enrollment": "attested-key",
    "delegation": false
  }
}
```

`transports` contains the supported profile names from this NIP, `enrollment` is exactly one enrollment mode, and `delegation` states whether separately validated delegation may be honored. Unknown fields MUST be ignored. A relay MUST NOT publish issuer-internal detail (tenant URLs, claim names, audiences) that is not already public.

## Privacy

Federated identities are typically personal data (employee identifiers). A conforming service MUST NOT publish `iss`, `sub`, assertion contents, or display-name claims in Nostr events or tags, and MUST NOT expose another user's binding state through rejection messages. Binding records, audit logs, and metrics are service-internal, and logs MUST NOT record raw bearer assertions.

## Security considerations

- **Issuer or proxy compromise** impersonates federated principals, but cannot satisfy Nostr proof for an already-bound uncompromised key, and in `attested-key` mode cannot bind an arbitrary key without also forging the key claim.
- **Assertion theft** cannot authorize an already-bound identity without control of the bound key. Its remaining power — enrolling a never-bound identity — exists only in `tofu` mode, which is why that mode is risk-labeled.
- **Header injection**: the trusted-proxy profile is void if clients can reach the verifier directly or the proxy forwards inbound copies of the assertion header. Deployments MUST verify both properties.
- **Algorithm confusion** is excluded by rejecting symmetric algorithms before key selection.
- **Availability vs. safety**: issuer, key-set, and storage outages deny. Availability MUST NOT override identity safety.
- **Cross-issuer collision**: identical `sub` values under different issuers are distinct identities and MUST never collide or inherit each other's bindings.

A companion formal model of this protocol — state machine, safety and liveness properties, and attack traces — accompanies this specification.

## Reference implementation

Buzz relay: corporate identity enforcement layered above NIP-42/NIP-98/media/git/audio ingress, with JWKS validation, TOFU and attested-key enrollment, atomic binding with conflict detection, and NIP-OA delegation composition.
