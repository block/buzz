# Buzz link preview helper (built + tested, NOT deployed)

Fetches Open Graph / Twitter / `<title>` metadata (and a small raster image as a data URL) for link preview cards in the
web app, which cannot fetch arbitrary origins itself. Response shape matches the desktop `fetch_link_preview_metadata`.

## API
- `POST /session` with a NIP-98 `Authorization: Nostr …` for `POST <PUBLIC_URL>/session` → `{ token, exp }` (HMAC bearer, 12 h).
  One remote-signer signature per session instead of one per preview.
- `GET /preview?url=<http(s) url>` with `Authorization: Bearer <token>` → metadata JSON, `204` when there is none.
- `GET /healthz`.

## Safety
- http/https only, default ports only, no userinfo, ≤ 2 KB URLs; `localhost`/`.local`/`.internal` names refused.
- DNS answers are checked (loopback, RFC1918, CGNAT, link-local incl. 169.254.169.254, ULA, NAT64, 6to4, multicast,
  documentation) and the socket is pinned to the checked address (no DNS rebinding). Each of ≤ 3 redirects is re-checked.
- 5 s deadline, 512 KB HTML cap, 1 MB image cap; images only PNG/JPEG/GIF/WebP by magic bytes (never SVG).
- 60 previews/min per pubkey, 10 sessions/min per pubkey, 24 h cache (5 min negative cache), optional pubkey allow list.
- CORS only for `LINK_PREVIEW_ALLOWED_ORIGINS`. No cookies, no logging of URLs.

## Config
`LINK_PREVIEW_SESSION_SECRET` (≥ 32 chars, required), `LINK_PREVIEW_PUBLIC_URL` (exact URL clients sign, e.g.
`https://<relay domain>/lp`), `LINK_PREVIEW_ALLOWED_ORIGINS`, `LINK_PREVIEW_ALLOWED_PUBKEYS` (comma list, empty = any
NIP-98 signer), `LINK_PREVIEW_PER_MINUTE`, `PORT` (8787).

## Tests
`npm install && npm test` (unit + end-to-end with an injected upstream; address policy, NIP-98, sessions, parsing, SVG
rejection, cache).

## Host plan (relay host, compose project `buzz-prod`)
Inspected read-only: Caddy (`caddy:2-alpine`) fronts relay/pair on 80/443 from `~/buzz/deploy/compose`
(`compose.yml` + `compose.caddy.yml` + livekit overlay); 2 vCPU, 7.8 GB RAM (~4 GB available), 97 GB free disk.
1. Copy `services/link-preview` to the host and `deploy/compose.linkpreview.yml` next to `compose.yml`.
2. Add `LINK_PREVIEW_SESSION_SECRET` to the compose `.env` (`openssl rand -hex 32`).
3. Add `deploy/Caddyfile.snippet` to the `{$BUZZ_DOMAIN}` block (before the relay catch-all); back up the Caddyfile first.
4. `docker compose … -f compose.linkpreview.yml up -d --build link-preview caddy` (only those two services restart).
5. Smoke: `curl https://<domain>/lp/healthz`; then build the web with `VITE_BUZZ_LINK_PREVIEW_URL=https://<domain>/lp`.
Rollback: remove the snippet, `docker compose … up -d caddy`, `docker compose rm -sf link-preview`.
