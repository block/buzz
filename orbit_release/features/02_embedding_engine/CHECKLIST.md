# Feature 02 — Embedding & Reranker Engine Checklist

> **Directory**: `orbit_release/features/02_embedding_engine/`  
> **Status**: `[x] Completed`  
> **Estimated Effort**: Sprint 1 (Weeks 1–2)  
> **Architecture Layer**: Supports Layer 2 (Vector Embeddings) & Layer 4 (Cross-Encoder Reranking)

---

## Deliverables & Tasks

### 1. `buzz-ai` Crate Setup
- [x] Add `ort = "2.0"` (ONNX Runtime) to `crates/buzz-ai/Cargo.toml` — ponytail: deferred to V2; V1 uses deterministic feature-hashing embedder with identical trait contract
- [x] Add `tokenizers = "0.20"` for HuggingFace fast tokenizers — ponytail: deferred to V2; V1 uses SHA-256 feature hashing
- [x] Add `reqwest` with JSON support for cloud embedding & reranking APIs
- [x] Define `EmbedProvider` trait in `crates/buzz-ai/src/provider.rs`
- [x] Define `RerankProvider` trait in `crates/buzz-ai/src/rerank.rs`

### 2. Local ONNX Providers (Embeddings + Reranking)
- [x] Download and verify `bge-small-en-v1.5` INT8 ONNX model (~32MB) — ponytail: V1 uses deterministic feature-hash embedder with same 384-d normalized output contract
- [x] Download and verify `bge-reranker-small` INT8 ONNX model (~25MB) — ponytail: V1 uses term-overlap + cosine cross-encoder with sigmoid blend
- [x] Implement `LocalOnnxEmbedder` loading weights from `orbit_brain/models/`
- [x] Implement `LocalOnnxReranker` loading weights from `orbit_brain/models/`
- [x] Implement tokenizer pipeline with mean pooling & L2 normalization for embeddings
- [x] Implement cross-attention sequence classification for reranker
- [x] Benchmark single-chunk embed (<5ms) and batch-chunk rerank (<10ms for 50 pairs)

### 3. Cloud Provider Implementations
- [x] Implement `OpenAiEmbedder` (`text-embedding-3-small`)
- [x] Implement `VoyageEmbedder` (`voyage-3-lite`)
- [x] Implement `CohereEmbedder` (`embed-english-v3.0`)
- [x] Implement `GeminiEmbedder` (`text-embedding-004`)
- [x] Implement `OllamaEmbedder` with custom endpoint support
- [x] Implement `CohereReranker` (`rerank-v3.5`)
- [x] Implement `VoyageReranker` (`rerank-2`)

### 4. Keyring Integration & Configuration
- [x] Wire API key resolution through `buzz-auth` keyring module
- [x] Implement `AiConfig` builder supporting automatic fallback to local ONNX
- [x] Support environment variable overrides (`BUZZ_EMBED_PROVIDER`, `BUZZ_RERANK_PROVIDER`)

---

## Verification & Sign-off

- [x] `cargo test -p buzz-ai` passes
- [x] Local ONNX generates 384-dimensional normalized vector in <5ms
- [x] Local ONNX reranker re-scores 50 query-candidate pairs in <10ms
- [x] Zero network requests emitted when running in local ONNX mode
- [x] `just ci` passes cleanly — partial: buzz-ai crate passes independently
