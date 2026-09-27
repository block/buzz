# Feature 02 — Embedding Engine (Local ONNX + Cloud Providers)

> **Priority**: P0 — Required for all vector similarity operations.  
> **Sprint**: Sprint 1 (Weeks 1–2)  
> **Dependencies**: Feature 01 (Storage Foundation)  
> **Crates**: `orbit-ai` (implementation), `buzz-auth` (keyring credentials)  
> **Environment Variables**: `BUZZ_EMBED_PROVIDER`, `BUZZ_EMBED_MODEL`, `BUZZ_EMBED_BATCH_SIZE`, `BUZZ_DATA_DIR`

---

## Overview

Feature 02 provides an embedding generation engine for Orbit. It features a zero-cost, zero-network **local ONNX runtime** using `bge-small-en-v1.5` as the default, while providing pluggable cloud providers (OpenAI, Voyage, Cohere, Gemini, Ollama) behind a unified async trait.

Model files are stored in `orbit_brain/models/`.

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

### 2. Local ONNX Engine: `LocalOnnxProvider`

- **Model**: `BAAI/bge-small-en-v1.5` quantized to INT8
- **Dimensions**: 384
- **Size**: ~32MB weights file
- **Runtime**: `ort` crate (ONNX Runtime bindings for Rust)
- **Performance**: <5ms per chunk on modern CPU
- **Location**: `~/.buzz/orbit_brain/models/bge-small-en-v1.5.onnx`

### 3. Cloud Provider Implementations

| Provider | Default Model | Dimensions | Auth Key Name |
|----------|---------------|------------|---------------|
| `OpenAiProvider` | `text-embedding-3-small` | 1536 (or truncated) | `BUZZ_OPENAI_API_KEY` |
| `VoyageProvider` | `voyage-3-lite` | 512 | `BUZZ_VOYAGE_API_KEY` |
| `CohereProvider` | `embed-english-v3.0` | 1024 | `BUZZ_COHERE_API_KEY` |
| `GeminiProvider` | `text-embedding-004` | 768 | `BUZZ_GEMINI_API_KEY` |
| `OllamaProvider` | `nomic-embed-text` | 768 | None (Local Endpoint) |

### 4. Keyring Security Integration

Cloud API keys are stored securely using the existing `buzz-auth` OS keyring helper:
- Windows: DPAPI / Windows Credential Manager
- macOS: Apple Keychain
- Linux: Secret Service API (freedesktop.org)

---

## Verification & Quality Gates

- Unit tests: verify 384-dimensional vector output for sample sentences.
- Mock tests for cloud providers to verify JSON payload contracts.
- Run `cargo test -p orbit-ai`.
