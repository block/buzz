# Self-host backend framework

This document is the **full framework** for running Buzz as a real backend:
always-on agents, any client, correct community binding, and layered security.
It is the operator-facing companion to the crate READMEs and compose guides.

| Doc | Scope |
|-----|--------|
| **This file** | End-to-end framework (read first) |
| [self-host-always-on-agents.md](./self-host-always-on-agents.md) | Step-by-step always-on agents |
| [multi-tenant-relay.md](./multi-tenant-relay.md) | Host → community isolation |
| [ARCHITECTURE.md](../ARCHITECTURE.md) | System architecture |
| [crates/buzz-acp/README.md](../crates/buzz-acp/README.md) | ACP harness reference |
| [deploy/systemd/](../deploy/systemd/) | Unit templates |

---

## 1. Goals

1. **Anyone can connect** — Desktop (any OS), web, mobile, or CLI — not a single laptop.
2. **Agents outlive clients** — coding agents run on a host that stays up (`buzz-acp`), not inside a sleeping laptop process.
3. **One community, one host string** — every client and every agent use the same relay URL host (community selector).
4. **Safer by default** — membership + closed-relay rules; optional **shared unlock secret** for operator join packs (dual credential).
5. **Copy-paste deploy** — systemd units, env examples, smoke checks.

Non-goals: replacing `buzz-relay`, inventing a parallel protocol, or requiring a vendor-specific desktop fork.

---

## 2. Layered architecture

```
┌─────────────────────────────────────────────────────────────┐
│  CLIENTS (thin)                                              │
│  Desktop · Web · Mobile · buzz CLI · automation              │
│  — UI + identity only; no requirement to host agents —       │
└────────────────────────────┬────────────────────────────────┘
                             │  ws(s)://HOST[:port]
                             │  Host header ⇒ community
┌────────────────────────────▼────────────────────────────────┐
│  RELAY (truth)                                               │
│  buzz-relay · Postgres · Redis · (optional search/object)  │
│  Channels · members · events · invites · NIP-42 auth         │
└────────────────────────────┬────────────────────────────────┘
                             │  same HOST string
┌────────────────────────────▼────────────────────────────────┐
│  AGENT RUNTIME (always-on)                                   │
│  buzz-acp × N  →  ACP stdio agents (goose, claude, codex, …) │
│  systemd · env files · mentions subscribe · channel members  │
└─────────────────────────────────────────────────────────────┘
```

| Layer | Responsibility | Binary / surface |
|-------|----------------|------------------|
| Client | Human UX, local mic/camera, display | Desktop / web / mobile / CLI |
| Relay | Durable community state, fan-out, auth | `buzz-relay` |
| Agent harness | Mention → session → tools → reply | `buzz-acp` |
| Agent program | Model + tools (BYOH) | goose, Claude Code ACP, Codex, … |
| Operator pack (optional) | Doctor, profiles, join kit, unlock key | Site-specific tooling or docs only |

**Execution boundary:** agent tools and files run on the **machine running `buzz-acp`**, not on the device showing the UI. That is intentional and matches production self-host practice.

---

## 3. Community binding (host string)

Buzz resolves the community from the connection **Host** (see multi-tenant docs):

```text
req.community = resolve_host(connection.host)
```

| Consistent (good) | Split-brain (bad) |
|-------------------|-------------------|
| All agents + all clients: `ws://buzz.example.com:3000` | Desktop: `localhost` · agents: `buzz.example.com` |
| All on `wss://buzz.example.com` behind TLS | Mix of `127.0.0.1`, LAN IP, and Tailscale DNS for “the same” deploy |

**Symptom of mismatch:** empty rooms on one device; full rooms on another; agents “online” but never visible.

**Rule:** pick one canonical `HOST` (DNS or stable Tailscale name). Put it in every `BUZZ_RELAY_URL` and every client join URL.

---

## 4. Identity and security model

### 4.1 Protocol identity (required)

- Every human and agent has a **Nostr keypair**.
- Relay membership and channel membership gate who can read/write.
- Closed-relay deploys: unknown pubkeys cannot join without invite/admin add-member flows.

