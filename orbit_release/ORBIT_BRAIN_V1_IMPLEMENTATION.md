# Buzz Brain V1 (Orbit Memory) — Full Implementation Blueprint: Architecture, Skills, Plugins & Roadmap

> **Status**: Active Engineering Blueprint  
> **Brand & Architecture Alignment**:  
> • **Buzz**: The active engineering architecture, crate ecosystem, and codebase naming standard.  
> • **Orbit**: The user-facing brand identity and long-term memory product vision.  
> • **Execution Rule**: All implementation plans, crate modifications, and configurations use the **current Buzz codebase architecture** to prevent variable collisions and architectural drift. Brand metadata updates occur post-release.  
> **Design Principle**: Pure implementation plan, specifications, and execution steps. No raw code snippets.

---

## Table of Contents

1. [Architectural Overview & Core Decisions](#1-architectural-overview--core-decisions)
2. [Unified Database Architecture — PostgreSQL 16+ with `pgvector` Everywhere](#2-unified-database-architecture--postgresql-16-with-pgvector-everywhere)
3. [Configurable Embedding Engine — Supermemory-Aligned Local & Cloud Models](#3-configurable-embedding-engine--supermemory-aligned-local--cloud-models)
4. [The 4-Layer Multi-RAG System Architecture (Graphiti & Cognee Innovations)](#4-the-4-layer-multi-rag-system-architecture-graphiti--cognee-innovations)
5. [Tri-Modal MCP Architecture & Centralized Ingestion Plan](#5-tri-modal-mcp-architecture--centralized-ingestion-plan)
6. [Plugins & Ingestion Toolchain (`buzz-plugins` & `buzz-recall`)](#6-plugins--ingestion-toolchain-buzz-plugins--buzz-recall)
7. [Skills & Agent Surface Map (`.agents`, `.claude`, `.codex`, `.goose`, `.cursor`)](#7-skills--agent-surface-map-agents-claude-codex-goose-cursor)
8. [Agent Auto-Wiring Toolchain (`buzz-cli`)](#8-agent-auto-wiring-toolchain-buzz-cli)
9. [Desktop UI & Packaging Implementation Plan](#9-desktop-ui--packaging-implementation-plan)
10. [Step-by-Step Implementation Roadmap (Sprints 1–6)](#10-step-by-step-implementation-roadmap-sprints-16)

---

## 1. Architectural Overview & Core Decisions

The Buzz Brain V1 implementation establishes a centralized, local-first long-term memory infrastructure and context engine for AI coding agents by extending the existing Buzz multi-crate workspace.

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                              BUZZ BRAIN V1 CORE ARCHITECTURE                           │
├────────────────────────────────┬───────────────────────────────┬──────────────────────┤
│ 1. UNIFIED POSTGRESQL 16+      │ 2. CONFIGURABLE EMBEDDINGS    │ 3. 4-LAYER MULTI-RAG │
├────────────────────────────────┼───────────────────────────────┼──────────────────────┤
│ • PostgreSQL 16+ with pgvector │ • Supermemory-aligned local   │ • Layer 1: Dense Vec │
│ • Local: Bundled service/Docker│   ONNX (bge-small-en-v1.5)    │ • Layer 2: BM25 FTS  │
│   (~150MB RAM, ~200MB disk)    │ • Cloud APIs: OpenAI, Cohere, │ • Layer 3: Graphiti  │
│ • Remote: Same PostgreSQL in   │   Voyage, Gemini, Ollama      │   (Bi-temporal Graph)│
│   cloud (zero migration!)      │ • Configurable via Settings   │ • Layer 4: Cognee    │
│ • 100% Identical SQL & Schemas │ • API key vault in OS keyring │   (Hierarchical Topo)│
├────────────────────────────────┴───────────────────────────────┴──────────────────────┤
│ 4. TRI-MODAL MCP FABRIC        │ 5. PLUGINS & RECALL TOOLCHAIN │ 6. DESKTOP & SYNC    │
├────────────────────────────────┼───────────────────────────────┼──────────────────────┤
│ • MCP Server (Agent tools)     │ • SourcePlugin (Files, Git)   │ • React 19 Explorer  │
│ • MCP Client (GitHub, GitLab,  │ • RecallPlugin (Retroactive)  │ • Settings Panel     │
│   Slack, Jira, Postgres Ingest)│ • AST Tree-sitter & Redactor  │ • E2EE Nostr Relay   │
│ • MCP Arbiter (Security Gate)  │ • Centralized Repository Sync │ • Zero-Docker Mode   │
└────────────────────────────────┴───────────────────────────────┴──────────────────────┘
```

### Core Decisions:
1. **Unified PostgreSQL 16+ with `pgvector` Everywhere**:
   * Eliminates the SQLite vs. PostgreSQL divide.
   * **Locally**: Runs a lightweight bundled local PostgreSQL service (or Docker container) consuming **~150 MB RAM** and **~200 MB disk** at idle (over 100x lighter than Oracle 23ai's 25 GB container).
   * **Remotely**: Runs the exact same PostgreSQL 16+ database on the relay server.
   * **Zero Migration Overhead**: Local and remote schemas are 100% identical.
2. **Configurable Supermemory-Aligned Embeddings**:
   * **Local Default**: `BAAI/bge-small-en-v1.5` (quantized INT8 ONNX, 384 dimensions, ~30 MB weights file) running in-process via ONNX Runtime in `<5 ms`.
   * **Configurable Cloud Providers**: OpenAI, Cohere, Voyage AI, Google Gemini, and Ollama.
   * **Key Security**: API keys stored in OS hardware keyring via `buzz-auth`.
3. **4-Layer Multi-RAG System**:
   * Synthesizes **Graphiti** (dynamic temporal knowledge graph with bi-temporal edges) and **Cognee** (hierarchical topological memory layers) into a unified hybrid retrieval engine.
4. **Tri-Modal Model Context Protocol (MCP) Fabric**:
   * **Server Mode**: Exposes memory tools to coding agents (Claude Code, Google Antigravity, Cursor, OpenHands).
   * **Client Mode**: Connects outward to external MCP servers (GitHub, GitLab, Slack, Jira, Notion, Postgres) to ingest live operational context into a centralized data repository.
   * **Arbiter Mode**: Enforces security policies, input sanitization, and indirect prompt injection defenses.

---

## 2. Unified Database Architecture — PostgreSQL 16+ with `pgvector` Everywhere

### 2.1 Deployment Specifications

| Deployment Dimension | Local Workstation Specification | Remote / Cloud Relay Specification |
| :--- | :--- | :--- |
| **Engine** | PostgreSQL 16+ | PostgreSQL 16+ |
| **Vector Extension** | `pgvector` (HNSW & IVFFlat indexes) | `pgvector` (HNSW & IVFFlat indexes) |
| **Full-Text Extension** | Native `tsvector` with GIN indexing | Native `tsvector` with GIN indexing |
| **Process Model** | Bundled lightweight binary or local Docker | Dedicated cloud container or RDS/Aurora |
| **Memory Footprint** | ~150 MB RAM at idle | Configured per server tier (1GB–16GB) |
| **Disk Footprint** | ~200 MB base installation | Scalable storage volume |
| **Schema Compatibility**| **100% Identical** | **100% Identical** |
| **Data Synchronization**| E2EE Nostr Events (NIP-29 / NIP-44) | Ingests encrypted or shared events |

### 2.2 Reused Existing Infrastructure in `crates/`
* **`buzz-db`**: Reused directly as the data access layer. Extends existing SQLx connection pools, connection observability, and migration runner to manage `pgvector` tables.
* **`buzz-search`**: Reused directly for full-text search. Extends `search_tsv` GIN queries to support Multi-RAG fusion.
* **`buzz-relay`**: Reused for WebSocket synchronization between local desktop and remote team relays.
* **`buzz-core`**: Reused for cryptographic event signing (secp256k1) and NIP-29 group scoping.
* **`buzz-auth`**: Reused for OS hardware keyring token storage (DPAPI, macOS Keychain, Linux Secret Service).

### 2.3 Storage Schema Specifications (Migration `0047_buzz_memory_pgvector.sql`)

The database migration introduces four core tables aligned with the 4-layer Multi-RAG architecture:

#### 1. `buzz_documents` (Cognee Tier 1: Raw Sources)
* `id` (UUID, Primary Key): Unique document identifier.
* `source_type` (VARCHAR(64)): Source classification (`file`, `session`, `git`, `slack`, `github`, `gitlab`, `jira`).
* `source_uri` (TEXT): Absolute local path or remote URL.
* `workspace_path` (TEXT): Workspace boundary for tenant isolation.
* `content_hash` (VARCHAR(64)): SHA-256 digest for deduplication.
* `mtime_ns` (BIGINT): File modification timestamp at index time for staleness detection.
* `metadata` (JSONB): Source-specific attributes (author, commit SHA, PR number).
* `created_at` / `updated_at` (TIMESTAMPTZ).

#### 2. `buzz_chunks` (Cognee Tier 2 & Dense Vector RAG)
* `id` (UUID, Primary Key): Unique chunk identifier.
* `document_id` (UUID, Foreign Key $\rightarrow$ `buzz_documents.id` ON DELETE CASCADE).
* `workspace_path` (TEXT, Indexed): Workspace boundary.
* `content` (TEXT): Normalized chunk text.
* `token_count` (INT): Estimated LLM token count.
* `chunk_index` (INT): Sequence index within document.
* `source_type` (VARCHAR(64)): Origin source type.
* `agent_name` (VARCHAR(64)): Ingesting or generating agent identifier.
* `scope` (VARCHAR(32)): Privacy scope (`shared`, `private`, `team`).
* `embedding` (VECTOR(384)): Dense vector column for local Supermemory-aligned model.
* `embedding_custom` (VECTOR): Dimension-flexible column for cloud embedding models.
* `search_tsv` (TSVECTOR): Generated column (`to_tsvector('english', content)`) with GIN index.
* `created_at` (TIMESTAMPTZ).

#### 3. `buzz_entities` (Graphiti / Cognee Tier 3: Semantic Nodes)
* `id` (UUID, Primary Key): Unique entity identifier.
* `workspace_path` (TEXT): Workspace boundary.
* `name` (VARCHAR(255)): Entity name (`DatabasePool`, `Alice`, `NIP-42`).
* `entity_type` (VARCHAR(64)): Entity category (`Technology`, `Person`, `File`, `Concept`).
* `description` (TEXT): Synthesized description of the entity.
* `summary_embedding` (VECTOR(384)): Vector representation of the entity description.
* `created_at` / `updated_at` (TIMESTAMPTZ).
* *Constraint*: Unique on `(workspace_path, name, entity_type)`.

#### 4. `buzz_relations` (Graphiti Tier 3: Bi-Temporal Edges)
* `id` (UUID, Primary Key): Unique relation identifier.
* `workspace_path` (TEXT): Workspace boundary.
* `source_entity_id` (UUID, Foreign Key $\rightarrow$ `buzz_entities.id`).
* `target_entity_id` (UUID, Foreign Key $\rightarrow$ `buzz_entities.id`).
* `relation_type` (VARCHAR(64)): Typed relationship (`implements`, `depends_on`, `decided_by`, `migrated_from`).
* `confidence` (FLOAT): Confidence score (0.0 to 1.0).
* `valid_at` (TIMESTAMPTZ): When this fact became true in the real world.
* `invalid_at` (TIMESTAMPTZ, Nullable): When this fact was contradicted or superseded.
* `recorded_at` (TIMESTAMPTZ): When the system ingested this assertion.
* `provenance_chunk_id` (UUID, Foreign Key $\rightarrow$ `buzz_chunks.id`).

---

## 3. Configurable Embedding Engine — Supermemory-Aligned Local & Cloud Models

### 3.1 Local Default Model
* **Model**: **`BAAI/bge-small-en-v1.5`** (quantized INT8 ONNX).
* **Dimensions**: 384 dimensions.
* **Weights File Size**: ~30 MB (bundled in desktop resources).
* **Runtime**: ONNX Runtime via Rust `ort` crate.
* **Latency & Cost**: `<5 ms` per chunk on CPU; 100% offline, zero API fees.

### 3.2 Configurable Provider Architecture
The embedding subsystem provides a unified abstraction supporting multiple backends:

| Provider | Supported Models | Default Dimensions | Authentication / Configuration |
| :--- | :--- | :--- | :--- |
| **Local ONNX** | `BAAI/bge-small-en-v1.5` | 384 | Bundled binary, zero config |
| **OpenAI** | `text-embedding-3-small`, `text-embedding-3-large` | 1536, 3072 | API Key in OS keyring |
| **Cohere** | `embed-english-v3.0`, `embed-multilingual-v3.0` | 1024 | API Key in OS keyring |
| **Voyage AI** | `voyage-3-lite`, `voyage-code-2` | 512, 1536 | API Key in OS keyring |
| **Google Gemini**| `text-embedding-004` | 768 | API Key in OS keyring |
| **Ollama** | `nomic-embed-text`, `mxbai-embed-large` | 768, 1024 | Custom endpoint URL |
| **Custom HTTP** | OpenAI-compatible embedding endpoints | Configurable | Custom URL + Bearer token |

### 3.3 Configuration Parameters
* `provider`: Active embedding backend selection.
* `model_name`: Exact model string passed to provider.
* `dimensions`: Vector dimension size (determines target column in `buzz_chunks`).
* `batch_size`: Chunk batching count for ingestion (default: 32 for local, 128 for cloud).
* `api_key`: Stored securely in OS hardware keyring; never stored in plaintext configuration files.
* `endpoint_url`: Base URL for Ollama or custom enterprise gateways.

---

## 4. The 4-Layer Multi-RAG System Architecture (Graphiti & Cognee Innovations)

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                 BUZZ MULTI-RAG SYSTEM                                  │
├────────────────────────────────────────────────────────────────────────────────────────┤
│  Layer 1: Dense Vector RAG (Semantic Search via pgvector HNSW)                         │
│  Layer 2: Lexical / Symbol RAG (Exact BM25 via PostgreSQL tsvector + GIN)               │
│  Layer 3: Temporal Knowledge Graph RAG (Graphiti Architecture: Bi-temporal Edges)       │
│  Layer 4: Hierarchical Topological RAG (Cognee Architecture: Multi-Tier Memory)        │
├────────────────────────────────────────────────────────────────────────────────────────┤
│  Reciprocal Rank Fusion (RRF) ──► Temporal Decay ──► Cross-Encoder Rerank ──► Knapsack │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### 4.1 Layer 1: Dense Vector RAG (Semantic Search)
* **Index**: HNSW (`m = 16, ef_construction = 64`) with cosine distance (`<=>`).
* **Role**: Matches high-level conceptual queries, natural language questions, and semantic intent.

### 4.2 Layer 2: Lexical / Symbol RAG (Exact BM25 Search)
* **Index**: GIN on `search_tsv` with `ts_rank_cd`.
* **Role**: Matches exact code symbols, variable names, file paths, UUIDs, git commit SHAs, and function signatures.

### 4.3 Layer 3: Temporal Knowledge Graph RAG (Graphiti Architecture)
* **Bi-Temporal Edge Modeling**: Tracks both when an event occurred (`valid_at`) and when it was recorded (`recorded_at`).
* **Temporal Reconciliation**: When an architectural decision is updated, older relations have `invalid_at` set to the current timestamp. Queries for current context filter `WHERE invalid_at IS NULL`.
* **Episodic vs. Semantic Distinction**: Raw interactions remain in episodic logs; synthesized consensus facts become semantic entities.
* **Multi-Hop Traversal**: Follows 2-hop neighborhoods across entity relationships.

### 4.4 Layer 4: Hierarchical Topological RAG (Cognee Architecture)
* **Multi-Tier Abstraction**:
  * *Tier 1 (Documents)*: Raw files, PR bodies, transcript logs.
  * *Tier 2 (Chunks)*: AST-parsed code blocks (Tree-sitter) and Markdown sections.
  * *Tier 3 (Entity Graph)*: Extracted entities, tools, and relational edges.
  * *Tier 4 (Cognitive Concepts)*: High-level architectural summaries and active goals.
* **Hierarchical Drill-Down**: Queries search Tier 4 concepts first, traverse Tier 3 relations, and extract precise Tier 2 code chunks.

### 4.5 Multi-RAG Fusion Pipeline
1. **Reciprocal Rank Fusion (RRF)**: Merges ranked candidate lists from all 4 layers:
   $$\text{RRF\_Score}(d) = \sum_{m \in \{\text{vec}, \text{lex}, \text{graph}, \text{concept}\}} \frac{w_m}{60 + \text{rank}_m(d)}$$
   Weights: $w_{\text{vec}} = 0.40$, $w_{\text{lex}} = 0.30$, $w_{\text{graph}} = 0.20$, $w_{\text{concept}} = 0.10$.
2. **Temporal Decay & Staleness Modulation**:
   $$\text{Final\_Score}(d) = \text{RRF\_Score}(d) \times e^{-\lambda \Delta t} \times S_{\text{staleness}} \times A_{\text{authority}}$$
3. **Cross-Encoder Reranking**: Reranks top 50 candidates down to top 15 using a lightweight model (`ms-marco-MiniLM-L-6-v2`).
4. **Token Budget Knapsack Packing**: Packs ranked chunks into configured token budgets (e.g. 4000 tokens) wrapped in `<orbit_context>` delimiter tags.

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
        CentralRepo["Centralized Context Repository (PostgreSQL 16+ pgvector)"]
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
| `orbit.search_context` | Read | `(query, limit?, workspace?) → [Item]` | Executes Multi-RAG hybrid search across persistent memory. |
| `orbit.store_memory` | Write | `(content, tags?, scope?) → Id` | Persists an architectural fact, decision, or user preference. |
| `orbit.get_project_context`| Read | `(path, max_tokens?) → Context` | Retrieves full project ADRs, dependencies, and active decisions. |
| `orbit.recall_session` | Read | `(agent?, query?, days?) → [Session]` | Recalls past turns and decisions from any agent. |
| `orbit.get_file_history` | Read | `(file_path) → [Decision]` | Returns modification and architectural decision history for a file. |
| `orbit.mark_decision` | Write | `(id, state, note?) → ()` | Marks an architectural decision accepted or superseded. |
| `orbit.get_index_stats` | Read | `() → Stats` | Reports total chunks, vector dimensions, and sync health. |
| `orbit.delete_memory` | Write | `(id) → ()` | Hard delete / cryptographic erasure (GDPR compliance). |

### 5.2 Mode 2: Buzz as MCP Client (Centralized Data Ingestion)
Buzz acts as an **MCP Client** connecting outward to external MCP servers to aggregate dispersed enterprise and developer data into the centralized `buzz_documents` and `buzz_chunks` repository:

| External MCP Target | Integration Protocol | Ingestion Workflow | Central Repository Mapping |
| :--- | :--- | :--- | :--- |
| **GitHub MCP Server** | Stdio / SSE subprocess | Fetches repo issues, PR bodies, review discussions, and commit diffs. | Normalized into `buzz_documents` with commit SHAs and author metadata. |
| **GitLab MCP Server** | Stdio / SSE subprocess | Queries merge requests, issue boards, and CI job error logs. | Ingested with branch tags and project boundaries. |
| **Slack MCP Server** | Stdio / SSE subprocess | Reads channel history and threaded architectural debates. | Filtered by public channels; redacted and stored with thread timestamps. |
| **Jira MCP Server** | Stdio / SSE subprocess | Pulls ticket descriptions, acceptance criteria, and sprint goals. | Mapped to Cognitive Concepts (Cognee Tier 4) for project context. |
| **Postgres MCP Server**| Stdio / SSE subprocess | Introspects external database schemas, table definitions, and DDL. | Generates architectural schema context chunks. |
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
1. **Source Plugins**: Ingest live, streaming, or filesystem-based data (Local Files, Git Repositories).
2. **Recall Plugins**: Retroactively index past conversation histories and transcripts from 34+ agent harnesses so Buzz starts full from day one.

### 6.2 Source Plugins
* **`LocalFilePlugin`**:
  * Watches active workspace directories using the native OS `notify` crate.
  * Parses source code using **Tree-sitter** for AST-aware chunking (Rust, TypeScript, Python, Go, Markdown).
  * Records file modification times (`mtime_ns`) for staleness detection.
* **`GitPlugin`**:
  * Reads local `.git` repository commit logs, branch histories, and diffs.
  * Associates code chunks with commit SHAs and author handles.

### 6.3 Recall Plugins (Retroactive History Ingestion)
* **`AntigravityRecallPlugin`**: Parses `~/.gemini/antigravity-ide/brain/*/transcript.jsonl`.
* **`ClaudeCodeRecallPlugin`**: Parses `~/.claude/projects/**/` session logs.
* **`CodexRecallPlugin`**: Parses `~/.codex/conversations/`.
* **`CursorRecallPlugin`**: Parses `~/.cursor/User/workspaceStorage/`.
* **`GooseRecallPlugin`**: Parses `~/.config/goose/sessions/`.

### 6.4 Secret Redaction Toolchain
Before any text chunk is vectorized or committed, it passes through the redaction engine to neutralize credentials:
* AWS Access Keys (`AKIA...`) $\rightarrow$ `[redacted:AWS_ACCESS_KEY]`
* GitHub Personal Access Tokens (`ghp_...`) $\rightarrow$ `[redacted:GITHUB_PAT]`
* Bearer Tokens (`Bearer ...`) $\rightarrow$ `[redacted:BEARER_TOKEN]`
* Private Keys (`-----BEGIN ... PRIVATE KEY-----`) $\rightarrow$ `[redacted:PEM_PRIVATE_KEY]`
* OpenAI / Cloud API Keys (`sk-...`) $\rightarrow$ `[redacted:OPENAI_API_KEY]`

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

```bash
buzz memory install --auto      # Auto-detects installed agents and configures skills/MCP
buzz memory install --list      # Lists detected agent harnesses on the workstation
buzz memory install --agent claude  # Configures a specific agent harness
```

### Auto-Detection Logic:
* Checks for `~/.gemini/` $\rightarrow$ Injects into `~/.gemini/config/mcp_config.json`.
* Checks for `~/.claude/` $\rightarrow$ Injects into `~/.claude/mcp_config.json` and `.claude/skills/`.
* Checks for `~/.codex/` $\rightarrow$ Injects into `~/.codex/config.json`.
* Checks for `~/.cursor/` $\rightarrow$ Injects into `.cursor/mcp.json`.
* Checks for `~/.config/goose/` $\rightarrow$ Injects into `.goose/skills/`.
* Always injects into `.agents/skills/orbit-memory/SKILL.md` for shared cross-agent discovery.

---

## 9. Desktop UI & Packaging Implementation Plan

### 9.1 Local PostgreSQL Bundling Strategy
To deliver a zero-configuration desktop experience:
* **Bundled PostgreSQL 16 Binary**: Packaged in `desktop/src-tauri/binaries/`.
* **Lifecycle**: Managed by Tauri background daemon; listens on `127.0.0.1:5433`.
* **Data Path**: `~/.orbit/postgres_data/` (or `~/.buzz/postgres_data/`).
* **Resource Ceiling**: Enforced memory target **~150 MB RAM**, **~200 MB initial disk**.
* **Optional Docker Mode**: Users with existing Docker setups can point to a standard container.

### 9.2 Desktop UI Views (`desktop/src/features/`)
1. **Memory Explorer (`/memory`)**:
   * Search bar with live Multi-RAG hybrid results.
   * Chunks list displaying source type, author, timestamp, and staleness indicator.
   * Entity Knowledge Graph canvas visualizing nodes and temporal edges.
   * Manual "Add Memory" and GDPR hard-delete confirmation dialogs.
2. **Settings Panel (`/settings/memory`)**:
   * **Embedding Provider**: Dropdown to select Local ONNX (`bge-small-en-v1.5`) or Cloud (OpenAI, Cohere, Voyage, Gemini, Ollama).
   * **API Key Management**: Secure password fields saving directly to OS hardware keyring via Tauri IPC.
   * **Database & Sync**: Toggle between Local Bundled PostgreSQL and Remote Cloud Relay with connection test and sync health indicators.
   * **External MCP Ingestion**: Management of connected external MCP servers (GitHub, GitLab, Slack, Jira).

---

## 10. Step-by-Step Implementation Roadmap (Sprints 1–6)

```
2026 ROADMAP
Sprint 1 ──────► Sprint 2 ──────► Sprint 3 ──────► Sprint 4 ──────► Sprint 5 ──────► Sprint 6
Storage & Embed  Multi-RAG & Plg  Tri-Modal MCP    Installer & UI   Desktop Polish   Packaging
```

### Sprint 1 — Storage Foundation & Configurable Embeddings (Weeks 1–2)
- [ ] Add `0047_buzz_memory_pgvector.sql` to `migrations/`
- [ ] Update `crates/buzz-db`: Wire `pgvector` SQLx types and connection pool configuration
- [ ] Implement core types: `CanonicalChunk`, `Provenance`, `MultiRagQuery`, `EntityNode`
- [ ] Implement storage CRUD operations for `buzz_chunks`, `buzz_entities`, and `buzz_relations`
- [ ] Implement Secret Redactor with regex patterns
- [ ] Implement Local ONNX embedder (`bge-small-en-v1.5`) via `ort` crate in `crates/buzz-ai`
- [ ] Implement Configurable API client for OpenAI, Cohere, Voyage, Gemini, Ollama
- [ ] Bundle `bge-small-en-v1.5` quantized ONNX model (~30MB) into `desktop/src-tauri/resources/`
- [ ] Validate `just ci` passes

### Sprint 2 — Ingestion Plugins & Multi-RAG Engine (Weeks 3–4)
- [ ] Implement `SourcePlugin` and `RecallPlugin` traits in `crates/buzz-plugins`
- [ ] Implement Retroactive session parsers for Antigravity, Claude Code, Codex, Cursor, Goose
- [ ] Implement Tree-sitter AST chunker (Rust, TypeScript, Python, Go, Markdown)
- [ ] Implement Local file watcher using `notify` crate
- [ ] Implement Layer 1 (Dense Vector) and Layer 2 (Lexical BM25) search in `buzz-search`
- [ ] Implement Layer 3 (Graphiti Bi-temporal Graph) and Layer 4 (Cognee Hierarchical Topology) in `buzz-db`
- [ ] Implement Reciprocal Rank Fusion (RRF) combiner with temporal decay and staleness scoring
- [ ] Validate `just ci` passes

### Sprint 3 — Tri-Modal MCP Fabric & Centralized Ingestion (Weeks 5–6)
- [ ] Extend `crates/buzz-dev-mcp` to expose the 8 standardized `orbit.*` tools (MCP Server)
- [ ] Implement MCP Client manager connecting to external MCP servers (GitHub, GitLab, Slack, Jira, Postgres)
- [ ] Implement Centralized Ingestion pipeline pulling external MCP data into `buzz_documents`
- [ ] Implement Context Arbiter security layer (delimiter fencing, input sanitization)
- [ ] Implement token budget knapsack context packer with `<orbit_context>` delimiter framing
- [ ] Validate MCP communication with Claude Code and Google Antigravity
- [ ] Validate `just ci` passes

### Sprint 4 — Auto-Installer & Skills Injection (Week 7)
- [ ] Implement agent auto-discovery logic in `crates/buzz-cli` (`buzz memory install --auto`)
- [ ] Create shared skill definition in `.agents/skills/orbit-memory/SKILL.md`
- [ ] Create agent-specific skill and config templates (Claude, Codex, Cursor, Goose)
- [ ] Verify automated installation across all detected agent harnesses
- [ ] Validate `just ci` passes

### Sprint 5 — Desktop UI & Settings Integration (Weeks 8–9)
- [ ] Build React 19 Memory Explorer view in `desktop/src/features/memory/`
- [ ] Build Settings UI for Embedding Models (Local vs Cloud, API Keys) and Database Sync
- [ ] Build External MCP Ingestion management view in Desktop Settings
- [ ] Wire Tauri IPC commands for search, store, delete, scan, and settings updates
- [ ] Configure MCP server as Tauri background process in `tauri.conf.json`
- [ ] Validate `just ci` passes
- [ ] Capture UI screenshot: `just desktop-screenshot --name memory`

### Sprint 6 — Packaging, Performance Validation & Hardening (Week 10)
- [ ] Verify bundled local PostgreSQL starts cleanly in `~/.orbit/postgres_data/` (~150MB RAM)
- [ ] Benchmark: P99 search latency `<25ms` on 10,000 chunks
- [ ] Package Windows `.exe`, macOS `.dmg`, and Linux `.AppImage`
- [ ] Verify zero-Docker mode on a clean workstation
- [ ] Update documentation and developer guides
- [ ] Full local gate validation (`just ci`)

---

*Document status: Living blueprint — aligned with Buzz Architecture, PostgreSQL 16+ `pgvector`, Tri-Modal MCP & Multi-RAG.*  
*Author: Buzz / Orbit Engineering · September 2026*
