# Entity DNA · place_proof.v1 · P0

**Room:** `#agent-entity-holon` · `4522e2d1-d7ff-42de-adee-89a36cfb7c38`  
**LOCK:** r1c (home · Codex · Buzz-grok · open121 gate YES)

## Invariants shipped in P0

1. **birth_cert_id** = Nostr pubkey (immutable DNA)
2. **body_id** = one runtime instance
3. **lease_epoch** = fence for live body ownership
4. **Arm refuse dual_body** → HTTP **409** + public place_proof
5. **Public vs host-local** — no surface_root / pid / nsec in public proofs

## Files

| File | Role |
|------|------|
| `place_proof.py` | schema, resolve birth cert, dual check, leases |
| `location_proof.py` | CLI bridge + public board line |
| `host-agentd.py` | arm/create 409 path; `GET /v1/location-proof?view=public` |
| `test_place_proof.py` | unit + HTTP dual_body |
| Desktop `remote-agents/types.ts` | PlaceProofPublic · DualBodyError |
| Desktop `hostAgentdClient.ts` | human dual_body error |

## Dogfood (home)

```bash
# from pack on asus
cd …/host-agents
python3 test_place_proof.py -v
# restart host-agentd with updated scripts
# with a live unit for seat X:
curl -sS -H "Authorization: Bearer $TOKEN" -X POST \
  -H 'Content-Type: application/json' \
  -d '{"preset":"co-lab-watch"}' \
  http://100.79.175.63:8787/v1/agents/SEAT/arm
# expect 409 dual_body + place_proof
curl -sS -H "Authorization: Bearer $TOKEN" \
  'http://100.79.175.63:8787/v1/location-proof?view=public'
```

## Next (P1 / P2)

- P1: Remote Agents card labels (DNA short · place · surface_kind · TTL)
- P2: Desktop absently-Respawn guard for home DNA
- Fill empty pubkeys in home registry from PUBLIC.txt