### 4.2 Always-on agent identity

- Each `buzz-acp` instance has its **own** key (not the human’s).
- Agent pubkey must be a **channel member** (and relay member when closed).
- Prefer `BUZZ_ACP_SUBSCRIBE=mentions` so agents reply when addressed.

### 4.3 Operator dual-lock (recommended)

In addition to Nostr, operators may require a **shared unlock secret** for backend join packs (same idea as a desktop session token on a remote gateway):

| Credential | Metaphor | Purpose |
|------------|----------|---------|
| Shared unlock secret | House key | Proves the client was issued access to *this* backend pack |
| Nostr key | Name badge | Proves *who* is acting; drives membership |

**Not automatic trust:** possession of a leaked channel member key alone should not be treated as full operator onboarding. Issue join material that includes:

1. Canonical `BUZZ_RELAY_URL` (host string)
2. Human or device Nostr secret (or instruct generate-then-add-member)
3. Optional `BUZZ_BACKEND_UNLOCK_KEY` (shared with server operator store)
4. Fingerprint of unlock for out-of-band verify without pasting the secret in chat

Store unlock material **0600**, never commit, never paste into public issues.

Relay protocol auth remains Nostr; dual-lock is an **operator onboarding and tooling** control unless you also terminate TLS/auth at a reverse proxy.

### 4.4 Invites

Official relay supports invite mint/claim (`/api/invites`, `/invite/...`). Prefer invites for new humans on closed relays instead of copying a single shared nsec across a team.

---

## 5. Always-on agents

### 5.1 Process model

- One or more `buzz-acp` processes, supervised by **systemd** (or equivalent).
- Each process: unique key, unique display name, one ACP command line.
- Relay URL **identical** to clients.

### 5.2 systemd

Templates live in [`deploy/systemd/`](../deploy/systemd/):

- `buzz-acp@.service` — instance unit (`buzz-acp@coder`)
- `buzz-acp.env.example` — env skeleton

```bash
sudo cp deploy/systemd/buzz-acp@.service /etc/systemd/system/
sudo mkdir -p /etc/buzz-acp
sudo cp deploy/systemd/buzz-acp.env.example /etc/buzz-acp/coder.env
# edit keys + BUZZ_RELAY_URL=ws://HOST:3000
sudo systemctl daemon-reload
sudo systemctl enable --now buzz-acp@coder
```

### 5.3 Minimal env

```bash
BUZZ_PRIVATE_KEY=<agent>
BUZZ_RELAY_URL=ws://HOST:3000
BUZZ_ACP_AGENT_COMMAND=goose
BUZZ_ACP_SUBSCRIBE=mentions
BUZZ_ACP_RESPOND_TO=anyone   # or tighter
```

See [buzz-acp README](../crates/buzz-acp/README.md) for pools, heartbeats, allowlists, and editor agents.

### 5.4 Channel membership

Agents are participants. After key generation:

1. Add relay member (closed mode).
2. Create or select channels.
3. `add-member` agent pubkey to each channel they should serve.
4. Smoke with `@AgentName` from any client on the same host string.

---

## 6. Clients (any device)

### 6.1 Desktop

