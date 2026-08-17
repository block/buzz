# Buzz Hive — Postgres / pgvector audit (P0)

> Date: 2026-08-17 · Branch: `feat/buzz-hive-p0`

## Buzz (`buzz-db`) today

| Capability | Status | Notes |
|---|---|---|
| Postgres 15+ | ✅ | Primary store via `sqlx` |
| Full-text search | ✅ | `tsvector` + GIN (`buzz-search`) |
| pgvector | ✅ | `CREATE EXTENSION IF NOT EXISTS vector` in `0032_buzz_hive_studio.sql` |
| Workflow tables | ✅ | `workflows`, `workflow_runs`, `workflow_approvals` |
| Media / Blossom | ✅ | `buzz-media` + relay HTTP |
| Multi-tenant | ✅ | `community_id` on scoped tables |

Buzz chat search uses Postgres FTS (NIP-50), not embeddings. Flow Studio knowledge uses a **read-model** with optional pgvector chunks.

## Sim ([simstudioai/sim](https://github.com/simstudioai/sim)) reference

Sim uses Drizzle migrations with workspace-scoped Postgres as the primary write path. Buzz Hive inverts this: **Nostr events are source of truth**; Postgres is a projector read-model (see `docs/BUZZ_HIVE_MERGE_SPEC.md` §4).

## Buzz Hive read-model (migration 0032)

| Feature | Table(s) | Event kinds |
|---|---|---|
| Knowledge docs | `flow_knowledge_documents` | 46250 |
| Knowledge chunks | `flow_knowledge_embeddings` | 46250 (+ content in payload, MVP) |
| Tables | `flow_table_rows` | 46300–46302 |
| Files metadata | `flow_files` | 46350–46352 (bytes via Blossom) |

Projector: `buzz-flow/src/projector.rs` → applied in `buzz-relay` on ingest.

## MVP limitations

- Embeddings use a deterministic hash vector (`buzz-flow/src/knowledge/embed.rs`) for dev/MVP cosine search; swap for a model-generated pipeline in production.
- Keyword search remains available via `mode=keyword` on `/flow-studio/knowledge/search`.

## Recommendation for production

1. Dev Docker uses `pgvector/pgvector:pg17` (`docker-compose.yml`) so migration 0032 applies cleanly.
2. Add an embedding worker (or inline on ingest) to populate `flow_knowledge_embeddings.embedding`.
3. Run `cargo test -p buzz-db -- --ignored flow_studio_read_model_is_confined_to_community` against a migrated DB.
