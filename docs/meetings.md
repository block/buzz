# Meetings (HiveTalk / LiveKit video)

Buzz "Meetings" adds live video conferencing to the desktop client using
[HiveTalk](https://premrelay.exe.xyz) for the control plane and LiveKit for
media. This document covers the **relay** side (Phase 1): an opt-in pass-through
proxy at `/meetings/*`.

## Enabling it

Set one environment variable on the relay:

```
BUZZ_HIVETALK_API_ROOT=https://premrelay.exe.xyz
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
   `subscribe_api`, `subscribe_url`, `message`). Public metadata endpoints
   (`plans`, `list-rooms`, `room-info`, `rooms-by-pubkey`, `auth/challenge`) are
   forwarded as-is with only a top-level array-length cap.

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
