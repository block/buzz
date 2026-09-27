# Feature 02 — Embedding Engine Checklist

> **Directory**: `orbit_release/features/02_embedding_engine/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 1 (Weeks 1–2)

---

## Deliverables & Tasks

### 1. `orbit-ai` Crate Setup
- [ ] Add `ort = "2.0"` (ONNX Runtime) to `crates/orbit-ai/Cargo.toml`
- [ ] Add `tokenizers = "0.20"` for HuggingFace fast tokenizers
- [ ] Add `reqwest` with JSON support for cloud embedding APIs
- [ ] Define `EmbedProvider` trait in `crates/orbit-ai/src/provider.rs`

### 2. Local ONNX Provider
- [ ] Download and verify `bge-small-en-v1.5` INT8 ONNX model
- [ ] Implement `LocalOnnxProvider` loading weights from `orbit_brain/models/`
- [ ] Implement tokenizer pipeline with mean pooling & L2 normalization
- [ ] Benchmark single-chunk and batch-chunk CPU embedding latency

### 3. Cloud Provider Implementations
- [ ] Implement `OpenAiProvider` (`text-embedding-3-small`)
- [ ] Implement `VoyageProvider` (`voyage-3-lite`)
- [ ] Implement `CohereProvider` (`embed-english-v3.0`)
- [ ] Implement `GeminiProvider` (`text-embedding-004`)
- [ ] Implement `OllamaProvider` with custom endpoint support

### 4. Keyring Integration & Configuration
- [ ] Wire API key resolution through `buzz-auth` keyring module
- [ ] Implement `EmbedConfig` builder supporting fallback to local ONNX
- [ ] Support environment variable overrides (`BUZZ_EMBED_PROVIDER`, `BUZZ_EMBED_MODEL`)

---

## Verification & Sign-off

- [ ] `cargo test -p orbit-ai` passes
- [ ] Local ONNX generates 384-dimensional normalized vector in <5ms
- [ ] Zero network requests emitted when running in local ONNX mode
- [ ] `just ci` passes cleanly
