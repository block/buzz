# LiveKit for Huddle screen share

Voice stays on buzz-relay Opus (`/huddle/{channel}/audio`). LiveKit carries
**one screen video track** per Huddle. See `docs/huddle-screen-share.md`.

## Local try

```bash
# Terminal A — LiveKit dev server (built-in key/secret printed on start)
docker run --rm -p 7880:7880 -p 7881:7881 -p 7882:7882/udp \
  livekit/livekit-server --dev

# Terminal B — relay with matching env (dev defaults from --dev):
#   API key:    APIdevkey (or as printed)
#   API secret: as printed by livekit-server --dev
export BUZZ_LIVEKIT_URL=ws://127.0.0.1:7880
export BUZZ_LIVEKIT_API_KEY=devkey   # use values printed by --dev
export BUZZ_LIVEKIT_API_SECRET=secret
# start buzz-relay as usual
```

In desktop Huddle, Share appears once the relay mints tokens successfully.

## EC2 / compose host

1. Copy this directory next to `deploy/compose`.
2. Set in `.env`:

```bash
BUZZ_LIVEKIT_URL=wss://livekit.example.com
BUZZ_LIVEKIT_API_KEY=APIxxxxxxxx
BUZZ_LIVEKIT_API_SECRET=long-random-secret
BUZZ_TURN_USER=buzz
BUZZ_TURN_PASS=long-random-turn-pass
BUZZ_TURN_REALM=turn.example.com
BUZZ_TURN_EXTERNAL_IP=<public ipv4>
```

3. Merge `Caddyfile.snippet` into the site Caddyfile (or run a second site).
4. Open firewall: TCP 443 (Caddy), UDP/TCP 3478 (TURN), UDP 7882 (RTC) as needed.
5. Restart relay so it loads `BUZZ_LIVEKIT_*`.

## Remaining manual ops

- Generate strong API key/secret; do not use `--dev` keys in prod.
- Point DNS `livekit.` at the host; confirm WSS upgrade works.
- Validate TURN with a NATed client (share appears for remote peer).
- Monitor LiveKit CPU when several Huddles share large screens.
