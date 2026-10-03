# Enterprise identity adapter contract

Buzz Desktop learns that a relay requires enterprise identity from the relay's
NIP-11 `federated_identity` advertisement. Discovery deliberately contains no
issuer URL, tenant ID, audience, or vendor-specific login details; those stay in
operator-controlled Desktop build configuration.

When a trusted build connects to a trusted enterprise relay, Desktop speaks the
provider-neutral HTTP contract below to the configured enterprise identity
adapter. The adapter may use Auth0, Okta, SAML, LDAP, or another upstream
identity system behind this boundary. Desktop does not implement or depend on
that upstream protocol.

## Build configuration

Enterprise builds set:

- `BUZZ_BUILD_ENTERPRISE_AUTH_RELAYS`: comma-separated allowlist of relay URLs
  for which Desktop will honor NIP-FI enterprise login.
- `BUZZ_BUILD_ENTERPRISE_AUTH_ADAPTER_BASE_URL`: base URL of the adapter
  implementing this document. Remote adapter URLs must use HTTPS. Plaintext
  HTTP is accepted only for loopback development hosts (`localhost`,
  `127.0.0.0/8`, or `::1`). Required when the relay allowlist is set.
- `BUZZ_BUILD_ENTERPRISE_PROFILE_PROJECTION`: optional `1`/`true` opt-in. When
  unset, adapter identity fields are private login state only and are never
  projected into a public Nostr profile.

Hosted-community Builderlab configuration is separate:

- `BUZZ_BUILD_BUILDERLAB_API_BASE_URL` continues to configure only Builderlab
  hosted-community management. It is not used for enterprise login.

## Browser login

Desktop starts a localhost callback and generates a high-entropy handoff secret.
It opens:

```text
GET {adapter_base}/v1/login/start?return_to={callback_url}&handoff_challenge={base64url(sha256(handoff_secret))}&handoff_challenge_method=S256
```

The adapter authenticates the user however the operator chooses, binds the
completed browser login to `handoff_challenge`, and redirects to the exact
`return_to` loopback callback. Desktop always supplies
`http://127.0.0.1:{ephemeral_port}/callback/{nonce}`; adapters should accept
loopback callbacks on any port as recommended by RFC 8252 §7.3 and reject
non-loopback callback hosts. Success redirects to:

```text
{callback_url}?code={single_use_code}
```

Failures may redirect to the same callback with `error` and optional
`error_description` query parameters. Desktop surfaces `error_description` when
present, otherwise `error`.

The code alone is not a credential. The adapter MUST accept it only when the
exchange presents the matching handoff secret. Browser navigation may follow the
adapter's identity-provider redirects; the non-browser adapter calls below must
not depend on redirect handling.

## Code exchange

Desktop exchanges the browser code with a no-redirect HTTP client. Any 3xx is
treated as a failed exchange so `{code, handoff_secret}` is never replayed to a
redirect target.

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

`email` and `profile_projection` are optional. `profile_projection` is ignored by
Desktop unless `BUZZ_BUILD_ENTERPRISE_PROFILE_PROJECTION` opts into publishing
those fields as the user's public Buzz profile. The exchange `expires_at` and a
later session-check `expires_at` must describe the same fixed adapter session;
Desktop rejects mismatched values during login so a code exchange cannot commit a
different session than the one verified by `/v1/session`.

## Session check

Desktop checks or reuses an in-memory enterprise adapter session with the same
no-redirect HTTP client. Any 3xx is treated as a failed session check so Bearer
session credentials are never replayed to a redirect target.

```http
GET /v1/session
Authorization: Bearer {session_token}
```

Success response uses the same shape as the exchange response except
`session_token` is omitted:

```json
{
  "expires_at": "2026-09-23T21:00:00Z",
  "email": "employee@example.com",
  "profile_projection": null
}
```

The adapter returns non-2xx when the token is invalid or expired. Desktop then
clears only the enterprise adapter session. Builderlab hosted-community state is
not affected.

## Relay assertion

Before connecting to a NIP-FI-protected relay, the client asks the adapter for a
short-lived NIP-FI assertion naming the Nostr key it will connect with. The
request carries two credentials: the adapter session, which proves who the user
is, and a NIP-98 proof, which proves the client controls the key.

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

- `relay_url` selects an adapter-configured relay; the adapter never fetches it.
  An unknown relay is denied.
- `nostr_pubkey` is the lowercase hex key the client will authenticate to the
  relay with. The NIP-98 event MUST be signed by this key, use kind `27235`,
  carry exactly one `u` tag equal to the absolute URL of this endpoint, a
  `method` tag of `POST`, and a `payload` tag equal to the lowercase hex SHA-256
  of the exact request body bytes. Its `created_at` MUST be current; adapters
  should allow no more than 60 seconds of age.
- The request body is limited to 4096 bytes and MUST NOT be content-encoded.
  Unknown fields are rejected.
- Clients and adapters MUST NOT log the values of the `Authorization` or
  `Nostr-Authorization` headers.

Success response (`200`, `Cache-Control: no-store`):

```json
{
  "assertion": "<compact-JWS-assertion>",
  "nostr_pubkey": "<64-char lowercase hex public key>",
  "expires_at": 1790000300
}
```

`expires_at` is the assertion `exp` as Unix seconds. Under this adapter contract
an assertion lives at most 5 minutes and never outlives the adapter session;
clients request a fresh one before it expires. The 5-minute cap is this
contract's rule, not a relay constant: the relay enforces the token `exp` and
its own deployment-configured `maximum_assertion_age`. The assertion itself
follows [NIP-FI](nips/NIP-FI.md): a dedicated assertion's protected `typ` is
`nip-fi+jwt`, and its `aud` MUST exactly match the canonical host URI of the
community the relay resolves from the connection's `Host`. The assertion's
`nostr_pubkey` MUST be the key that signs the NIP-42 relay login; the relay
rejects any other key.

Denials return a JSON body `{"error": "<code>"}` with `Cache-Control: no-store`:

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

A missing session is always 401 `session_required`, never 400. Network failures
are handled like 429 and 503: keep the session and retry with bounded backoff.

An invalid, expired or revoked credential may return `session_required`;
adapters that distinguish ended sessions may return `session_expired`. Clients
handle both identically.

Assertions for agent keys are not covered by this contract yet and need a
separate design.

## NIP-FI assertion transport

The adapter session header above is only for Desktop↔adapter account/session
requests. It is not the relay's NIP-FI proof transport.

When Desktop (or another Buzz client) later accesses a NIP-FI-protected relay or
HTTP route, the relay proof uses the NIP-FI headers defined by
[docs/nips/NIP-FI.md](nips/NIP-FI.md):

```http
Authorization: Nostr <base64-NIP-98-event>
Nostr-Federated-Identity: Bearer <compact-JWS-assertion>
```

That separation is intentional: `Authorization: Bearer <session_token>` belongs
to the enterprise identity adapter, while relay proof keeps the NIP-98 event in
`Authorization` and carries the identity assertion in `Nostr-Federated-Identity`.
The public contract MUST NOT use `Authorization` for both an adapter session and
a NIP-FI assertion on the same request, and MUST NOT use Builderlab-specific
`X-BB-Session-Credential` or `Authorization: BBIdentity` schemes.

## Privacy

NIP-FI defines no public identity projection. Adapter responses are private login
state by default. Operators that want managed public profiles must make that a
build-time policy choice and provide explicitly publishable `profile_projection`
fields; raw upstream claims, legal names, emails, issuer identifiers, subjects,
and assertion contents must not be written to public Nostr events implicitly.
