# Feature 01 — Storage Foundation (Local-First Embedded Memory Store)

> **Priority**: P0 — foundational storage for ORBIT's local-first context engine.  
> **Sprint**: Sprint 1  
> **Dependencies**: None  
> **Primary principle**: Keep the desktop memory substrate embedded, file-backed, Rust-native, and replaceable.

## Overview

The original plan used a bundled PostgreSQL 16 + pgvector runtime as the universal local and remote database. That is now superseded for the desktop product.

ORBIT's primary workload is a **single-user, local-first AI memory engine inside a Tauri/Rust application**. The desktop database must therefore minimize external processes, installation size, startup cost, operational complexity, and failure surfaces.

The recommended V1 storage architecture is:

- **Relational/state metadata:** SQLite (file-backed) for documents, chunks, provenance, sync state, settings, and job metadata.
- **Vector retrieval:** LanceDB as the embedded persistent vector store.
- **Knowledge graph:** embedded Ladybug/Kuzu-compatible graph backend for entity/relation traversal.
- **Context engine:** ORBIT-owned Rust retrieval/orchestration layer over these stores.
- **Remote/team evolution:** adapters for PostgreSQL/pgvector and graph services are deferred to the sync/enterprise layer.

This direction aligns with the current Rust implementation of Cognee, which uses SQLite + embedded Ladybug + embedded LanceDB as its default no-external-service stack. Cognee-RS exposes the same high-level memory lifecycle (`remember`, `recall`, `improve`, `forget`) and supports pluggable vector/graph providers. citeturn115698search0turn115698search1turn474137search5

## Why the architecture changes

### 1. PostgreSQL is an infrastructure service; ORBIT needs an embedded product component

Bundling PostgreSQL means shipping, initializing, supervising, upgrading, and recovering a complete DBMS. This conflicts with ORBIT's core product requirement: a desktop app that behaves like a native local utility rather than a local server stack.

The old `<150MB RAM` target should not be treated as proof that PostgreSQL is lightweight; it was an architectural estimate, not a measured release benchmark. The new target is measured process RSS for **Tauri + ORBIT storage + retrieval + model runtime**, with no Docker and no user-managed DB server.

### 2. Do not force one database to perform three different jobs

The memory model has three different access patterns:

| Workload | Required operation | V1 store |
|---|---|---|
| Source metadata / provenance / jobs | transactional rows, constraints, updates | SQLite |
| Semantic retrieval | approximate nearest-neighbor search | LanceDB |
| Entity/relation reasoning | graph traversal, neighborhood expansion | Ladybug/Kuzu |

The abstraction belongs above the databases, not inside a single universal DB.

### 3. Keep the Rust/Tauri hot path native

Cognee-RS now provides a directly relevant Rust reference: its default build is an embedded, no-external-service stack using SQLite, Ladybug, and LanceDB; its vector and graph providers are behind interfaces, which is the pattern ORBIT should mirror rather than importing the Python Cognee runtime. citeturn115698search1turn115698search0

## Storage model

### SQLite: authoritative metadata and provenance

Tables:

- `documents`
- `chunks`
- `memory_items`
- `sources`
- `sessions`
- `decisions`
- `sync_log`
- `ingestion_jobs`
- `memory_feedback`
- `working_contexts`

SQLite remains the source of truth for IDs, relationships that do not require graph traversal, timestamps, hashes, scopes, permissions, and lifecycle state.

### LanceDB: vector index

Store:

- `chunk_id`
- `document_id`
- `workspace_id`
- `embedding`
- normalized text or compact retrieval payload
- metadata needed for filtering

LanceDB is embedded, file-backed, supports vector + metadata search, has Rust support, and is designed specifically for local/AI retrieval workloads. citeturn474137search2

### Ladybug/Kuzu-compatible graph store

Store:

- entities
- concepts
- files
- sessions
- agents
- decisions
- typed relations
- `valid_at`
- `invalid_at`
- provenance IDs

Graph traversal must return IDs and evidence references; full text remains in SQLite/LanceDB rather than being duplicated in the graph.

Cognee currently documents Ladybug/Kuzu as its embedded graph backend and explicitly recommends graph-native storage rather than PostgreSQL-as-graph for production graph workloads. citeturn115698search5

