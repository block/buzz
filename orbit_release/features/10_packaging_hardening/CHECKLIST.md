# Feature 10 — Packaging & Hardening Checklist

> **Directory**: `orbit_release/features/10_packaging_hardening/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 6 (Week 10)

---

## Deliverables & Tasks

### 1. `orbit_brain/` Structured Storage & Cloud Sync Readiness
- [ ] Initialize `orbit_brain/` folder layout:
  - [ ] `orbit_brain/storage/` (PostgreSQL cluster)
  - [ ] `orbit_brain/models/` (ONNX weights)
  - [ ] `orbit_brain/transcripts/` (Agent logs cache)
  - [ ] `orbit_brain/exports/` (Memory backups)
  - [ ] `orbit_brain/sync/` (Replication WAL & metadata)
- [ ] Implement `changelog.wal` writer in `buzz-db` for cloud sync staging
- [ ] Implement snapshot export/import utilities for cross-device migration

### 2. Zero-Docker Embedded PostgreSQL Engine
- [ ] Bundle PostgreSQL 16 binaries with `pgvector` in `desktop/src-tauri/binaries/`
- [ ] Implement Tauri background lifecycle manager for PostgreSQL (auto-start / auto-stop)
- [ ] Configure non-conflicting port (`5433`)
- [ ] Add graceful shutdown hooks to prevent WAL corruption

### 3. Native Packaging (Windows & macOS)
- [ ] Configure Tauri 2 bundle settings in `desktop/src-tauri/tauri.conf.json`
- [ ] **Windows**: Configure NSIS installer script (`Orbit-Setup-x64.exe`)
- [ ] **Windows**: Configure standalone portable build (`Orbit-Portable-x64.exe`)
- [ ] **macOS**: Configure `.dmg` creation with drag-to-Applications layout
- [ ] **macOS**: Set up code signing and Apple notarization pipeline

### 4. Performance & Reliability Benchmarking
- [ ] Benchmark Multi-RAG search P99 latency (`< 25ms` across 10,000 chunks)
- [ ] Benchmark local ONNX embedding latency (`< 5ms` per chunk)
- [ ] Benchmark idle memory consumption (`< 150MB` RAM)
- [ ] Stress-test simultaneous agent recall ingestion and search queries

---

## Verification & Sign-off

- [ ] Windows `.exe` installer installs and runs cleanly on a virgin Windows environment
- [ ] macOS `.dmg` mounts and installs cleanly on Apple Silicon & Intel macOS
- [ ] `orbit_brain/` directory structure created properly with zero external dependencies
- [ ] All latency and resource benchmarks pass
- [ ] `just ci` passes cleanly
