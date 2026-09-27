# Feature 04 — Multi-RAG Search Engine Checklist

> **Directory**: `orbit_release/features/04_multi_rag_search/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 2 (Weeks 3–4)

---

## Deliverables & Tasks

### 1. Dense Vector Search (`crates/buzz-search`)
- [ ] Implement `search_dense_vector(pool, query_vector, limit, workspace) -> Result<Vec<ScoredChunk>>`
- [ ] Utilize `<=>` cosine distance against pgvector HNSW index
- [ ] Add workspace isolation filter

### 2. Lexical BM25 Search (`crates/buzz-search`)
- [ ] Implement `search_lexical_bm25(pool, query_text, limit, workspace) -> Result<Vec<ScoredChunk>>`
- [ ] Utilize `ts_rank_cd` against `search_tsv` GIN index
- [ ] Add query sanitization for full-text search operators

### 3. Reciprocal Rank Fusion (RRF) & Decay
- [ ] Implement `calculate_rrf()` function with customizable weights (`w_vec`, `w_lex`, `w_graph`)
- [ ] Implement temporal decay weighting (`exp(-lambda * dt)`)
- [ ] Implement staleness and authority penalties
- [ ] **[V1.1 DEFERRED]** Note cross-encoder reranker `ms-marco-MiniLM-L-6-v2` as post-MVP milestone

### 4. Context Assembly & Knapsack Packing
- [ ] Implement `TokenBudgetPacker` with max token budget constraint (default 4000)
- [ ] Add deduplication of overlapping sentence spans
- [ ] Format output with provenance markers (`[Source: <path> L<start>-L<end>]`)
- [ ] Wrap in `<orbit_context>` XML tags

---

## Verification & Sign-off

- [ ] `cargo test -p buzz-search` passes
- [ ] Multi-RAG query returns relevant chunks for both semantic and exact keyword queries
- [ ] P99 latency is verified `<25ms` under 10,000 chunks benchmark
- [ ] `just ci` passes cleanly
