NIP-FA
======

Federated Identity Adapter
--------------------------

`draft` `optional`

**Depends on**: NIP-FI (Federated Identity), NIP-98 (HTTP Auth), NIP-42 (Authentication of Clients to Relays)

## Abstract

This NIP defines a provider-neutral HTTP contract between a Nostr client and an enterprise identity adapter. The adapter authenticates a user through a browser login, holds an adapter session for that user, and issues short-lived [NIP-FI](NIP-FI.md) assertions that bind the user to a Nostr key the client proves it controls with [NIP-98](https://github.com/nostr-protocol/nips/blob/master/98.md).

It also defines how a client obtains and uses an owner-authorized grant for an agent key.

The adapter MAY use an OIDC or SAML provider, LDAP, or any other upstream identity system behind this boundary. Clients do not implement or depend on that upstream protocol. Discovery of which relays require enterprise identity, and of which adapter serves them, is outside this NIP.

## Terminology

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in BCP 14 [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119) [RFC 8174](https://www.rfc-editor.org/rfc/rfc8174) when, and only when, they appear in all capitals, as shown here.

- **adapter**: The HTTP service implementing this NIP. `{adapter_base}` is its base URL. The adapter is a NIP-FI assertion issuer, or fronts one: the assertions it returns are verified by relays exactly as NIP-FI assertions from that issuer.
- **adapter session**: The opaque `session_token` the adapter issues after browser login, and the fixed lifetime it describes.
- **agent grant**: The opaque credential an adapter issues for one agent key under an owner authorization.
- **handoff secret**: A high-entropy random value the client generates per login and reveals only during code exchange.

## Browser login

The client MUST generate a high-entropy handoff secret, start a loopback callback listener, and open:

```text
GET {adapter_base}/v1/login/start?return_to={callback_url}&handoff_challenge={base64url(sha256(handoff_secret))}&handoff_challenge_method=S256
```

The client's `callback_url` MUST use the `http` scheme with a loopback IP literal host (`127.0.0.1` or `[::1]`), as defined in [RFC 8252 §7.3](https://www.rfc-editor.org/rfc/rfc8252#section-7.3).

The adapter authenticates the user however the operator chooses, binds the completed browser login to `handoff_challenge`, and redirects to the exact `return_to` loopback callback. Adapters SHOULD accept loopback callbacks on any port, as recommended by [RFC 8252 §7.3](https://www.rfc-editor.org/rfc/rfc8252#section-7.3), and MUST reject any `return_to` whose host is not exactly `127.0.0.1` or `[::1]`, including `localhost` and other `127.0.0.0/8` addresses. Success redirects to:

```text
{callback_url}?code={single_use_code}
```

Failures MAY redirect to the same callback with `error` and optional `error_description` query parameters.

The code alone is not a credential. The adapter MUST accept it only when the exchange presents the matching handoff secret, MUST accept each code at most once, and SHOULD expire unexchanged codes within minutes. Adapters MUST reject any `handoff_challenge_method` other than `S256`.

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

`expires_at` is an [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339) timestamp. `email` and `profile_projection` are OPTIONAL. See [Privacy](#privacy) for when a client may publish `profile_projection`.

The exchange `expires_at` and a later session-check `expires_at` MUST describe the same fixed adapter session. After code exchange, the client MUST call `/v1/session` before committing the login. Clients MUST compare the two values as instants, not as strings, and MUST reject the login when they differ, so a code exchange cannot commit a different session than the one verified by `/v1/session`.

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

The adapter MUST return 401 `session_required` or `session_expired`, with the same body shape and meaning as in [Denials](#denials), when the token is missing, invalid, or expired. The client MUST discard the adapter session, and only the adapter session, on those responses. Clients MUST handle 429, 5xx, and network failures as in [Denials](#denials): keep the session and retry with bounded backoff. Any other non-success response is a failed check that does not discard the session.

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

- `relay_url` selects an adapter-configured relay; the adapter MUST NOT fetch it. An adapter MAY obtain its exact relay list from a configured, authenticated source; it MUST still compare `relay_url` exactly. Clients MUST send it in the canonical form `wss://host[:port]`, with the scheme in lowercase, a DNS host as its lowercase ASCII (A-label) form with no trailing dot, an IPv4 host in dotted-decimal form, an IPv6 host in brackets in its [RFC 5952](https://www.rfc-editor.org/rfc/rfc5952) text form (for example `wss://[2001:db8::1]:8443`), the default port omitted, and no trailing slash, path, query, or fragment. Relay authorities on port 80 are not supported: clients MUST NOT send, and adapters MUST NOT configure, a `relay_url` with port 80. Adapters MUST configure relays in this canonical form and MUST compare `relay_url` against it exactly. An unknown relay MUST be rejected with 403 `authorization_denied`.
- `nostr_pubkey` is the lowercase hex key the client will authenticate to the relay with. The NIP-98 event MUST be signed by this key, use kind `27235`, carry exactly one `u` tag equal to the absolute URL of this endpoint, exactly one `method` tag of `POST`, and exactly one `payload` tag equal to the lowercase hex SHA-256 of the exact request body bytes. Its `created_at` MUST be no more than 60 seconds old and MUST NOT be more than 5 seconds in the future.
- The request body is limited to 4096 bytes and MUST NOT be content-encoded. A content-encoded body, such as a gzipped body, MUST be rejected with 400 `invalid_request`. Unknown fields MUST be rejected with 400 `invalid_request`.
- Credential handling is covered in [Security Considerations](#security-considerations).

Success response (`200`, `Cache-Control: no-store`):

```json
{
  "assertion": "<compact-JWS-assertion>",
  "nostr_pubkey": "<64-char lowercase hex public key>",
  "expires_at": 1790000300
}
```

`expires_at` is the assertion `exp` as Unix seconds. The assertion lifetime, `exp - iat`, is the adapter operator's policy. Adapters MUST NOT set `exp` later than the adapter session's expiry, and MUST keep the lifetime within the `maximum_assertion_age` the relay is configured to accept for the adapter's issuer; the relay rejects an assertion that exceeds it. Clients MUST NOT refuse an assertion solely because of its lifetime. The lifetime bounds how long a revoked key can keep reconnecting after a relay restart, because a relay MAY forget active [NIP-FI](NIP-FI.md) deny entries on restart; operators SHOULD choose it with that exposure in mind.

Adapters MUST issue dedicated [NIP-FI](NIP-FI.md) assertions whose protected `typ` is exactly `nip-fi+jwt` and that carry the NIP-FI required claims; as NIP-FI requires, `sub` is never an email address. The assertion's `aud` MUST be `https://` followed by the authority of the canonical `relay_url`, unchanged: `wss://community.example.com` maps to `https://community.example.com`, `wss://c.example.com:8443` to `https://c.example.com:8443`, and `wss://[2001:db8::1]:8443` to `https://[2001:db8::1]:8443`. This is the canonical host URI of the community the relay resolves from the connection's `Host`. The assertion's `nostr_pubkey` MUST be the key that signs the NIP-42 relay login; the relay rejects any other key. Clients MUST reject a response whose `nostr_pubkey` differs from the key sent in the request.

An adapter MUST NOT issue an assertion for an agent key through a user's own authentication. It MUST issue agent-key assertions only under an owner-authorized grant: a durable record, created through the owner's own authentication, that binds the agent key to the owner. A grant MAY be held by the agent as a credential or kept by the adapter for agents it operates. The adapter MUST issue only while the owner authorization recorded in the grant remains valid. Revoking the owner MUST invalidate every grant the owner authorized before the revocation, and a later authentication by the owner MUST NOT revalidate them. Agent-key assertions follow NIP-FI "Agent keys". [Agent keys](#agent-keys) defines the grant routes.

A client that rejects a `200` response, including one it cannot parse, MUST treat it as a refusal: keep the session, show it as refused, and not retry automatically.

### Denials

Denials return a JSON body `{"error": "<code>"}` with `Cache-Control: no-store`. Clients MUST take the client action listed for each code:

| Status | `error` | Meaning | Client action |
|---|---|---|---|
| 400 | `invalid_request` | Malformed body, unknown fields, a missing or repeated `Nostr-Authorization` header, a repeated `Authorization` header, or any session credential other than exactly one `Authorization: Bearer` header | Keep the session, show the error, do not retry automatically |
| 401 | `session_required` | No adapter session was presented, or the presented one is not usable | Clear the adapter session and return to browser login |
| 401 | `session_expired` | The adapter session ended | Clear the adapter session and return to browser login |
| 403 | `authorization_denied` | The user or relay is not authorized for assertions | Keep the session, disconnect from that relay, show access denied, do not retry automatically; a manual retry or app restart asks again |
| 403 | `invalid_proof` | The NIP-98 proof failed verification | Keep the session, show the error, do not retry automatically |
| 403 | `binding_mismatch` | The adapter has bound this user to a different Nostr key | Keep the session, show the error, do not retry automatically |
| 413 | `request_too_large` | The body exceeds 4096 bytes | Keep the session, show the error, do not retry automatically |
| 429 | `rate_limited` | Too many requests | Keep the session, retry with bounded backoff |
| 503 | `issuance_unavailable` | The adapter could not issue an assertion | Keep the session, retry with bounded backoff |

Clients MUST keep the session and retry 429 and any 5xx status with bounded backoff, whatever the `error` code. Clients MUST treat any other status, or a 400/401/403/413 with a code this NIP does not define, as a refusal: keep the session, show it as refused, and not retry automatically.

A missing session MUST be reported as 401 `session_required`, never 400. Clients MUST handle network failures like 429 and 5xx: keep the session and retry with bounded backoff. Clients SHOULD honor `Retry-After` when present, and SHOULD show the failure to the user after a bounded number of attempts.

An adapter MAY return `session_required` for an invalid, expired, or revoked credential; adapters that distinguish ended sessions MAY return `session_expired`. Clients MUST handle both identically.

## Agent keys

An agent key acts for an owner key under an owner-authorized grant (see [Relay assertion](#relay-assertion)). This section defines the grant a client holds for an agent key it controls, the assertion request made with it, and the owner's revocation. An adapter that operates an agent itself MAY keep the grant internally and issue that agent's assertions without these routes.

The owner authorization is a [NIP-OA](NIP-OA.md) `auth` tag, sent as the compact JSON array of its four strings. The adapter MUST verify the tag as NIP-OA defines, with the agent key as `event.pubkey`, and MUST refuse a tag whose owner is not the owner key the adapter has bound to the authenticating user. Adapters MUST accept tags with empty conditions, and MUST refuse a tag carrying any condition clause they do not support.

### Agent grant

A client holding both the owner's adapter session and the agent key exchanges them for an agent grant, using the [RFC 8693](https://www.rfc-editor.org/rfc/rfc8693) token exchange. The client MUST send it with an HTTP client that does not follow redirects, and MUST treat any 3xx response as a failed exchange.

```http
POST /v1/nip-fi/agent-grant
Content-Type: application/x-www-form-urlencoded
Nostr-Authorization: Nostr {base64-NIP-98-event}

grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange
&subject_token={session_token}
&subject_token_type=urn%3Anostr%3Aparams%3Aoauth%3Atoken-type%3Anip-fa-session
&requested_token_type=urn%3Anostr%3Aparams%3Aoauth%3Atoken-type%3Anip-fi-agent-grant
&agent_pubkey={64-char lowercase hex agent public key}
&owner_attestation={NIP-OA auth tag JSON}
```

- The body MUST contain exactly these six fields, each once. Unknown or repeated fields MUST be rejected with 400 `invalid_request`. `grant_type`, `subject_token_type`, and `requested_token_type` MUST have exactly the values shown.
- The session travels only as `subject_token`. The adapter MUST reject the request with 400 `invalid_request` when it also carries an `Authorization` header or a cookie.
- The NIP-98 event MUST be signed by `agent_pubkey` and follow the [Relay assertion](#relay-assertion) proof rules for this endpoint's URL and the exact form body bytes. A request without exactly one `Nostr-Authorization` header MUST be rejected with 400 `invalid_request`.
- Each proof is single-use on the agent grant and agent assertion routes: the adapter MUST reject a proof whose event `id` it has already accepted on that route, with 400 `invalid_request`, and clients MUST sign a fresh proof for every attempt, including retries.
- The body is limited to 8192 bytes and MUST NOT be content-encoded.

Success response (`200`, `Cache-Control: no-store`), in the [RFC 8693 §2.2.1](https://www.rfc-editor.org/rfc/rfc8693#section-2.2.1) shape:

```json
{
  "access_token": "opaque-agent-grant",
  "issued_token_type": "urn:nostr:params:oauth:token-type:nip-fi-agent-grant",
  "token_type": "N_A",
  "expires_in": 691200,
  "nostr_owner_pubkey": "<64-char lowercase hex owner public key>",
  "nip_fi_issuer": "https://issuer.example.com"
}
```

The grant's lifetime, and whether use extends it, is adapter policy; `expires_in` describes it when issued. Clients MUST reject a response whose `issued_token_type` or `token_type` differs from the values shown, whose `nostr_owner_pubkey` differs from the owner in the tag sent, or whose `access_token` or `nip_fi_issuer` is empty.

An agent key belongs to one owner for good. A successful enrollment, by this exchange or any other enrollment method an adapter offers, replaces every earlier grant for that agent key. The adapter MUST refuse with 400 `invalid_grant` an enrollment of an agent key it has bound to a different owner, or of an agent key that has been revoked. An expired grant does not end the enrollment: the owner MAY enroll the same agent key again under a valid owner authorization.

Exchange denials use the body shape of [Denials](#denials):

| Status | `error` | Meaning | Client action |
|---|---|---|---|
| 400 | `invalid_request` | Malformed form, a header rule above was broken, the NIP-98 proof failed verification, or the `subject_token` is invalid or unacceptable, including a session whose owner may not enroll agents, as [RFC 8693 §2.2.2](https://www.rfc-editor.org/rfc/rfc8693#section-2.2.2) requires | Show the error, do not retry automatically |
| 400 | `invalid_grant` | The owner authorization is invalid, or the agent key cannot be enrolled | Show the refusal, do not retry automatically |
| 403 | `agent_unsupported` | The adapter does not issue agent grants | Show the refusal, do not retry automatically |
| 413 | `request_too_large` | The body exceeds 8192 bytes | Show the error, do not retry automatically |
| 429 | `rate_limited` | Too many requests or agents | Retry with bounded backoff |
| 503 | `issuance_unavailable` | The adapter could not issue a grant | Retry with bounded backoff |

### Agent assertion

The client requests an agent-key assertion with the grant in place of the adapter session:

```http
POST /v1/nip-fi/agent-assertion
Authorization: Bearer {agent_grant}
Nostr-Authorization: Nostr {base64-NIP-98-event}
Content-Type: application/json

{
  "relay_url": "wss://community.example.com",
  "nostr_pubkey": "<64-char lowercase hex agent public key>"
}
```

The body, the NIP-98 proof (signed by the agent key, for this endpoint's URL), the success response, and the client's response checks are those of [Relay assertion](#relay-assertion), except that the assertion's protected `typ` is exactly `nip-fi-agent+jwt` and it carries `nostr_owner_pubkey` as [NIP-FI](NIP-FI.md) "Agent keys" requires, and that its `exp` is not capped by any adapter session's expiry: the adapter MUST issue only while the grant and the owner authorization recorded in it are valid, and the assertion lifetime limits otherwise apply unchanged. Clients MUST reject an assertion whose `nostr_owner_pubkey` differs from the grant's, or whose `iss` differs from the grant response's `nip_fi_issuer`. The adapter MUST NOT accept an adapter session on this route, and MUST reject with 400 `invalid_request` a `nostr_pubkey` other than the grant's agent key.

Denials follow [Denials](#denials), with the grant in place of the session:

| Status | `error` | Meaning | Client action |
|---|---|---|---|
| 401 | `invalid_grant` | No grant was presented, or the grant is unknown, expired, or revoked, or its owner authorization has ended | Discard the grant; enroll again only through a new owner authorization |
| 403 | `authorization_denied` | The relay is not authorized for this agent's assertions | Keep the grant, disconnect from that relay, show access denied, do not retry automatically |
| 403 | `agent_unsupported` | The adapter does not issue agent assertions | Keep the grant, show the refusal, do not retry automatically |

The remaining codes and the rules for 429, 5xx, network failures, and undefined codes are as in [Denials](#denials), with "keep the session" read as "keep the grant".

### Agent revocation

The owner revokes one of their agents with their adapter session:

```http
POST /v1/nip-fi/agent-revoke
Authorization: Bearer {session_token}
Content-Type: application/x-www-form-urlencoded

agent_pubkey={64-char lowercase hex agent public key}
```

The body MUST contain exactly one field, `agent_pubkey`; any other form MUST be rejected with 400 `invalid_request`. The body is limited to 1024 bytes; a larger body MUST be rejected with 413 `request_too_large`.

Success response (`200`, `Cache-Control: no-store`): `{"revoked": true}`. Revocation is idempotent: revoking an agent the owner already revoked succeeds and changes nothing. The adapter MUST invalidate every grant for that agent key and stop issuing assertions for it, and MUST disconnect it with the [NIP-FI](NIP-FI.md) Admin disconnect API. Revocation MUST NOT affect the owner's own key or sessions. An agent key the session's owner never enrolled, including another owner's, MUST be answered as 404 `agent_not_found`, so revocation does not reveal other owners' agents. Session denials are as in [Session check](#session-check); a missing session is 401 `session_required`.

## NIP-FI assertion transport

The adapter session header above is only for client-to-adapter requests. It is not the relay's NIP-FI proof transport, and adapter sessions MUST use only `Authorization: Bearer`, except in the [Agent grant](#agent-grant) exchange, where the session travels only as `subject_token`. The origin and redirect rules in [Security Considerations](#security-considerations) apply to it there too.

For HTTP routes protected by NIP-98, the relay proof uses the headers defined by [NIP-FI](NIP-FI.md):

```http
Authorization: Nostr <base64-NIP-98-event>
Nostr-Federated-Identity: Bearer <compact-JWS-assertion>
```

WebSocket connections send only `Nostr-Federated-Identity` on the upgrade request and then prove the key with NIP-42 AUTH. Blossom media routes carry a kind `24242` event in `Authorization`. See NIP-FI for both.

## Security Considerations

- Clients MUST send `session_token` only to the adapter's origin (the scheme, host, and port of `{adapter_base}`), and MUST NOT send it to a relay or any other origin. A leaked session token lets its holder request assertions for a key they control until the adapter binds the user to a key.
- Clients MUST send agent grants only to the adapter's origin, under the same rule. Clients MUST NOT send a session token or agent grant through, or replay it to, a redirect target.
- Clients and adapters MUST NOT log credentials: the `Authorization` and `Nostr-Authorization` header values on every adapter request, including `/v1/session`; the `handoff_secret` and `session_token` values in the code exchange request and response; `subject_token`, `owner_attestation`, and `access_token` in the agent grant exchange.

## Privacy

NIP-FI defines no public identity projection. Adapter responses are private login state by default. Clients MUST NOT publish `profile_projection` fields to a public Nostr profile unless the operator has explicitly opted into managed public profiles. Operators that want managed public profiles MUST make that an explicit policy choice and provide explicitly publishable `profile_projection` fields. Raw upstream claims, legal names, emails, issuer identifiers, subjects, and assertion contents MUST NOT be written to public Nostr events implicitly.

## Out of scope

- Discovery of which relays require enterprise identity and which adapter serves them.
- The upstream identity protocol between the adapter and its identity provider.
