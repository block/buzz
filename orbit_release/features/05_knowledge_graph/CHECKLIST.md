# Feature 05 — Knowledge Graph Checklist

### 1. Graph engine

- [ ] Embedded Ladybug/Kuzu graph initializes from `orbit_brain/graph/`
- [ ] Graph backend is accessed through `GraphStore`
- [ ] No graph server is required for desktop startup

### 2. Entity & relation extraction

- [ ] Entity extraction generates deterministic IDs
- [ ] Relation extraction records evidence IDs
- [ ] Bi-temporal fields are persisted
- [ ] Contradiction resolution marks obsolete edges invalid

### 3. Retrieval integration

- [ ] 1–2 hop graph search is available to SuperRAG
- [ ] 3-hop traversal is only used during retry
- [ ] Graph candidates can be fused with lexical/vector candidates
- [ ] Graph failures do not break recall

### 4. Adapter readiness

- [ ] Neo4j adapter boundary is documented
- [ ] FalkorDB adapter boundary is documented
- [ ] No graph-vendor types leak into `buzz-search`

## Verification & Sign-off

- [ ] `cargo test -p buzz-core`
- [ ] Graph persistence/reopen test passes
- [ ] Contradiction-resolution test passes
