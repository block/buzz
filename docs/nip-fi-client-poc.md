# NIP-FI desktop + agent POC

This PR now contains executable client integration, not just an effort sketch.
It remains a **draft POC, disabled by default**, against an assumed adapter API.
It does not implement or deploy kgoose, Okta configuration, or relay enforcement.
Mobile files from the first pass remain partial plumbing; this pass targets
**desktop, local agents, detached agents, CLI, and MCP**.

## What to run

Build the desktop with explicit protected origins and the adapter location:

```sh
. ./bin/activate-hermit
export BUZZ_BUILD_NIP_FI_ORIGINS=https://enterprise-relay.example
export BUZZ_BUILD_LOGIN_API_URL=https://adapter.example/api/goose
export BUZZ_BUILD_NIP_FI_ASSERTION_URL=https://adapter.example/api/goose/v1/buzz/identity/assertions
just dev
```

These are build-time settings: changing them requires rebuilding the Rust app.
No corporate origin is learned from NIP-11, token claims, image URLs, or renderer
input. Empty origins retain ordinary OSS operation. Invalid configuration fails
closed. HTTPS is required except explicit IP-loopback development fixtures.

Open the configured community. The normal community-init gate now acquires an
assertion before mounting the connected app. Without a login it offers **Sign in
with your work account**, runs the existing browser/code exchange, acquires the
assertion natively, and retries initialization. There is also an in-app recovery
notice: an expired login does not unmount the draft-containing app subtree.
The existing hosted-account sign-out clears the native session and assertions.
Sessions are memory-only in this POC: restarting the app requires sign-in again.

## Assumed API contract

Only the login/code-exchange portion of PR #7626 was used as a reference. Local
Nostr keys still sign everything; none of the remote-custody implementation is
used. The existing Builderlab login flow is reused, with a configurable API base:

1. Browser `/v1/auth/login?type=cli&product=buzz&returnTo=...`.
2. Random ephemeral loopback callback receives a one-time code.
3. Native `POST /v1/auth/login/exchange` and `GET /v1/auth/me`.
4. Native `POST /v1/buzz/identity/assertions`.

The assertion request contains:

```json
{"nostr_pubkey":"<local proof key>","relay_url":"<configured relay>","auth_tag":null}
```

It has `X-BB-Session-Credential: <login or scoped agent credential>` and
`Authorization: Nostr <base64 NIP-98 event>`. The proof binds the exact endpoint,
POST method, and SHA-256 of the exact body bytes. For an agent, `auth_tag` carries
its existing NIP-OA attestation; the adapter **must independently authorize** the
employee-to-agent relationship and lifecycle. Knowing/signing for a key is not
permission to enroll it.

Response:

```json
{"assertion":"<compact JWS>","nostr_pubkey":"<same proof key>","expires_at":1790000000}
```

`expires_at` is Unix seconds and is the effective deadline: the adapter must
include any maximum-age/connection policy in that deadline, not just JWT `exp`.
The POC accepts only `typ: nip-fi+jwt`; it rejects generic/ID-token JWTs, missing
claims, inconsistent keys, invalid times, and response expiry beyond JWT expiry.
It pins `(iss, sub, aud)` per key during the native login generation. This is
metadata validation, not client-side signature verification: the trusted adapter
is the acquisition boundary and the relay still verifies signatures, issuer,
audience, deny entries, and membership.

Responses are capped at 24 KiB; tokens at 16 KiB. Acquisition has a 30-second
request deadline, no redirects, a native single-flight mutex, generation fences,
and a ten-second failure cooldown. The native renewal loop checks every ten
seconds and refreshes with less than sixty seconds remaining. HTTP callers also
ensure current authority before sending. No assertion or login credential enters
renderer IPC responses, localStorage, events, filters, URLs, or public discovery.

## Implemented desktop paths

- Main renderer-facing native WebSocket upgrade; ordinary local NIP-42 AUTH.
- Native background relay socket: assertion-bearing upgrade and expiry/logout
  lease; desired subscriptions are restored by its existing reconnect loop.
- HTTP `/query`, signed event submission, authenticated relay GETs, explicit
  agent profile/engram publishers, and huddle transcription posts.
- Human huddle and agent TTS upgrades; expiry/logout cancels their existing audio
  transport token. **The POC ends the huddle at the admitted deadline; the user
  rejoins. Seamless audio reconnect remains work.**
- Blossom upload/download, avatar fetch, localhost streaming proxy, and
  `buzz-media` URI handler. Proofs have sixty-second lifetimes; legacy upload
  retry signs a fresh proof. Enterprise missing-proof paths fail closed and
  authenticated redirects are refused. Proxy responses use `no-store` in
  enterprise mode.
- Git project clone/fetch/push/merge gets a fresh assertion at invocation time,
  including explicit agent-owner merge paths. The ordinary Nostr credential
  helper is retained; the second header is supplied through ephemeral,
  origin-scoped Git configuration, never written to a config file.

