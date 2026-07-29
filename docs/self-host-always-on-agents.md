# Self-host: always-on agents (any client → server backend)

Run **agents and the relay on a machine that stays on**. Connect from **any**
Buzz client — Desktop (macOS / Windows / Linux), web, mobile, or `buzz` CLI —
without tying agent lifetime to a single person’s laptop.

This is the thin-client / remote-backend layout (same idea as pointing a desktop
app at a remote server process).

| Role | Runs | Examples |
|------|------|----------|
| **Clients** (many) | Human UI only | Desktop on a laptop or workstation, web UI, mobile, `buzz` CLI, CI bots that are *not* coding agents |
| **Backend** (one or more hosts) | `buzz-relay` + `buzz-acp` (+ Postgres/Redis per deploy) | VPS, bare metal, homelab, k8s node |

Agents stay online when clients disconnect, sleep, or switch devices.

```
  Desktop ─┐
  Web     ─┼── ws(s)://HOST:3000 ──►  buzz-relay
  Mobile  ─┤                            ▲
  CLI     ─┘                       buzz-acp @ agent
                                        │
                                   ACP stdio agent
                                (goose, claude, codex,
                                 cursor-agent, grok, …)
```

Anyone on the community can @mention server-side agents. You do **not** need a
laptop specifically — only a client that speaks the relay URL.

## Why the URL string matters

Buzz selects the **community** from the connection **Host** header
(`req.community = resolve_host(connection.host)`).

Use the **same host string** on every client **and** every `buzz-acp`:

| Good | Bad |
|------|-----|
| `ws://buzz.example.com:3000` everywhere | One device uses `localhost`, agents use the public hostname |
| `ws://myserver.tailnet.ts.net:3000` consistently | Mixing Tailscale DNS and LAN IP for “the same” deploy |
| `wss://buzz.example.com` behind TLS terminator | Clients on `https://…` / `wss://…` while agents still on plain `ws://127.0.0.1` without a matching host map |

If a client shows empty rooms while the server CLI sees agents, you are almost
always in a **different community** (wrong host) or on a **fresh Nostr identity**
that is not a channel member.

## 1. Backend: relay

Follow the main deploy guide (`deploy/compose` or your preferred install) so
`GET http://HOST:3000/_liveness` returns ok and WebSocket `ws://HOST:3000`
(or `wss://…`) accepts clients.

Ensure the relay is reachable from every client network path you care about
(public DNS, Tailscale, LAN, reverse proxy with WebSocket support). Bind and
firewall so remote clients are intentional, not accidental open internet
without TLS/auth policy.

## 2. Backend: always-on agent with systemd

Install `buzz-acp` and `buzz` (CLI) from this repo
(`cargo build --release -p buzz-acp -p buzz-cli`).

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
BUZZ_RELAY_URL=ws://HOST:3000          # MUST match every client host string
BUZZ_ACP_AGENT_COMMAND=goose           # or claude-agent-acp, etc.
# BUZZ_ACP_AGENT_ARGS=                 # comma-separated if needed
BUZZ_ACP_SUBSCRIBE=mentions
BUZZ_ACP_RESPOND_TO=anyone             # or owner-only / allowlist
```

See [crates/buzz-acp/README.md](../crates/buzz-acp/README.md) for goose, Codex,
Claude Code, multi-agent pools, and heartbeats.

## 3. Clients: anyone joins the same community

### Buzz Desktop (any OS)

1. Install [Buzz Desktop](https://github.com/block/buzz/releases) on the machine
   people actually sit at (laptop, desktop workstation, etc.).
2. Use a Nostr identity that is a **member** of the channels you care about
   (a random new key will not see existing private membership).
3. Connect / join with the **same** relay URL the agents use:

   `ws://HOST:3000` or `wss://HOST`

4. Confirm each agent is a **channel member** (same as adding a person).
5. `@mention` the agent.

Optional env (builds that honor it):

```bash
export BUZZ_RELAY_URL=ws://HOST:3000
export BUZZ_AUTO_CONNECT_DEFAULT_RELAY=1
buzz-desktop
```

### Web / mobile

Point the client at the same community URL. Identity and membership rules are
unchanged: same host string, keys that are members of the right channels.

### CLI / automation

```bash
export BUZZ_PRIVATE_KEY=<human or bot>
export BUZZ_RELAY_URL=http://HOST:3000   # HTTP origin for REST; host must map to same community
buzz channels list
buzz messages send --channel <uuid> --content '@AgentName hello'
```

## 4. Smoke test (from any machine that can reach the relay)

```bash
export BUZZ_PRIVATE_KEY=<member key>
export BUZZ_RELAY_URL=http://HOST:3000
buzz messages send --channel <uuid> --content '@AgentName ping — reply PONG'
buzz messages get --channel <uuid> --limit 10
```

## Who this is for

| Audience | Use |
|----------|-----|
| Solo operator | Agents on a VPS; Desktop or CLI at home |
| Small team | Shared community URL; each human brings their own client |
| Homelab | Relay + agents on NAS/mini-PC; phones + laptops join via Tailscale |
| CI / ops | CLI and agents on servers; humans on Desktop/web |

Not laptop-specific. Laptop is only the common *example* of a client that should
not host production agents.

## Troubleshooting

| Symptom | Check |
|---------|--------|
| Empty channels on one device | That device’s host string ≠ agent `BUZZ_RELAY_URL`; or new key |
| Agent never replies | `systemctl status buzz-acp@…`; agent member of channel; subscribe=mentions |
| Works on server CLI only | Other clients connected to different host/community |
| Closed relay rejects a client | Add that client’s pubkey as relay member; improve join/invite UX upstream |
| Voice huddle mic denied | OS microphone permission on the **device running the UI**; remote desktop/noVNC often has no real mic |

## Related

- [buzz-acp README](../crates/buzz-acp/README.md)
- [ARCHITECTURE.md](../ARCHITECTURE.md) — host → community binding
- Closed-relay client join: track upstream issues on membership / owner bootstrap
