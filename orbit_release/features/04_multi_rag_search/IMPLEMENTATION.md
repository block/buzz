# Feature 04 — SuperRAG Retrieval Layer (Pre-Retrieval Data Re-Trial, Multi-Modal Fusion & Cross-Encoder Reranking)

> **Priority**: P0 — Core retrieval and context assembly engine for AI agents and developer queries (Layer 4 of Orbit's 5-Layer Context Architecture).  
> **Sprint**: Sprint 2 (Weeks 3–4)  
> **Dependencies**: Feature 01 (Storage Foundation), Feature 02 (Embedding & Reranker Engine)  
> **Crates**: `buzz-search` (extend query & fusion), `buzz-db` (orchestrator), `buzz-ai` (reranker runtime)  
> **Environment Variables**: `BUZZ_DATABASE_URL`, `BUZZ_SEARCH_LIMIT`, `BUZZ_TOKEN_BUDGET`, `BUZZ_RERANK_PROVIDER`, `BUZZ_RERANK_MODEL`

---

## Overview

Feature 04 implements the **Orbit SuperRAG Retrieval Layer (Layer 4)**, a high-throughput, multi-stage retrieval and context compilation engine designed to replace traditional, slow database CRUD operations. 

In conventional architectures, every agent query executes brute-force database searches and discrete CRUD queries, creating severe latency bottlenecks (>100ms), lock contention, and irrelevant context. Orbit’s SuperRAG solves this by introducing a **Pre-Retrieval Data Re-Trial & Query Routing Arbiter**: when an agent or user issues a query, it **hits the Data Re-Trial Layer first** before touching any raw storage.

### Core SuperRAG Pipeline:
1. **Pre-Retrieval Arbiter & Query Router**:
   - **Semantic Query Cache**: Sub-millisecond return for repeated or active context lookups.
   - **Intent Classification & Query Decomposition**: Routes intelligently based on query type (symbol lookup, architectural decision, multi-hop relation, or general concept).
   - **Multi-Query Expansion**: Generates keyword-dense and semantic variations to maximize recall.
2. **Parallel Multi-Modal Candidate Retrieval**:
   - **Dense Vector Search**: embedded vector store index (`<=>` cosine distance) over `orbit_chunks`.
   - **Lexical BM25 Search**: PostgreSQL GIN index (`search_tsv` with `ts_rank_cd`).
   - **Knowledge Graph Traversal**: Bi-temporal 2-hop recursive graph walk over `orbit_entities` and `orbit_relations`.
   - **Metadata & Scope Filters**: Workspace isolation, permissions/ACLs, and agent boundary filtering.
3. **Reciprocal Rank Fusion (RRF) & Temporal Modulation**:
   - Merges candidate rankings using RRF ($k = 60$).
   - Modulates scores using exponential temporal decay ($e^{-\lambda \Delta t}$) and source authoritativeness ($A_{\text{source}}$).
4. **Data Re-Trial & Confidence Verification Loop**:
   - Dynamically evaluates candidate distribution and top-rank confidence score ($\theta_{\text{conf}}$).
   - If confidence is insufficient or ambiguous, executes an adaptive **re-trial pass** (automatic query reformulation, relaxed filters, or 3-hop graph expansion) before returning candidates.
5. **Cross-Encoder Reranking Engine (MVP Native)**:
   - Feeds top 50 fused candidates into a fast local cross-encoder (`bge-reranker-small` INT8 ONNX via `ort` or FlashRank, with cloud API fallback) to perform deep query-document cross-attention in `<10ms`.
   - Re-ranks candidates into the top 10–15 highest-precision chunks.
6. **Layer 5 Working Context Assembly & Knapsack Packing**:
   - Greedily packs winning chunks into the configured token budget (e.g., 4000 tokens).
   - Attaches strict provenance headers (`[Source: crates/buzz-relay/src/auth.rs L12-45]`).
   - Wraps assembled context in `<orbit_context>` semantic XML tags for the consuming LLM.

---

## Architecture & Data Flow

```
Agent / User Query: "How does the relay authenticate connections?"
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│              STAGE 1: PRE-RETRIEVAL DATA RE-TRIAL & ROUTING ARBITER          │
├─────────────────────────────────────────────────────────────────────────────┤
│ 1. Semantic Cache Check (<1ms hit → return compiled context directly)       │
│ 2. Intent Classifier: [Code Symbol | Architecture | Temporal ADR | General] │
│ 3. Query Decomposition & Multi-Query Expansion (Dense vector + Exact tokens)│
└─────────────────────────────────┬───────────────────────────────────────────┘
                                  │ (Cache Miss: Selective Dispatch)
        ┌─────────────────────────┼─────────────────────────┐
        ▼                         ▼                         ▼
┌─────────────────┐       ┌─────────────────┐       ┌─────────────────┐
│ Dense Vector    │       │ Lexical BM25    │       │ Knowledge Graph │
│ (embedded vector store) │       │(SQLite FTS5 BM25)│      │ (2-Hop Walk)    │
└────────┬────────┘       └────────┬────────┘       └────────┬────────┘
         │                         │                         │
         └─────────────────────────┼─────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│          STAGE 2: RECIPROCAL RANK FUSION (RRF) & HEURISTIC DECAY            │
├─────────────────────────────────────────────────────────────────────────────┤
│ RRF_Score(d) = Σ [ w_m / (60 + rank_m(d)) ]                                 │
│ Modulate: Score(d) × e^(-λΔt) × Source_Authority × Scope_Boost               │
└──────────────────────────────────┬──────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│          STAGE 3: DATA RE-TRIAL & CONFIDENCE VERIFICATION LOOP              │
├─────────────────────────────────────────────────────────────────────────────┤
│ Top Candidate Confidence Score >= θ_conf (default 0.65)?                    │
│   ├── NO  ──► Adaptive Re-Trial Pass: Query reformulation / HyDE relaxation │
│   └── YES ──► Proceed to Cross-Encoder Reranking                            │
└──────────────────────────────────┬──────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│           STAGE 4: MVP CROSS-ENCODER RERANKING ENGINE (<10ms)               │
├─────────────────────────────────────────────────────────────────────────────┤
│ Local ONNX `bge-reranker-small` (INT8) or FlashRank cross-attention         │
│ Evaluates Query × Document pairs simultaneously: Top 50 ──► Top 15 Precision│
└──────────────────────────────────┬──────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│        STAGE 5: LAYER 5 WORKING CONTEXT COMPILER (Knapsack Packing)         │
├─────────────────────────────────────────────────────────────────────────────┤
│ • Greedy token budget allocation (e.g., 4000 tokens)                        │
│ • Overlap sentence deduplication & provenance stamping                      │
│ • Output: Delimited XML `<orbit_context>...</orbit_context>`                │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Detailed Component Specifications

### 1. Stage 1: Pre-Retrieval Data Re-Trial & Query Routing Arbiter (`crates/buzz-search/src/router.rs`)

When a query is received, the Arbiter inspects the query string and metadata to determine the optimal retrieval strategy without executing blind full-scan queries:

```rust
pub enum QueryIntent {
    /// Exact code symbol, identifier, error message, or commit SHA
    Symbolic { identifier: String },
    /// Architectural pattern, system design, or concept
    Conceptual { semantic_query: String },
    /// Historical decision, ADR, or superseded workflow
    TemporalDecision { topic: String, before_time: Option<DateTime<Utc>> },
    /// Broad workspace overview or multi-hop dependency query
    MultiHopRelationship { seed_entity: String },
}

pub struct QueryRouter;

impl QueryRouter {
    /// Classifies incoming query into targeted intent to selectively dispatch indices
    pub fn classify_intent(query: &str) -> QueryIntent {
        // 1. Detect commit SHA (40-char hex) or UUID
        if is_hex_sha_or_uuid(query) {
            return QueryIntent::Symbolic { identifier: query.trim().to_string() };
        }
        // 2. Detect code identifier with path, scope operator, or function call (e.g. `buzz::auth`, `verify_nip42()`)
        if query.contains("::") || query.contains("()") || query.contains('/') || query.contains('.') {
            return QueryIntent::Symbolic { identifier: query.trim().to_string() };
        }
        // 3. Detect temporal questions ("why did we change", "deprecated", "decision on")
        if query.starts_with("why did") || query.contains("decision") || query.contains("superseded") {
            return QueryIntent::TemporalDecision { topic: query.to_string(), before_time: None };
        }
        // Default to conceptual search with multi-hop capability
        QueryIntent::Conceptual { semantic_query: query.to_string() }
    }
}
```

### 2. Stage 2: Parallel Multi-Modal Candidate Retrieval

#### A. Dense Vector Search (`crates/buzz-search/src/dense.rs`)
Queries the `VectorStore` abstraction (LanceDB in desktop V1) using cosine distance.:

```rust
let vector_hits = stores
    .vectors
    .search(&query_embedding, vector_filter, 50)
    .await?;
```

#### B. Lexical BM25 Search (`crates/buzz-search/src/lexical.rs`)
Queries the ORBIT lexical index (SQLite FTS5 in desktop V1) with weighted lexical ranking.:

```rust
let lexical_hits = stores
    .lexical
    .search(&query_text, lexical_filter, 50)
    .await?;
```

#### C. Bi-Temporal Graph Neighborhood Traversal (`crates/buzz-search/src/graph.rs`)
Performs 2-hop recursive traversal from seed entities extracted during query analysis:

```rust
let graph_hits = stores
    .graph
    .neighborhood(&seed_entities, 2)
    .await?;
```

---

### 3. Stage 3: Reciprocal Rank Fusion (RRF) & Heuristic Decay

Combines ranks across modalities into a single candidate score:

$$\text{RRF\_Score}(d) = \sum_{m \in \{\text{vec}, \text{lex}, \text{graph}\}} \frac{w_m}{60 + \text{rank}_m(d)}$$

Where weights default to $w_{\text{vec}} = 0.45$, $w_{\text{lex}} = 0.35$, $w_{\text{graph}} = 0.20$.

Modulated by temporal decay and source authoritativeness:

$$\text{Final\_Score}(d) = \text{RRF\_Score}(d) \times e^{-\lambda \cdot \Delta t} \times A_{\text{source}} \times W_{\text{workspace}}$$

- $\Delta t$: Elapsed time since chunk creation/verification in days.
- $\lambda$: Decay rate ($\lambda = 0.0$ for ADRs/architecture; $\lambda = 0.05$ for chat logs).
- $A_{\text{source}}$: Source authority (1.2 for verified git commit, 1.0 for files, 0.8 for unreviewed chats).
- $W_{\text{workspace}}$: 1.5 multiplier for active repository match.

```rust
pub fn calculate_rrf(
    vector_results: &[(Uuid, f32)],
    lexical_results: &[(Uuid, f32)],
    graph_results: &[(Uuid, f32)],
    k: f32, // default 60.0
) -> Vec<(Uuid, f32)> {
    let mut scores: HashMap<Uuid, f32> = HashMap::new();
    let w_vec = 0.45;
    let w_lex = 0.35;
    let w_graph = 0.20;

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

---

### 4. Stage 4: Data Re-Trial & Confidence Verification Loop

Before spending inference cycles on reranking or returning inadequate context, the retrieval layer validates retrieval quality:

1. **Confidence Threshold**: $\theta_{\text{conf}} = 0.65$. If top candidate normalized score $< \theta_{\text{conf}}$ OR candidate pool $< 3$ results:
2. **Re-Trial Execution**:
   - **Step 1: Query Reformulation**: Strip punctuation, stem keywords, and generate HyDE (Hypothetical Document Embedding) representation.
   - **Step 2: Relaxation**: Widen time horizon and expand graph traversal from 2 hops to 3 hops.
   - **Step 3: Re-execute Retrieval**: Re-run lexical and vector pass with expanded parameters.
3. This guarantees that ambiguous or poorly phrased developer questions still surface high-recall candidates rather than empty sets.

---

### 5. Stage 5: Cross-Encoder Reranking Engine (MVP Native)

Unlike bi-encoders (which compute vector similarity independently), the **Cross-Encoder Reranker** passes the query $q$ and document text $d$ jointly through transformer self-attention layers:

$$\text{Rerank\_Score}(q, d) = \text{Softmax}(\mathbf{W} \cdot \text{Transformer}([CLS] \circ q \circ [SEP] \circ d \circ [SEP]))$$

#### Runtime Implementation (`crates/buzz-ai/src/rerank.rs`):
- **Local Default Engine**: `BAAI/bge-reranker-small` (INT8 quantized ONNX, ~25MB weights file) executed via the Rust `ort` crate.
- **Latency**: `<10 ms` for 50 candidate chunks on a standard multi-core CPU.
- **Pluggable Cloud Fallback**: Cohere Rerank API (`rerank-v3.5`) or Voyage Rerank API (`rerank-2`) when configured in desktop settings.

```rust
#[async_trait]
pub trait RerankProvider: Send + Sync {
    /// Rerank a candidate list of texts against a target query
    async fn rerank(&self, query: &str, candidates: &[&str], top_n: usize) -> Result<Vec<(usize, f32)>>;
    
    fn provider_name(&self) -> &'static str;
}
```

---

### 6. Stage 6: Layer 5 Working Context Assembly & Knapsack Packing

The winning top 10–15 reranked chunks are transformed into a clean, bounded working context package for the active agent turn:

1. **Greedy Knapsack Packing**: Packs chunks in descending order of rerank score until `token_budget` (default: 4000 tokens) is exhausted.
2. **Lexical Deduplication**: Prunes overlapping sentences or redundant code lines across chunks.
3. **Provenance Annotation**: Attaches verified source path, line ranges, and author commit SHAs.
4. **Delimiter Fencing**: Wraps the package in semantic XML tags to neutralize indirect prompt injection:

```xml
<orbit_context workspace="/path/to/repo" total_chunks="4" budget_tokens="1280">
  <chunk index="1" source="crates/buzz-relay/src/auth.rs" lines="42-89" score="0.942" authority="verified_git">
    // Extracted verified code or architectural fact
  </chunk>
  <chunk index="2" source="docs/decisions/0004-hnsw.md" lines="1-35" score="0.891" authority="adr">
    // Architecture Decision Record
  </chunk>
</orbit_context>
```

---

## 4-Tier Implementation Layer Architecture

### 1. Frontend Tier: SuperRAG Search & Retrieval UX
- **Omnibar Search (`desktop/src/features/memory/SuperRagSearchBar.tsx`)**:
  - Embedded directly inside the top of the **"AI Brain"** view (`/ai-brain`).
  - Real-time search query dispatch with debouncing.
  - Candidate Confidence Gauge: Visual indicator showing confidence score ($\theta_{\text{conf}}$).
  - Graph Highlighting: Pulses and focuses top matched nodes and their 1-hop neighborhood in the Obsidian graph.
- **Result Inspector Drawer (`desktop/src/features/memory/NodeDetailsDrawer.tsx`)**:
  - Displays retrieved chunks with provenance citations, similarity scores, and authority badges.

### 2. Desktop Backend Tier: Tauri Rust IPC & Query Pipeline
- **Location**: `desktop/src-tauri/src/commands/brain_graph.rs`
- Tauri IPC command: `query_superrag(query: String, limit: Option<usize>, workspace_path: Option<String>) -> Result<SuperRagPayload, String>`.
- Coordinates query flow: Semantic cache lookup $\rightarrow$ parallel multi-modal retrieval $\rightarrow$ RRF fusion $\rightarrow$ in-process reranker $\rightarrow$ XML context packer.

### 3. Core Workspace Crates Tier (`crates/buzz-search`, `crates/buzz-ai`, `crates/buzz-db`)
- `crates/buzz-search`:
  - Pre-Retrieval Data Re-Trial & Query Routing Arbiter.
  - In-memory semantic cache (`orbit_query_cache`).
  - Reciprocal Rank Fusion combiner with temporal decay ($e^{-\lambda \Delta t}$).
  - Knapsack token budget packer generating `<orbit_context>`.
- `crates/buzz-ai`:
  - In-process Cross-Encoder Reranker (`bge-reranker-small`) scoring top 50 candidates in `<10ms`.
- `crates/buzz-db`:
  - SQLite FTS5 lexical index and LanceDB vector index access.

### 4. Packaging, Bundling & Container Tier
- **Zero Docker**: Retrieval and reranking execute 100% locally on CPU without external processes.
- **Model Weights**: `bge-reranker-small.onnx` bundled in `desktop/src-tauri/resources/models/`.

---

- **Unit Tests**:
  - `calculate_rrf` ranks multi-source matches higher than single-source matches.
  - `QueryRouter::classify_intent` correctly segments symbols, decisions, and conceptual queries.
  - `bge-reranker-small` produces deterministic cross-attention scores for test pairs.
- **Performance Benchmarks**:
  - Full end-to-end retrieval latency (Query Routing $\rightarrow$ Vector + BM25 $\rightarrow$ RRF $\rightarrow$ Reranker $\rightarrow$ Context Pack) must verify **P99 $< 25$ ms** on a database of 10,000 chunks.
  - Idle RAM overhead of the retrieval engine remains **$< 40$ MB**.
- **Quality Gates**:
  - `cargo test -p buzz-search`
  - `cargo test -p buzz-ai`
  - `just ci`

## Local-first retrieval rule

When the user is signed in, the default retrieval path still queries the local indexes. Cloud retrieval is an explicit fallback/capability, not an automatic dependency.

This preserves IDE responsiveness and offline continuity. A hosted sync subscription should therefore never cause every `orbit.search_context` call to wait for a remote PostgreSQL/vector request.

The retrieval response should report whether each evidence item came from the local brain or an explicitly authorized hosted source.


## Permission and policy gate

Permission and workspace-policy filtering precedes expensive candidate generation. A result from another tenant, another enterprise project, a local-only source, or a policy-disallowed processor must never reach RRF or the reranker simply because it is semantically similar. Hosted retrieval must apply the same logical policy contract as local retrieval.
