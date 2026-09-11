# Enterprise HTTPS signer: implementation experiment

**Status: partial proof, not an enterprise-ready build.** This change adds an
explicit browser signer adapter at the existing Nostr signing seam. It does not
wire corporate login, desktop/mobile settings, or release defaults. Do not enable
it for employees until the authorization and rollout requirements below are met.

Enterprise custody is an intentional exception to `VISION_SOVEREIGN.md`'s
self-custody promise: an organization owns and can revoke its employee identity.
OSS users retain their local/NIP-07 keys. Hosting and relay permissions remain
independent of the signer; signed events still use NIP-01 unchanged.

## Two endpoints

The base URL includes any deployment prefix. All calls are HTTPS POST, JSON,
no redirects, no HTTP/browser caches. The prototype uses an explicit
`X-BB-Session-Credential` header supplied by the platform login integration, not
ambient cookies. The adapter does not persist the credential. A browser deployment
must explicitly allow its trusted origin and this header through CORS; it must not
use wildcard credential forwarding or accept cookie-only cross-origin signing.

### `/v1/buzz/enterprise-signer/session`

Request: `{}`. Response:

```json
{
  "pubkey": "64 lowercase hex characters",
  "relayWsUrl": "wss://community.example",
  "relayHttpUrl": "https://community.example"
}
```

Success means the server has checked corporate authorization and confirmed relay
membership for its custodied account key. It must not return a user-uploaded public
key or claim membership merely because a database identity binding exists.
Failures propagate; the client must not assume admission or switch identities.

### `/v1/buzz/enterprise-signer/events/sign`

```json
{
  "purpose": "nip42-auth",
  "event": {
    "kind": 22242,
    "created_at": 1789167600,
    "tags": [["relay", "wss://community.example"], ["challenge", "relay-challenge"]],
    "content": ""
  }
}
```

Response: `{ "event": <seven-field signed Nostr event> }`.
No `pubkey`, `id`, `sig`, account selector, or unknown template fields are accepted.
The server derives the account from verified corporate authorization and validates
purpose independently of the client's claim. The adapter verifies the signature,
public key, and exact template before returning the result.

Purposes:

- `nip42-auth`: kind 22242, empty content, exact relay and challenge tags. After
  authentication, subscriptions connect directly to Buzz; incoming events never
  call the signer.
- `http-auth`: kind 27235, empty content, authorized HTTPS origin and HTTP method;
  body-bearing requests require a SHA-256 `payload` tag. HTTP bearer proof expiry
  and replay protection are enforced by the relay.
- `media-read`: kind 24242, `t=get`, exact `server` authority, expiration at most
  five minutes away. Cache the resulting Nostr header only for this account/host
  and only before expiration. This PR does **not** add a media credential cache.
- `media-upload`: kind 24242, `t=upload`, exact `server`, short expiration and
  lowercase SHA-256 `x` tag. File bytes go directly to the relay.
- `publish`: the initial backend proof deliberately allows only kinds 1, 5, 7, 9,
  and 11. Profiles and delegation are rejected, not silently supported. The full
  Buzz event-kind inventory still needs policy review.

The prototype accepts timestamps from 60 seconds ago through 30 seconds ahead,
64 KiB of content and 64 KiB of tag strings, at most 256 tags, with at most 16
strings per tag. The request is capped at 128 KiB; the adapter response at 256 KiB.
Transport limits must also be applied **before** the server buffers request bodies.
Never log credentials, key bytes, request/event bodies, or signed bearer proofs.

## Retry identity

Preserve the complete unsigned template including `created_at` across signing
retries; Schnorr auxiliary randomness can change the signature but not the event
ID. Persist the signed event before publication and resend that exact event after
an ambiguous acknowledgement. The signer does not publish and does not provide an
idempotency ledger. Native outbox integration is still required; reconstructing a
template with a fresh timestamp on retry is incorrect.

## Browser integration seam

```ts
configureEnterpriseSigner(new EnterpriseSigner({
  baseUrl: managedSignerUrl,
  credential: () => corporateLogin.currentCredential(),
}));
```

Install before opening relay connections. The existing `signNostrEvent` path then
uses enterprise custody ahead of NIP-07 or anonymous fallback, and the invite UI
recognizes it as durable signing. Credential expiry must leave this signer
installed and fail closed. Calling `configureEnterpriseSigner(null)` is an explicit
return to a self-custody community, **not** enterprise logout. Reconfiguration
fences in-flight signing results. The login lifecycle is not wired by this PR.

## Remaining work and release gates

1. **Corporate authorization:** Auth0 federation is only login. Recheck a
   sufficiently fresh corporate authorization lease per operation and propagate
   disablement to signer access and relay membership. An existing eight-hour app
   session is not an employment check. Live WebSocket termination belongs to the
   separate revocation branch, but signer denial and membership revocation do not.
2. **Authoritative profiles:** publish corporate name/email from the directory;
   prevent user-chosen profile events from asserting another employee's name.
   Decide migration from current self-custody keys and how old profiles display.
3. **Provisioning:** atomically persist the account key, then durably reconcile
   external relay admission/profile updates. A database transaction cannot make a
   remote relay write atomic. Test crashes between relay acknowledgement and local
   state, and make retries reuse the same key and event IDs.
4. **Desktop:** `AppState` owns `Mutex<Keys>`; submission, native relay auth, media,
   huddles, pairing, backups, and agent authorization access key material directly.
   Introduce an async identity/signer abstraction and explicitly disable or replace
   secret-dependent operations. Replacing only the `sign_event` IPC is insufficient.
5. **Mobile:** message signing, `RelaySocket`, media read auth, uploads and secondary
   relay sessions each decode nsecs. Media header getters are synchronous; remote
   signing needs asynchronous prefetch/cache and expiration-aware recovery.
6. **Release wiring:** trusted managed configuration, login/refresh UI, corporate
   origin policy, enterprise settings, and native signed build defaults remain
   absent. Do not advertise the adapter as a usable enterprise release.
7. **Operational security:** key deletion/retention, restore and access audit,
   per-account rate limits, pre-buffer transport bounds, short auth lifetimes and
   security review are mandatory before enabling custody.

## Validation

`pnpm --dir web test` exercises the actual browser signing seam with a local HTTP
fake and real Nostr signatures; `pnpm --dir web typecheck` checks the adapter. These
are not a deployed Auth0-to-relay integration test. No live corporate identity or
relay membership was changed by this experiment.
