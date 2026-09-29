# Feature 04 — SuperRAG Retrieval Layer Checklist

> **Directory**: `orbit_release/features/04_multi_rag_search/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 2 (Weeks 3–4)  
> **Architecture Layer**: Layer 4 (Retrieval Layer) & Layer 5 (Working Context Compiler)

---

## Deliverables & Tasks

### 1. Pre-Retrieval Data Re-Trial & Query Routing Arbiter (`crates/buzz-search`)
- [ ] Implement `QueryRouter::classify_intent()` in `crates/buzz-search/src/router.rs`
- [ ] Implement detection of code symbols (`::`, `()`, `/`, `.`), commit SHAs, and UUIDs
- [ ] Implement detection of temporal questions and architectural decisions
- [ ] Implement in-memory Semantic Query Cache with LRU eviction and TTL for sub-1ms repeated hits
- [ ] Implement multi-query expansion / decomposition pipeline for ambiguous prompts

### 2. Multi-Modal Candidate Retrieval (`crates/buzz-search`)
- [ ] **Dense Vector Search**: Implement `search_dense_vector(pool, query_vector, limit, workspace)` using `<=>` cosine distance over pgvector HNSW index
- [ ] **Lexical BM25 Search**: Implement `search_lexical_bm25(pool, query_text, limit, workspace)` using `ts_rank_cd` over GIN `search_tsv`
- [ ] **Graph Walk Search**: Implement 2-hop recursive graph traversal over active relations (`invalid_at IS NULL`)
- [ ] Add workspace isolation filter and permissions/ACL pre-filtering

### 3. Reciprocal Rank Fusion (RRF) & Heuristic Decay
- [ ] Implement `calculate_rrf()` combiner with weights ($w_{\text{vec}} = 0.45$, $w_{\text{lex}} = 0.35$, $w_{\text{graph}} = 0.20$) and $k = 60$
- [ ] Implement exponential temporal decay ($e^{-\lambda \Delta t}$) with category-specific decay constants
- [ ] Implement source authority weighting (Git commits: 1.2, ADRs: 1.1, Code: 1.0, Chats: 0.8)
- [ ] Implement workspace boost ($1.5\times$ for active project)

### 4. Data Re-Trial & Confidence Verification Loop
- [ ] Implement confidence score evaluation ($\theta_{\text{conf}} \ge 0.65$) on top candidate chunks
- [ ] Implement automatic adaptive re-trial pass on low confidence or sparse result pools (<3 results)
- [ ] Add query reformulation, keyword stem relaxation, and graph walk expansion to 3 hops in re-trial loop

### 5. MVP Cross-Encoder Reranking Engine (`crates/orbit-ai`)
- [ ] Define `RerankProvider` trait in `crates/orbit-ai/src/rerank.rs`
- [ ] Implement local ONNX cross-encoder provider using `BAAI/bge-reranker-small` (INT8 quantized, ~25MB) via `ort` crate
- [ ] Implement candidate scoring pipeline: re-evaluates top 50 candidates down to top 15 precision chunks in `<10ms`
- [ ] Implement cloud API fallback for Cohere Rerank (`rerank-v3.5`) and Voyage Rerank (`rerank-2`)
- [ ] Wire API key resolution via `buzz-auth` OS keyring

### 6. Layer 5 Working Context Assembly & Knapsack Packing
- [ ] Implement `TokenBudgetPacker` with greedy knapsack packing algorithm (default 4000 tokens)
- [ ] Implement lexical sentence deduplication across overlapping chunks
- [ ] Attach provenance headers (`[Source: <path> L<start>-L<end>]`) and verified commit metadata
- [ ] Wrap assembled context in `<orbit_context>` semantic XML delimiter tags

---

## Verification & Sign-off

- [ ] `cargo test -p buzz-search` passes
- [ ] `cargo test -p orbit-ai` passes (embeddings + reranking)
- [ ] Multi-RAG query returns high-relevance chunks for both conceptual and exact symbol queries
- [ ] Data Re-Trial loop successfully triggers and recovers relevant candidates on ambiguous queries
- [ ] Cross-encoder reranker runs on CPU in `<10ms` for 50 candidates
- [ ] End-to-end P99 search & context assembly latency is verified `<25ms` under 10,000 chunks benchmark
- [ ] `just ci` passes cleanly
