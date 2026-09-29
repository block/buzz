# ORBIT Build Overrides — Current Architecture Has Priority

> This document resolves legacy implementation details that remain inside preserved feature documents. It does not delete those source details; it tells the build agent how to interpret them under the current ORBIT architecture.

## Priority order

When documents appear to disagree, use this order:

1. `AGENTS.md`
2. this `BUILD_OVERRIDES.md`
3. `docs/architecture/ORBIT_STORAGE_ARCHITECTURE_DECISION_2026-09-29.md`
4. the deployment-specific architecture document
5. feature `IMPLEMENTATION.md`
6. feature `CHECKLIST.md`
7. reference/research material

## Storage overrides

### Local V1

- PostgreSQL/pgvector references are **not** local runtime requirements.
- Dense vector retrieval uses the `VectorStore` abstraction backed by local LanceDB.
- Lexical retrieval uses SQLite FTS5/BM25 behind the lexical/search abstraction.
- Graph retrieval uses `GraphStore` backed by the embedded local graph store.
- SQL/DB-specific types must not leak into retrieval/business crates.
- A `pool` argument in preserved search examples means the storage/retrieval context or repository abstraction, not a mandatory PostgreSQL connection pool.

### Hosted / enterprise

- PostgreSQL + pgvector remain valid hosted storage choices.
- Hosted graph tables are the initial graph implementation; a dedicated graph service is optional and must be justified by measured workload.
- Hosted data is authoritative for synchronized logical state; device indexes remain rebuildable derived state.

## Retrieval overrides

If a preserved feature document says:

- `pgvector <=> cosine` -> implement through `VectorStore.search_dense`.
- `GIN/ts_rank_cd/search_tsv` -> implement through SQLite FTS5/BM25 abstraction locally.
- PostgreSQL recursive graph CTE -> implement through `GraphStore` traversal locally.
- PostgreSQL JSONB -> use the logical metadata model serialized through the local store's supported JSON representation.

The algorithmic intent remains the same: dense + lexical + graph candidates -> RRF -> context-aware reranking -> temporal/permission filters -> context packing.

## Feature-specific interpretation

### F03 — Ingestion

Git/source metadata belongs in the shared document metadata model. Do not require a PostgreSQL JSONB column for local V1.

### F04 — SuperRAG

Keep the existing retrieval/ranking behavior and formulas, but route each retrieval source through storage traits. Do not introduce a PostgreSQL database just to satisfy the preserved pseudocode/checklist wording.

### F05 — Knowledge Graph

Neo4j and FalkorDB are optional adapters/reference alternatives, not local V1 dependencies.

### F10 — Packaging

Any PostgreSQL/pgvector mention is for team/hosted/relay deployment only. A local desktop release must not start a database server.

### F11 — Hosted Enterprise

PostgreSQL/pgvector are valid here because F11 is the hosted data plane. Keep tenant isolation and policy enforcement ahead of retrieval.

### F12 / F13 — Identity, Sync & Governance

Identity, device metadata, subscription state, sync events, and enterprise policies are control-plane/workspace data. They are not long-term memory content unless explicitly imported by a user/workspace policy.

## External-reference rule

The architecture package can contain research citations for historical context, but an implementation agent must not browse or adopt an external design merely because a preserved document contains a citation. Use the repository specification as the build source of truth.
