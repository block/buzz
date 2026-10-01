# Feature 10 — Packaging & Hardening Checklist

> **Directory**: `orbit_release/features/10_packaging_hardening/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 6 (Week 10)

---

## Deliverables & Tasks

### 1. `~/.orbit/brain/` Storage Layout Initialization
- [ ] Auto-create `~/.orbit/brain/db/orbit.db` on first boot with `orbit_*` tables
- [ ] Auto-create `~/.orbit/brain/vectors/chunks/` for LanceDB
- [ ] Auto-create `~/.orbit/brain/graph/knowledge/` for Ladybug/Kùzu
- [ ] Auto-create `models/`, `transcripts/`, `exports/`, and `sync/` directories
- [ ] Verify directory permissions (0700 on Unix, restricted ACL on Windows)

### 2. Local In-Process Embedder & Model Bundling
- [ ] Pre-package quantized `bge-small-en-v1.5.onnx` (~30MB) into `desktop/src-tauri/resources/models/`
- [ ] Pre-package quantized `bge-reranker-small.onnx` (~45MB) into `desktop/src-tauri/resources/models/`
- [ ] Implement first-boot model extractor in `desktop/src-tauri/src/models.rs`
- [ ] Verify offline cold-start loads models without network requests
- [ ] Verify CPU embedding latency <5ms per chunk
- [ ] Verify CPU cross-encoder reranking latency <10ms for 50 candidate pairs

### 3. Tauri Packaging Configuration (`desktop/src-tauri/tauri.conf.json`)
- [ ] Configure `bundle.externalBin` to include:
  - [ ] `binaries/buzz-mcp`
  - [ ] `binaries/buzz-dev-mcp`
  - [ ] `binaries/buzz-agent`
  - [ ] `binaries/buzz-acp`
  - [ ] `binaries/buzz`
- [ ] Configure `bundle.resources` to include:
  - [ ] `resources/models/*`
  - [ ] `resources/plugins/*`
- [ ] Configure native Windows icons and `.msi` / `.exe` installer targets
- [ ] Configure macOS `.dmg` background and permissions entitlements

### 4. Zero-Docker Native Desktop Verification
- [ ] Confirm no PostgreSQL service binary is bundled or started by desktop app
- [ ] Confirm no Redis service binary is bundled or started by desktop app
- [ ] Confirm no Neo4j or FalkorDB service binary is bundled or started
- [ ] Confirm app executes 100% natively without Docker installed on host

### 5. Container & Relay Separation (Enterprise / Team Sync)
- [ ] Verify `docker-compose.yml` for hosted team relay (`crates/buzz-relay`, PostgreSQL 16 + pgvector, Redis) builds cleanly
- [ ] Verify Helm charts for Kubernetes staging cluster deployment
- [ ] Confirm local desktop app connects to relay as an optional NIP-29 client without requiring local container infrastructure

### 6. Sync Architecture & Mutation Log
- [ ] `sync/changelog.jsonl` records application-level logical mutations
- [ ] Snapshot metadata tracks replication watermarks
- [ ] Replay test reconstructs a clean brain from mutation log
- [ ] Database page/WAL files are never used as cross-device replication payload

### 7. Performance & Memory Gates
- [ ] Cold start to interactive UI: < 2.0 s
- [ ] In-process embedding: < 5 ms per chunk
- [ ] Vector retrieval (LanceDB): < 20 ms p95
- [ ] Graph 2-hop traversal: < 20 ms p95
- [ ] Hybrid retrieval: < 40 ms p95
- [ ] Cross-encoder reranking: < 10 ms p95
- [ ] Idle desktop RAM: < 60 MB
- [ ] Restart/recovery: 100% preserved state

---

## Verification & Sign-off

- [ ] `cargo test --workspace` passes cleanly
- [ ] Clean-machine installation test on Windows passes without Docker
- [ ] Clean-machine installation test on macOS passes without Docker
- [ ] Offline launch test successfully performs embeddings and RAG search
- [ ] `just ci` passes cleanly
