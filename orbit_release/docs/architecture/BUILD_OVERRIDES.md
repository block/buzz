# ORBIT Build Overrides — Current Architecture Has Priority

> This document resolves legacy implementation details that remain inside preserved feature documents. It does not delete those source details; it tells the build agent how to interpret them under the current ORBIT architecture.

## Priority order

When documents appear to disagree, use this order:

1. `AGENTS.md`
2. this `BUILD_OVERRIDES.md`
3. `docs/architecture/ORBIT_STORAGE_ARCHITECTURE_DECISION_2026-09-29.md`
4. the deployment-specific architecture document
5. feature `IMPLEMENTATION.md`
6. feature `CHECKLIST.md`
7. reference/research material

## Storage overrides

### Local V1

- PostgreSQL/pgvector references are **not** local runtime requirements.
- Dense vector retrieval uses the `VectorStore` abstraction backed by local LanceDB.
- Lexical retrieval uses SQLite FTS5/BM25 behind the lexical/search abstraction.
- Graph retrieval uses `GraphStore` backed by the embedded local graph store.
- SQL/DB-specific types must not leak into retrieval/business crates.
- A `pool` argument in preserved search examples means the storage/retrieval context or repository abstraction, not a mandatory PostgreSQL connection pool.

### Hosted / enterprise

- PostgreSQL + pgvector remain valid hosted storage choices.
- Hosted graph tables are the initial graph implementation; a dedicated graph service is optional and must be justified by measured workload.
- Hosted data is authoritative for synchronized logical state; device indexes remain rebuildable derived state.

## Retrieval overrides

If a preserved feature document says:

- `pgvector <=> cosine` -> implement through `VectorStore.search_dense`.
- `GIN/ts_rank_cd/search_tsv` -> implement through SQLite FTS5/BM25 abstraction locally.
- PostgreSQL recursive graph CTE -> implement through `GraphStore` traversal locally.
- PostgreSQL JSONB -> use the logical metadata model serialized through the local store's supported JSON representation.

The algorithmic intent remains the same: dense + lexical + graph candidates -> RRF -> context-aware reranking -> temporal/permission filters -> context packing.

## Feature-specific interpretation

### F03 — Ingestion

Git/source metadata belongs in the shared document metadata model. Do not require a PostgreSQL JSONB column for local V1.

### F04 — SuperRAG

Keep the existing retrieval/ranking behavior and formulas, but route each retrieval source through storage traits. Do not introduce a PostgreSQL database just to satisfy the preserved pseudocode/checklist wording.

### F05 — Knowledge Graph

Neo4j and FalkorDB are optional adapters/reference alternatives, not local V1 dependencies.

### F10 — Packaging

Any PostgreSQL/pgvector mention is for team/hosted/relay deployment only. A local desktop release must not start a database server.

### F11 — Hosted Enterprise

PostgreSQL/pgvector are valid here because F11 is the hosted data plane. Keep tenant isolation and policy enforcement ahead of retrieval.

### F12 / F13 — Identity, Sync & Governance

Identity, device metadata, subscription state, sync events, and enterprise policies are control-plane/workspace data. They are not long-term memory content unless explicitly imported by a user/workspace policy.

## Code Structure vs User Surface & Storage Overrides

### Code Architecture & Variables
- All internal crates follow the existing Buzz workspace naming scheme (`crates/buzz-*`): `buzz-core`, `buzz-db`, `buzz-search`, `buzz-ai`, `buzz-ingest`, `buzz-mcp`, `buzz-plugins`, `buzz-recall`, `buzz-dev-mcp`, `buzz-cli`, etc.
- In-code variables, struct fields, traits, and modules must use the `buzz` / `buzz_*` naming standard to prevent variable collisions or architectural divergence.

### User Interface, Database & Local Storage
- All database tables must be named `orbit_*` (`orbit_documents`, `orbit_chunks`, `orbit_entities`, `orbit_relations`, `orbit_query_cache`, `orbit_working_contexts`).
- Local database file is `orbit.db` under `~/.orbit/brain/db/`.
- Local storage root for data, models, and indexes must be `~/.orbit/` or `~/.orbit/brain/` (never `~/.buzz/`).
- The user interface and local files visible to the user must carry the Orbit identity exclusively with no user-visible Buzz naming.