## ORBIT storage abstraction

Create a `MemoryStore` facade in Rust:

```rust
pub trait MetadataStore: Send + Sync {
    async fn upsert_document(&self, doc: Document) -> Result<()>;
    async fn get_document(&self, id: DocumentId) -> Result<Option<Document>>;
    async fn record_feedback(&self, feedback: MemoryFeedback) -> Result<()>;
}

pub trait VectorStore: Send + Sync {
    async fn upsert_embeddings(&self, items: &[EmbeddingRecord]) -> Result<()>;
    async fn search(&self, query: &[f32], filter: VectorFilter, k: usize)
        -> Result<Vec<VectorHit>>;
}

pub trait GraphStore: Send + Sync {
    async fn upsert_entities(&self, entities: &[Entity]) -> Result<()>;
    async fn upsert_relations(&self, relations: &[Relation]) -> Result<()>;
    async fn neighborhood(&self, seed_ids: &[EntityId], hops: u8)
        -> Result<Vec<GraphHit>>;
}
```

The rest of ORBIT must depend on these traits, not vendor-specific APIs.

## Data flow

```text
                    ORBIT MEMORY FACADE
                           |
          +----------------+----------------+
          |                |                |
       SQLite          LanceDB       Ladybug/Kuzu
     metadata/txns       vectors         graph
          |                |                |
          +----------------+----------------+
                           |
                    SuperRAG / Recall
                           |
                    Working Context
                           |
                        MCP / IDE
```

## What is retained from the old design

The following are retained:

- five-layer context model
- immutable source hashes
- AST-aware chunks
- 384-d or configurable embeddings
- lexical retrieval
- RRF
- graph retrieval
- cross-encoder reranking
- token-budget context packing
- provenance
- temporal memory
- MCP tools
- recall plugins
- sync changelog

Only the **storage implementation boundary** changes.

## What is removed from V1

Remove from the desktop V1 path:

- bundled PostgreSQL server
- local pgvector dependency
- PostgreSQL GIN/tsvector dependency
- PostgreSQL recursive CTE as graph traversal
- PostgreSQL as the authoritative local graph store

These remain future adapters for remote/team deployments.

## Local/Hosted data contract

ORBIT has two product modes but one logical memory model:

| Concern | Local mode | Cloud-sync / Enterprise mode |
|---|---|---|
| Interactive processing | Local Rust engine | Still local by default |
| Metadata authority | Local SQLite | Local SQLite + server canonical event/state replica |
| Vector index | Local LanceDB | Local LanceDB on each device; optional server-side `pgvector` index |
| Graph index | Local embedded graph | Local graph on device; optional hosted graph service |
| Raw source | Local filesystem/object store | Local by default; explicit opt-in sync to encrypted cloud object storage |
| Sync unit | N/A | Logical memory/source/event mutations, never DB page/WAL files |
| Offline operation | Full | Full for already-downloaded data |
| Device identity | Local device ID | Registered device + workspace/user scope |

The hosted service is a **replication and coordination layer**, not a requirement for the desktop memory engine. A cloud subscription enables encrypted synchronization of selected ORBIT data between devices; it does not turn local retrieval into a network dependency.

### Derived indexes are rebuildable

Treat embeddings, FTS indexes, graph materializations, caches and reranker artifacts as derived state. The canonical sync payload is the logical record/event model plus immutable source references. This lets a Windows desktop, macOS desktop and future mobile client rebuild compatible local indexes without attempting to copy database files between platforms.

### Optional warm-sync artifacts

For faster second-device startup, the hosted service may also store versioned derived artifacts such as embeddings, summaries and compact retrieval metadata. These are optional accelerators, never the only copy of a memory. A client must be able to rebuild them from canonical records.

## Verification & Quality Gates

- Start ORBIT with no Docker and no separately installed database.
- Create a fresh brain in an empty directory.
- Ingest 10k code/text chunks.
- Verify crash-safe reopen of all stores.
- Verify vector IDs and metadata remain consistent after restart.
- Verify graph IDs resolve back to SQLite/LanceDB evidence.
- Verify deletion removes/invalidates data across all three stores.
- Benchmark startup, RSS, ingestion throughput, vector search, graph traversal, and end-to-end recall on real ORBIT fixtures.
