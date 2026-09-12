# Organization-managed desktop identity

This is a release-selected build mode, not a runtime switch or an ordinary service
publish option. OSS/local-key builds retain their existing behavior. The managed
build cannot fall back to generating, importing, recovering or exporting a local
human signing key when corporate login or signing fails.

## Build and service contract

`BUZZ_BUILD_ENTERPRISE` is one JSON object with **exactly seven** fields:
`signerUrl`, `issuer`, `clientId`, `audience`, `organization`, `connection`,
`redirectUri`. The release environment selector is not a field in this object.
The native build consumer validates and canonically embeds it as
`BUZZ_DESKTOP_BUILD_ENTERPRISE`; corporate builds require `system-keyring`.
Do not put client secrets, tokens, or user-selected relay/identity values in it.
Signer and issuer URLs follow the shared release schema (HTTPS, no credentials,
queries or fragments, and no explicit port). Signer URL is a path prefix ending
in `/`, not an identity endpoint. Enrollment and signing are respectively:

- `POST <signerUrl>v1/buzz/identity/ensure`, body `{}`;
- `POST <signerUrl>v1/buzz/identity/sign`, body containing the exact unsigned event.

There are no legacy endpoint aliases. Enrollment pins the returned public key and
matching HTTPS/WSS relay origin. Backend-supported non-default relay ports are
allowed; this does not relax signer or issuer validation. Event verification
rejects any changed public key, kind, content, timestamp, tag, ID or signature.
The remote signer rejects off-origin HTTP, relay-auth and media proofs before
asking for credentials or contacting the service.

Browser login uses state and PKCE. The callback binds **exactly** the configured
`http://127.0.0.1:<fixed-port>/enterprise-callback`; a busy port is a sign-in error,
not permission to select a new port. The callback wait is bounded. The same
seven-field validator is consumed by build.rs and native contract tests.

## Ownership and offboarding

`EnterpriseIdentity` owns secure token storage, refresh and session cancellation.
The signing interface is only `public_key()` plus asynchronous exact-event
`sign()`; it does not own enrollment, event construction, publication, encryption,
secret export or login lifecycle. Refresh is singleflight and never re-enrolls.
Before rotating a refresh credential the owner durably writes a rotation
pending marker, then atomically stores the replacement. Interrupted/ambiguous
rotation or any storage/refresh failure requires a fresh sign-in; the old token
is not replayed. A failed durable cleanup is reported, not silently declared
successful.

Sign-out/login replacement cancels the old owner, native relay sessions (including
pending auth/reconnect), and huddle/upload authorization lifetimes. Responses from
an old signer cannot establish the new identity. A cancelled in-memory owner is
not restored from old disk credentials. Logout removes the saved session; it does
not wipe downloaded files, existing local caches/preferences, or revoke already
issued proofs. Operator offboarding must also remove backend/relay membership and
corporate authorization. A request accepted before cancellation cannot be undone
by cancelling the client.

Corporate media read proofs last at most 120 seconds; uploads at most 300 seconds.
One read-proof entry is owned by each login session and matched by identity and
exact relay origin (including port). Concurrent misses singleflight, cached proofs
are not used within ten seconds of expiry, and cancelled signing cannot populate
or serve the cache. New logins never inherit it. Corporate proof failure (including no session or logout) aborts both media
proxies and bounded downloads before any upstream transport; only OSS recovery
retains optional unsigned reads. The avatar card caller also propagates proof
failure. Authenticated media clients do not follow redirects. Membership checks remain server-side: cached proofs are
not an offboarding bypass.

## Deliberate exclusions

The service supplies event signatures, **not local private-key/NIP-44 operations**.
Accordingly:

- No private-key reveal/export/import, NIP-49 key backup, or key-based pairing.
- No encrypted cross-device read-state, sidebar stars/mutes/sort/sections,
  project-sidebar membership, or appearance sync. Existing device-local stores
  and local read-marker persistence remain available; sync managers do not fetch,
  subscribe, encrypt, publish, or retry in managed mode. No plaintext relay fallback.
- Encrypted reminders are explicitly unavailable (no background reminder poll).
- Local managed-agent creation, start, instance-key loading and runtime spawning
  are denied natively. The Agents surface explains the exclusion. Existing remote
  relay identities remain public identities, not a promise of local management.
- Other commands requiring a local human secret, including encrypted agent/team
  archives and local git credential/agent delegation setup, fail explicitly.
- Corporate community selection is pinned to the enrolled relay; importing a local
  identity or applying another community is rejected by the native workspace command.

Settings disclose these differences. These are intentional scope limitations,
not equivalent replacements for the local-key features or the remote-agent vision.

## Retry semantics and validation limits

Relay submission authenticates the captured relay and signer, and success requires
an ACK whose ID exactly matches the submitted signed event. Already-signed event
APIs preserve those bytes for caller-managed retries; media's legacy route retry
reuses its signed proof. Not every interactive command has a durable signed-event
outbox. After an ambiguous network failure, a new user action may construct a new
event: do not promise automatic exactly-once delivery or restart-safe retries.
There is no broad durable-outbox implementation in this port.

Synthetic tests cover the production scope guard before network/credentials,
exact-event verification, delayed signer relay capture/ACK matching, token-owner
faults, native cancellation, media cache fencing, encrypted-feature gating and
device-local reads. An opt-in compiled corporate-mode test constructs real AppState
and proves no local key/import persistence; use only synthetic build configuration.
Loopback OAuth tests exercise the actual fixed callback listener. These do not
prove live corporate SSO enrollment, signing-service authorization, deployment
schema integration, or operator offboarding. Live staging remains blocked while
infrastructure policy is deny-only. Build artifact/source markers alone are not
runtime proof. No production credentials are needed or used by these tests.

## Known draft limitations and dependent work

Directory/profile-editing operations remain exposed while the managed backend
excludes ordinary profile mutation: these fail rather than changing directory
names. The UI/backend mismatch is not solved by remote signing. Mobile additionally
still invokes ensure during refresh and has weaker direct parser/proof preflight
than Rust; see `mobile/MANAGED_IDENTITY.md`.

This feature is stacked on extraction draft #7600. Service draft
`squareup/cash-server#124571` signs but never publishes ordinary events; release
draft `squareup/buzz-releases#94` provides the seven compiled fields (its
`environment` selector is release-only). Terraform draft #1385 is unbound and
deny-only; an approved corporate authority and authorization for the active
infrastructure repository remain prerequisites. No schema apply, enrollment,
deployment, live keychain/SSO, native GUI or signed installer was validated here.
Empty local sidecar placeholders are not packaging evidence.
