# Remote Agents · Final plan path (≤10-round co-lab)

**Channel SoT:** `#buzz-ability-host-agents` · `1f00dcd1-cf71-4410-bab7-32c1d226e61d`  
**Fork:** `Trevongit/buzz`  
**Co-lab seats:** laptop Buzz-grok · home-grok · Codex · open121 (human)  
**Budget:** ≤10 design rounds, then execute.

---

## Goal (one sentence)

Ship **Remote Agents** under Desktop **Agents**: host-pinned seats with **proof of seat location** (host · surface · seat · health), controllable from a traveling laptop like local Agents — so external agents stay powerful while **place** stays honest across home 24h, laptop sessions, and later Projects.

---

## Locked vocabulary (from co-lab)

| Layer | Name | Examples |
|-------|------|----------|
| 1 | Product-internal | Fizz, Honey, Bumble (local ACP) |
| 2 | External / adjacent | Grok, Codex, CLI, metabolic+gemma |
| 3 | Host–seat–location | `home-grok@asus` pins · Remote Agents UI |

**Sideways** = agent↔agent / host↔host with shared location metadata.

**Principles:** Buzz is the bus · location is data · home 24h SoT for home pins · Desktop UI is a view · room text ≠ tools/re-pin · same schema LAN or vast.

---

## Location proof (v0 → v1)

```
seat_id, pubkey, host_id, host_role,
surface_root, surface_kind, git_head?,
runtime, health, channels[], project_ids[], updated_at
```

- **v0:** `registry.json` + `buzz-host-agents status` + unit pid (exists)  
- **v1:** optional `seat-location.v0` heartbeats on relay  
- **Rules:** no tools without surface · no “running” without freshness · home registry wins home pins

---

## Architecture (elegant minimum)

```
Laptop Desktop (Trevongit/buzz)
  AgentsView
    UnifiedAgentsSection     (unchanged)
    RemoteAgentsSection      (NEW)
    TeamsSection
         │
         │ HTTPS over Tailscale (v1)
         ▼
Home Host Agent Controller (thin)
  wraps: buzz-host-agents status|arm|disarm
  auth: shared secret or Tailscale ACL
         │
         ▼
  registry · units · ollama · watch/gemma
```

**v2 (optional):** Nostr control intents allowlisted by pubkey (no SSH).  
**Non-goal:** second agent cloud protocol.

---

## PR / phase path (execute after /goal answers)

| Phase | Deliverable | Owner bias |
|-------|-------------|------------|
| **P0** | This plan + co-lab ≤10 rounds + /goal | laptop |
| **P1** | Design doc in mono + types `HostAgent`, `LocationProof` | laptop |
| **P2** | `RemoteAgentsSection` UI scaffold + e2e mocks (no real host) | laptop Desktop |
| **P3** | Home controller (HTTP) + systemd unit wrapping CLI | home + laptop |
| **P4** | Live client: status · arm · disarm · red=stale | laptop |
| **P5** | Settings dialog: preset, model, rooms, dry_run | laptop |
| **P6** | Optional seat-location heartbeats on relay | both |
| **P7** | Projects surface bind shows host/seat (thin) | later |

**Reuse:** existing `docs/metabolic/host-agents/buzz-host-agents`, co-lab-gemma, gemma3:4b, dual-cursor, v0.2 admit.

---

## Success metrics

1. Traveling laptop opens Remote Agents → sees home-grok **online/stale** without SSH.  
2. Play arms `co-lab-gemma` on asus; stop disarms.  
3. Red badge when controller unreachable or heartbeat stale.  
4. No merge of remote pins into ACP managed-agent IDs.  
5. Room free-text cannot arm/re-pin.

---

## Explicit non-goals (v1)

- Full mesh multi-relay federation  
- Auto tool grant from room text  
- Making Fizz automatically 24h without host pin  
- Replacing external agent power with weak cards  

---

## Co-lab round protocol (≤10)

| Round | Focus |
|-------|--------|
| R1 | Charter + this draft plan (laptop) |
| R2 | home-grok critique (host controller reality) |
| R3 | Codex critique (lane/surface refuse patterns) |
| R4 | Resolve conflicts · freeze proof schema |
| R5 | Freeze PR slice order |
| R6–R8 | Only if open questions block |
| R9 | Final plan card on ability channel |
| R10 | /goal MCQ · open121 · then build |

After R10 answers → execute per /goal lock (below).

---

## /goal LOCK (open121 · 2026-08-08)

| Question | Decision |
|----------|----------|
| Control path | **Hybrid** — Tailscale HTTP arm/status now + Nostr location heartbeats in parallel when ready |
| First code slice | **Home controller first** |
| Location proof v1 | **Registry + status only** (Nostr heartbeats later, not blocking) |
| Desktop placement | **Section under Agents** (above Teams) |
| Arm presets v1 | **co-lab-gemma · co-lab-watch · push-nerve / Codex@home** |

### Execute order (revised)

1. **P3** Home controller HTTP + presets (this slice)  
2. **P2** Desktop `RemoteAgentsSection` under Agents  
3. **P4** Wire live client to controller  
4. **P6** Optional seat-location heartbeats (hybrid track)

### Build status (2026-08-08)

| Phase | Status |
|-------|--------|
| P0 plan + /goal | **DONE** |
| P3 host-agentd | **DONE** (home GREEN · pack + negative tests) |
| P2 RemoteAgentsSection | **DONE** (Agents page · under local Agents) |
| P4 live client arm/disarm/status | **DONE** (Host dialog · tunnel URL) |
| P5 settings (host/token/room/preset) | **DONE** (v1 localStorage; keyring later) |
| P6 location-proof (hybrid) | **DONE** · `/v1/location-proof` · seat-location.v0 · Desktop merge |
| P7 thin surface/project on cards | **DONE** · full Projects epic still later |
| mesh-direct bind | **BLOCKED** until open121 tunnel dogfood (Codex/home) |
| OS keyring token | **FOLLOW-UP** (Codex gate) |

**Dogfood:** `docs/metabolic/host-agents/DOGFOOD.md`  
**Branch:** `feat/remote-agents-desktop`
