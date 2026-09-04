# Meetings (HiveTalk / LiveKit video)

Buzz "Meetings" adds live video conferencing to the desktop client using
[HiveTalk](https://l402relay.exe.xyz) for the control plane and LiveKit for
media. This document covers the **relay** side (Phase 1): an opt-in pass-through
proxy at `/meetings/*`.

## Enabling it

Set one environment variable on the relay:

```
BUZZ_HIVETALK_API_ROOT=https://l402relay.exe.xyz
```

- **Unset (default):** every `/meetings/*` route returns `404` before doing any
  work, and the NIP-11 document does not advertise Meetings. The feature is
  invisible to clients.
- **Set:** the proxy routes are live and NIP-11 advertises the `buzz-meetings`
  extension with a `meetings` descriptor (`{ provider, proxy, api_base }`), which
  the client uses to feature-detect. `api_base` echoes `BUZZ_HIVETALK_API_ROOT`
  so the client can build the `u` tag of its *own* HiveTalk-signed request —
  HiveTalk verifies that signature against the upstream URL, not the relay URL.
  It is a public URL, never a credential.

`BUZZ_HIVETALK_API_ROOT` is **not a secret** — see "No credential" below. It can
live in a plain ConfigMap / `.env`, unlike `BUZZ_KLIPY_API_KEY`.

It is validated at startup and the relay **refuses to boot** if it is not an
`https` URL with a host and no credentials, query, or fragment. That is stricter
than it looks: clients accept the descriptor only if `api_base` parses as
`https:`, so a plain `http://` root or a bare hostname makes Meetings *silently*
unavailable — the feature is simply absent in the UI, with nothing logged on
either side. Failing the boot is how that typo becomes visible.

Optional tuning:

```
BUZZ_RATE_LIMIT_MEETING_ACTIONS_PER_MIN=10   # default 10, per pubkey
```

## What the proxy does

For each request it:

1. Authenticates the caller as a **Buzz community member** using the existing
   relay stack — NIP-98 over the exact `/meetings/*` URL, HTTP admission
   rate-limit, NIP-98 replay guard, and `relay_members` membership gate.
2. For money / quota / token actions (`subscribe`, `payment/status`,
   `register-room`, `room/edit`, `room/delete`, `get-token`, moderation), also
   applies a dedicated `MeetingActions` rate-limit bucket.
3. Forwards the request to `{BUZZ_HIVETALK_API_ROOT}/api/...` with:
   - the **raw request body bytes** unchanged (HiveTalk signatures cover
     `sha256(rawBody)`; re-serializing would break them);
   - the caller's HiveTalk auth material, re-mapped from
     `X-Hivetalk-Authorization` → `Authorization` and
     `X-Hivetalk-Challenge` → `X-Challenge` (challenge flow), or copied verbatim
     to `Authorization` for moderation's LiveKit-JWT auth, or left in the JSON
     body for `get-token`.
4. Returns HiveTalk's status code (so the client can branch on `402`/`403`/`409`)
   with the response body **filtered to an allowlist** — documented success
   fields plus the standard error envelope (`error`, `reason`, `plans`,
   `subscribe_api`, `subscribe_url`, `message`). This applies to **every**
   endpoint, including the public metadata ones (`plans`, `list-rooms`,
   `room-info`, `rooms-by-pubkey`, `auth/challenge`) and to the elements of an
   array or envelope response, not just its top level. The allowlists come from
   HiveTalk's `openapi.yaml` unioned with the fields the desktop client's own
   types name; a field in neither is a provider addition and is dropped.

Redirects from HiveTalk are never followed (a signed request must not be
replayed to another host) and map to `502`. Upstream responses over 2 MiB are
rejected.

## No credential

The HiveTalk integration is **per-user**: any member who wants to *host* a
meeting brings their own HiveTalk subscription, and every operation is authorized
by that member's own signed request. The relay holds no HiveTalk API key, pool
key, or attestation key, and stores nothing about meetings.

## Media never transits the relay

`/meetings/*` is the **control plane only**. Audio/video RTC flows directly from
the client's LiveKit SDK to the LiveKit SFU using the `token` + `url` from
`get-token`. The relay is not in the media path.

## Proxied endpoints

| Relay route | HiveTalk endpoint | Auth forwarded |
|-------------|-------------------|----------------|
| `GET /meetings/auth/challenge` | `GET /api/auth/challenge` | none |
| `GET /meetings/plans` | `GET /api/plans` | none |
| `GET /meetings/subscription` | `GET /api/subscription` | challenge headers |
| `POST /meetings/subscribe` | `POST /api/subscribe` | challenge headers |
| `GET /meetings/payment/status?id=` | `GET /api/payment/status` | challenge headers |
| `POST /meetings/register-room` | `POST /api/register-room` | challenge headers |
| `POST /meetings/room/edit` | `POST /api/room/edit` | challenge headers |
| `POST /meetings/room/delete` | `POST /api/room/delete` | LiveKit JWT |
| `GET /meetings/room-info?room_name=` | `GET /api/room-info` | none |
| `GET /meetings/rooms-by-pubkey?pubkey=` | `GET /api/rooms-by-pubkey` | none |
| `GET /meetings/list-rooms` | `GET /api/list-rooms` | none |
| `POST /meetings/get-token` | `POST /api/get-token` | signed event in body |
| `POST /meetings/room/stage/{promote,demote}` | `POST /api/room/stage/*` | LiveKit JWT |
| `POST /meetings/room/moderator/{promote,demote}` | `POST /api/room/moderator/*` | LiveKit JWT |
| `POST /meetings/kick-user` | `POST /api/kick-user` | LiveKit JWT |
| `POST /meetings/mute-user` | `POST /api/mute-user` | LiveKit JWT |
| `POST /meetings/room/notify-lock` | `POST /api/room/notify-lock` | LiveKit JWT |
| `POST /meetings/room/mute-on-join` | `POST /api/room/mute-on-join` | LiveKit JWT |
| `POST /meetings/room/audience-mode` | `POST /api/room/audience-mode` | LiveKit JWT |

## Why this is HTTP and not an event kind

`AGENTS.md` says to model new operations as Nostr event kinds rather than
endpoint-specific HTTP routes. Meetings is an explicit, bounded exception, and
these 21 routes are not a precedent for new Buzz features:

- **The bytes cannot change.** HiveTalk verifies `sha256(rawBody)` over the
  caller's own signed request. Carrying that request inside a Nostr event means
  re-encoding it, which breaks the signature by construction. The proxy forwards
  raw bytes for exactly this reason.
- **CORS is the only reason the relay is here.** HiveTalk's signed endpoints send
  no `Access-Control-Allow-Origin`, so a desktop WebView `fetch` is blocked. The
  relay is a transport shim, not a protocol participant.
- **This is someone else's API surface.** The route list mirrors HiveTalk's
  control plane one-to-one, deliberately. It is not a Buzz protocol to be
  designed; a new event kind would buy no realtime fan-out, no NIP-29 scoping,
  and no auth reuse, because HiveTalk — not the relay — is the authority for
  every one of these operations.

What the relay *does* own — membership gating, rate limiting, response filtering
— is applied uniformly in `proxy()`. A new third-party control plane should reach
for this same shape; a new **Buzz** operation should still be an event kind.

## Desktop client

The desktop side (Phases 2–6) is gated behind the **`meetings` preview feature**.
It only appears when **both** are true:

1. The connected community relay advertises the `buzz-meetings` extension in its
   NIP-11 `supported_extensions` (i.e. the operator set `BUZZ_HIVETALK_API_ROOT`).
2. The user has enabled **Meetings** under Settings → Preview features.

If the relay does not advertise the capability, the Meetings screen shows an
"unavailable on this community" state and the channel **Start meeting** button is
hidden even with the preview flag on.

### Entry points

- **Sidebar → Meetings** — opens the Meetings screen: the room list for the
  current community plus a **Start a meeting** form.
- **Channel header → Start meeting** (video icon, next to Buzz Term) — non-DM
  channels only. Deep-links to `/#/meetings?room=<derived>&action=start` with the
  room name pre-derived from the channel (normalized name + short id suffix,
  clamped to 64 chars) and the start form focused. Joiners find the live room in
  the list rather than being auto-joined, so room registration stays host-driven.

### Hosting a meeting

1. Open the start form, confirm the room name, **Start meeting**.
2. The client signs a kind-27235 `create-room` event and calls
   `POST /meetings/register-room` through the relay proxy.
3. If you have no active HiveTalk subscription the call returns `402` and the
   **Subscribe** dialog opens:
   - plans are fetched from `GET /meetings/plans`;
   - picking a plan signs a `subscribe` event, posts `POST /meetings/subscribe`,
     and shows the BOLT11 invoice (copy button + QR + live expiry countdown).
     On `l402relay` that invoice arrives as an **L402 `402`** (with a
     `WWW-Authenticate: L402 macaroon=..., invoice=...` header) rather than the
     `201` the previous deployment sent; the body is the same intent, so
     `subscribe()` treats a `402` *carrying a `bolt11`* as success. A `402`
     without one — what `register-room` and `get-token` send — stays an error
     and routes to this dialog. The macaroon/preimage retry half of L402 is not
     implemented: `GET /api/payment/status` still settles the entitlement;
   - the client polls `GET /meetings/payment/status?id=<intent_id>` every ~3 s
     (read-only, not metered) until `settled`, then closes the dialog and
     **auto-retries** `register-room`.
4. On success the client mints a LiveKit token via `POST /meetings/get-token`
   and mounts the call view (`action=join`).

### In the call

- Camera / mic toggles, device pickers, leave.
- Host controls (shown when the caller holds the owner LiveKit JWT): lock room,
  mute-on-join, mute / kick a participant, promote / demote stage & moderator,
  audience mode. Moderation calls invalidate the **room lists only** — never the
  `["meetings"]` root — so a lock/kick/mute does not re-mint the host's own token
  and drop their call.

### Failure states the UI handles

| Upstream | UI |
|----------|-----|
| `402` + `bolt11` on `/subscribe` (L402) | invoice step — not an error |
| `402 subscription_required` / `subscription_expired` | Subscribe dialog |
| `403 room_not_registered` | "ephemeral rooms are dashboard-only" explainer |
| `401` | signature-freshness / nonce error, retryable |
| `409 pending_invoice` | re-show the existing invoice |
| `5xx` on the payment poll | give up after 5 consecutive failures |
| `503` | retry with backoff |

### Operator notes

- Nothing to configure on the desktop beyond the relay env var; the client
  feature-detects.
- The relay proxy adds no HiveTalk credential and stores no meeting state — see
  "No credential" above.
- The `meeting_actions_per_min` bucket (default 10/min per pubkey) covers
  subscribe / register-room / room-edit / room-delete / get-token / moderation.
  The invoice poll is deliberately excluded; raising the limit is rarely needed.

### Changelog

Release changelog entries are generated from the merged PR at release time; no
manual `CHANGELOG.md` edit is required for this feature.
