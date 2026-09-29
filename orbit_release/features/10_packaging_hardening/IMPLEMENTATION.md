# Feature 10 — Packaging & Hardening (Native Local Brain, Bundled Embedder & Zero-Docker Architecture)

> **Priority**: P0 — Distribution, packaging, and local execution guarantees.  
> **Sprint**: Sprint 6 (Week 10)  
> **Dependencies**: Features 01–09  
> **Target Artifacts**: Windows `.exe` / `.msi`, macOS `.dmg` / `.app`, Linux `.AppImage` / `.deb`  
> **Packaging Config**: `desktop/src-tauri/tauri.conf.json`

---

## Overview

Feature 10 packages Orbit as a native, self-contained desktop application with **zero Docker requirements, zero external database servers, and an embedded in-process AI runtime**.

The desktop runtime bundles:
- Tauri v2 application shell and React 19 frontend
- Embedded SQLite (`orbit.db` with `orbit_*` tables and FTS5)
- Embedded LanceDB for vector storage
- Embedded Ladybug/Kùzu for knowledge graph traversal
- Embedded ONNX Runtime (`ort`) with pre-packaged `bge-small-en-v1.5` and `bge-reranker-small` models
- Bundled external binaries (`buzz-mcp`, `buzz-dev-mcp`, `buzz-agent`, `buzz-acp`, `buzz`)

---

## 4-Tier Implementation Layer Architecture

### 1. Frontend Tier: Update & Packaging UX
- **Desktop Settings (`desktop/src/features/settings/ui/`)**:
  - Model status card: Displays bundled model health, disk usage, and inference latency.
  - Storage footprint manager: Displays size of `~/.orbit/brain/` (db, vectors, graph, models, transcripts).
  - Clear cache / Re-index button.
- **Auto-Updater**: Integrates `tauri-plugin-updater` for seamless differential app updates.

### 2. Desktop Backend Tier: In-Process Embedder & Provisioning
- **Local Model Provisioning (`desktop/src-tauri/src/models.rs`)**:
  - On application boot, checks `~/.orbit/brain/models/`.
  - If model weights are missing, extracts pre-packaged models from Tauri's application resource directory (`desktop/src-tauri/resources/models/`) into `~/.orbit/brain/models/`.
  - Zero network download is required for cold-start initialization.
- **In-Process ONNX Runtime (`ort`)**:
  - Dynamically links platform-native ONNX libraries (`onnxruntime.dll` on Windows, `libonnxruntime.dylib` on macOS, `libonnxruntime.so` on Linux).
  - Executes embedding inference (`bge-small-en-v1.5`, 384 dimensions) in `<5ms`.
  - Executes cross-encoder reranking (`bge-reranker-small`) in `<10ms` over 50 candidate pairs on CPU.

### 3. Core Workspace Crates Tier (`crates/buzz-*`)
- `crates/buzz-ai`: Embedding and reranker abstractions, tensor tokenization, and ONNX session management.
- `crates/buzz-db`: Authoritative SQLite schema and migrations for all `orbit_*` tables.
- `crates/buzz-mcp` & `crates/buzz-dev-mcp`: Compiled into standalone binaries bundled with the desktop package.

### 4. Packaging, Bundling & Container Architecture Tier

#### Tauri Configuration: `desktop/src-tauri/tauri.conf.json`
```json
{
  "bundle": {
    "active": true,
    "targets": "all",
    "externalBin": [
      "binaries/buzz-acp",
      "binaries/buzz-agent",
      "binaries/buzz-backend-kubernetes",
      "binaries/buzz-dev-mcp",
      "binaries/buzz-mcp",
      "binaries/git-credential-nostr",
      "binaries/buzz"
    ],
    "resources": [
      "resources/models/*",
      "resources/plugins/*"
    ]
  }
}
```

#### Container Architecture & Boundary
- **Desktop Application (Local Mode)**:
  - 100% native execution.
  - **Zero Docker**: The installer must never install or require Docker, PostgreSQL, Redis, or Neo4j.
- **Hosted Team Relay (Enterprise Mode)**:
  - Docker Compose (`docker-compose.yml`) and Kubernetes Helm charts are exclusively reserved for centralized relay synchronization (`crates/buzz-relay`), hosting PostgreSQL 16 + pgvector, and Redis pub/sub.
  - The local desktop app never runs inside these containers and operates independently when disconnected.

