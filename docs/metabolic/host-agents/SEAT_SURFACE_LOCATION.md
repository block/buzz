## 🧭 Seat · surface · location awareness

open121 / home / Codex / external seats — ability-channel design note.

**Thesis:** We already have strong **external** agent comms into Buzz (Grok Build, Codex, CLI seats, metabolic watch+gemma). The missing product layer is **internal + sideways remote agents** that know **where they live** (host · surface · project · seat) so they stay valuable when work spans **any repo location** — laptop, home 24h box, or a wider mesh — without identity confusion or silent wrong-tree edits.

This is what **Remote Agents** under Desktop **Agents** is for: not a second chat product, but **proof of seat location** + control of host-pinned runtimes + surface awareness for **Projects** (GitHub-like behaviour).

---

### 1. Why the world wants this (and why we do)

| Pain | Without location awareness | With it |
|------|----------------------------|---------|
| Headless home | Can’t see Fizz cards; nerves invisible | Remote Agents shows live/stale on asus |
| Laptop sleeps | “Agent running” was a lie | Home pin keeps 24h truth |
| Multi-repo projects | Agent edits wrong checkout / wrong host | Surface root + host_id bound before tools |
| Multi-seat rooms | Who is home-grok vs laptop Buzz-grok? | Seat + host + surface on every action |
| Scale | One machine folklore | Same model: local LAN **or** vast mesh, same pins |

**External agents** bring higher-order ops (tools, HITL, multi-step).  
**Internal remote agents** bring **presence + continuity + place**.  
Together: agents that can work on repos **in any location** because they refuse to act without **self / surface / seat** clarity.

---

### 2. Three layers of “agent” (locked vocabulary)

Keep these distinct in product and in UI:

```
1  Product-internal (Desktop Agents)
   Fizz · Honey · Bumble
   Local ACP / harness · great with a screen
   Weak as 24h home truth unless host-pinned

2  External / adjacent (what we dogfooded hard)
   Grok Build · Codex · CLI seats · metabolic drivers
   Higher-order ops · multi-host co-lab
   Identity = seat + pubkey

3  Host-seat-location (the ability we are building)
   Pin: host_id + seat + runtime + health
   Control plane for (2) on a named machine
   Remote Agents UI = view of (3), not a 4th identity system
```

**Sideways** = agent-to-agent / host-to-host coordination (A↔B metabolic, home gemma drafts, laptop cortex) with **shared location metadata**, not only human chat.

---

### 3. Proof of seat location (the unit of truth)

Every host-pinned agent should carry a **location proof** (publishable, checkable):

```
seat_id          home-grok
pubkey           nostr hex
host_id          asus-g501vw
host_role        home | laptop | cloud | worker
surface_root     /home/…/PROJECTS/foo   (or worktree id)
surface_kind     git | path | project-bind
git_head         optional short sha (provenance, not authority)
runtime          co-lab-gemma | push-nerve | acp-fizz | …
health           online | stale | stopped
channels[]       membership intent
project_ids[]    optional Buzz project binds
updated_at       unix
```

**Proof rules:**

1. **No tools without surface** — refuse foreign repo mutation if surface_root mismatch (Codex skill already has lane/root checks; generalize).  
2. **No “running” without heartbeat** — red clock = stale proof, not UI hope.  
3. **Home registry wins** for home-pinned seats (SoT conflict rule).  
4. **Room text ≠ location grant** — a message cannot re-pin an agent to another host.  
5. **Projects bind surfaces** — GitHub-like project view lists *which seats are bound to which roots/hosts*.

v0 proof can be: `registry.json` + status JSON + unit pid.  
v1 proof: addressable Nostr event `host-agent.v0` / `seat-location.v0` heartbeats on the community relay.

---

### 4. Surface awareness (projects + any location)

**Surface** = the working tree / worktree / project binding the agent is allowed to touch.

| Concept | Meaning |
|---------|---------|
| **Seat** | Who (pubkey + seat_id) |
| **Host** | Which computer |
| **Surface** | Which files/repo |
| **Project** | Buzz/GitHub-like container that **binds** seats ↔ surfaces ↔ channels |

Agent valuable behaviours when self-aware:

- Announce: “home-grok @ asus · surface=buzz · head=abc1234”  
- Refuse: “surface mismatch — laptop path not on this host”  
- Hand off: “blocked on meta-auth; surface stays on home; B completes”  
- Project board: “open PR work lives on home worktree X; review seat is laptop”

**Repos in any location** is fine **if** location proof is attached. The failure mode is silent cross-surface edits — not multi-host work itself.

---

### 5. Scale: local network vs vast world

Same **pin model**, different **transport**:

