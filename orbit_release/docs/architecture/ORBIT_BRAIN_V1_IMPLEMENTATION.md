# Buzz Brain V1 (Orbit Memory) — Full Implementation Blueprint: Architecture, 5-Layer Context Engine, SuperRAG & Skills

> **Status**: Active Engineering Blueprint  
> **Brand & Architecture Alignment**:  
> • **Buzz**: The active engineering architecture, crate ecosystem (`crates/buzz-*`), and code variable naming standard (`buzz_*`).  
> • **Orbit**: The user-facing brand identity, user interface, database tables (`orbit_*`), and local storage folder (`~/.orbit/`).  
> • **Execution Rule**: All code implementation plans, crate modifications, and in-code variables use the **current Buzz codebase architecture** to prevent variable collisions and architectural drift. Everything that remains on the local machine and that the user sees (UI screens, local folders, database tables) must be Orbit with no user-visible Buzz. Brand metadata updates occur post-release.  
> **Design Principle**: Pure implementation plan, specifications, and execution steps. No raw code snippets.

---

## Table of Contents

1. [Architectural Overview & Core Decisions](#1-architectural-overview--core-decisions)
2. [Local-First Storage Architecture — SQLite + LanceDB + Embedded Graph](#2-local-first-storage-architecture--sqlite--lancedb--embedded-graph)
3. [Configurable Embedding & Reranker Engine — Local ONNX & Cloud Models](#3-configurable-embedding--reranker-engine--local-onnx--cloud-models)
4. [The 5-Layer Compact Context Architecture & SuperRAG Retrieval Engine](#4-the-5-layer-compact-context-architecture--superrag-retrieval-engine)
5. [Tri-Modal MCP Architecture & Centralized Ingestion Plan](#5-tri-modal-mcp-architecture--centralized-ingestion-plan)
6. [Plugins & Ingestion Toolchain (`buzz-plugins` & `buzz-recall`)](#6-plugins--ingestion-toolchain-buzz-plugins--buzz-recall)
7. [Skills & Agent Surface Map (`.agents`, `.claude`, `.codex`, `.goose`, `.cursor`)](#7-skills--agent-surface-map-agents-claude-codex-goose-cursor)
8. [Agent Auto-Wiring Toolchain (`buzz-cli`)](#8-agent-auto-wiring-toolchain-buzz-cli)
9. [Desktop UI & Packaging Implementation Plan](#9-desktop-ui--packaging-implementation-plan)
10. [Step-by-Step Implementation Roadmap (Sprints 1–6)](#10-step-by-step-implementation-roadmap-sprints-16)
11. [Post-V1 Hosted / Enterprise Roadmap](#11-post-v1-hosted--enterprise-roadmap)

---


# 0. Product deployment model

ORBIT is designed as a local-first desktop brain with an optional hosted/enterprise control plane. The two modes share the same logical memory schema, retrieval stages, ranking logic and APIs. Only storage/transport adapters and optional server-side processing differ.

```text
                 ORBIT Memory Engine
                        │
          ┌─────────────┴─────────────┐
          │                           │
      Local mode                 Hosted mode
          │                           │
  SQLite + LanceDB +         PostgreSQL + pgvector
  embedded graph             + object storage
          │                           │
  local processing            local processing first
  offline capable             + encrypted cloud sync
                                      │
                              optional hosted compute
```

### Account and sign-in

First launch presents `Log In`, `Sign Up`, and `Continue Local`. Signup and password handling happen on the hosted ORBIT Auth Server; the desktop stores a secure session in the OS keychain after a deep-link authorization exchange. Existing authenticated sessions should reopen like a normal native desktop app.


Local use does not require an account. Signup/login happens on the ORBIT website. The desktop app launches the external browser and returns through an ORBIT-owned app/universal link using a native OAuth public-client flow with PKCE. This follows RFC 8252 guidance for native apps. citeturn658627search0turn658627search1

### Cloud subscription model

A subscription enables the cloud layer: account/device registration, encrypted synchronization and multi-device continuity. It does **not** make interactive retrieval dependent on the network. Hosted processing is a separate, explicit policy.

### Multi-device rule

Never sync SQLite/LanceDB/graph database files directly. Sync canonical logical events/memories, then rebuild device-local indexes. Optional derived artifacts may be transferred as accelerators.

### Future mobile

A future mobile ORBIT client uses the same logical sync protocol so a user can open the same brain on a phone. The phone may use a smaller local index and still keep its interactive processing local-first.

## 1. Architectural Overview & Core Decisions

The Buzz Brain V1 implementation establishes a centralized, local-first long-term memory infrastructure and context engine for AI coding agents by extending the existing Buzz multi-crate workspace.

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                              BUZZ BRAIN V1 CORE ARCHITECTURE                           │
├────────────────────────────────┬───────────────────────────────┬──────────────────────┤
│ 1. PLUGGABLE STORAGE           │ 2. EMBEDDING & RERANK ENGINE  │ 3. 5-LAYER CONTEXT   │
├────────────────────────────────┼───────────────────────────────┼──────────────────────┤
│ • Local: SQLite + LanceDB + embedded graph    │ • Local ONNX embedder (<5ms)  │ • L1: Source Layer   │
│ • Hosted: PostgreSQL + pgvector                │   BAAI/bge-small-en-v1.5      │ • L2: Processed Ctx  │
│ • Same logical memory contract                 │ • Local ONNX reranker (<10ms) │ • L3: Memory Layer   │
│ • No local DB server                           │   BAAI/bge-reranker-small     │ • L4: SuperRAG Layer │
│ • Sync uses logical events, not DB files       │ • Cloud APIs: OpenAI, Cohere, │   (Data Re-Trial)    │
│ • Local and hosted use adapter-specific stores│   Voyage, Gemini, Ollama      │ • L5: Working Context│
├────────────────────────────────┴───────────────────────────────┴──────────────────────┤
│ 4. TRI-MODAL MCP FABRIC        │ 5. PLUGINS & RECALL TOOLCHAIN │ 6. DESKTOP & SYNC    │
├────────────────────────────────┼───────────────────────────────┼──────────────────────┤
│ • MCP Server (Agent tools)     │ • SourcePlugin (Files, Git)   │ • React 19 Explorer  │
│ • MCP Client (GitHub, GitLab,  │ • RecallPlugin (9 IDE parsers)│ • Catppuccin Theme   │
│   Slack, Jira, PostgreSQL Ingest)│ • AST Tree-sitter & Redactor  │ • Type Contract Token│
│ • MCP Arbiter (Security Gate)  │ • Centralized Repository Sync │ • Zero-Docker Mode   │
└────────────────────────────────┴───────────────────────────────┴──────────────────────┘
```

### Core Decisions:

1. **Compact 5-Layer Context Architecture vs. Traditional Database CRUD**:
   * Traditional database CRUD operations (isolated `SELECT`, `JOIN`, `UPDATE` cycles per query) introduce severe latency penalties (>100ms), database lock contention, and cache invalidation thrashing during rapid AI agent interaction loops.
   * Orbit collapses memory operations into **one compact, cohesive context engine spanning 5 distinct layers**:
     * **Layer 1: Source Layer**: Immutable raw data (documents, code, chats, git history, notes) preserving original content hashes, modification timestamps (`mtime_ns`), and access permissions.
     * **Layer 2: Processed Context Layer**: Normalized, AST-chunked text, 384-d dense embeddings, SQLite FTS5 lexical indexes, and bidirectional provenance links.
     * **Layer 3: Memory Layer**: Extracted durable knowledge, architectural decision records (ADRs), user preferences, recurring workflows, and bi-temporal graph relations (`valid_at`, `invalid_at`).
     * **Layer 4: Retrieval Layer (SuperRAG & Data Re-Trial)**: High-speed multi-stage retrieval featuring a Pre-Retrieval Data Re-Trial & Query Routing Arbiter, parallel multi-modal search, Reciprocal Rank Fusion (RRF), adaptive re-trial loop, and native MVP cross-encoder reranking.
     * **Layer 5: Working Context Layer**: Short-lived, task-specific compiled context packages bounded by greedy knapsack token budgets and protected by semantic XML delimiters (`<orbit_context>`).
2. **Pre-Retrieval Data Re-Trial & Query Routing Arbiter**:
   * Rather than issuing brute-force queries against every database index, all incoming queries **hit the Data Re-Trial Layer first**.
   * Inspects semantic cache (<1ms response), classifies intent (symbolic, conceptual, temporal ADR, multi-hop), executes multi-query expansion, and adaptively verifies candidate confidence ($\theta_{\text{conf}}$) with automatic query reformulation before expensive context packing.
3. **Native MVP Cross-Encoder Reranking Engine**:
   * Integrated directly into the MVP build (Sprint 2) rather than deferred.
   * Evaluates top 50 fused candidates through deep query-document cross-attention using a local quantized ONNX model (`bge-reranker-small`, ~25MB weights, `<10ms` inference) or cloud rerank APIs (Cohere/Voyage), delivering top 10–15 precision chunks with near-zero hallucination.
4. **Local-first pluggable storage stack**:
   * Desktop uses SQLite for authoritative metadata/events, LanceDB for embedded vectors and an embedded graph backend for relationship traversal.
   * Hosted/enterprise uses PostgreSQL + pgvector as a server adapter; graph infrastructure remains replaceable.
   * Local and hosted modes share the same logical memory schema, retrieval pipeline and Rust traits.
   * Cloud sync replicates logical records/events, never database page/WAL files.
5. **Configurable Supermemory-Aligned Embeddings & Rerankers**:
   * **Local Defaults**: `BAAI/bge-small-en-v1.5` (embeddings) and `BAAI/bge-reranker-small` (cross-encoder reranking) running in-process via ONNX Runtime (`ort`).
   * **Cloud Providers**: OpenAI, Cohere, Voyage AI, Google Gemini, and Ollama.
   * **Key Security**: API keys stored in OS hardware keyring via `buzz-auth`.
6. **Tri-Modal Model Context Protocol (MCP) Fabric**:
   * **Server Mode**: Exposes memory tools to coding agents (Antigravity, Claude Code, Cursor, OpenHands).
   * **Client Mode**: Ingests operational context from external MCP servers (GitHub, GitLab, Slack, Jira, PostgreSQL).
   * **Arbiter Mode**: Enforces security policies, input sanitization, and delimiter fencing.

---

## Account, device and workspace contract

The V1 memory engine must remain independent from authentication, but its logical records need stable ownership fields so the same brain can later be synchronized or managed by an enterprise tenant.

Every cloud-capable memory/event should be attributable to:

```text
owner_type
owner_id
tenant_id (optional for personal mode)
workspace_id
project_id (optional)
device_id
privacy_class
processing_policy
retention_policy_id
policy_version
```

Authentication itself stays outside the memory store. The desktop receives an account/session identity from the ORBIT auth server and passes only the authorization context required by the local memory API.

## 2. Local-First Storage Architecture — SQLite + LanceDB + Embedded Graph

### 2.2 Deployment Specifications

| Deployment | Metadata | Vector | Graph |
|---|---|---|---|
| Desktop V1 | SQLite | LanceDB | Ladybug/Kuzu-compatible |
| Optional self-hosted relay | PostgreSQL | pgvector | configurable graph adapter |
| Enterprise | PostgreSQL | pgvector | Neo4j/FalkorDB/other supported adapter |


| Deployment Dimension | Local Desktop | Hosted / Enterprise |
| :--- | :--- | :--- |
| **Metadata** | SQLite | PostgreSQL |
| **Vectors** | LanceDB | pgvector / future vector service |
| **Graph** | Embedded graph | Relational graph first; dedicated graph only when justified |
| **Process Model** | Embedded in Tauri/Rust | Managed services and workers |
| **Processing** | Local-first | Local-first; hosted processing explicit |
| **Sync** | Off by default | Logical event/state replication |
| **Large artifacts** | Filesystem | Object storage |
| **Logical schema** | Shared | Shared |

### 2.3 Reused Existing Infrastructure in `crates/`
* **`buzz-db`**: Reused directly as the data access layer. Extends existing SQLx connection pools, connection observability, and migration runner to manage `pgvector` tables.
* **`buzz-search`**: Reused directly for full-text search. Houses the Pre-Retrieval Data Re-Trial Arbiter and extends `search_tsv` GIN queries for Multi-RAG fusion.
* **`buzz-relay`**: Reused for WebSocket synchronization between local desktop and remote team relays.
* **`buzz-core`**: Reused for cryptographic event signing (secp256k1) and NIP-29 group scoping.
* **`buzz-auth`**: Reused for OS hardware keyring token storage (DPAPI, macOS Keychain, Linux Secret Service).

### 2.4 Logical Storage Schema (Store-Agnostic)

The database migration maps directly onto the 5 layers:

#### 1. `orbit_documents` (Layer 1: Source Layer — Raw Sources)
* `id` (UUID, Primary Key): Unique document identifier.
* `source_type` (VARCHAR(64)): Source classification (`file`, `session`, `git`, `slack`, `github`, `gitlab`, `jira`).
* `source_uri` (TEXT): Absolute local path or remote URL.
* `workspace_path` (TEXT): Workspace boundary for tenant isolation.
* `content_hash` (CHAR(64)): SHA-256 digest for deduplication and change detection.
* `mtime_ns` (BIGINT): File modification timestamp in nanoseconds for staleness detection.
* `size_bytes` (BIGINT): Byte size of original content.
* `permissions` (JSONB): ACLs and access boundaries (default `{"public": true}`).
* `metadata` (JSONB): Source attributes (author, commit SHA, PR number, branch, channel ID).
* `created_at` / `updated_at` (TIMESTAMPTZ).
* *Constraints & Indexes*: Unique on `(workspace_path, content_hash)`. B-tree indexes on `workspace_path` and `source_uri`.

#### 2. `orbit_chunks` (Layer 2: Processed Context Layer — Normalized Chunks & Vectors)
* `id` (UUID, Primary Key): Unique chunk identifier.
* `document_id` (UUID, Foreign Key $\rightarrow$ `orbit_documents.id` ON DELETE CASCADE).
* `workspace_path` (TEXT): Workspace boundary.
* `chunk_index` (INT): Sequence index within the source document.
* `content` (TEXT): Normalized chunk text.
* `token_count` (INT): Estimated LLM token count.
* `embedding` (VECTOR(384)): Dense vector for local Supermemory-aligned model (`bge-small-en-v1.5`).
* `search_fts` (FTS5): External/content-indexed lexical representation for BM25-style exact-term retrieval.
* `scope` (VARCHAR(64)): Privacy scope (`project`, `global`, `agent_private`).
* `agent_name` (VARCHAR(64)): Ingesting or generating agent identifier.
* `line_start` / `line_end` (INT): Provenance tracking line numbers in source document.
* `created_at` (TIMESTAMPTZ).
* *Indexes*: Vector index managed by LanceDB. Lexical index managed by SQLite FTS5. B-tree indexes on `workspace_path` and `agent_name`.

#### 3. `orbit_entities` (Layer 3: Memory Layer — Semantic Nodes)
* `id` (UUID, Primary Key): Unique entity identifier.
* `workspace_path` (TEXT): Workspace boundary.
* `name` (VARCHAR(255)): Entity name (`DatabasePool`, `Alice`, `NIP-42`, `SuperRAG`).
* `entity_type` (VARCHAR(64)): Entity category (`Technology`, `Person`, `File`, `Concept`, `Architecture`).
* `description` (TEXT): Synthesized description of the entity.
* `summary_embedding` (VECTOR(384)): Vector representation of the entity description.
* `metadata` (JSONB): Verified evidence links, status, author, aliases.
* `created_at` / `updated_at` (TIMESTAMPTZ).
* *Constraints & Indexes*: Unique on `(workspace_path, name, entity_type)`. B-tree index on `workspace_path`.

#### 4. `orbit_relations` (Layer 3: Memory Layer — Bi-Temporal Edges)
* `id` (UUID, Primary Key): Unique relation identifier.
* `workspace_path` (TEXT): Workspace boundary.
* `source_entity_id` (UUID, Foreign Key $\rightarrow$ `orbit_entities.id` ON DELETE CASCADE).
* `target_entity_id` (UUID, Foreign Key $\rightarrow$ `orbit_entities.id` ON DELETE CASCADE).
* `relation_type` (VARCHAR(64)): Typed relationship (`implements`, `depends_on`, `decided_by`, `authored_by`, `supersedes`).
* `confidence` (REAL): Confidence score (0.0 to 1.0).
* `valid_at` (TIMESTAMPTZ): When this fact became true in the real world.
* `invalid_at` (TIMESTAMPTZ, Nullable): When this fact was contradicted or superseded (`NULL` = active fact).
* `recorded_at` (TIMESTAMPTZ): When the system learned this fact.
* `provenance_chunk_id` (UUID, Foreign Key $\rightarrow$ `orbit_chunks.id` ON DELETE SET NULL).
* `metadata` (JSONB): Extraction confidence, model version, notes.
* *Indexes*: B-tree indexes on `source_entity_id`, `target_entity_id`, and partial index on `(workspace_path, relation_type) WHERE invalid_at IS NULL`.

#### 5. `orbit_query_cache` (Layer 4: Retrieval Layer — Pre-Retrieval Routing Cache)
* `query_hash` (CHAR(64), Primary Key): SHA-256 of normalized query string + workspace path.
* `workspace_path` (TEXT): Workspace boundary.
* `intent_type` (VARCHAR(64)): Classified intent (`symbolic`, `conceptual`, `temporal`, `multihop`).
* `candidate_chunk_ids` (UUID[]): Pre-computed candidate list.
* `reranked_chunk_ids` (UUID[]): Top reranked chunk identifiers.
* `compiled_tokens` (INT): Total token count of cached context.
* `hit_count` (INT): Number of query cache hits.
* `expires_at` / `created_at` (TIMESTAMPTZ).
* *Indexes*: B-tree index on `workspace_path`.

#### 6. `orbit_working_contexts` (Layer 5: Working Context Layer — Active Task Context)
* `id` (UUID, Primary Key): Unique working context identifier.
* `workspace_path` (TEXT): Workspace boundary.
* `task_id` (VARCHAR(128)): Active agent conversation ID or ticket ID.
* `agent_name` (VARCHAR(64)): Consuming agent identifier.
* `token_budget` (INT): Configured token budget (default 4000).
* `allocated_tokens` (INT): Exact token count of compiled package.
* `active_memory_ids` (UUID[]): Included entity/ADR memory identifiers.
* `active_chunk_ids` (UUID[]): Included source chunk identifiers.
* `context_xml` (TEXT): Pre-compiled, delimited `<orbit_context>` XML package.
* `is_active` (BOOLEAN): Active status indicator.
* `created_at` / `updated_at` (TIMESTAMPTZ).
* *Indexes*: B-tree index on `(workspace_path, task_id)`.

---

## 3. Configurable Embedding & Reranker Engine — Local ONNX & Cloud Models

### 3.1 Local ONNX Default Runtimes (`crates/buzz-ai`)
* **Embedding Engine**: `BAAI/bge-small-en-v1.5` (quantized INT8 ONNX, 384 dimensions, ~32MB weights file). Latency: `<5 ms` per chunk on CPU.
* **Cross-Encoder Reranker Engine**: `BAAI/bge-reranker-small` (quantized INT8 ONNX, ~25MB weights file). Latency: `<10 ms` for 50 candidate pairs on CPU.
* **Runtime**: In-process via the Rust `ort` crate. 100% offline, zero API fees.
* **Storage Location**: `~/.orbit/brain/models/`.

### 3.2 Provider Abstraction Contracts
* **`EmbedProvider` Contract**: Async trait in `crates/buzz-ai/src/provider.rs` defining batch embedding generation (`embed_batch`), output dimension resolution (`dimensions`), and provider identifier (`provider_name`).
* **`RerankProvider` Contract**: Async trait in `crates/buzz-ai/src/rerank.rs` defining query-document cross-attention scoring (`rerank(query, candidates, top_n) -> Vec<RerankResult>`) and provider identifier (`provider_name`).

### 3.3 Supported Provider Matrix

| Engine Role | Provider | Default Model | Dimensions / Output | Keyring Key Name |
| :--- | :--- | :--- | :--- | :--- |
| **Embedding** | **Local ONNX** | `bge-small-en-v1.5` | 384 | Zero Config |
| **Embedding** | OpenAI | `text-embedding-3-small` | 1536 | `BUZZ_OPENAI_API_KEY` |
| **Embedding** | Voyage AI | `voyage-3-lite` | 512 | `BUZZ_VOYAGE_API_KEY` |
| **Embedding** | Cohere | `embed-english-v3.0` | 1024 | `BUZZ_COHERE_API_KEY` |
| **Embedding** | Google Gemini | `text-embedding-004` | 768 | `BUZZ_GEMINI_API_KEY` |
| **Embedding** | Ollama | `nomic-embed-text` | 768 | Local Endpoint URL |
| **Reranking** | **Local ONNX** | `bge-reranker-small` | Re-scored Float | Zero Config |
| **Reranking** | Cohere Rerank | `rerank-v3.5` | Re-scored Float | `BUZZ_COHERE_API_KEY` |
| **Reranking** | Voyage Rerank | `rerank-2` | Re-scored Float | `BUZZ_VOYAGE_API_KEY` |

---

## 4. The 5-Layer Compact Context Architecture & SuperRAG Retrieval Engine

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                        THE 5-LAYER COMPACT CONTEXT ARCHITECTURE                        │
├────────────────────────────────────────────────────────────────────────────────────────┤
│  LAYER 1: SOURCE LAYER (Immutable raw files, chats, git commits, notes, hashes, ACLs)  │
│                                           │                                            │
│                                           ▼                                            │
│  LAYER 2: PROCESSED CONTEXT LAYER (Normalized AST chunks, 384-d vectors, GIN tsvector) │
│                                           │                                            │
│                                           ▼                                            │
│  LAYER 3: MEMORY LAYER (Durable facts, ADR decisions, bi-temporal Graphiti graph)      │
│                                           │                                            │
│   ┌───────────────────────────────────────┴───────────────────────────────────────┐    │
│   │               QUERY INGESTION FIRST HITS THE RETRIEVAL LAYER                  │    │
│   ▼                                                                               ▼    │
│  LAYER 4: RETRIEVAL LAYER (SuperRAG Engine)                                            │
│    • Stage 1: Pre-Retrieval Data Re-Trial & Query Routing Arbiter (<1ms Cache hit)     │
│    • Stage 2: Parallel Multi-Modal Candidate Retrieval (Vector + BM25 + Graph)        │
│    • Stage 3: Reciprocal Rank Fusion (RRF) & Temporal Heuristic Modulation            │
│    • Stage 4: Data Re-Trial & Confidence Verification Loop (Adaptive Reformulation)   │
│    • Stage 5: MVP Native Cross-Encoder Reranking Engine (Top 50 ──► Top 15 in <10ms)  │
│                                           │                                            │
│                                           ▼                                            │
│  LAYER 5: WORKING CONTEXT LAYER (Greedy Knapsack Packing, <orbit_context> XML Fencing) │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### 4.1 Detailed Layer Specifications

#### Layer 1: Source Layer (Raw Ingestion)
- **Role**: Preserves original, unchanged source data from all connected apps.
- **Content**: Code files, markdown notes, chat messages, git commit trees, PR descriptions, and issue threads.
- **Invariants**: Strict change-data-capture tracking using `content_hash` (SHA-256), filesystem modification timestamp (`mtime_ns`), and access-control permissions (`permissions`).

#### Layer 2: Processed Context Layer (Searchable Chunks)
- **Role**: Normalizes, segments, and embeds raw content for instant retrieval.
- **Content**: Language-aware AST chunks produced by Tree-sitter (functions, structs, classes, markdown sections) bounded to ~512 tokens.
- **Invariants**: Every chunk maintains strict bidirectional links (`document_id`, `line_start`, `line_end`) back to the Layer 1 source record.

#### Layer 3: Memory Layer (Durable Knowledge & Graph)
- **Role**: Knowledge extracted from multiple interactions and operational sources.
- **Content**: Synthesized architectural decision records (ADRs), user preferences, recurring developer workflows, episodic agent sessions, and bi-temporal graph edges (`valid_at`, `invalid_at`, `recorded_at`).
- **Invariants**: Contradiction resolver invalidates stale facts by setting `invalid_at = NOW()`, preserving historical auditability while preventing obsolete context from entering active queries.

#### Layer 4: Retrieval Layer (SuperRAG & Data Re-Trial Engine)
- **Role**: Dynamically discovers, verifies, and ranks relevant information across all layers.
- **The Data Re-Trial Front Door**: When an agent issues a query, it **hits this layer first** before any raw database query executes:
  1. **Pre-Retrieval Routing**: Checks the in-memory semantic cache (`orbit_query_cache`). If cached context is valid, returns in `<1ms`. If missed, classifies query intent into `Symbolic`, `Conceptual`, `TemporalDecision`, or `MultiHopRelationship`.
  2. **Multi-Modal Candidate Retrieval**: Dispatches targeted queries in parallel to LanceDB vector index (dense), PostgreSQL GIN (BM25), and 2-hop recursive graph walks.
  3. **Reciprocal Rank Fusion (RRF)**: Merges candidates using smoothed reciprocal ranking ($k=60$) modulated by temporal decay ($e^{-\lambda \Delta t}$) and source authoritativeness ($A_{\text{source}}$).
  4. **Data Re-Trial & Confidence Verification Loop**: Evaluates candidate score distribution. If top-ranked confidence $< \theta_{\text{conf}}$ (0.65) or candidate count $< 3$, triggers an automated adaptive re-trial pass (query expansion, keyword relaxation, and graph walk widening) before proceeding.
  5. **MVP Cross-Encoder Reranking**: Passes top 50 candidates through the local ONNX `bge-reranker-small` cross-attention model to produce the final top 10–15 precision chunks in `<10ms`.

#### Layer 5: Working Context Layer (Task-Specific Execution Context)
- **Role**: Compiles a compact, task-specific context package for the active conversation or agent turn.
- **Content**: Recent turns, active project ADRs, selected memories from Layer 3, and reranked excerpts from Layer 4.
- **Invariants**: Packed greedily within a strict token budget (e.g., 4000 tokens) using knapsack packing, deduplicated, and delimited with `<orbit_context>` semantic XML tags to prevent full knowledge-base flooding and prompt injection.

---

## 5. Tri-Modal MCP Architecture & Centralized Ingestion Plan

Buzz implements a **Tri-Modal Model Context Protocol (MCP)** architecture to serve as both an agent memory interface and a centralized ingestion fabric:

```mermaid
graph TD
    subgraph AgentRuntimes["AI Agent Runtimes"]
        Claude["Claude Code"]
        Antigravity["Google Antigravity"]
        Cursor["Cursor"]
        OpenHands["OpenHands / Goose"]
    end

    subgraph BuzzCorePlatform["Buzz Core Platform"]
        MCPServer["Mode 1: Buzz MCP Server (Agent Memory Interface)"]
        MCPArbiter["Mode 3: Buzz Context Arbiter (Security & Policy Gate)"]
        CentralRepo["Centralized Context Repository (5-Layer Context Engine)"]
        MCPClient["Mode 2: Buzz MCP Client (External Connector Mesh)"]
        
        MCPServer <--> MCPArbiter
        MCPArbiter <--> CentralRepo
        CentralRepo <--> MCPClient
    end

    subgraph ExternalMCPServers["External Platform MCP Servers"]
        GH_MCP["GitHub MCP Server"]
        GL_MCP["GitLab MCP Server"]
        Slack_MCP["Slack MCP Server"]
        Jira_MCP["Jira MCP Server"]
        PG_MCP["PostgreSQL MCP Server"]
        Drive_MCP["Google Drive / Notion MCP"]
    end

    AgentRuntimes <-->|MCP Stdio / Named Pipes| MCPServer
    MCPClient <-->|MCP Client Protocol (Stdio / SSE)| ExternalMCPServers
```

### 5.1 Mode 1: Buzz as MCP Server (Agent-Facing)
Exposes 8 standardized memory and context tools to external coding agents over `stdio` and local named pipes (`\\.\pipe\buzz-memory` on Windows, `/tmp/buzz-memory.sock` on Unix):

| Tool Name | Type | Signature | Description |
| :--- | :---: | :--- | :--- |
| `orbit.search_context` | Read | `(query, limit?, workspace?) → [Item]` | Executes SuperRAG hybrid search across persistent memory. |
| `orbit.store_memory` | Write | `(content, tags?, scope?) → Id` | Persists an architectural fact, decision, or user preference in Layer 3. |
| `orbit.get_project_context`| Read | `(path, max_tokens?) → Context` | Retrieves full project ADRs, dependencies, and active decisions from Layer 5. |
| `orbit.recall_session` | Read | `(agent?, query?, days?) → [Session]` | Recalls past turns and decisions from any agent. |
| `orbit.get_file_history` | Read | `(file_path) → [Decision]` | Returns modification and architectural decision history for a file. |
| `orbit.mark_decision` | Write | `(id, state, note?) → ()` | Marks an architectural decision accepted or superseded in Layer 3. |
| `orbit.get_index_stats` | Read | `() → Stats` | Reports total chunks, vector dimensions, and sync health. |
| `orbit.delete_memory` | Write | `(id) → ()` | Hard delete / cryptographic erasure (GDPR compliance). |

### 5.2 Mode 2: Buzz as MCP Client (Centralized Data Ingestion)
Buzz acts as an **MCP Client** connecting outward to external MCP servers to aggregate dispersed enterprise and developer data into the centralized `orbit_documents` and `orbit_chunks` repository:

| External MCP Target | Integration Protocol | Ingestion Workflow | Central Repository Mapping |
| :--- | :--- | :--- | :--- |
| **GitHub MCP Server** | Stdio / SSE subprocess | Fetches repo issues, PR bodies, review discussions, and commit diffs. | Normalized into `orbit_documents` with commit SHAs and author metadata. |
| **GitLab MCP Server** | Stdio / SSE subprocess | Queries merge requests, issue boards, and CI job error logs. | Ingested with branch tags and project boundaries. |
| **Slack MCP Server** | Stdio / SSE subprocess | Reads channel history and threaded architectural debates. | Filtered by public channels; redacted and stored with thread timestamps. |
| **Jira MCP Server** | Stdio / SSE subprocess | Pulls ticket descriptions, acceptance criteria, and sprint goals. | Mapped to Cognitive Concepts (Cognee Tier 4) for project context. |
| **PostgreSQL MCP Server**| Stdio / SSE subprocess | Introspects external database schemas, table definitions, and DDL. | Generates architectural schema context chunks. |
| **Notion / Drive MCP** | Stdio / SSE subprocess | Extracts page blocks and shared design documents. | Chunked via Markdown AST and indexed for semantic search. |

### 5.3 Mode 3: Buzz as Context Arbiter (Security & Policy Gate)
Sits between agents and tools to enforce security:
* **Semantic Delimiter Fencing**: Wraps untrusted external context in immutable XML tags (`<orbit_untrusted_context source="...">`) to prevent prompt injection.
* **Secret Redaction**: Regex-based redaction of API keys, tokens, and credentials before indexing.
* **Mutation Gates**: Requires explicit user confirmation for memory deletion or superseding operations.

---

## 6. Plugins & Ingestion Toolchain (`buzz-plugins` & `buzz-recall`)

### 6.1 Plugin Architecture
The plugin toolchain organizes ingestion into two distinct abstractions:
1. **Source Plugins**: Ingest live, streaming, or filesystem-based data into Layer 1 (Source) and Layer 2 (Processed Context).
2. **Recall Plugins**: Retroactively index past conversation histories and transcripts from 9 agent harnesses so Buzz starts full from day one.

### 6.1 Source Plugins
* **`LocalFilePlugin`**:
  * Watches active workspace directories using the native OS `notify` crate.
  * Parses source code using **Tree-sitter** for AST-aware chunking (Rust, TypeScript, Python, Go, Markdown).
  * Records file modification times (`mtime_ns`) for staleness detection.
* **`GitPlugin`**:
  * Reads local `.git` repository commit logs, branch histories, and diffs.
  * Associates code chunks with commit SHAs and author handles.

### 6.2 Recall Plugins (Retroactive History Ingestion)
* **`AntigravityRecallPlugin`**: Parses `~/.gemini/antigravity-ide/brain/*/transcript.jsonl`.
* **`ClaudeCodeRecallPlugin`**: Parses `~/.claude/projects/**/` session logs.
* **`CodexRecallPlugin`**: Parses `~/.codex/conversations/`.
* **`CursorRecallPlugin`**: Parses `~/.cursor/User/workspaceStorage/`.
* **`GooseRecallPlugin`**: Parses `~/.config/goose/sessions/`.
* **`OpenCodeRecallPlugin`**: Parses `~/.opencode/sessions/`.
* **`ZCodeRecallPlugin`**: Parses `~/.zcode/conversations/`.
* **`AgyCliRecallPlugin`**: Parses `~/.gemini/transcripts/`.
* **`KimiRecallPlugin`**: Parses `~/.kimi/chats/`.

---

## 7. Skills & Agent Surface Map (`.agents`, `.claude`, `.codex`, `.goose`, `.cursor`)

### 7.1 Shared Cross-Agent Skill (`.agents/skills/orbit-memory/SKILL.md`)
Read by **Google Antigravity**, Goose, and generic agent harnesses. Defines the operating protocol:
* Explains available tools (`orbit.search_context`, `orbit.store_memory`, `orbit.get_project_context`, etc.).
* Enforces usage checkpoints:
  1. Call `orbit.get_project_context(".")` at session start.
  2. Call `orbit.get_file_history("path")` before modifying critical modules.
  3. Call `orbit.search_context("query")` before making architectural decisions.
  4. Call `orbit.store_memory("decision...")` after completing a major task.

### 7.2 Agent-Specific Configurations
* **Claude Code**: `.claude/skills/orbit-memory/SKILL.md` and `~/.claude/mcp_config.json`.
* **Codex**: `.codex/skills/orbit-memory/SKILL.md` and `~/.codex/config.json`.
* **Cursor**: `.cursor/mcp.json`.
* **Goose**: `.goose/skills/orbit-memory/SKILL.md`.

---

## 8. Agent Auto-Wiring Toolchain (`buzz-cli`)

Buzz provides an auto-wiring subcommand in `buzz-cli` to detect installed agents on the developer's workstation and automatically install skill files and MCP configurations:

* `buzz memory install --auto`: Auto-detects installed agents and configures skills/MCP configurations.
* `buzz memory install --list`: Lists detected agent harnesses on the workstation.
* `buzz memory install --agent claude`: Configures a specific agent harness manually.

---

## 9. Desktop UI & Packaging Implementation Plan

### 9.1 UI Typography & Theme Contract Alignment
The Orbit Desktop Brain UI is built in React 19 (`desktop/src/features/memory/`) and strictly complies with the design system contract established in `desktop/src/shared/styles/globals/`:

- **Typography Contract (`typography.css`)**:
  - Root scale: Derived dynamically from `var(--buzz-type-scale)` and `var(--buzz-type-rem)`.
  - Body & Message Text: `var(--conversation-message-font-size)` (14px at 100% zoom), line-height `var(--conversation-message-line-height)`.
  - Node Labels & Metadata: `var(--text-xs)` (calc(`var(--buzz-type-rem) * 0.75`)).
  - Section Headers: `var(--text-lg)` and `var(--text-xl)` with `font-weight: 600`.
- **Catppuccin Theme Contract (`theme.css`)**:
  - Canvas Surface: `hsl(var(--background))` with subtle radial glow.
  - Inspection Drawer: `hsl(var(--card))` with border `hsl(var(--border))` and `--radius: 0.625rem`.
  - Accent Color: `hsl(var(--primary))` (mauve accent).

### 9.2 Local-first packaging and hosted connection

The desktop installer ships no database server. The native runtime contains:

- Tauri v2
- Rust memory/retrieval engine
- SQLite
- embedded LanceDB
- embedded graph backend
- optional ONNX models
- MCP/agent integrations

The product may contain an optional **Account / Cloud Sync** entry point. That action opens the ORBIT website in the external browser; it does not embed the signup flow into the desktop app.

After browser authentication, the website returns a short-lived authorization result through the registered ORBIT deep link. The desktop then establishes the native session and registers the device. Cloud sync remains asynchronous and must never be required for local retrieval.

The desktop package therefore does **not** install:

- PostgreSQL
- Redis
- Neo4j
- FalkorDB server
- Docker

Hosted infrastructure is documented separately in `ORBIT_HOSTED_ENTERPRISE_ARCHITECTURE.md` and feature 11/12.

## 10. Step-by-Step Implementation Roadmap (Sprints 1–6)

```
2026 ROADMAP
Sprint 1 ──────► Sprint 2 ──────► Sprint 3 ──────► Sprint 4 ──────► Sprint 5 ──────► Sprint 6
Storage & Embed  SuperRAG & Plg   Tri-Modal MCP    Installer & UI   Desktop Polish   Packaging
```

### Sprint 1 — Storage Foundation & Embed/Rerank Engines (Weeks 1–2)
- [ ] Create the local storage schema for SQLite metadata, events, provenance and memory lifecycle state (`~/.orbit/brain/db/orbit.db` with `orbit_*` tables)
- [ ] Implement `MetadataStore`, `VectorStore`, `GraphStore` and `MemoryStore` traits
- [ ] Implement SQLite metadata/event adapter, LanceDB vector adapter and embedded graph adapter
- [ ] Implement atomic logical mutation handling so derived stores can be updated idempotently
- [ ] Implement Secret Redactor with regex patterns in `crates/buzz-db/src/redactor.rs`
- [ ] Implement Local ONNX embedder (`bge-small-en-v1.5`) via `ort` crate in `crates/buzz-ai`
- [ ] Implement Local ONNX cross-encoder reranker (`bge-reranker-small`) in `crates/buzz-ai`
- [ ] Implement Configurable API clients for OpenAI, Cohere, Voyage, Gemini, Ollama
- [ ] Bundle ONNX models (~32MB embed, ~25MB rerank) into `desktop/src-tauri/resources/`
- [ ] Validate `just ci` passes

### Sprint 2 — Ingestion Pipeline & SuperRAG Retrieval Layer (Weeks 3–4)
- [ ] Implement `SourcePlugin` and `RecallPlugin` traits in `crates/buzz-plugins`
- [ ] Implement Tree-sitter AST chunker (Rust, TypeScript, Python, Go, Markdown) in `crates/buzz-ingest`
- [ ] Implement Local file watcher using `notify` crate
- [ ] Implement Pre-Retrieval Data Re-Trial & Query Routing Arbiter in `crates/buzz-search/src/router.rs`
- [ ] Implement in-memory Semantic Query Cache (`orbit_query_cache`) with sub-millisecond return
- [ ] Implement Layer 1 (Dense Vector) over LanceDB and Layer 2 (Lexical BM25-style retrieval) over SQLite FTS5 in `buzz-search`
- [ ] Implement Reciprocal Rank Fusion (RRF) combiner with temporal decay and source authority weighting
- [ ] Implement Data Re-Trial & Confidence Verification loop with adaptive query reformulation
- [ ] Wire MVP Cross-Encoder Reranker (`bge-reranker-small`) to re-score top 50 candidates to top 15 precision chunks in `<10ms`
- [ ] Implement Layer 5 Working Context Knapsack Packer wrapping context in `<orbit_context>` XML tags
- [ ] Benchmark interactive retrieval and context compilation on defined reference hardware; record p50/p95/p99
- [ ] Validate `just ci` passes

### Sprint 3 — Knowledge Graph & Retroactive Recall (Weeks 5–6)
- [ ] Implement Layer 3 Bi-temporal Entity and Relation Extraction in `buzz-db`
- [ ] Implement Contradiction Resolver setting `invalid_at = NOW()` for superseded assertions
- [ ] Implement 2-hop neighborhood traversal through the embedded graph adapter in `buzz-search`
- [ ] Implement 9 IDE Recall Plugins (Antigravity, Claude Code, Codex, Cursor, Goose, OpenCode, ZCode, AGY CLI, Kimi) in `crates/buzz-recall`
- [ ] Ingest past session histories idempotently using `content_hash` deduplication into `orbit_documents` and `orbit_chunks`
- [ ] Validate `just ci` passes

### Sprint 4 — Tri-Modal MCP Fabric & Centralized Ingestion (Week 7)
- [ ] Extend `crates/buzz-dev-mcp` (or `crates/buzz-mcp`) to expose the 8 standardized `orbit.*` tools
- [ ] Implement MCP Client manager connecting to external MCP servers (GitHub, GitLab, Slack, Jira, PostgreSQL)
- [ ] Implement Centralized Ingestion pipeline pulling external MCP data into `orbit_documents`
- [ ] Implement Context Arbiter security layer (delimiter fencing, input sanitization)
- [ ] Implement agent auto-discovery logic in `crates/buzz-cli` (`buzz memory install --auto`)
- [ ] Create shared skill definition in `.agents/skills/orbit-memory/SKILL.md`
- [ ] Validate `just ci` passes

### Sprint 5 — Desktop UI & Obsidian-Style Brain Graph (Weeks 8–9)
- [ ] Build React 19 Memory Explorer view in `desktop/src/features/memory/`
- [ ] Implement D3 force-directed physics graph visualizing the 5-layer knowledge universe
- [ ] Strictly apply codebase typography contract (`--buzz-type-scale`, `--text-xs`, `--text-sm`) and Catppuccin theme tokens
- [ ] Build Settings UI for Embedding Models, Rerankers, Local/Cloud Sync, and processing policy
- [ ] Wire Tauri IPC commands for graph fetch, search, store, delete, and settings updates
- [ ] Capture UI screenshot: `just desktop-screenshot --name memory-graph`
- [ ] Validate `just ci` passes

### Sprint 6 — Packaging, Performance Validation & Hardening (Week 10)
- [ ] Verify clean desktop install creates SQLite/LanceDB/graph stores with no external database service
- [ ] Verify local-only mode works with the network disconnected
- [ ] Verify account/cloud-sync entry point opens the website and returns through the ORBIT deep link
- [ ] Benchmark: P99 search latency `<25ms`, embed latency `<5ms`, rerank latency `<10ms`
- [ ] Package Windows `.exe` installer (NSIS) and portable `.exe`
- [ ] Package macOS `.dmg` and Linux `.AppImage`
- [ ] Verify zero-Docker mode on a clean workstation
- [ ] Full local gate validation (`just ci`)

---

*Document status: Living engineering blueprint — aligned with Buzz Architecture, 5-Layer Context Engine, SuperRAG Retrieval, local embedded storage, hosted PostgreSQL/pgvector adapters & Tri-Modal MCP.*  
*Author: Buzz / Orbit Engineering · September 2026*


## 11. Post-V1 Hosted / Enterprise Roadmap

The local V1 roadmap intentionally ends with a complete offline-first desktop brain. Hosted work begins only after local retrieval quality, memory consolidation and storage consistency are validated.

### H1 — Website identity and device registration

- browser-first signup/login
- OAuth authorization code + PKCE
- HTTPS app/universal link and private-use scheme fallback
- device registration/revocation
- OS keyring session storage

### H2 — Cloud sync

- logical memory/event sync
- encrypted outbox/inbox
- sync cursors
- tombstones
- idempotent replay
- two-device conflict handling

### H3 — Hosted data plane

- PostgreSQL + pgvector
- object storage
- hosted lexical/vector retrieval
- hosted relational graph
- background workers

### H4 — Enterprise controls

- tenant isolation
- workspace ACLs
- retention and deletion
- audit logs
- device management
- region selection
- organization identity integration

### H5 — Optional hosted processing

- hosted embeddings
- hosted reranking
- hosted extraction/consolidation
- explicit workspace processing policies

### H6 — Infrastructure specialization

Introduce Neo4j, FalkorDB or dedicated vector infrastructure only when measured workloads require it. The storage traits should make these adapter changes rather than retrieval-engine rewrites.
