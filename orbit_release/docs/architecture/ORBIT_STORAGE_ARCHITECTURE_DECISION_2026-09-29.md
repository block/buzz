# ORBIT Storage & Context Architecture Decision — 2026-09-29

## Decision

For the **desktop/local-first ORBIT V1**, use:

**SQLite + LanceDB + embedded Ladybug/Kuzu-compatible graph, behind ORBIT-owned Rust storage traits.**

Do **not** make Neo4j, FalkorDB, or PostgreSQL/pgvector the mandatory local desktop database.

Use **Cognee as architectural/reference inspiration and selectively reuse Cognee-RS components or concepts**, but do not make the entire Python Cognee stack a hard runtime dependency of the Tauri app.

## Why

ORBIT needs four properties simultaneously:

1. persistent long-term memory
2. fast semantic recall
3. graph-based multi-hop reasoning
4. native local execution with low operational/storage overhead

No single candidate solves all four equally well in the current product shape.

### Candidate comparison

| Option | Local/embedded fit | Vector | Graph | Rust/Tauri fit | Operational cost | ORBIT role |
|---|---|---|---|---|---|---|
| PostgreSQL + pgvector | Medium | Excellent | Weak as native graph | Medium | High | Remote/team adapter |
| Neo4j | Medium | Can support vector workflows | Excellent | Medium-low for native Rust app | High | Remote graph adapter |
| FalkorDB | Medium | Good via ecosystem | Excellent | Medium | Medium-high | Optional remote graph adapter |
| LanceDB + SQLite + graph | Excellent | Excellent | Excellent when paired with embedded graph | Excellent | Low | **Desktop V1** |
| Cognee Python | Low-medium | Good | Good | Low for direct Tauri embedding | Medium | Reference/integration layer |
| Cognee-RS | Excellent | Excellent | Excellent | **Excellent** | Low-medium | **Reference / candidate engine** |

Cognee's current project supports interface-based graph/vector adapters, and Cognee-RS documents an embedded default using SQLite, Ladybug, and LanceDB. citeturn684197search1turn115698search0

## Why Cognee is relevant to ORBIT

Cognee solves a different layer than “which database should I choose.”

It provides memory lifecycle/orchestration:

`remember → recall → improve → forget`

and internally composes ingestion, graph construction, embeddings, retrieval and memory operations. Cognee-RS exposes that model in Rust. citeturn474137search4turn474137search5

That makes the correct ORBIT strategy:

```text
                 ORBIT PRODUCT LAYER
        --------------------------------------
        MCP + IDE adapters + policy + UX
                        |
                ORBIT Context API
                        |
            Cognee-inspired memory layer
       remember / recall / improve / forget
                        |
              ORBIT Retrieval Arbiter
              /        |                       /         |                   SQLite     LanceDB      GraphStore
      metadata     vectors      Ladybug/Kuzu
             \         |          /
              \        |         /
               Context Compiler
                     |
                 Agent turn
```

## What ORBIT should own

Keep these ORBIT-specific:

- IDE transcript parsers
- source watchers
- Git provenance
- workspace scoping
- security/redaction
- MCP tool policy
- context-budget compiler
- agent harness wiring
- sync protocol
- user-facing brain graph
- feedback signals and memory promotion policy

## What should be reused or adapted

Prefer these ideas/components from Cognee-RS:

- `remember` / `recall` / `improve` / `forget` lifecycle
- store abstractions
- graph/vector separation
- deterministic graph IDs
- graph-aware retrieval routing
- local-first default backends
- memory improvement from feedback

Do not directly depend on the Python Cognee runtime in the Tauri hot path.

Cognee's Python implementation is organized around Python services and FastAPI, while Cognee-RS is the Rust-native option intended for on-device use. citeturn474137search1turn474137search4

## Recall architecture

The long-term-memory loop should be:

```text
INGEST
  ↓
Normalize + redact + hash
  ↓
Chunk + embed
  ↓
SQLite source/provenance
  +
LanceDB vector index
  +
Graph entity/relation extraction
  ↓
MEMORY PROMOTION
  ↓
episodic → durable semantic memory
  ↓
QUERY
  ↓
intent router
  ↓
parallel vector + lexical + graph retrieval
  ↓
RRF
  ↓
temporal + authority + workspace weighting
  ↓
reranker
  ↓
evidence validation
  ↓
context compiler
  ↓
MCP
  ↓
agent
  ↓
feedback
  ↓
IMPROVE
```

## Important architectural correction

Do **not** treat “RAG database” as ORBIT's memory.

The database only stores and indexes evidence.

The actual memory system is:

**storage + extraction + temporalization + consolidation + retrieval + feedback + context compilation.**

