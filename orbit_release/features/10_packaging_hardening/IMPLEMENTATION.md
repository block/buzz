# Feature 10 — Packaging & Hardening (Windows .exe, macOS .dmg & Cloud-Sync Storage)

> **Priority**: P0 — Packaging, zero-Docker desktop engine, and hardening.  
> **Sprint**: Sprint 6 (Week 10)  
> **Dependencies**: All Features (01 through 09)  
> **Target Artifacts**: Windows `.exe` (NSIS & Portable), macOS `.dmg` / `.app`  
> **Environment Variables**: `BUZZ_DATA_DIR`, `BUZZ_DATABASE_URL`

---

## Overview

Feature 10 provides the final integration, packaging, and hardening of Orbit. It bundles an embedded PostgreSQL 16 + pgvector runtime for a **zero-Docker, zero-setup desktop experience**, packages native Windows `.exe` and macOS `.dmg` deliverables, enforces performance benchmarks, and structures the `orbit_brain/` directory for **future cloud synchronization**.

---

## Data Architecture: `orbit_brain/` Layout & Cloud Sync

All application data is consolidated under `orbit_brain/` (default: `~/.buzz/orbit_brain/`, configurable via `BUZZ_DATA_DIR`):

```
orbit_brain/
├── storage/              ← Embedded PostgreSQL 16 cluster + pgvector tables
├── models/               ← Local ONNX weights (e.g. bge-small-en-v1.5.onnx)
├── transcripts/          ← Cached raw IDE session logs (Antigravity, Claude, Cursor, etc.)
├── exports/              ← Semantic memory snapshots & JSONL export archives
└── sync/                 ← Cloud synchronization staging engine:
    ├── changelog.wal     ← Append-only delta log of inserted/updated chunks & relations
    ├── snapshot_meta.json← State vector & replication watermarks
    └── peers.json        ← Configured remote sync relays / Orbit Cloud endpoints
```

### Cloud Synchronization Architecture (Future-Proof Design)

The `sync/changelog.wal` records atomic mutation events:
```json
{
  "csn": 10482,
  "timestamp": "2026-09-20T22:00:00Z",
  "operation": "UPSERT_CHUNK",
  "workspace_id": "ws-123",
  "content_hash": "a1b2c3d4...",
  "payload": { ... }
}
```
When cloud syncing is enabled in future updates:
1. The desktop daemon synchronizes WAL records with the remote Orbit cloud relay via secure WebSockets.
2. Vector embeddings and knowledge graph relations replicate bidirectionally across the developer's devices without schema conflicts.

---

## Zero-Docker Embedded PostgreSQL Engine

For standalone desktop operation, Orbit bundles a lightweight PostgreSQL 16 binary with `pgvector` pre-compiled:
- **Location**: `desktop/src-tauri/binaries/postgres`
- **Data Cluster**: `~/.buzz/orbit_brain/storage/`
- **Port**: `127.0.0.1:5433` (isolated from any system Postgres on 5432)
- **Lifecycle**: Automatically started and stopped by the Tauri background process.
- **Resource Footprint**: <150MB RAM at idle.

---

## Native Packaging Deliverables

### 1. Windows: Native `.exe`
- **Installer**: NSIS Setup executable (`Orbit-Setup-x64.exe`)
- **Portable**: Standalone executable (`Orbit-Portable-x64.exe`)
- **Signing**: Authenticode signed.
- **Tauri Config**: Built via `pnpm tauri build --target x86_64-pc-windows-msvc`.

### 2. macOS: `.dmg` & `.app`
- **Package**: Drag-and-drop disk image (`Orbit-macOS-Universal.dmg`)
- **Bundle**: Universal binary `.app` supporting both Apple Silicon (M1/M2/M3/M4) and Intel.
- **Signing & Notarization**: Apple Developer ID signed and notarized via `notarytool`.

---

## Performance Benchmark Gates

Before release, the package must pass the following automated performance gates:

| Metric | Target | Verification Tool |
|--------|--------|-------------------|
| **Multi-RAG Search P99** | `< 25ms` (10,000 chunks) | `cargo bench -p buzz-search` |
| **Local ONNX Embedding** | `< 5ms` / chunk (CPU) | `cargo bench -p orbit-ai` |
| **Idle Memory Footprint** | `< 150MB` RAM | System monitor validation |
| **App Cold Start** | `< 1.8s` to interactive UI | Tauri lifecycle probe |
| **Recall Parsing** | `> 500` turns / second | `cargo bench -p buzz-recall` |

---

## Verification & Quality Gates

- Windows install: run `Orbit-Setup-x64.exe` on clean Windows sandbox → verify desktop launch & DB init.
- macOS install: open `Orbit-macOS-Universal.dmg` on clean macOS VM → verify app runs without security warnings.
- Verify `orbit_brain/` directory structure created with all subdirectories (`storage/`, `models/`, `transcripts/`, `exports/`, `sync/`).
- Run `just ci`.
