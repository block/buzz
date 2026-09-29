# Huddle screen share (desktop)

Screen-share-only video for Hula Huddles. Voice stays on the existing Opus
WebSocket path. One LiveKit SFU track carries the shared screen. Mobile keeps
audio; screen share is out of scope for mobile in this phase.

## Topology

```text
Nostr (lifecycle / membership / 48105 share events)
  │
  ├─ Opus WS  /huddle/{ephemeral}/audio   ← mic, agents, mobile (unchanged)
  │
  └─ LiveKit SFU (parallel)               ← ONE screen video track
       ↑
       short-lived JWT from buzz-relay
       POST /api/huddle/{ephemeral}/screen-token
```

- Huddle start/join/leave/end and participant presence stay on Nostr + Opus.
- Desktop clients that want video also mint a LiveKit token and join the SFU
  room derived from the ephemeral (backing) channel id.
- The SFU never replaces audio. Mic and agent TTS continue on Opus.

## Token mint flow

1. Desktop signs a NIP-98 `Authorization: Nostr …` header (same pattern as
   `/api/invites` and `/gifs/*`).
2. `POST /api/huddle/{channel_id}/screen-token` with JSON body:
   - `parent_channel_id` (required when the channel is ephemeral)
   - `intent`: `"subscribe"` (default) or `"publish"`
3. Relay binds tenant from `Host`, verifies NIP-98 + replay, enforces relay
   membership, then reuses the same huddle channel access rules as audio join
   (`ensure_membership`: archived reject, parent link for ephemeral, open /
   parent-member auto-add).
4. When LiveKit env is unset, respond `503` with
   `{"error":"screen_share_unavailable"}`. Desktop hides Share.
5. When set, mint a short-lived HS256 JWT (LiveKit claims):
   - `iss` = API key, `sub` = caller pubkey hex
   - `video.room` = `huddle-{channel_id}`
   - `video.roomJoin` + `canSubscribe` always
   - `canPublish` + `canPublishSources: ["screen_share"]` only for `publish`
6. Response: `{ "url", "token", "room", "can_publish", "current_sharer" }`.

`POST /api/huddle/{channel_id}/screen-stop` releases the relay-side sharer
slot when the caller is the current sharer (best-effort; clients also stop
local tracks on leave).

## One-sharer rule

Product rule: at most one active screen publisher per Huddle room.

- **Relay soft lock:** in-process map
  `(community_id, channel_id) → sharer_pubkey`. Publish mint claims the slot;
  another publisher gets `409` with `current_sharer`. Stop / leave clears it.
  Multi-pod deployments treat this as soft (per pod); document LiveKit room
  metadata / egress admin as a later hard lock if needed.
- **Client:** disable Share when `current_sharer` is another pubkey or a remote
  screen track is already subscribed. Local preview while sharing.

## Kind `48105` — screen share started / stopped

Gap between liveness (`48104`) and guidelines (`48106`).

- Regular event on the **parent** channel (same placement as other huddle
  lifecycle overlays).
- Content JSON: `{ "ephemeral_channel_id", "state": "started"|"stopped" }`.
- Tags: `["h", parent]`, `["p", sharer]` (optional echo of author).
- Desktop may publish on start/stop for timeline presence; audio join does not
  require it. Unknown clients ignore the kind.

## Desktop permissions

- `getDisplayMedia({ video: true, audio: false })` — no system audio in v1.
- `Info.plist`: `NSScreenCaptureUsageDescription` (keep existing mic + camera
  strings; camera remains for animated avatars only).
- macOS Screen Recording permission is OS-gated; first Share triggers the
  system prompt.

## Deploy (LiveKit + TURN + Caddy)

See `deploy/compose/livekit/` stubs and `.env.example` notes:

| Env | Purpose |
| --- | --- |
| `BUZZ_LIVEKIT_URL` | Client-facing LiveKit URL (`wss://livekit.example.com`) |
| `BUZZ_LIVEKIT_API_KEY` | API key (JWT `iss`) |
| `BUZZ_LIVEKIT_API_SECRET` | API secret (HS256). Redacted in config Debug |
| Partial set | Treated as unset → screen share unavailable |

Caddy terminates TLS for LiveKit WebSocket + TURN (UDP/TCP). Relay only mints
tokens; it does not proxy media.

## Phased rollout

1. **Dev:** local LiveKit (`livekit-server --dev`) + relay env → desktop Share.
2. **Staging:** LiveKit + coturn behind Caddy on the Hula host; one community.
3. **Prod:** pin image tags, TURN relay ports, monitor SFU CPU; keep Opus as
   voice source of truth.
4. **Later (non-goals now):** multi-sharer, camera tiles, mobile screen share,
   SFU-based audio migration, recording.

## Non-goals

- Camera participant tiles in this PR
- Changing `/huddle/{channel}/audio` wire format or mobile contract
  (`mobile/HUDDLES.md`)
- LiveKit Server SDK in the relay (JWT via `jsonwebtoken` only)
- Hard cross-pod publisher lock
- System / tab audio capture
