# Feature 04 — Multi-RAG Search Engine (Dense Vector + BM25 + RRF Fusion)

> **Priority**: P0 — Core retrieval capability for AI agents and developer queries.  
> **Sprint**: Sprint 2 (Weeks 3–4)  
> **Dependencies**: Feature 01 (Storage), Feature 02 (Embeddings)  
> **Crates**: `buzz-search` (extend), `orbit-storage` (orchestrator)  
> **Environment Variables**: `BUZZ_DATABASE_URL`, `BUZZ_SEARCH_LIMIT`, `BUZZ_TOKEN_BUDGET`

---

## Overview

Feature 04 implements a hybrid Multi-RAG search engine by extending the existing `buzz-search` crate. It combines:
1. **Layer 1: Dense Vector Search** via pgvector HNSW index (`<=>` cosine distance).
2. **Layer 2: Lexical BM25 Search** via PostgreSQL GIN `search_tsv` index.
3. **Layer 3: Knowledge Graph Traversal** (via Feature 05).
4. **Reciprocal Rank Fusion (RRF)** + **Temporal Decay Scoring**.
5. **Token Budget Knapsack Packer** to format context for LLM agents.

> [!NOTE]
> **V1.1 Roadmap Highlight**: Cross-encoder reranking via `ms-marco-MiniLM-L-6-v2` (top 50 → top 15) is documented and scheduled for **V1.1**. In V1 MVP, Reciprocal Rank Fusion (RRF) with temporal decay delivers sub-25ms P99 latency with zero extra memory footprint.

---

## Architecture & Algorithms

### 1. Hybrid Search Architecture

```
Query: "How does the relay authenticate connections?"
                    │
       ┌────────────┴────────────┐
       ▼                         ▼
Layer 1: Dense Vector     Layer 2: Lexical BM25
(pgvector HNSW Cosine)    (Postgres GIN ts_rank_cd)
       │                         │
       └────────────┬────────────┘
                    ▼
Reciprocal Rank Fusion (RRF)
RRF_Score(d) = Σ (w_m / (60 + rank_m(d)))
                    │
                    ▼
Temporal Decay & Authority Weighting
Final_Score(d) = RRF_Score(d) × e^(-λΔt) × S_staleness × A_authority
                    │
                    ▼
Token Budget Knapsack Packing (e.g. 4000 tokens)
                    │
                    ▼
Delimited Context: <orbit_context>...</orbit_context>
```

### 2. Layer 1: Dense Vector Search Query

```sql
SELECT 
    id, document_id, workspace_path, content, token_count, scope, agent_name, created_at,
    1 - (embedding <=> $1::vector) AS vector_similarity
FROM buzz_chunks
WHERE workspace_path = $2
ORDER BY embedding <=> $1::vector
LIMIT $3;
```

### 3. Layer 2: Lexical BM25 Search Query

```sql
SELECT 
    id, document_id, workspace_path, content, token_count, scope, agent_name, created_at,
    ts_rank_cd(search_tsv, plainto_tsquery('english', $1)) AS lexical_rank
FROM buzz_chunks
WHERE workspace_path = $2 AND search_tsv @@ plainto_tsquery('english', $1)
ORDER BY lexical_rank DESC
LIMIT $3;
```

### 4. Reciprocal Rank Fusion (RRF)

```rust
pub fn calculate_rrf(
    vector_results: &[(Uuid, f32)],
    lexical_results: &[(Uuid, f32)],
    graph_results: &[(Uuid, f32)],
    k: f32, // default 60.0
) -> Vec<(Uuid, f32)> {
    let mut scores: HashMap<Uuid, f32> = HashMap::new();
    let w_vec = 0.50;
    let w_lex = 0.35;
    let w_graph = 0.15;

    for (rank, (id, _)) in vector_results.iter().enumerate() {
        *scores.entry(*id).or_default() += w_vec / (k + (rank + 1) as f32);
    }
    for (rank, (id, _)) in lexical_results.iter().enumerate() {
        *scores.entry(*id).or_default() += w_lex / (k + (rank + 1) as f32);
    }
    for (rank, (id, _)) in graph_results.iter().enumerate() {
        *scores.entry(*id).or_default() += w_graph / (k + (rank + 1) as f32);
    }

    let mut ranked: Vec<(Uuid, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
}
```

### 5. Token Budget Knapsack Packer

- Greedily packs top-ranked chunks within configured token limit (default: 4000 tokens).
- Appends provenance headers: `[Source: crates/buzz-relay/src/auth.rs L12-45]`.
- Wraps assembled context in `<orbit_context>` semantic XML tags.

---

## Verification & Quality Gates

- Unit test: verify RRF scoring ranks combined hits higher than single-layer hits.
- Benchmark: verify P99 search latency `<25ms` on 10,000 chunks.
- Run `cargo test -p buzz-search`.
- Run `just ci`.