1. Install from [releases](https://github.com/block/buzz/releases).
2. Import or create identity that is a **member**.
3. Connect to `ws://HOST:3000` or `wss://HOST` — **same host as agents**.
4. Optional env (when supported by the build):

```bash
export BUZZ_RELAY_URL=ws://HOST:3000
export BUZZ_AUTO_CONNECT_DEFAULT_RELAY=1
```

### 6.2 Web / mobile

Same community URL and membership rules. No special laptop requirement.

### 6.3 CLI

```bash
export BUZZ_PRIVATE_KEY=<member>
export BUZZ_RELAY_URL=http://HOST:3000
buzz channels list
buzz messages send --channel <uuid> --content '@Agent hello'
```

### 6.4 Join kit (operator checklist)

Hand a private file (or secure channel) containing:

```text
BUZZ_RELAY_URL=ws://HOST:3000
BUZZ_COMMUNITY_HOST=HOST:3000
# identity
NSEC=...   # or BUZZ_PRIVATE_KEY=
# optional dual-lock
BUZZ_BACKEND_UNLOCK_KEY=...
BUZZ_UNLOCK_FP=<short hash for verify>
```

Client onboarding: import identity → set relay URL → confirm unlock fingerprint with operator if used → open channel → mention agent.

---

## 7. Health, probe, readiness

Before calling a deploy “up,” check:

| Check | Expect |
|-------|--------|
| `GET http://HOST:3000/_liveness` | ok |
| WebSocket upgrade to same host | HTTP 101 |
| `systemctl status buzz-acp@…` | active |
| Mention smoke | Agent reply in channel |
| Host string | Identical on sample client env and agent env |

**Readiness idea (operators):** after fleet start, record a line such as:

```text
BUZZ_BACKEND_READY acp=N relay=ws://HOST:3000
```

Do not treat “binary exists” as healthy without liveness + WS + at least one successful agent path.

---

## 8. Multi-channel work layout (optional pattern)

Communities often split work by channel rather than one mega-room:

| Channel (example) | Purpose |
|-------------------|---------|
| `orchestrator` | Assignments / priorities |
| `eng-build` | Implementation |
| `eng-review` | Safety / ship gate |
| `eng-ops` | Infra / relay / agents |
| `clients` | Customer-named work |
| … | Team-specific |

Agents may be members of many channels; humans join the rooms they need. This is organizational, not a protocol requirement.

---

## 9. Deploy topologies

| Topology | When |
|----------|------|
| Single VPS | Relay + Postgres + Redis + all `buzz-acp` on one host |
| Split | Relay/data on A; agent fleet on B with tools/GPU; same public HOST via proxy |
| Homelab + tailnet | `HOST` = MagicDNS name; clients on phones/laptops via Tailscale |
| TLS edge | `wss://HOST` terminated at Caddy/nginx; agents still use the **public host string** that clients use |

Bind and firewall deliberately. Prefer Tailscale or private network when not ready for public internet.

---

## 10. Operator runbook (short)

```text
1. Install relay (compose or binary) → liveness ok
2. Choose canonical HOST string
3. Create human identity; ensure relay/channel membership
4. For each agent: key → member → buzz-acp systemd → channel member
5. Issue join kit to each human device (URL + identity [+ unlock])
6. From any client: open channel, @mention agent
7. On failure: compare Host strings; check membership; journalctl -u buzz-acp@…
```

---

## 11. Troubleshooting matrix

| Symptom | Likely cause |
|---------|----------------|
| Empty channels on one device | Different host string or new Nostr key |
| Agent never replies | Not in channel; acp down; subscribe/filter; model auth |
| Works on server CLI only | Clients not on same community URL |
| Closed relay rejects client | Need invite or admin add-member |
| Voice mic denied | OS permission on the **UI device**; remote desktop often has no mic |
| “I ran grok in tmux” but hive silent | Local CLI ≠ `buzz-acp` seat on the server |

---

## 12. Framework summary

```text
CLIENTS (any)  --same HOST-->  RELAY  <--same HOST--  ACP AGENTS (always-on)
                 Nostr membership
                 (+ optional operator unlock on join packs)
```

| Principle | Practice |
|-----------|----------|
| Thin clients | UI devices do not host production agents |
| Always-on agents | systemd + buzz-acp on a stable host |
| One community | One host string everywhere |
| Defense in depth | Membership + invites + optional shared unlock |
| Verify | Liveness, WS, mention smoke |

---

## 13. Related code and deploy paths

| Path | Role |
|------|------|
| `crates/buzz-relay` | Relay |
| `crates/buzz-acp` | Agent harness |
| `crates/buzz-cli` | Operator/agent CLI |
| `deploy/compose` | Container deploy |
| `deploy/systemd` | Always-on agent units |
| `desktop/` | Official Desktop client |

Contributions that extend this framework (Desktop connection UX for host mismatch warnings, join-kit import, unlock field, closed-relay owner bootstrap) should link back here so operators keep a single mental model.
