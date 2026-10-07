NIP-FA
======

Federated Identity Adapter
--------------------------

`draft` `optional`

**Depends on**: NIP-FI (Federated Identity), NIP-98 (HTTP Auth), NIP-42 (Authentication of Clients to Relays)

## Abstract

This NIP defines a provider-neutral HTTP contract between a Nostr client and an enterprise identity adapter. The adapter authenticates a user through a browser login, holds an adapter session for that user, and issues short-lived [NIP-FI](NIP-FI.md) assertions that bind the user to a Nostr key the client proves it controls with [NIP-98](https://github.com/nostr-protocol/nips/blob/master/98.md).

The adapter MAY use Auth0, Okta, SAML, LDAP, or any other upstream identity system behind this boundary. Clients do not implement or depend on that upstream protocol. Discovery of which relays require enterprise identity, and of which adapter serves them, is outside this NIP.

## Terminology

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in BCP 14 [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) [RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when, and only when, they appear in all capitals, as shown here.

- **adapter**: The HTTP service implementing this NIP. `{adapter_base}` is its base URL.
- **adapter session**: The opaque `session_token` the adapter issues after browser login, and the fixed lifetime it describes.
- **handoff secret**: A high-entropy random value the client generates per login and reveals only during code exchange.

## Browser login

The client MUST generate a high-entropy handoff secret, start a loopback callback listener, and open:

```text
GET {adapter_base}/v1/login/start?return_to={callback_url}&handoff_challenge={base64url(sha256(handoff_secret))}&handoff_challenge_method=S256
```

The client's `callback_url` MUST be a loopback callback of the form `http://127.0.0.1:{ephemeral_port}/callback/{nonce}`.

The adapter authenticates the user however the operator chooses, binds the completed browser login to `handoff_challenge`, and redirects to the exact `return_to` loopback callback. Adapters SHOULD accept loopback callbacks on any port, as recommended by [RFC 8252 §7.3](https://www.rfc-editor.org/rfc/rfc8252#section-7.3), and SHOULD reject non-loopback callback hosts. Success redirects to:

```text
{callback_url}?code={single_use_code}
```

Failures MAY redirect to the same callback with `error` and optional `error_description` query parameters.

The code alone is not a credential. The adapter MUST accept it only when the exchange presents the matching handoff secret.

Browser navigation MAY follow the adapter's identity-provider redirects. The non-browser adapter calls below MUST NOT depend on redirect handling.

## Code exchange

The client MUST exchange the browser code with an HTTP client that does not follow redirects, and MUST treat any 3xx response as a failed exchange, so that `{code, handoff_secret}` is never replayed to a redirect target.

```http
POST /v1/login/exchange
Content-Type: application/json

{
  "code": "single-use-code-from-callback",
  "handoff_secret": "base64url-random-secret"
}
```

Success response:

```json
{
  "session_token": "opaque-adapter-session-token",
  "expires_at": "2026-09-23T21:00:00Z",
  "email": "employee@example.com",
  "profile_projection": {
    "username": "employee",
    "display_name": "Employee Name"
  }
}
```

`email` and `profile_projection` are OPTIONAL. See [Privacy](#privacy) for when a client may publish `profile_projection`.

The exchange `expires_at` and a later session-check `expires_at` MUST describe the same fixed adapter session. Clients MUST reject the login when the two values differ, so a code exchange cannot commit a different session than the one verified by `/v1/session`.

## Session check

The client checks or reuses an adapter session with an HTTP client that does not follow redirects, and MUST treat any 3xx response as a failed session check, so that Bearer session credentials are never replayed to a redirect target.

```http
GET /v1/session
Authorization: Bearer {session_token}
```

The success response uses the same shape as the exchange response, except `session_token` is omitted:

```json
{
  "expires_at": "2026-09-23T21:00:00Z",
  "email": "employee@example.com",
  "profile_projection": null
}
```

The adapter MUST return a non-2xx status when the token is invalid or expired. The client then MUST discard the adapter session, and only the adapter session.

## Relay assertion

Before connecting to a NIP-FI-protected relay, the client asks the adapter for a short-lived NIP-FI assertion naming the Nostr key it will connect with. The request carries two credentials: the adapter session, which proves who the user is, and a NIP-98 proof, which proves the client controls the key.

```http
POST /v1/identity/assertions
Authorization: Bearer {session_token}
Nostr-Authorization: Nostr {base64-NIP-98-event}
Content-Type: application/json

{
  "relay_url": "wss://community.example.com",
  "nostr_pubkey": "<64-char lowercase hex public key>"
}
```

- `relay_url` selects an adapter-configured relay; the adapter MUST NOT fetch it. Clients MUST send it in the canonical form `wss://host[:port]`, with the scheme in lowercase, the host as its lowercase ASCII (A-label) form with no trailing dot, the default port omitted, and no trailing slash, path, query, or fragment. Adapters MUST configure relays in this canonical form and MUST compare `relay_url` against it exactly. An unknown relay MUST be rejected with 403 `authorization_denied`.
- `nostr_pubkey` is the lowercase hex key the client will authenticate to the relay with. The NIP-98 event MUST be signed by this key, use kind `27235`, carry exactly one `u` tag equal to the absolute URL of this endpoint, a `method` tag of `POST`, and a `payload` tag equal to the lowercase hex SHA-256 of the exact request body bytes. Its `created_at` MUST be no more than 60 seconds old and MUST NOT be more than 5 seconds in the future.
- The request body is limited to 4096 bytes and MUST NOT be content-encoded. A content-encoded body, such as a gzipped body, MUST be rejected with 400 `invalid_request`. Unknown fields MUST be rejected with 400 `invalid_request`.
- Clients and adapters MUST NOT log the values of the `Authorization` or `Nostr-Authorization` headers.

Success response (`200`, `Cache-Control: no-store`):

```json
{
  "assertion": "<compact-JWS-assertion>",
  "nostr_pubkey": "<64-char lowercase hex public key>",
  "expires_at": 1790000300
}
```

`expires_at` is the assertion `exp` as Unix seconds. Adapters MUST issue assertions with `exp - iat <= 300` seconds and MUST NOT set `exp` later than the adapter session's expiry. Clients MAY refuse an assertion with more than 300 seconds remaining when it arrives. The 5-minute cap is a rule of this NIP, not a relay constant: the relay enforces the token `exp` and its own deployment-configured `maximum_assertion_age`.

The assertion itself follows [NIP-FI](NIP-FI.md): a dedicated assertion's protected `typ` MUST be exactly `nip-fi+jwt`, and its `aud` MUST exactly match the canonical host URI of the community the relay resolves from the connection's `Host`. The assertion's `nostr_pubkey` MUST be the key that signs the NIP-42 relay login; the relay rejects any other key. Clients MUST reject a response whose `nostr_pubkey` differs from the key sent in the request.

A client that rejects a `200` response, including one it cannot parse, MUST treat it as a refusal: keep the session, show it as refused, and not retry automatically.

### Denials

Denials return a JSON body `{"error": "<code>"}` with `Cache-Control: no-store`. Clients MUST take the client action listed for each code:

| Status | `error` | Meaning | Client action |
|---|---|---|---|
| 400 | `invalid_request` | Malformed body, unknown fields, a missing or repeated `Nostr-Authorization` header, a repeated `Authorization` header, or more than one kind of session credential | Keep the session, show the error, do not retry automatically |
| 401 | `session_required` | No adapter session was presented, or the presented one is not usable | Clear the adapter session and return to browser login |
| 401 | `session_expired` | The adapter session ended | Clear the adapter session and return to browser login |
| 403 | `authorization_denied` | The user or relay is not authorized for assertions | Keep the session, disconnect from that relay, show access denied, do not retry automatically; a manual retry or app restart asks again |
| 403 | `invalid_proof` | The NIP-98 proof failed verification | Keep the session, show the error, do not retry automatically |
| 403 | `binding_mismatch` | The adapter has bound this user to a different Nostr key | Keep the session, show the error, do not retry automatically |
| 413 | `request_too_large` | The body exceeds 4096 bytes | Keep the session, show the error, do not retry automatically |
| 429 | `rate_limited` | Too many requests | Keep the session, retry with bounded backoff |
| 503 | `issuance_unavailable` | The adapter could not issue an assertion | Keep the session, retry with bounded backoff |

Clients MUST retry 429 and 503 with bounded backoff whatever the `error` code. Clients MUST treat any other status, or a 400/401/403/413 with a code this NIP does not define, as a refusal: keep the session, show it as refused, and not retry automatically.

A missing session MUST be reported as 401 `session_required`, never 400. Clients MUST handle network failures like 429 and 503: keep the session and retry with bounded backoff.

An adapter MAY return `session_required` for an invalid, expired, or revoked credential; adapters that distinguish ended sessions MAY return `session_expired`. Clients MUST handle both identically.

## NIP-FI assertion transport

The adapter session header above is only for client-to-adapter account and session requests. It is not the relay's NIP-FI proof transport.

When a client later accesses a NIP-FI-protected relay or HTTP route, the relay proof uses the headers defined by [NIP-FI](NIP-FI.md):

```http
Authorization: Nostr <base64-NIP-98-event>
Nostr-Federated-Identity: Bearer <compact-JWS-assertion>
```

`Authorization: Bearer <session_token>` belongs to the adapter, while the relay proof keeps the NIP-98 event in `Authorization` and carries the identity assertion in `Nostr-Federated-Identity`. Implementations MUST NOT use `Authorization` for both an adapter session and a NIP-FI assertion on the same request, and MUST NOT use the vendor-specific `X-BB-Session-Credential` header or `Authorization: BBIdentity` scheme.

## Privacy

NIP-FI defines no public identity projection. Adapter responses are private login state by default. Clients MUST NOT publish `profile_projection` fields to a public Nostr profile unless the operator has explicitly opted into managed public profiles. Operators that want managed public profiles MUST make that an explicit policy choice and provide explicitly publishable `profile_projection` fields. Raw upstream claims, legal names, emails, issuer identifiers, subjects, and assertion contents MUST NOT be written to public Nostr events implicitly.

## Out of scope

- Assertions for agent keys. This NIP covers assertions for keys a user authenticates with directly; issuing assertions for agent keys requires a separate design.
- Discovery of which relays require enterprise identity and which adapter serves them.
- The upstream identity protocol between the adapter and its identity provider.