Login replacement, logout, community changes, and key imports invalidate the
native generation. Main/native sockets and huddles observe leases. Primary HTTP
query/GET/submit/download/upload operations cancel on generation/expiry changes;
response consumption is inside the guard for query/GET/submit/download. A
cancelled write reports that its outcome may be unknown, not definitive failure
followed by an automatic second mutation. Existing signed event IDs are retained.

## Local agents

The desktop starts a loopback assertion broker. At the actual process-spawn
boundary it supplies a random capability bound to the agent key, relay, and login
generation. It never gives the agent the employee's login credential or human JWT.

The broker requires both that capability and a signed, fresh, exact-body NIP-98
proof. It checks that the agent remains managed locally, requests/caches an
assertion for **that agent's key**, and returns it only to native agent clients.
It rejects browser-origin requests. There are at most 256 grant/key entries and
24 KiB request bodies. Logout invalidates the grants; agents must be restarted
following a new login to receive new capabilities.

Reserved environment variables:

- `BUZZ_NIP_FI_ENDPOINT`: loopback broker (local) or adapter (detached).
- `BUZZ_NIP_FI_CREDENTIAL`: key-scoped capability/renewal credential.
- `BUZZ_NIP_FI_ORIGINS`: explicit protected relay origins.

The ACP harness, CLI HTTP operations, CLI one-shot WebSocket publications, MCP
media image fetches, and MCP subprocess forwarding consume these variables.
Partial configuration fails closed. Agent sockets carry a fixed effective
admission deadline and check broker/adapter liveness every ten seconds; failed
checks drop the socket into existing bounded reconnect/catch-up behavior. Agent
HTTP requests renew per operation (and ACP per retry), not just at process boot.

`buzz git ...` acquires fresh authority and invokes Git with the second header.
The built-in dev-MCP installs a Git shim that routes ordinary `git` calls through
that launcher. Other/custom agent shells must use `buzz git` for enterprise Git;
there is no universal interception of arbitrary external harnesses' Git binaries.
Curl/Git tracing is disabled for credential-bearing invocations. ACP request
logging no longer serializes `session/new` credentials and EnvVar Debug is redacted.

## Detached agents: one additional backend assumption

A laptop-local broker cannot support an agent after the laptop closes. The POC
therefore assumes `POST /v1/buzz/identity/agent-delegations`, with the same signed
request plus the employee login credential. Response is the assertion response
above **plus `agent_credential`**, a revocable renewal credential bound by the
adapter to exactly that agent, owner, and realm. Subsequent `/assertions` calls
use that credential and the agent's possession proof.

Desktop provider deployment injects these three values into `launch.policy_env`
only in the invocation payload, never the managed-agent record or config snapshot.
The Kubernetes provider already materializes launch environment into its per-run
Secret. No new desktop-to-substrate control channel is introduced, preserving
VISION_REMOTE_AGENTS.md. Third-party providers must preserve this secret-env
contract. Adapter offboarding/revocation must revoke agent credentials as well as
employee access. Desktop logout alone does not revoke a detached credential.

**This is an explicit extension assumption, not a claim that NIP-FI/NIP-OA or the
current kgoose implementation already grants agent access.** If issuance policy
does not authorize agents, the client fails rather than substituting a human JWT.

## Evidence and remaining refinement

Automated coverage includes real native HTTP/WS header fixtures, real assumed
adapter HTTP exchange with possession/body checks, metadata validation, key
isolation, stale generation refusal, immutable socket leases, a spawned CLI
against a protected fake adapter/relay, native late-result cancellation, and
Playwright startup/sign-in/OSS recovery. Fixtures establish client behavior, not
real Okta or deployed relay admission. Test results are recorded in the PR body.

Remaining work before calling this production-ready:

- Run the actual adapter and combined enforcing relay PRs, package native builds,
  and exercise real account offboarding, long-running agents, huddles, and Git.
- Finalize enrollment/challenge and detached-agent issuance/revocation policy;
  validate the exact real issuer/audience configuration and API errors.
- Seamless huddle reconnect; native archive `limit:0` subscriptions still need
  finite gap repair on forced reconnect. Renderer live subscriptions and ACP
  already have reconnect catch-up machinery; native archive tail does not.
- Finish all late-result/media streaming/playback cache fences. Existing local
  history and already-rendered content remain available after logout; this POC
  does not claim remote wipe or an enterprise offline-retention policy.
- Revisit per-operation detached-agent acquisition cost, broker concurrency and
  per-key cache tuning, minimum useful token TTL, and multi-community realms with
  different audiences. Current explicit origin list represents one realm.
- Git headers are fresh at invocation, not per underlying HTTP request inside a
  long transfer. A transfer extending past admission expiry can fail; it is not
  silently retried. External/custom harnesses require `buzz git` or equivalent.
- Native pairing uses its independent ephemeral-key pairing relay and deliberately
  receives no human assertion. Mobile login/renewal, notification extensions,
  and browser-only client access are not completed by this desktop/agent pass.

The intent is now to refine runnable code against the real services, rather than
use a remaining-work document as a substitute for client implementation.