This distinction is important because it means switching PostgreSQL → LanceDB or Neo4j → embedded graph later does not invalidate the cognitive architecture.

## Why pgvector should stay available

pgvector remains valuable for the future synchronized/team architecture because it gives vector indexing inside PostgreSQL and supports HNSW/IVFFlat. The HNSW index has a strong speed/recall profile but consumes more memory and takes longer to build than IVFFlat. citeturn684197search5

Therefore:

- local = LanceDB
- relay/team = PostgreSQL + pgvector
- same `VectorStore` trait above both

## Why Neo4j should stay an adapter

Neo4j is appropriate when ORBIT eventually needs a shared/enterprise knowledge graph, advanced graph tooling, or external graph infrastructure. It should not be a required desktop component.

Neo4j provides local development/embedded options, but its current product documentation is centered around DBMS/Desktop deployment rather than the smallest possible native-app runtime. citeturn684197search0turn684197search8

## Why FalkorDB should stay an adapter

FalkorDB is attractive for high-performance graph workloads and has an official Rust client with an embedded mode. However, the embedded mode still launches `redis-server` with the FalkorDB module, and the bundled module carries SSPL licensing implications. citeturn830492search0turn830492search3

That is unnecessary runtime complexity for ORBIT V1.

## Final architecture

```text
Tauri / Rust
│
├── buzz-core
│   ├── Memory API
│   ├── lifecycle
│   └── policy
│
├── buzz-ingest
│   ├── filesystem
│   ├── Git
│   └── IDE transcripts
│
├── buzz-search / buzz-recall
│   ├── lexical
│   ├── vector
│   ├── graph
│   ├── RRF
│   ├── rerank
│   └── context compiler
│
├── buzz-db
│   ├── MetadataStore → SQLite (~/.orbit/brain/db/orbit.db)
│   ├── VectorStore   → LanceDB (~/.orbit/brain/vectors/)
│   └── GraphStore    → Ladybug/Kuzu (~/.orbit/brain/graph/)
│
├── buzz-ai
│   ├── embeddings
│   ├── reranker
│   └── optional local LLM
│
└── buzz-mcp / buzz-dev-mcp
    ├── search_context
    ├── store_memory
    ├── recall_session
    ├── mark_decision
    └── forget

Optional remote adapters:
    PostgreSQL + pgvector
    Neo4j
    FalkorDB
```

## Migration from the old blueprint

1. Replace Feature 01 PostgreSQL schema work with storage traits + three embedded stores.
2. Replace Feature 04 PostgreSQL vector/FTS queries with VectorStore + SQLite FTS5 adapters.
3. Replace Feature 05 PostgreSQL recursive graph CTEs with GraphStore traversal.
4. Replace Feature 10 embedded PostgreSQL bundling with embedded stores under `orbit_brain/`.
5. Keep all higher-level retrieval/memory logic.
6. Add compatibility adapters only after local V1 is working.\n\n# Revised deployment decision — Local + Hosted editions\n\nThe storage decision is now explicitly **two-mode** rather than "one database everywhere":\n\n| Layer | Local / desktop | Hosted / enterprise |\n|---|---|---|\n| Metadata/events | SQLite | PostgreSQL |\n| Vectors | LanceDB | pgvector / future vector service |\n| Graph | Embedded graph | Relational graph initially; dedicated graph later if justified |\n| Raw artifacts | Local filesystem | Object storage, encrypted and policy controlled |\n| Processing | Local-first | Local-first; hosted processing optional |\n| Sync | None by default | Logical event/state replication |\n\nThe important invariant is the **logical contract**, not identical database engines. This preserves the lightweight Tauri product while avoiding a later rewrite when cloud synchronization and enterprise scale are introduced.\n\nThe cloud service must not require a user to install or run PostgreSQL/Redis/Neo4j locally.\n

## Identity and enterprise governance boundary (2026-09-29 update)

The storage architecture now has three deliberately separated classes of state:

1. **Identity/control-plane state** — hosted auth service: account/email/password lifecycle, sessions, subscriptions and organization membership.
2. **Device/security state** — random device IDs, device keys and limited security telemetry, stored separately from memory content.
3. **Brain/workspace state** — SQLite/LanceDB/embedded graph locally and PostgreSQL/pgvector/object storage for hosted enterprise data.

The desktop must not store plaintext passwords and the memory database must not become the identity database. A raw MAC address is not required as an authenticator; use a random installation ID and, where justified, a derived device-risk signal.

Enterprise ownership is modeled explicitly with user-vs-organization ownership, tenant/workspace/project scope, classification, retention and processing policy. Policy gates apply before sync, hosted processing, retrieval and context export.
