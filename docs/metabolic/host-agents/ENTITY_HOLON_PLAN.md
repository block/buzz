# Entity Holon · multi-round development plan

**Room:** `#agent-entity-holon` · branch `feat/remote-agents-desktop`  
**Thesis:** Birth cert (DNA) ≠ face ≠ body ≠ place. Privacy-shaped proofs. Core-adoptable force multiplier.

## What we learned (repo + upstream)

| Source | Lesson for us |
|--------|----------------|
| **VISION_REMOTE_AGENTS** | Same key, **new body** = resurrection; not dual clone. At-most-one live; relay is tether |
| **docs/remote-agents.md** | I3 presence-is-status · I4 at-most-one · no secret in config |
| **#5138** (open) | Remote liveness from **presence**, not `backend_agent_id` bookkeeping |
| **#2857** (open) | Before local start, check presence — fail closed if same DNA already writing |
| **#4417 / #4394** (open) | Mobile needs **presence snapshot** on track, not “offline until next heartbeat” |
| **#4666** | Redact secrets from deploy payloads — same scrub discipline as place_proof public |
| **Our P0 (GREEN on asus)** | `409 dual_body` · birth_cert=pubkey · lease_epoch · public vs host-local |

**Align wording with core:** “same agent, new body” = **transfer/adopt/fork**, never silent dual.

## Architecture target (complete elegant system)

```
Face (display name)     soft, collides OK
  └── DNA (pubkey)      birth_cert · immutable
        └── Body        body_id · lease_epoch · one live default
              └── Place host_id · host_role · surface_kind · surface_id (public)
                        surface_root (host-local only)
Presence                online|away|offline + place when known
Launchers               Desktop ACP · host-agentd · provider (K8s/SSH)
                        all honor dual refuse / presence preflight
```

## Rounds (force-multiplier verticals)

### Round 0 — P0 host refuse ✅ DONE (dogfood GREEN)
- place_proof.v1 · dual_body 409 · leases · public redaction · tests

### Round 1 — P1 Remote Agents UI (this implement slice)
- Cards show: DNA short · body_id · surface_kind · surface_id · host · health  
- **Never** render full `surface_root` in multi-user UI  
- Prefer `location-proof?view=public`  
- Arm disabled / “already live” when body online (don’t invite dual)  
- dual_body error already humanized in client  

### Round 2 — P2 Desktop Absently-Respawn guard
- `startManagedAgentWithRules` / Respawn: if presence online for pubkey → refuse or confirm “live elsewhere”  
- Align with upstream #2857 spirit (presence preflight)  
- Copy: “Start on **this computer**” vs bare Respawn for home-named seats  

### Round 3 — Self-location injection ✅ (home GREEN + laptop Desktop spawn)
- ACP / host unit env: `BUZZ_HOST_ID` · `BUZZ_HOST_ROLE` · `BUZZ_SURFACE_ID` · birth_cert · body_id  
- System prompt block once (token-wise); PLACE_PROMPT.txt public-only  
- Desktop: `self_location.rs` inject after user env on spawn  
- Host: `location_proof.py --inject-seat` + arm sources self-location.env  
- Fork = new key; adopt = no second process  

### Round 4 — Presence with place ✅
- Desktop: `presencePlace.ts` · dual-body messages can include host/role/surface  
- Mobile: presence **snapshot on track** via `POST /query` kind:20001 authors ( #4417 spirit)  
- Live events win over older snapshots (created_at fence)  
- Optional relay seat-location heartbeats still later (not blocking)  

### Round 5 — Core handoff packaging ✅
- `CORE_HANDOFF_ENTITY_HOLON.md` — vocabulary · I1–I7 · schema · PR stack  
- Maps to VISION_REMOTE_AGENTS + remote-agents.md + open PRs #2857/#4417/#5138/#4666  

## Success metrics

1. Cannot silently dual-arm same DNA on home (409 + UI)  
2. Laptop cannot believe “Respawn = continue home workspace” without guard  
3. Public proofs never leak paths/secrets  
4. Card always shows **where** and **which DNA**  
5. Core reviewer can map every piece to VISION_REMOTE_AGENTS + remote-agents.md  

## Non-goals (still)

- Second agent cloud protocol  
- Key rotation / UUID birth certs (v0 = pubkey)  
- Weakening external harness power  

## Execute now

**Round 1** on this branch → then Round 2 if time in session.
