# Managed identity on mobile

Managed identity is an opt-in **native build contract**, not an environment
variable read at runtime. An absent `BUZZ_BUILD_ENTERPRISE` Dart define retains
OSS self-custody. A present malformed define fails closed at login.

The define is JSON with exactly seven non-secret fields: `signerUrl`, `issuer`,
`clientId`, `audience`, `organization`, `connection`, and `redirectUri`.
`redirectUri` must be `buzz://enterprise-login`; `signerUrl` is an HTTPS prefix
ending in `/`, without the API version. The only identity routes are
`<prefix>/v1/buzz/identity/ensure` and `<prefix>/v1/buzz/identity/sign`.
Release selection, credentials, backend enrollment and deployment are separate
responsibilities; do not put an access token or private key in a build define.

## Authority and lifecycle

`EnterpriseIdentity` owns browser authorization-code/PKCE login, secure-storage
mutations, rotating refresh, and logout. `RemoteEventSigner` is an immutable
login-generation/public-key/community capability, not a private-key source.
Responses must contain a valid Nostr signature and exactly the requested author,
kind, content, tags and timestamp. Superseded login work cannot publish or
recreate saved credentials. Refresh is single-flight, verifies the ensured
identity/community, and fails closed without retrying a potentially consumed
rotating token. Interactive corporate sign-in is the recovery path.

Callers inject `RelayConfig.signer`; there is no global signing helper which can
silently select a different account. `SignedEventRelay` captures the session
lease at construction. `RelaySessionNotifier.publish` and `sendRaw` require a
lease captured **before** asynchronous signing. A provider rebuild/disposal
retires that lease; an ordinary reconnect does not. Publication rechecks after
rate-limit waits, callbacks cannot move a send into a replacement community,
and acknowledgements must identify the signed event. Direct status updates
check the lease again after the acknowledgement before updating state/cache.
HTTP query authentication also retains its originating lease.

## Media

Corporate read proof is limited to `/media/` on the exact HTTPS host and
effective port of the signer-selected relay. Userinfo is rejected before URL
normalization. Identity/signing/query/upload/download HTTP requests do not
follow redirects. Corporate auth failure never becomes an unsigned retry.
Managed read proofs live for 120 seconds and refresh at 110 seconds (ten-second
margin), within the backend's 300-second ceiling. OSS reads retain 600-second
proofs and a 60-second margin. The cache uses the proof construction time, not
the time the signing request finishes.
An upload's single signed proof is reused for the bounded legacy-route fallback.

Corporate video uses authenticated Dart download to a temporary file, never a
native network player; downloads are capped at 256 MiB and two-minute header/body
waits. The viewer checks scope after awaits and owns cleanup per effect so an
old effect cannot delete a new community's file. Corporate posterless timeline
previews do not download videos. Corporate audio uses the bounded Dart file
path (32 MiB/30-second download limit), not native redirect-following networking.
Corporate emoji uses the scoped Flutter picker/image path; the OSS native emoji
loader also rejects redirects. Media image cache identity includes auth scope,
and scope replacement resets the displayed image rather than preserving an old
account's pixels while replacement data loads.

## Explicit capability limits

This corporate build intentionally does not provide local-key export/backup,
device pairing, self-enrollment, arbitrary community selection, or the existing
local-secret-dependent encrypted read-state/personal-preference/theme/reminder
synchronization. It does not substitute plaintext publication for encryption.
Push/NSE authenticated reads needing a local key are unavailable. Local-key OSS
flows remain separate, including their existing best-effort unsigned media-read
behavior for invalid local key material; that behavior is not a corporate
fallback. Settings explains these limits and offers corporate sign-out/sign-in.

This intentionally differs from the default portable self-custodied identity
vision: the organization holds the key and fixes the community, while preserving
Nostr event verification and community isolation. It does not redesign the OSS
identity model.

## Validation boundaries

Tests exercise the production remote signer and publication/lease seam, same-key
login and community supersession, callback/disposal/rate-limit cancellation,
ACK identity, exact signed fields, redirect behavior, media limits and upload
proof reuse. `enterprise_build_config_test.dart` should run both without defines
and with a synthetic seven-field `--dart-define-from-file` plus
`EXPECT_MANAGED=true`; the latter calls the real default production parser.

Unit/widget tests and Swift compilation are not proof of live corporate login,
remote enrollment, device networking, native redirect runtime behavior, signed
AOT release artifacts or deployment. These require separate authorized runtime
and release validation. See the integration result manifest for the exact frozen
source hash, commands, results and remaining gaps.

## Known draft limitations (not rollout-ready)

- Directory/profile-editing operations remain exposed although the managed
  backend excludes ordinary profile mutation; those requests fail rather than
  updating directory-owned names. This UI/backend mismatch is not resolved here.
- Refresh currently calls `ensure` again and verifies the returned pinned
  identity/community; unlike desktop it is not authorization-only refresh.
- Mobile's direct config parser and generic signer proof preflight remain less
  strict than Rust's. The media path has its own exact-origin checks; the backend
  policy remains required, not replaced by client validation.
- There is no full operator offboarding/live relay eviction protocol or
  restart-durable signed-event outbox. Ambiguous retries can construct new events.
- Release draft 94 and backend draft 124571 are dependencies, not deployed
  services. Terraform draft 1385 is unbound/deny-only pending an approved
  corporate authority and active-infrastructure authorization. No live staging
  readiness, keychain/device login or signed installer is claimed.
