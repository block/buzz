# Feature 10 — Packaging & Hardening (Native Local Brain)

> **Priority**: P0  
> **Sprint**: Sprint 6  
> **Dependencies**: Features 01–09  
> **Target Artifacts**: Windows `.exe`, macOS `.dmg` / `.app`

## Overview

Feature 10 packages ORBIT as a native desktop product with **no Docker and no user-managed database server**.

The desktop runtime consists of:

- Tauri v2
- Rust core
- SQLite
- embedded LanceDB
- embedded Ladybug/Kuzu-compatible graph
- optional local ONNX models
- ORBIT MCP/agent integrations

## `orbit_brain/` layout

```text
orbit_brain/
├── db/
│   └── orbit.db
├── vectors/
│   └── chunks/
├── graph/
│   └── knowledge/
├── models/
│   └── *.onnx
├── transcripts/
├── exports/
└── sync/
    ├── changelog.jsonl
    ├── snapshot_meta.json
    └── peers.json
```

This directory contains ORBIT's product-owned state. Store-level implementations can change without changing the user-visible brain layout.

## Sync architecture

Do not replicate database files directly between devices.

Instead, ORBIT emits an application-level append-only mutation log:

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

The sync layer later translates these logical mutations into local SQLite/LanceDB/graph updates. This prevents engine-specific page/WAL files from becoming the replication protocol.

## Zero-server packaging

The installer must not install or start:

- PostgreSQL
- Redis
- Neo4j
- FalkorDB server
- Docker

Optional enterprise connectors can use remote servers when explicitly configured.

## Graph/vector backend policy

Desktop defaults:

- relational: SQLite
- vector: LanceDB
- graph: Ladybug/Kuzu

Remote adapters:

- PostgreSQL + pgvector for team/relay deployments
- Neo4j or FalkorDB for graph-heavy shared deployments

LanceDB provides Rust/Node/TypeScript/Python support and is explicitly designed as an embedded retrieval library. citeturn474137search2

## Performance gates

Replace generic DB-process targets with end-to-end product measurements:

| Metric | Initial target |
|---|---:|
| Cold start to interactive UI | < 2.0 s |
| Vector retrieval | < 20 ms p95 on 10k–50k chunks |
| Graph 2-hop retrieval | < 20 ms p95 on local benchmark |
| Hybrid retrieval before reranking | < 40 ms p95 |
| Reranking | measured on target model/CPU |
| Recall context assembly | < 100 ms p95 excluding remote LLM calls |
| Idle ORBIT storage overhead | measured and reported |
| Fresh local brain disk | measured and reported |
| 10k-chunk ingestion | benchmarked |
| Restart recovery | 100% successful |

Do not publish absolute RAM/disk claims until they are measured on release builds across Windows and macOS.

## Verification & Quality Gates

- [ ] Clean Windows install creates a brain without DB setup
- [ ] Clean macOS install creates a brain without DB setup
- [ ] Kill/restart test preserves the last committed memory state
- [ ] Export/import test reconstructs a brain
- [ ] Sign-in opens the external browser and returns through the registered ORBIT deep link
- [ ] Authorization-code interception test confirms PKCE binding
- [ ] Cloud-sync test never copies SQLite/LanceDB/graph database files between devices
- [ ] Offline mode continues to retrieve locally available context after cloud disconnect
- [ ] Sync-log replay reconstructs logical state
- [ ] `cargo test --workspace`
- [ ] release benchmark suite passes

## Account, subscription, and hosted-sync behavior

ORBIT remains fully usable as a local-first desktop application. First launch presents `Log In`, `Sign Up`, and `Continue Local`. Account creation and cloud features are website-first. The desktop app opens the ORBIT web login in the external browser and receives a one-time authorization result back through a registered deep link. The native app must use a public-client OAuth flow with PKCE; do not embed the login page inside a WebView. RFC 8252 recommends external user-agent authorization for native apps and requires PKCE for public native clients. citeturn658627search0turn658627search1

Tauri's deep-link plugin supports desktop custom schemes and app/universal links, so ORBIT can register both a branded HTTPS app link and a private-use fallback scheme. citeturn658627search4

Example flow:

```text
ORBIT desktop
    │
    ├── Local mode → continue without account
    │
    └── Sign in / Enable Cloud Sync
              │
              ▼
       https://orbit.example.com/login
              │
              ▼
       account + subscription checks
              │
              ▼
       one-time authorization code
              │
              ▼
       https://app.example.com/oauth/callback
              │
              └──────► ORBIT desktop deep-link handler
                            │
                            ▼
                   token exchange + device registration
                            │
                            ▼
                    local session established
```

The authorization callback must not contain long-lived access or refresh tokens in a URL. The callback carries a short-lived, single-use code bound to the login transaction/PKCE verifier; the app exchanges it through the token service. OAuth security guidance requires encrypted network transport and recommends protections against authorization-code interception. citeturn658627search2turn658627search3

### Local-first subscription behavior

| Product state | Local processing | Cloud data | Cross-device context |
|---|---|---|---|
| Local / no account | Yes | None | No |
| Signed-in, no sync entitlement | Yes | Account/device metadata only | No |
| Cloud Sync subscription | Yes | Selected encrypted brain data | Yes |
| Enterprise | Yes by default | Policy-controlled tenant storage and optional hosted processing | Yes |

A subscription should unlock **replication and account services**, not make basic local memory dependent on an internet connection. If the user stops the subscription, previously downloaded local data remains locally available; new cloud synchronization and subscription-only services become unavailable according to the service policy.

### Packaging/data directories

Extend the existing layout with account and sync state:

```text
orbit_brain/
├── db/orbit.db
├── vectors/
├── graph/
├── models/
├── transcripts/
├── exports/
└── sync/
    ├── changelog.jsonl
    ├── outbox/
    ├── inbox/
    ├── snapshots/
    ├── device.json
    └── account.json
```

Do not store bearer tokens in plain files. Store refresh credentials or equivalent native session secrets in the platform keyring and keep only non-sensitive account/device metadata in `account.json`.


### Identity/device data boundary

Account and device metadata are control-plane state, not memory state. Store session secrets in the OS keychain, use a random installation `device_id`, and do not make raw MAC addresses a required authentication factor. Enterprise telemetry must not include source-code, memory text or embeddings unless explicitly enabled by policy.
