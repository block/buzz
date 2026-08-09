# Core handoff · Entity DNA · place-safe bodies · presence with place

**Audience:** block/buzz core maintainers  
**Branch dogfood:** `feat/remote-agents-desktop` (Trevongit) + home host-agentd  
**Room SoT:** `#agent-entity-holon`  
**Status:** R0–R5 implemented on fork; ready for phased upstream PR stack  

---

## Why this belongs in core

Multi-machine Buzz already creates **clone-body confusion** (same face, two processes, no place). Community reports and VISION_REMOTE_AGENTS agree: **same DNA, one live body** (resurrection, not silent dual). Formal `docs/remote-agents.md` already states **presence-is-status** and **at-most-one-live** — the product UI and host launchers did not fully enforce them for local Desktop + headless home.

This work is a **force multiplier**: fail-safe by design, privacy-shaped, additive, no second agent cloud.

---

## Vocabulary (LOCK)

| Term | Meaning |
|------|---------|
| **birth_cert / DNA** | Immutable entity id = Nostr **pubkey** (v0) |
| **legal_name / face** | Display name + avatar (collisions OK) |
| **body** | One live process instance (`body_id` · `lease_epoch`) |
| **place** | `host_id` · `host_role` · `surface_kind` · `surface_id` |
| **surface_root** | Worktree path — **host-local only**, never room/UI public |
| **adopt / transfer / fork** | Attach · drain+epoch · **new** DNA |
| **refuse** | Second live body → hard fail |

**Anti-vocab:** “the Fizz”, “same agent” without DNA, “online” without place when place is known.

---

## Invariants (map to existing docs)

| Id | Invariant | Upstream rhyme |
|----|-----------|----------------|
| I1 | Birth cert immutable | keypair identity |
| I2 | At most one live body per DNA per scope | remote-agents I4 |
| I3 | Presence without place is incomplete for multi-host UI | I3 presence-is-status |
| I4 | Place-scoped Start ≠ remote adopt | VISION “new body” |
| I5 | nsec never in room / Remote Agents payloads | #4666 redaction |
| I6 | Room text never re-pins DNA or grants tools | metabolic guards |
| I7 | Buzz is the bus — proofs ride events + thin host controller | VISION_REMOTE_AGENTS |

---

## Public schema (place_proof.v1)

```json
{
  "schema": "place_proof.v1",
  "birth_cert_id": "<pubkey hex>",
  "body_id": "<instance id>",
  "host_id": "asus-g501vw",
  "host_role": "home",
  "surface_kind": "desktop-local|cli-seat|host-unit|remote-view",
  "surface_id": "bind:…",
  "health": "ok|degraded|stale|down",
  "lease_epoch": 1,
  "issued_at": 0,
  "expires_at": 0,
  "attestation": "host-local-v0"
}
```

**Never public:** `surface_root`, `unit_pid`, nsec, tokens, controller secrets.

---

## What shipped on the fork (by round)

| Round | Deliverable |
|-------|-------------|
| **R0** | host-agentd dual_body **409** · leases · public location-proof · tests · dogfood GREEN on home |
| **R1** | Remote Agents cards: DNA short · body · place · Arm=Live when body up · no paths |
| **R2** | Desktop Start/Respawn refuse when presence online/away elsewhere |
| **R3** | Self-location env + system-prompt block (Desktop spawn + host unit inject) |
| **R4** | Mobile presence **snapshot on track** · Desktop place labels / place-aware dual messages |
| **R5** | This handoff + PR stack |

Key paths:

- `docs/metabolic/host-agents/place_proof.py`, `host-agentd.py`, `ENTITY_HOLON_PLAN.md`
- `desktop/src/features/remote-agents/*`
- `desktop/src/features/agents/lib/managedAgentControlActions.ts` (`refuseDualBodyIfPresentElsewhere`)
- `desktop/src-tauri/src/managed_agents/self_location.rs`
- `mobile/lib/features/profile/presence_cache_provider.dart` (snapshot)
- `desktop/src/features/presence/lib/presencePlace.ts`

---

## Suggested upstream PR stack (small, reviewable)

1. **docs:** vocabulary + invariants + place_proof.v1 (this file trimmed into `docs/`)  
2. **host-agentd / metabolic pack** (or equivalent): dual_body 409 + public proof (optional until host feature lands)  
3. **desktop:** Remote Agents place cards + dual_body error UX  
4. **desktop:** `refuseDualBodyIfPresentElsewhere` on local start (aligns #2857)  
5. **desktop:** self_location inject on spawn  
6. **mobile:** presence snapshot on track (aligns #4417 / #4394)  

Each PR independent; no second protocol.

---

## Relation to open core PRs

| PR | Relationship |
|----|----------------|
| #5138 presence liveness for remote | Complementary — we use presence for **local** dual refuse |
| #2857 avoid duplicate starts | Same spirit; we generalize to presence preflight on Respawn |
| #4417 mobile presence snapshot | We implement the same product fix on this branch |
| #4666 secret redaction | Our public place_proof is the host-side twin |

---

## Non-goals (keep out of first core landings)

- Second global agent bus  
- UUID birth cert / key rotation  
- Weakening external harness power  
- Auto tool grant from room text  

---

## Dogfood evidence

- Home asus: `PLACE_PROOF_P0_OK` · `409 dual_body` · `R3_PROMPT_PUBLIC_OK`  
- Co-lab channel: `#agent-entity-holon` design freeze (home · Codex · open121)

---

## One-liner for core

> **Treat agent identity as DNA (pubkey), bodies as place-bound instances with leases, presence as status (and place when known), and refuse silent dual-spawn — with public proofs that never leak home paths.**

`core-handoff · entity-holon · force-multiplier · Buzz is the bus`