---

## Local Directory Layout: `~/.orbit/brain/`

```text
~/.orbit/brain/
├── db/
│   └── orbit.db                  # Authoritative SQLite database (orbit_* tables + FTS5)
├── vectors/
│   └── chunks/                   # Embedded LanceDB vector storage (384-d dense embeddings)
├── graph/
│   └── knowledge/                # Embedded Ladybug/Kuzu graph storage
├── models/                       # Bundled ONNX model weights
│   ├── bge-small-en-v1.5.onnx    # Dense embedder (~30MB)
│   └── bge-reranker-small.onnx   # Cross-encoder reranker (~45MB)
├── transcripts/                  # Ingested IDE session logs
├── exports/                      # JSON-LD & markdown graph snapshots
└── sync/                         # Logical sync records (when cloud sync is active)
    ├── changelog.jsonl
    ├── snapshot_meta.json
    └── peers.json
```

---

## Sync Architecture

Do not replicate database files directly between devices.

Instead, Orbit emits an application-level append-only mutation log:

```json
{
  "seq": 10482,
  "timestamp": "2026-09-20T22:00:00Z",
  "operation": "UPSERT_MEMORY",
  "workspace_id": "ws-123",
  "entity_id": "mem-456",
  "content_hash": "a1b2c3d4...",
  "payload_ref": "exports/..."
}
```

The sync layer translates these logical mutations into local SQLite/LanceDB/graph updates. This prevents engine-specific page/WAL files from becoming the replication protocol.

---

## Performance Gates

Measured end-to-end product targets:

| Metric | Target |
|---|---:|
| Cold start to interactive UI | < 2.0 s |
| In-process vector embedding (`bge-small-en-v1.5`) | < 5 ms per chunk |
| Vector retrieval (LanceDB) | < 20 ms p95 on 10k–50k chunks |
| Graph 2-hop retrieval (Kùzu) | < 20 ms p95 |
| Hybrid retrieval before reranking | < 40 ms p95 |
| Cross-encoder reranking (`bge-reranker-small`) | < 10 ms p95 for 50 pairs |
| SuperRAG context compilation | < 80 ms p95 |
| Idle desktop memory footprint | < 60 MB RAM |
| Fresh local brain disk consumption | < 120 MB (including models) |
| Restart recovery | 100% successful |

---

## Account, Subscription, and Hosted-Sync Behavior

Orbit remains fully usable as a local-first desktop application. First launch presents `Log In`, `Sign Up`, and `Continue Local`. Account creation and cloud features are website-first. The desktop app opens the Orbit web login in the external browser and receives a one-time authorization result back through a registered deep link (`orbit://oauth/callback`). The native app uses a public-client OAuth flow with PKCE (RFC 8252); do not embed the login page inside a WebView.

### Local-First Subscription Behavior

| Product state | Local processing | Cloud data | Cross-device context |
|---|---|---|---|
| Local / no account | Yes (100% offline) | None | No |
| Signed-in, no sync entitlement | Yes | Account/device metadata only | No |
| Cloud Sync subscription | Yes | Selected encrypted brain data | Yes |
| Enterprise | Yes by default | Policy-controlled tenant storage and optional hosted processing | Yes |

A subscription unlocks **replication and account services**, not basic local memory. If the user stops the subscription, previously indexed local data remains locally available.

---

## Verification & Quality Gates

- [ ] Clean Windows installation initializes `~/.orbit/brain/` without Docker or DB servers.
- [ ] Clean macOS installation initializes `~/.orbit/brain/` without Docker or DB servers.
- [ ] Pre-packaged ONNX models extract and load successfully on offline first boot.
- [ ] In-process embedding and reranking execute within performance targets on CPU.
- [ ] Kill/restart test preserves the last committed memory state in `orbit.db`.
- [ ] Export/import test reconstructs a brain cleanly.
- [ ] Deep-link authorization with PKCE completes without leaking credentials in URLs.
- [ ] Offline mode continues to retrieve locally available context when network is disconnected.
- [ ] `cargo test --workspace` passes cleanly.
- [ ] `just ci` passes cleanly.
