# ORBIT Architecture & Product Decision History
> Historical record consolidated from the prior release package. The original decision content is preserved below.

---

# ORBIT Architecture Update — Changes from `orbit_release.zip`

## Replaced

**Old local storage**
- Bundled PostgreSQL 16
- pgvector
- PostgreSQL GIN/tsvector
- PostgreSQL recursive CTE graph traversal
- PostgreSQL as the universal desktop storage engine

**New local storage**
- SQLite for authoritative metadata/provenance/state
- LanceDB for embeddings/vector retrieval
- embedded Ladybug/Kuzu-compatible graph for entity/relation traversal
- Rust storage traits as the stable interface

## Retained

- 5-layer context model
- AST chunking
- semantic embeddings
- hybrid retrieval
- RRF
- temporal weighting
- reranking
- working-context compilation
- bi-temporal memory
- recall plugins
- MCP server/client/arbiter concepts
- cloud/team sync concept

## New architectural rule

Databases are **implementation details behind ORBIT memory interfaces**.

The durable public contract is:

```text
MetadataStore
VectorStore
GraphStore
MemoryStore
RecallEngine
ContextCompiler
```

This lets V1 stay tiny and local while preserving a clean path to:

```text
PostgreSQL + pgvector
Neo4j
FalkorDB
hosted vector stores
```

for future collaborative/enterprise deployments.

## Why this is better for ORBIT

The user-facing goal is long-term memory for IDE agents, not a generic database product. The important unit is the **memory lifecycle**:

```text
ingest → normalize → embed → extract → consolidate → retrieve
       → rerank → compile context → agent → feedback → improve
```

Storage should support this lifecycle without dictating its architecture.

---

# ORBIT Hosted / Enterprise Update — 2026-09-29

## What changed

The implementation package now models ORBIT as a **local-first product with an optional hosted/enterprise edition**.

### Added

- Website-first signup/login.
- Desktop OAuth public-client flow with PKCE.
- Tauri deep-link callback handling.
- Account/device registration.
- Subscription entitlement model.
- Optional encrypted cloud synchronization.
- Multi-device event-based synchronization.
- Future mobile client compatibility.
- Hosted PostgreSQL + pgvector architecture.
- Object storage and background workers.
- Enterprise tenant/workspace/security controls.
- Explicit separation between cloud storage and hosted processing.

### Preserved

- Five-layer ORBIT context architecture.
- Hybrid RAG and RRF.
- Context-aware reranking.
- Temporal memory.
- Knowledge graph.
- Local SQLite + LanceDB + embedded graph stack.
- MCP and agent auto-wiring.

### Important architectural rule

**Cloud sync copies logical state, not database files.**

SQLite/LanceDB/graph databases remain device-local execution indexes. The hosted service stores canonical logical memory/event state, plus optional derived artifacts for faster rehydration.

## Why this matters

This lets ORBIT provide all three experiences without splitting the product into separate architectures:

1. **Private local brain** — no account, no cloud.
2. **Personal cloud-synced brain** — local processing plus encrypted multi-device replication.
3. **Enterprise brain** — local-first desktop processing plus tenant-controlled hosted storage, identity, policy and optional hosted compute.

The same Rust memory/retrieval contracts support all three.

---

# ORBIT Identity, Device & Enterprise Governance Update — 2026-09-29

## Product decision

ORBIT now treats account identity as a hosted control-plane capability while keeping the memory engine local-first. First launch can show `Log In`, `Sign Up`, and `Continue Local`. Signup/login happen on the ORBIT web authentication server. The resulting session is stored securely on the desktop so returning users experience a normal native-app session.

## Added

- dedicated ORBIT Auth Server boundary
- email/password signup and recovery owned by the web service
- desktop deep-link authorization with one-time code exchange
- persistent OS-keychain session state
- random per-installation device IDs and device key pairs
- device registry/revocation
- privacy-limited device/security telemetry
- personal versus enterprise workspace ownership model
- enterprise data-classification and processing policy
- local-only / cloud-sync / hosted-processing gates
- retention, deletion, export and audit extension points
- enterprise model/provider allow-lists
- policy version attached to processing decisions

## Security rule added

A raw MAC address is not treated as an authenticator or permanent account identifier. When extra device-risk information is useful, ORBIT should use a purpose-limited derived signal with explicit retention and product-policy controls.

## Sync rule

Identity metadata and device telemetry are never synchronized as memory. Cloud sync transports canonical logical brain/workspace records and events, subject to workspace policy. Local indexes remain rebuildable derived state.

## Enterprise rule

Personal and organization-owned workspaces are separate authorization domains. Enterprise membership does not grant access to personal memory by default, and enterprise memory must not flow into a personal workspace through generic sync.
