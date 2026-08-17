# Buzz Hive — Merge Notes

> Branch: `feat/buzz-hive-p0` · Started: 2026-08-17

## Upstream references (no local vendored copies)

| Upstream | Buzz target |
|---|---|
| [simstudioai/sim](https://github.com/simstudioai/sim) | `crates/buzz-flow`, `desktop/src/features/flow-studio` |
| [Ngxba/claude-code-cli-ui](https://github.com/Ngxba/claude-code-cli-ui) | `crates/buzz-agent-studio`, `desktop/src/features/agent-studio` |
| [block/buzz](https://github.com/block/buzz) | core relay, auth, workflow engine |

Design specs: `docs/BUZZ_HIVE_MERGE_SPEC.md`, `docs/BUZZ_HIVE_IMPLEMENTATION_PLAN.md`.

## Nostr kind allocation

| Module | Range | Registry |
|---|---|---|
| Flow Studio | 46200–46399 | `crates/buzz-core/src/kind.rs` |
| Agent Studio | 47200–47399 | `crates/buzz-core/src/kind.rs` |

## Implementation status — complete (in-repo P0–P5)

| Phase | Status |
|---|---|
| P0 Skeleton | ✅ crates, kinds, migration 0032, ingest, NOTICE, docker pgvector |
| P1 Agent Studio | ✅ graph, skills, telemetry 47300, UI |
| P2 Flow Studio | ✅ canvas, YAML run, graph save/load, inline approval |
| P3 Knowledge/Tables/Files | ✅ projector, semantic + keyword search, CRUD panels, isolation test |
| P4 Cost monitor | ✅ `/agent-studio/costs` (ACP + Flow block rollup via kind 46201) |
| P5 Docs & smoke | ✅ VISION.md, E2E smoke (`hive-studio.spec.ts`), schema.sql parity |

## Ops-only (outside this repo)

- Fork `buzz-hive` git remote + release tag `v0.1.0-buzz-hive`
- Replace hash embeddings with a production embedding model when ready

## Key paths

```
crates/buzz-flow/
crates/buzz-agent-studio/
crates/buzz-relay/src/api/flow_studio.rs
crates/buzz-relay/src/api/agent_studio.rs
crates/buzz-db/src/flow_studio.rs
desktop/src/features/flow-studio/
desktop/src/features/agent-studio/
desktop/src-tauri/src/commands/hive_studio.rs
migrations/0032_buzz_hive_studio.sql
```
