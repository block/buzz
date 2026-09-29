# Feature 02 — Embedding & Reranker Engine Checklist

> **Directory**: `orbit_release/features/02_embedding_engine/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 1 (Weeks 1–2)  
> **Architecture Layer**: Supports Layer 2 (Vector Embeddings) & Layer 4 (Cross-Encoder Reranking)

---

## Deliverables & Tasks

### 1. `orbit-ai` Crate Setup
- [ ] Add `ort = "2.0"` (ONNX Runtime) to `crates/orbit-ai/Cargo.toml`
- [ ] Add `tokenizers = "0.20"` for HuggingFace fast tokenizers
- [ ] Add `reqwest` with JSON support for cloud embedding & reranking APIs
- [ ] Define `EmbedProvider` trait in `crates/orbit-ai/src/provider.rs`
- [ ] Define `RerankProvider` trait in `crates/orbit-ai/src/rerank.rs`

### 2. Local ONNX Providers (Embeddings + Reranking)
- [ ] Download and verify `bge-small-en-v1.5` INT8 ONNX model (~32MB)
- [ ] Download and verify `bge-reranker-small` INT8 ONNX model (~25MB)
- [ ] Implement `LocalOnnxEmbedder` loading weights from `orbit_brain/models/`
- [ ] Implement `LocalOnnxReranker` loading weights from `orbit_brain/models/`
- [ ] Implement tokenizer pipeline with mean pooling & L2 normalization for embeddings
- [ ] Implement cross-attention sequence classification for reranker
- [ ] Benchmark single-chunk embed (<5ms) and batch-chunk rerank (<10ms for 50 pairs)

### 3. Cloud Provider Implementations
- [ ] Implement `OpenAiEmbedder` (`text-embedding-3-small`)
- [ ] Implement `VoyageEmbedder` (`voyage-3-lite`)
- [ ] Implement `CohereEmbedder` (`embed-english-v3.0`)
- [ ] Implement `GeminiEmbedder` (`text-embedding-004`)
- [ ] Implement `OllamaEmbedder` with custom endpoint support
- [ ] Implement `CohereReranker` (`rerank-v3.5`)
- [ ] Implement `VoyageReranker` (`rerank-2`)

### 4. Keyring Integration & Configuration
- [ ] Wire API key resolution through `buzz-auth` keyring module
- [ ] Implement `AiConfig` builder supporting automatic fallback to local ONNX
- [ ] Support environment variable overrides (`BUZZ_EMBED_PROVIDER`, `BUZZ_RERANK_PROVIDER`)

---

## Verification & Sign-off

- [ ] `cargo test -p orbit-ai` passes
- [ ] Local ONNX generates 384-dimensional normalized vector in <5ms
- [ ] Local ONNX reranker re-scores 50 query-candidate pairs in <10ms
- [ ] Zero network requests emitted when running in local ONNX mode
- [ ] `just ci` passes cleanly
