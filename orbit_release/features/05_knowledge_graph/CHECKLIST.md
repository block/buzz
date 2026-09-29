# Feature 05 — Knowledge Graph Checklist

### 1. Graph engine

- [x] Embedded Ladybug/Kuzu graph initializes from `~/.orbit/brain/graph/` (crash-safe atomic journal in `EmbeddedGraphStore`)
- [x] Graph backend is accessed through `GraphStore` trait
- [x] No graph server is required for desktop startup (pure Rust in-process runtime)

### 2. Entity & relation extraction

- [x] Entity extraction generates deterministic IDs (`Entity::deterministic_id` via SHA-256)
- [x] Relation extraction records evidence IDs (`provenance_chunk_id`)
- [x] Bi-temporal fields are persisted (`valid_at`, `invalid_at`, `recorded_at`, `confidence`)
- [x] Contradiction resolution marks obsolete edges invalid (`invalid_at = replacement.valid_at`)

### 3. Retrieval integration

- [x] 1–2 hop graph search is available to SuperRAG (Stage 2 multi-modal retrieval)
- [x] 3-hop traversal is only used during retry (Stage 4 confidence verification loop)
- [x] Graph candidates can be fused with lexical/vector candidates (Stage 3 RRF combiner $w_{\text{graph}} = 0.20$)
- [x] Graph failures do not break recall (graceful degradation to dense vector + lexical retrieval)

### 4. Adapter readiness

- [x] Neo4j adapter boundary is documented (`crates/buzz-db/src/memory/adapters.md`)
- [x] FalkorDB adapter boundary is documented (`crates/buzz-db/src/memory/adapters.md`)
- [x] No graph-vendor types leak into `buzz-search` (strictly typed domain abstractions)

## Verification & Sign-off

- [x] `cargo test -p buzz-core` (267 unit + 2 doc tests passed)
- [x] Graph persistence/reopen test passes (`crates/buzz-db/tests/f05_knowledge_graph_tests.rs`)
- [x] Contradiction-resolution test passes (`tc_f05_002_temporal_invalidation`)
- [x] Architecture Reviewer Sign-off: PASS (Findings 1-4 resolved)
