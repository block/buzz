# Enterprise HTTPS signer — implementation status and contract

**Status: draft implementation, not an enterprise-ready build.** The backend now
has a production authorization/provisioning implementation behind opt-in config.
Buzz has browser and shared native Rust signer transports, a native WebSocket auth
entry point, and a browser media-credential cache. Desktop/mobile login, settings,
complete signing-path conversion, durable client outboxes and release defaults
remain unfinished. Do not enable employee rollout yet.

Enterprise custody intentionally differs from `VISION_SOVEREIGN.md`: the
organization owns and can revoke its employee identity. OSS users retain local
or NIP-07 keys. Hosting, Nostr events, and community/channel permissions do not
move into the signer.

## Two endpoints

The base URL includes the deployment prefix. Both routes are HTTPS POST, JSON,
no redirects, no caches. Require both explicit headers:

- `X-BB-Session-Credential`: authenticated platform CLI/app credential; cookies
  alone are not accepted.
- `X-Buzz-Corporate-Authorization`: a dedicated short-lived Auth0 API access token
  bound to the same subject and organization, not an ordinary app ID token.

The server verifies issuer, RS256 signature, audience, subject, organization,
enterprise connection, `buzz:sign` permission, and signed corporate entitlement.
Authorization expires at most five minutes after the issuer's corporate check,
not five minutes after an arbitrary refresh. An issuer-side entitlement check on
login **and refresh** is an external prerequisite; the signer cannot turn a
long-lived application session into proof of current employment.

Browser deployment must explicitly allow trusted origins and both headers through
CORS. Never allow ambient cookie signing or wildcard credential forwarding.
Never log these headers, private keys, request bodies, or signed bearer proofs.

### `/v1/buzz/enterprise-signer/session`

Request: `{}`. Response:

```json
{
  "pubkey": "64 lowercase hex characters",
  "relayWsUrl": "wss://community.example",
  "relayHttpUrl": "https://community.example"
}
```

The service atomically creates or reads the account's encrypted Nostr key.
First admission and a directory-derived kind:0 profile are journaled as exact
signed events before relay publication. The journal is retained on failure;
retries replay its event IDs, including after an ambiguous acknowledgement.
Membership and key creation cannot be a single database transaction across an
external relay, so session success waits for both relay acknowledgements.
Completed accounts are checked as members, never automatically re-added after
removal. The configured enterprise relay **must be membership-gated**.

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
The service selects the key from the authenticated account, rechecks corporate
entitlement and relay membership, then validates the event's purpose. Clients
verify the signature, public key and exact unsigned template.

- `nip42-auth`: kind 22242, empty content, exact relay/challenge. Subscriptions
  connect directly to Buzz; incoming messages never visit the signer.
- `http-auth`: kind 27235, authorized HTTPS origin/method; body-bearing requests
  require the SHA-256 `payload`. Relay proof expiry/replay checks still apply.
- `media-read`: kind 24242, `t=get`, exact `server`, expiry within five minutes.
- `media-upload`: kind 24242, `t=upload`, exact `server`, short expiry and SHA-256
  `x` tag. File bytes travel directly to Buzz, not the signer.
- `publish`: explicit user-kind allowlist covering chat, forums, channel actions,
  read state, social lists, git, workflow controls and huddles. Generic kind:0
  profiles, identity/delegation, pairing, agent custody, relay-admin commands and
  relay-only sidecars remain denied. The relay still enforces channel ACLs.

Bearer templates accept timestamps from 60 seconds ago to 30 seconds ahead.
Durable publish templates permit older timestamps so offline retries retain their
identity. Content and tag strings each have a 64 KiB limit, with at most 256 tags
and 16 strings/tag. Application requests are capped at 128 KiB and signer replies
at 256 KiB. A **pre-buffer ingress limit** is still required. Backend work admission
is bounded per pod (32 concurrent operations, 120/account/minute, 10,000 buckets);
fleet-wide quotas remain a rollout prerequisite.

## Client seams

Browser:

```ts
configureEnterpriseSigner(new EnterpriseSigner({
  baseUrl: managedSignerUrl,
  credential: () => corporateLogin.currentCredential(),
  corporateAuthorization: () => corporateLogin.currentSignerAccessToken(),
  expectedSession: corporateLogin.pinnedSignerSession(),
}));
```

Install before relay connections. Expiry must leave this signer installed and
fail closed. `configureEnterpriseSigner(null)` explicitly selects self-custody,
not enterprise logout. Reconfiguration fences in-flight signatures. Identity and
community are pinned across credential refresh; changed values require login.
First-login discovery/pinning is still owned by the unimplemented login flow.

`EnterpriseMediaCredentials` caches one host/account's read proof for two minutes,
refreshes before expiry, deduplicates concurrent requests, and fences in-flight
results when cleared. Uploads send only the hash for signing. Callers must clear
on logout/account/community changes and auth rejection, disable redirects, and
send the exact hashed upload bytes. This helper is not yet wired into app UI.

Native Rust: `buzz-ws-client::enterprise::EnterpriseSigner` uses the same protocol,
pins identity/community, bounds transport time/size and treats credential headers
as sensitive. `NostrWsConnection::authenticate_enterprise` signs the challenge
through it, then uses the direct relay connection. Credentials are request-owned,
not persisted by the adapter. Desktop and CLI do not yet select this path.

## Retry identity

Persist the full unsigned template **before** signing, including `created_at`.
Schnorr auxiliary randomness can change a signature but not the event ID. Persist
the signed result before publishing, then resend that exact event after an
ambiguous acknowledgement. Refresh HTTP bearer proofs independently, with a new
nonce, without reconstructing the durable event. Native client outbox integration
is not implemented by these adapters.

## Remaining implementation and rollout gates

1. Auth0 API/audience and corporate entitlement claim issuance, login/refresh UI,
   secure credential lifecycle and trusted first-login identity discovery.
2. Desktop: replace `AppState`'s raw-key dependency across submission, native relay
   auth, media, huddles and deferred work. Disable or replace backups, pairing,
   mesh/agent authorization and other secret-dependent operations in enterprise
   mode. A single IPC replacement is insufficient.
3. Mobile: introduce an async signer in message, socket, media and secondary-relay
   paths. Synchronous media headers need prefetch/cache and expiry recovery.
4. Durable client outboxes, settings, enterprise signed-build defaults, profile
   refresh and existing self-custody identity migration.
5. Offboarding-driven signer denial and durable membership removal/reconciliation.
   Live WebSocket termination is intentionally left to the existing separate
   branch. Pending provisioning vs revocation needs explicit generation fencing
   before rollout; a short token lease is not instantaneous revocation.
6. Pre-buffer ingress limits, header/log redaction, fleet-wide rate limits,
   custody audit/retention/restore policy and security review.

## Validation

Browser tests exercise the production signing seam, real Nostr signatures,
credential denial, pinned identity, response bounds and media cache/hash behavior.
Rust tests exercise the actual transport with a loopback fake and real signatures,
including wrong-account/template/signature rejection. Kgoose tests cover custody,
corporate claims, provisioning crash/retry behavior and TLS relay transport.
These are not a deployed Auth0-to-native-app-to-relay acceptance test. No live
corporate identity or relay membership was changed.
