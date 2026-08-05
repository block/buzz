# Local relay host alignment (Windows)

When developing against a single-node compose relay on Windows, two separate
host problems often get mixed together. This note captures the working pattern
so desktop, CLI, and ACP agents stay on one tenant.

## Prefer IPv4 loopback for clients

On many Windows setups, `localhost` resolves to **IPv6** (`::1`) first. If the
relay (or Docker port publish) is only reliably reachable on **IPv4**, client
connects to `ws://localhost:3000` can hang while `ws://127.0.0.1:3000` succeeds
immediately.

**Recommendation for local Windows clients:**

| Client | Prefer |
|--------|--------|
| Desktop community **Relay URL** | `ws://127.0.0.1:3000` |
| `BUZZ_RELAY_URL` (CLI / ACP) | `ws://127.0.0.1:3000` |
| Health checks | `http://127.0.0.1:3000/health` or `/_liveness` |

Leave the API token empty unless you have created a real row in `api_tokens`.
NIP-42 auth is enough for a normal local closed-relay session.

## Tenant routing uses the WebSocket Host header

The multi-tenant relay maps the request **Host** to `communities.host`
(unique, case-insensitive). That string must match what the client puts in the
URL host (including port when non-default for the scheme as stored).

| Client URL | Host header | Required `communities.host` |
|------------|-------------|------------------------------|
| `ws://127.0.0.1:3000` | `127.0.0.1:3000` | `127.0.0.1:3000` |
| `ws://localhost:3000` | `localhost:3000` | `localhost:3000` |

If the client uses `127.0.0.1` but the row is still `localhost:3000`, the
upgrade fails with **HTTP 404** and a body along the lines of *no community is
configured for this host*.

### Fix (local compose Postgres)

```sql
UPDATE communities
SET host = '127.0.0.1:3000'
WHERE lower(host) = 'localhost:3000';
```

Then restart or recreate the relay container if needed. Verify:

```bash
# Expect 101 Switching Protocols when Host matches a community
curl -i -N \
  -H "Connection: Upgrade" \
  -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
  -H "Host: 127.0.0.1:3000" \
  "http://127.0.0.1:3000/"
```

## Compose / CORS

For local Tauri + Vite, include loopback origins used by the desktop shell in
`BUZZ_CORS_ORIGINS` (for example `http://127.0.0.1:1420`,
`http://localhost:57204`, `http://tauri.localhost`). Point `RELAY_URL` at the
same host form you store in `communities.host`.

See also [deploy/compose/README.md](../deploy/compose/README.md).

## ACP agents and channel replies

`buzz-acp` must connect with the same relay URL host form. For the agent to
**publish** channel messages (not only stream model text), set
`BUZZ_ACP_MCP_COMMAND` to a built `buzz-dev-mcp` binary so the model can run
`buzz messages send`. Mentions-only subscribe modes ignore posts without a `p`
tag for the bot.

## Checklist (desktop “can’t reach the relay”)

1. `curl -fsS http://127.0.0.1:3000/_liveness` (or `/health`) succeeds.
2. Desktop / CLI use `ws://127.0.0.1:3000` (not only `localhost`).
3. `SELECT host FROM communities;` matches that host:port.
4. Process shows ESTABLISHED TCP to `127.0.0.1:3000` (not stuck on `::1`).
5. Soft Nest “Access denied” alone does not mean the relay is down.