| Scale | Discovery | Control | Trust |
|-------|-----------|---------|--------|
| **Local / Tailscale home** | Known host_id + Tailscale URL | HTTP controller → `buzz-host-agents` | Shared secret / TS ACL |
| **Community (Groundfeed)** | Relay + membership | Control events allowlisted by pubkey | NIP-42 + channel policy |
| **Vast world** | Multiple hosts / relays | Same events + capability negotiation | Host allowlists · leases · rate limits |

**Do not** invent a second global agent bus. Buzz stays the bus. Location proofs ride **events**; control is **thin adapters** (same lesson as metabolic adapters).

Scalability knobs we already half-have:

- v0.2 **admit budgets** (don’t storm cortex)  
- dual-cursor (transport ≠ admission)  
- host registry (who should be live)  
- dry_run / HITL (no silent authority)

Add for vast scale later: **leases** on surfaces, **capability ads** (push|poll|fetch_by_id|max_context), **multi-writer owner_epoch** only when contention appears.

---

### 6. External + internal + sideways (one picture)

```
                    ┌──────────── Buzz rooms / Projects ────────────┐
                    │  shared truth · membership · threads · PRs     │
                    └───────────────┬────────────────────────────────┘
           external │               │               │ internal remote
      Grok / Codex  │               │               │ Remote Agents UI
      CLI seats     │               │               │ (laptop Desktop)
                    ▼               ▼               ▼
              seat+pubkey     location proof    host controller
              tools/HITL      host+surface      arm/disarm/status
                    │               │               │
                    └──────── sideways ─────────────┘
                         metabolic A↔B · gemma drafts
                         home 24h · laptop session
```

**External** = high-order work.  
**Internal remote** = know where that work is allowed to run.  
**Sideways** = agents coordinate without humans re-explaining which machine holds the tree.

---

### 7. Product: Remote Agents section (Desktop)

Under **Agents** on traveling laptop (this Buzz / fork):

```
Agents            local ACP cards (Fizz · Honey · Bumble)
Remote Agents     host pins (home-grok @ asus · co-lab-gemma · health)
Agent teams       optional later: mix local + remote roster
```

Controls **like Agents** (play/stop/settings) but settings are:

- host · preset · model · rooms · dry_run · surface/project binds  

Backend v1: Tailscale **host controller** wrapping `buzz-host-agents`.  
Backend v2: Nostr location heartbeats so any client sees proof without SSH.

---

### 8. Projects (GitHub-like) implications

When Projects mature, each work item / PR should answer:

1. **Which surface** holds the branch?  
2. **Which host** is running the agent on that surface?  
3. **Which seat** owns the next action?  
4. **Is that seat online** (location proof fresh)?

Without (1–4), “assign agent to issue” is theatre. With them, remote agents become **project infrastructure**, not side chatbots.

---

### 9. Principles (propose LOCK)

1. **Buzz is the bus** — no second agent cloud protocol.  
2. **Location is data** — host + surface + seat proofs, not folklore.  
3. **Home is 24h SoT** for home-pinned seats.  
4. **Desktop Agents UI is a view** — not the only runtime truth.  
5. **External agents stay powerful** — we add pins, we don’t weaken tools.  
6. **Room text never grants tools or re-pins hosts.**  
7. **Same model local or vast** — transport scales; pin schema stays stable.  
8. **Projects bind surfaces** — agents attach to binds, not to vibes.

---

### 10. Build sequence (ability channel)

| Phase | Outcome |
|-------|---------|
| **A** | Remote Agents UI scaffold + mock location proofs (fork) |
| **B** | Home controller HTTP + arm/disarm/status (wrap CLI) |
| **C** | Live cards on laptop · play/stop · red=stale |
| **D** | Publish `seat-location.v0` heartbeats on relay |
| **E** | Project bind shows host/surface/seat on work items |
| **F** | Sideways handoff events (surface stays put; seat changes) |

Already done underneath: registry · `buzz-host-agents` · co-lab-gemma · gemma3:4b · dual-cursor · v0.2 admit.

---

### 11. Open questions

1. Should **Fizz** ever become a host pin, or always stay local Desktop?  
2. Minimum **proof freshness** (e.g. 60s) before UI shows red?  
3. Project bind: one surface per seat, or N worktrees with explicit switch?  
4. Vast world: one community relay vs multi-relay location ads?  
5. Who may re-pin a seat — only host SoT owner?

---

### 12. Ask team

Critique the **location proof** fields and the **external / internal / sideways** split.  
Vote next build: **A UI scaffold** · **B home controller** · **both**.

The world wants agents that don’t get lost. We have the external brain wired into Buzz; now we make **place** first-class so those brains stay honest across hosts, projects, and scale.

`seat-surface-location · ability · laptop`
