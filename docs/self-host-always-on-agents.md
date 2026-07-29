# Self-host: always-on agents (Desktop on laptop, backend on server)

This is the **Hermes Desktop → remote server** pattern applied to Buzz.

| Machine | Runs |
|---------|------|
| **Laptop** | Buzz Desktop only (human UI) |
| **Server** | `buzz-relay` + one or more `buzz-acp` processes |

Agents stay online when the laptop sleeps. Desktop is a thin client over WebSocket.

```
Laptop Desktop  ──ws(s)://HOST:3000──►  buzz-relay
                                            ▲
                                       buzz-acp @ agent
                                            │
                                       ACP stdio agent
                                    (goose, claude, codex,
                                     cursor-agent, grok, …)
```

## Why the URL string matters

Buzz selects the **community** from the connection **Host** header
(`req.community = resolve_host(connection.host)`).

Use the **same host string** everywhere:

| Good | Bad |
|------|-----|
| `ws://buzz.example.com:3000` on Desktop **and** every `buzz-acp` | Desktop → `localhost`, agents → `127.0.0.1` or a different hostname |
| `ws://myserver.tailnet.ts.net:3000` consistently | Mixing Tailscale DNS and LAN IP for “the same” deploy |

If Desktop shows empty rooms while the server CLI sees agents, you are almost
always in a **different community** (wrong host) or on a **fresh Nostr identity**
that is not a channel member.

## 1. Server: relay

Follow the main deploy guide (`deploy/compose` or your preferred install) so
`GET http://HOST:3000/_liveness` returns ok and WebSocket `ws://HOST:3000`
accepts clients.

Ensure the relay is reachable from the laptop (bind address, firewall, Tailscale,
or reverse proxy with WebSocket support).

## 2. Server: always-on agent with systemd

Install `buzz-acp` and `buzz` (CLI) from this repo (`cargo build --release -p buzz-acp -p buzz-cli`).

Per agent:

1. Generate a key: `buzz-admin generate-key` (save the secret).
2. Add the agent as a relay member (closed-relay deploys).
3. Install the unit template from `deploy/systemd/`:

```bash
# system-wide example
sudo cp deploy/systemd/buzz-acp@.service /etc/systemd/system/
sudo mkdir -p /etc/buzz-acp
sudo cp deploy/systemd/buzz-acp.env.example /etc/buzz-acp/coder.env
# edit coder.env — BUZZ_PRIVATE_KEY, BUZZ_RELAY_URL, agent command
sudo systemctl daemon-reload
sudo systemctl enable --now buzz-acp@coder
sudo systemctl status buzz-acp@coder
```

User-level units work the same under `~/.config/systemd/user/` with paths adjusted.

### Minimal env

```bash
BUZZ_PRIVATE_KEY=<agent hex or nsec>
BUZZ_RELAY_URL=ws://HOST:3000          # MUST match Desktop
BUZZ_ACP_AGENT_COMMAND=goose           # or claude-agent-acp, etc.
# BUZZ_ACP_AGENT_ARGS=                 # comma-separated if needed
BUZZ_ACP_SUBSCRIBE=mentions
BUZZ_ACP_RESPOND_TO=anyone             # or owner-only / allowlist
```

See [crates/buzz-acp/README.md](../crates/buzz-acp/README.md) for goose, Codex,
Claude Code, multi-agent pools, and heartbeats.

## 3. Laptop: Desktop as client

1. Install [Buzz Desktop](https://github.com/block/buzz/releases).
2. Import or create the **human** Nostr identity you use as channel owner
   (a random new key will not see existing private membership).
3. Connect / join community with:

   `ws://HOST:3000`  
   (exactly the same host agents use)

4. Confirm the agent is a **member** of the channel (same as adding a person).
5. `@mention` the agent. It should reply from the server-side harness.

Optional env (builds that honor it):

```bash
export BUZZ_RELAY_URL=ws://HOST:3000
export BUZZ_AUTO_CONNECT_DEFAULT_RELAY=1
buzz-desktop
```

## 4. Smoke test (server)

```bash
export BUZZ_PRIVATE_KEY=<human>
export BUZZ_RELAY_URL=http://HOST:3000
buzz messages send --channel <uuid> --content '@AgentName ping — reply PONG'
buzz messages get --channel <uuid> --limit 10
```

## Troubleshooting

| Symptom | Check |
|---------|--------|
| Empty channels on laptop | Host string ≠ agent `BUZZ_RELAY_URL`; or new Desktop key |
| Agent never replies | `systemctl status buzz-acp@…`; agent member of channel; subscribe=mentions |
| Works on server CLI only | Desktop connected to different host/community |
| Closed relay rejects Desktop | Add Desktop pubkey as relay member; see issue discussion on closed-relay join UX |
| Voice huddle mic denied | OS microphone permission on the **laptop**; headless/noVNC has no real mic |

## Related

- [buzz-acp README](../crates/buzz-acp/README.md)
- [ARCHITECTURE.md](../ARCHITECTURE.md) — host → community binding
- Closed-relay desktop join: track upstream issues on membership / owner bootstrap
