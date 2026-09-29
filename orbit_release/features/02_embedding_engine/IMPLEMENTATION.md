# Feature 02 — Embedding & Reranker Engine (Local ONNX + Cloud Providers)

> **Priority**: P0 — Required for all vector similarity and cross-encoder reranking operations.  
> **Sprint**: Sprint 1 (Weeks 1–2)  
> **Dependencies**: Feature 01 (Storage Foundation)  
> **Crates**: `orbit-ai` (implementation), `buzz-auth` (keyring credentials)  
> **Environment Variables**: `BUZZ_EMBED_PROVIDER`, `BUZZ_EMBED_MODEL`, `BUZZ_RERANK_PROVIDER`, `BUZZ_RERANK_MODEL`, `BUZZ_DATA_DIR`

---

## Overview

Feature 02 provides both an **Embedding Generation Engine** and a high-performance **Cross-Encoder Reranker Engine** for Orbit within the `orbit-ai` crate. It features zero-cost, zero-network **local ONNX runtimes** (`bge-small-en-v1.5` for embeddings and `bge-reranker-small` for reranking), while providing pluggable cloud providers (OpenAI, Voyage, Cohere, Gemini, Ollama) behind unified async traits.

Model weights files are stored locally under `orbit_brain/models/`.

---

## Architecture & Traits

### 1. `EmbedProvider` Trait (`crates/orbit-ai/src/provider.rs`)

```rust
use async_trait::async_trait;
use crate::error::Result;

#[async_trait]
pub trait EmbedProvider: Send + Sync {
    /// Generate embeddings for a batch of text chunks
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// Output vector dimensions (e.g. 384 for bge-small-en-v1.5)
    fn dimensions(&self) -> usize;

    /// Identifier name of the provider
    fn provider_name(&self) -> &'static str;
}
```

### 2. `RerankProvider` Trait (`crates/orbit-ai/src/rerank.rs`)

```rust
use async_trait::async_trait;
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct RerankResult {
    pub index: usize,
    pub score: f32,
}

#[async_trait]
pub trait RerankProvider: Send + Sync {
    /// Rerank a candidate list of candidate chunk texts against a target query
    async fn rerank(&self, query: &str, candidates: &[&str], top_n: usize) -> Result<Vec<RerankResult>>;

    /// Identifier name of the rerank provider
    fn provider_name(&self) -> &'static str;
}
```

---

## Local ONNX Engines

### 1. Local Embedding Engine: `LocalOnnxEmbedder`
- **Model**: `BAAI/bge-small-en-v1.5` quantized to INT8
- **Dimensions**: 384
- **Size**: ~32MB weights file
- **Runtime**: `ort` crate (ONNX Runtime bindings for Rust)
- **Performance**: `<5 ms` per chunk on CPU
- **Location**: `~/.buzz/orbit_brain/models/bge-small-en-v1.5.onnx`

### 2. Local Cross-Encoder Reranker Engine: `LocalOnnxReranker`
- **Model**: `BAAI/bge-reranker-small` quantized to INT8
- **Architecture**: Cross-encoder (computes cross-attention over `[CLS] query [SEP] candidate [SEP]`)
- **Size**: ~25MB weights file
- **Runtime**: `ort` crate running on CPU (SIMD/AVX-512 optimized)
- **Performance**: `<10 ms` for 50 candidate pairs
- **Location**: `~/.buzz/orbit_brain/models/bge-reranker-small.onnx`

---

## Cloud Provider Implementations

| Subsystem | Provider | Default Model | Dimensions / Output | Auth Key Name |
|-----------|----------|---------------|---------------------|---------------|
| **Embedding** | `OpenAiEmbedder` | `text-embedding-3-small` | 1536 | `BUZZ_OPENAI_API_KEY` |
| **Embedding** | `VoyageEmbedder` | `voyage-3-lite` | 512 | `BUZZ_VOYAGE_API_KEY` |
| **Embedding** | `CohereEmbedder` | `embed-english-v3.0` | 1024 | `BUZZ_COHERE_API_KEY` |
| **Embedding** | `GeminiEmbedder` | `text-embedding-004` | 768 | `BUZZ_GEMINI_API_KEY` |
| **Embedding** | `OllamaEmbedder` | `nomic-embed-text` | 768 | None (Local Endpoint) |
| **Reranking** | `CohereReranker` | `rerank-v3.5` | Re-scored Float | `BUZZ_COHERE_API_KEY` |
| **Reranking** | `VoyageReranker` | `rerank-2` | Re-scored Float | `BUZZ_VOYAGE_API_KEY` |

---

## Keyring Security Integration

Cloud API keys are stored securely using the existing `buzz-auth` OS keyring helper:
- Windows: DPAPI / Windows Credential Manager
- macOS: Apple Keychain
- Linux: Secret Service API (freedesktop.org)

Keys are never stored in plaintext configuration files or database tables.

---

## Verification & Quality Gates

- Unit tests: verify 384-dimensional vector output for sample sentences.
- Unit tests: verify cross-encoder score ranking matches semantic relevance on test pairs.
- Mock tests for cloud providers to verify JSON payload contracts.
- Run `cargo test -p orbit-ai`.
- Benchmark: CPU latency `<5ms` for embed, `<10ms` for 50-candidate rerank.
- Run `just ci`.