## UI Navigation & Settings Overrides

### Primary Navigation: "AI Brain" on Left Sidebar
- The main left sidebar navigation (`desktop/src/features/sidebar/ui/AppSidebar.tsx` / `CommunityRail.tsx`) must feature a prominent, first-class navigation entry labeled **"AI Brain"** (with glowing brain/cosmos icon and keyboard shortcut e.g. `Cmd+B` / `Ctrl+B`).
- Routing: Navigates to `/ai-brain` (`desktop/src/app/routes/ai-brain.tsx`), providing access to the Obsidian-style Knowledge Graph, live Ingestion status HUD, SuperRAG query search, and connected MCP/agent session monitors.

### Settings UI: "Plugins & MCP Tools" Configuration
- The desktop application settings view (`desktop/src/features/settings/ui/SettingsPanels.tsx` and `SettingsView.tsx`) must include a dedicated settings panel: **"Plugins & MCP Tools"** (`PluginsMcpSettingsPanel.tsx`).
- Users must be able to:
  - Toggle built-in tools (`orbit.search_context`, `orbit.store_memory`, `buzz-dev-mcp:shell`, `buzz-dev-mcp:file_edit`, etc.) ON or OFF.
  - Register custom MCP servers (stdio command/args/env or SSE/HTTP endpoint + auth headers).
  - Test connection and discover tools in real-time.
  - Install, enable, and configure plugins (GitHub, Slack, Jira, Postgres, etc.).
  - Set security approval policies per tool (Auto-Approve read-only vs Confirm-on-Execute).

## Local Runtime Embedder, Bundling & Container Architecture Overrides

### Zero-Docker Native Desktop Packaging
- Local mode is 100% native, self-contained, and offline-capable. The installer must never install or require Docker, PostgreSQL, Redis, or Neo4j.
- Local storage relies on embedded SQLite (`orbit.db`), LanceDB for vectors, and embedded Ladybug/Kùzu for graph traversal in `~/.orbit/brain/`.

### Local Runtime Embedder Bundling
- Embedded ONNX Runtime (`ort` crate) and quantized model weights (`bge-small-en-v1.5` at ~30MB, `bge-reranker-small` at ~45MB) must be pre-packaged directly in `desktop/src-tauri/resources/models/`.
- On initial launch, the desktop backend automatically verifies or populates `~/.orbit/brain/models/` without requiring external network downloads.
- `desktop/src-tauri/tauri.conf.json` must bundle all required external binaries in `bundle.externalBin`: `buzz-mcp`, `buzz-dev-mcp`, `buzz-agent`, `buzz-acp`, and `buzz`.

### Container & Server Boundary
- Containers (Docker Compose, Kubernetes Helm charts) are strictly reserved for the central team relay and hosted enterprise sync deployments (`crates/buzz-relay`, PostgreSQL + pgvector, Redis pub/sub).
- The desktop client never runs inside or depends on these containers for local operation.

## Mandatory 4-Tier Implementation Layer Structure
Every feature implementation plan in `orbit_release` must explicitly detail work across four architectural layers:
1. **Frontend**: Desktop UI (React 19, TanStack Router, Catppuccin CSS variables, D3 canvas, Tailwind).
2. **Desktop Backend**: Tauri v2 Rust IPC commands (`desktop/src-tauri/src/commands/`), local embedded engines, OS keyring.
3. **Core Workspace Crates**: Reusable Rust crates (`crates/buzz-*`: `buzz-core`, `buzz-db`, `buzz-search`, `buzz-ai`, `buzz-ingest`, `buzz-mcp`, `buzz-recall`, etc.).
4. **Server & Enterprise Layer**: Hosted team relay (`buzz-relay`), cloud sync, containers, and data governance.

## External-reference rule

The architecture package can contain research citations for historical context, but an implementation agent must not browse or adopt an external design merely because a preserved document contains a citation. Use the repository specification as the build source of truth.
