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
completed browser login to `handoff_challenge`, and redirects to:

```text
{callback_url}?code={single_use_code}
```

The code alone is not a credential. The adapter MUST accept it only when the
exchange presents the matching handoff secret.

## Code exchange

Desktop exchanges the browser code with:

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
those fields as the user's public Buzz profile.

## Session check

Desktop checks or reuses an in-memory enterprise adapter session with:

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
