# Enterprise HTTPS signer — integration and release contract

**Status: draft native integration.** Desktop and mobile now have release-selected
corporate login, refresh and remote signing. The real Auth0/Okta applications,
Kgoose deployment and end-to-end acceptance in that environment are separate
rollout work. Do not infer production readiness from mock/transport tests.

Enterprise custody intentionally differs from `VISION_SOVEREIGN.md`: the
organization controls its employee identity. OSS builds retain local/NIP-07 keys.
Nostr events, external relay hosting and community/channel permissions remain.

## Release selection — no user configuration

Desktop packaging sets `BUZZ_BUILD_ENTERPRISE` to non-secret JSON before compiling
Tauri. Mobile passes the same JSON using `--dart-define=BUZZ_BUILD_ENTERPRISE=...`
(or a define file). Absence selects existing OSS self-custody; malformed enterprise
configuration fails closed, never falls back to local credentials.

```json
{
  "signerUrl": "https://signer.example/api",
  "issuer": "https://corporate-login.example/",
  "clientId": "registered-native-public-client",
  "audience": "buzz-enterprise-signer",
  "organization": "org_corporate",
  "connection": "okta-employees"
}
```

Mobile additionally requires `"redirectUri":"buzz://enterprise-login"`. Desktop
uses `http://127.0.0.1:<ephemeral-port>/enterprise-callback`. Register the native
client's appropriate redirect policy in Auth0; confirm support for desktop's
loopback ephemeral port. Never bake a client secret, access token or refresh token.
The internal release pipeline lives outside this repository; its real values and
packaging changes must be supplied there. The OSS release process is unchanged.

## Login and token lifecycle

Both native clients open the system browser for authorization-code/PKCE S256.
Random state and callback validation bind the browser response to the attempt.
The client exchanges the code with Auth0, then calls the trusted signer's session
endpoint to discover and pin its identity/community. Employees do not choose a
signer, import an nsec, or configure a relay in enterprise mode.

Tokens stay in native secure storage (desktop OS keyring, mobile secure storage),
not renderer/localStorage. Refresh is single-flight. Rotating refresh credentials
are removed from durable storage before exchange so a lost response cannot cause
a restart to replay a consumed credential. Failure requires browser login; a
successful replacement is saved before further signing. This intentionally favors
safe reauthentication over uninterrupted access after a crash during rotation.
Stored credentials are bound to the release-selected login configuration.

Desktop reports expiry via its login gate; mobile invalidates authentication when
refresh fails. Account/community changes are rejected on refresh. In-flight signing
is bound to the captured identity; logging out or replacing it cannot silently
retarget an event. Reads after successful NIP-42 auth go directly to Buzz.

## Two signer endpoints

All requests use HTTPS POST and JSON, no redirects or caches, and exactly one
`Authorization: Bearer <dedicated-corporate-access-token>` header. No second
bbidentity app-session credential is required. Cookie-only and ordinary app-token
requests cannot authorize signing. The backend resolves the existing stable
bbidentity account from the verified Auth0 organization and subject.

The token contract checks issuer, RS256 signature, audience, subject, organization,
enterprise connection, `buzz:sign`, and signed corporate entitlement. Authorization
expires at most five minutes after the issuer's corporate check, not five minutes
after an arbitrary refresh. Auth0-side entitlement verification on login **and
refresh** remains an external prerequisite; a stale app session is not employment
proof. See the companion Kgoose design for the precise namespaced claims.

### POST `/v1/buzz/enterprise-signer/session`

Request `{}`; response:

```json
{
  "pubkey": "64 lowercase hex characters",
  "relayWsUrl": "wss://community.example",
  "relayHttpUrl": "https://community.example"
}
```

Creates or reads the atomic encrypted account key. First membership admission and
corporate-derived kind:0 profile are journaled as signed events before publication.
Retries retain event IDs after ambiguous acknowledgements. Completed accounts
check current membership rather than silently re-adding a removed employee.
The enterprise relay must be membership-gated.

### POST `/v1/buzz/enterprise-signer/events/sign`

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

Returns `{ "event": <seven-field signed Nostr event> }`. No identity selector,
`pubkey`, `id`, `sig` or unknown template field is accepted. Clients verify the
signature, public key and exact template. The service rechecks corporate authority
and membership before signing.

Purposes: `nip42-auth`, `http-auth`, `media-read`, `media-upload`, `publish`.
NIP-98 proofs bind exact origin/method/body hash. Media proofs bind server authority,
short expiry and upload hash; file bytes travel directly to Buzz. Generic profile,
delegation, pairing, agent custody and relay-admin signing remain denied.

Bearer timestamps are within -60/+30 seconds; durable publish templates may be
older. Content and tag strings are each capped at 64 KiB, 256 tags, 16 strings/tag.
Application request limit is 128 KiB; signer response limit is 256 KiB. Deployment
must impose a pre-buffer ingress limit, redact bearer headers/bodies, and configure
fleet quotas. Backend admission adds per-pod concurrency/account bounds.

## Native signing coverage and deliberate exclusions

Desktop's `SigningIdentity` snapshots route generic event IPC, message/channel/DM
submission, query HTTP auth, native relay sessions, media and human huddle auth/STT
through either local keys or the corporate signer. `signing_keys()` explicitly
rejects enterprise mode. The renderer never receives corporate tokens.

Mobile's `signClientEvent` routes normal submissions, NIP-42, HTTP query auth,
media uploads, typing/status and human huddle auth through the corporate signer.
Media image/file/audio/video callers await host-scoped read credentials. Read
proofs cache for two minutes with an expiry margin; file bytes bypass Kgoose.

First-build exclusions are explicit, not alternate local identities:
- Key export/import/backup/pairing and local managed-agent creation/start.
- Directory-controlled profile edits.
- Secret-dependent observer decryption, git private-key helper operations, mesh
  identity and encrypted preference synchronization.
- Cross-device encrypted read-state sync; local read markers continue to work.
- Existing private-key-based mobile push lease/NSE paths are not enabled for the
  keyless corporate community. Native corporate push support is a later feature.

Authorized independently operated relay agents remain usable in conversations.
The signing abstraction is not a promise that every existing secret-dependent
feature has a remote equivalent.

## Minimal sends, no new durable outbox

Build a template, sign, publish and await the relay ACK using existing flows.
Preserve the template/signed event for retries inside the operation; never change
`created_at` to recreate a potentially delivered event. A crash or lost ACK can
still leave delivery uncertain. No new durable client outbox is implemented in
this milestone; the backend provisioning journal is separate.

## Remaining rollout/security gates

- Deploy Kgoose/schema/admin/encryption config; configure Auth0 native apps,
  audience/claims/refresh and Okta federation; set internal release build values.
- Exercise real login, refresh, second device, chat/media/huddles, denial and OSS
  regression in deployed native apps. Current evidence is unit/transport/mocked UI.
- Complete offboarding-driven signer denial and durable membership removal with
  pending-provisioning generation fencing. Active WebSocket termination remains
  on the separate branch. A short lease is not instantaneous revocation.
- Profile refresh/migration, custody retention/deletion/audit/restore, header
  redaction, ingress size limits and fleet-wide quotas/security approval.
