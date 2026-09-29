# Feature 09 — Obsidian-Style Desktop Brain Graph (D3 Force Graph, "AI Brain" Sidebar Navigation & Memory Explorer)

> **Priority**: P0 — Primary visual intelligence surface for developers.  
> **Sprint**: Sprint 5 (Weeks 8–9)  
> **Dependencies**: Feature 04 (SuperRAG Search), Feature 05 (Knowledge Graph), Feature 07 (MCP Tools & Settings)  
> **Frontend Surface**: Sidebar (`desktop/src/features/sidebar/ui/AppSidebar.tsx`), Route (`desktop/src/app/routes/ai-brain.tsx`), Components (`desktop/src/features/memory/`)  
> **Backend Surface**: Tauri IPC (`desktop/src-tauri/src/commands/brain_graph.rs`, `desktop/src-tauri/src/graph.rs`)  
> **Crates**: `buzz-core`, `buzz-db`, `buzz-search`, `buzz-ai`  
> **Aesthetic Reference**: Obsidian Graph View (dark universe, glowing neon nodes, fluid physics)  
> **Codebase Design System**: Catppuccin Theme & Buzz Typography contract (`desktop/src/shared/styles/globals/`)

---

## Overview

Feature 09 establishes the **"AI Brain"** as a primary, first-class surface on the main desktop page of Orbit:
1. Adds a prominent **"AI Brain"** navigation item on the left sidebar navigation (`AppSidebar.tsx` / `CommunityRail.tsx`), providing 1-click access to the central memory and context hub.
2. Implements a **dynamic, interactive Obsidian-style Brain Graph** in React 19 using D3 force simulation, visualizing projects, conversation threads, agents, files, concepts, and architectural decisions.
3. Provides an integrated **Live Ingestion HUD**, **SuperRAG Search Bar**, and **Slide-out Node Inspection Drawer**.
4. Implements the complete 4-tier layer architecture across Frontend UI, Desktop Backend IPC, Core Crates, and Packaging.

---

## 4-Tier Implementation Layer Architecture

### 1. Frontend Tier: "AI Brain" Primary Navigation & Route View
- **Left Sidebar Navigation Entry (`desktop/src/features/sidebar/ui/AppSidebar.tsx` / `CommunityRail.tsx`)**:
  - Primary navigation item labeled **"AI Brain"** located alongside channels and messages on the left side of the app.
  - Icon: Glowing neural/cosmos orb icon with animated pulsing indicator when background indexing is active.
  - Keyboard Shortcut: `Cmd+B` / `Ctrl+B`.
  - Tooltip & Badge: Displays current indexed chunk count and connected active agent sessions.
- **Route Screen (`desktop/src/app/routes/ai-brain.tsx`)**:
  - Hosted at route `/ai-brain`.
  - **Live Ingestion & Health HUD (`desktop/src/features/memory/BrainHUD.tsx`)**:
    - Status pills: Monitored Workspaces count, Total Files Indexed, Chunks in `orbit_chunks`, Vector Index health, Connected MCP Agents.
    - Cloud Sync state indicator: `Local (On Device)` / `Synced (Encrypted)` / `Syncing...`.
  - **SuperRAG Search Input (`desktop/src/features/memory/SuperRagSearchBar.tsx`)**:
    - Omnibar search input executing hybrid dense/lexical queries.
    - Real-time candidate confidence gauge and instant node match highlighting on the canvas.
  - **D3 Force Graph Canvas (`desktop/src/features/memory/BrainGraph.tsx`)**:
    - Responsive SVG/Canvas renderer powered by D3 force simulation (`d3-force`).
    - Fluid zoom and pan behaviors with mouse wheel, trackpad, and keyboard chords.
    - Physics Sliders Bar: Live adjustment of Charge Repulsion, Link Spring Distance, and Gravity.
    - Filter Toggles: Filter by Agent (`Claude`, `Antigravity`, `Cursor`, etc.), Project, or Node Type.
    - Temporal Playback Slider: Scrub back through time to witness the evolutionary growth of decisions and code.
  - **Node Inspection Drawer (`desktop/src/features/memory/NodeDetailsDrawer.tsx`)**:
    - Docks smoothly on the right side upon clicking any node.
    - Displays connected chats, code diffs, authoring agents, and temporal ADR status (`valid_at`, `invalid_at`).
    - 1-Click Action Buttons: "Jump to Chat Thread", "Open File in Editor", "Invalidate Decision".

### 2. Desktop Backend Tier: Tauri Rust IPC & Graph Provider
- **Location**: `desktop/src-tauri/src/commands/brain_graph.rs` and `desktop/src-tauri/src/graph.rs`
- **Tauri IPC Command Handlers**:
  ```rust
  #[tauri::command]
  pub async fn fetch_brain_graph(
      workspace_path: Option<String>,
      filter_agent: Option<String>,
      start_time: Option<DateTime<Utc>>,
      end_time: Option<DateTime<Utc>>,
  ) -> Result<BrainGraphPayload, String> {
      // Queries orbit_documents, orbit_chunks, orbit_entities, orbit_relations
      // Returns nodes & links ready for D3 force simulation
  }

  #[tauri::command]
  pub async fn query_superrag(
      query: String,
      limit: Option<usize>,
      workspace_path: Option<String>,
  ) -> Result<SuperRagPayload, String>;

  #[tauri::command]
  pub async fn get_brain_stats() -> Result<BrainStatsPayload, String>;

  #[tauri::command]
  pub async fn delete_memory_node(node_id: String) -> Result<(), String>;
  ```
- **Registration**: Registered in `desktop/src-tauri/src/lib.rs` inside `generate_handler![]`.

### 3. Core Workspace Crates Tier (`crates/buzz-*`)
- **`crates/buzz-db`**:
  - Relational queries against SQLite tables: `orbit_documents`, `orbit_chunks`, `orbit_entities`, `orbit_relations`, `orbit_query_cache`, `orbit_working_contexts`.
  - Bi-temporal relation filtering (`valid_at <= T AND (invalid_at IS NULL OR invalid_at > T)`).
- **`crates/buzz-search`**:
  - SuperRAG hybrid retrieval (dense LanceDB vector search + SQLite FTS5 lexical search).
  - Reciprocal Rank Fusion (RRF) with temporal decay ($e^{-\lambda \Delta t}$).
- **`crates/buzz-core`**:
  - Node & edge schema definitions (`GraphNode`, `GraphEdge`, `NodeKind`, `EdgeKind`).

### 4. Packaging & Bundling Tier
- 100% native, self-contained desktop packaging with zero Docker and no external database servers.
- Model weights (`bge-small-en-v1.5`, `bge-reranker-small`) bundled in `desktop/src-tauri/resources/models/`.

---

## UI Typography & Design System Contract

The Brain Graph and Memory Explorer components strictly consume the design tokens and typography contract defined in the codebase (`desktop/src/shared/styles/globals/typography.css` and `theme.css`):

### 1. Typography Hierarchy
All typography scales with `--buzz-type-scale` and `--buzz-type-rem` (1rem-relative) to support keyboard zoom and user density settings without breaking layout geometry:
- **Graph Header / Title**: `var(--text-xl)` (calc(`var(--buzz-type-rem) * 1.25`)), font-weight 600.
- **Node Labels**: `var(--text-xs)` (calc(`var(--buzz-type-rem) * 0.75`)), `font-variant-ligatures: no-contextual`, with background pill contrast.
- **Drawer Body Text**: `var(--conversation-message-font-size)` (calc(`var(--buzz-type-rem) * 0.875`)), line-height `var(--conversation-message-line-height)`.
- **Drawer Timestamps / Metadata**: `var(--conversation-timestamp-font-size)` (calc(`var(--buzz-type-rem) * 0.75`)), color `hsl(var(--muted-foreground))`.
- **Code Diffs / Monospace Excerpts**: Monospace font family (`ui-monospace`, `SFMono-Regular`, `Menlo`), font-size `var(--text-xs)`.

### 2. Catppuccin Theme Palette & Surface Tokens
The panel consumes the active Catppuccin theme variables:
- **Background Surface**: `hsl(var(--background))` with deep canvas gradient.
- **Card & Drawer Surface**: `hsl(var(--card))` with border `hsl(var(--border))` and `--radius: 0.625rem`.
- **Primary Accent**: `hsl(var(--primary))` (mauve accent, 266 85.05% 58.04%).
- **Interactive State**: Hover states utilize `hsl(var(--accent))` and text `hsl(var(--accent-foreground))`.

---

## Visual & Physics Architecture

### 1. The Obsidian-Style Universe

```
               (Agent: Claude) 🟣
                      │
                      ▼
               [Chat: Fix Auth] 🔵 ───────► (File: auth.rs) 🟠
                      │                            ▲
                      ▼                            │
             (Entity: NIP-42) 🟡 ─────────► [Project: Orbit Workspace] 🟢
                      ▲
                      │
           (Decision: Use HNSW) ⚪ ───────► [Chat: Pgvector RFC] 🔵
                      ▲
                      │
             (Agent: Antigravity) 🟣
```

### 2. Node Classification & Visual Semantics

| Node Type | Color Token | Icon / Visual | Represents |
|-----------|-------------|---------------|------------|
| 🟢 **Project** | Emerald Green (`#10B981`) | Large pulsating orb | Workspaces, git repos, crates (`buzz-relay`, `buzz-db`) |
| 🔵 **Chat / Thread** | Cyan Blue (`#06B6D4`) | Medium orb with glow | Relay message threads, agent conversation sessions |
| 🟣 **Agent** | Violet Purple (`#8B5CF6`) | Hexagonal orb | Active/past agents (`Antigravity`, `Claude`, `Cursor`, `Kimi`) |
| 🟠 **File / Doc** | Amber Orange (`#F59E0B`) | Standard circular node | Code files, markdown docs, specs |
| 🟡 **Concept / Entity** | Gold Yellow (`#EAB308`) | Small crisp node | Technologies, libraries, domain terms (`pgvector`, `NIP-29`) |
| ⚪ **Decision / Memory** | White / Slate (`#F8FAFC`) | Diamond node | Explicit architectural choices, ADRs, facts |

### 3. Edge Types & Semantics

- `authored_by` / `worked_on`: Connects Agents to Chats and Files
- `mentions`: Connects Chats to Files and Entities
- `depends_on`: Connects Files to Entities / Projects
- `decided_in`: Connects Decisions to Chats and Projects
- `supersedes`: Dashed red arrow indicating an obsolete decision replaced by a newer one

---

## Local versus Cloud State Surface

The Brain panel displays the storage mode clearly without making cloud mode feel like a different brain:

```text
Brain: Local
Processing: On device
Cloud Sync: Off

Brain: Synced
Processing: On device
Cloud Sync: Last synced 2 min ago
Devices: 3

Brain: Enterprise
Processing: On device (default)
Cloud policy: Workspace managed
```

The UI exposes `Sync now`, sync health, pending changes, and connected devices. It never displays access tokens, secret material, or raw remote payloads.

---

## Verification & Quality Gates

- [ ] Click **"AI Brain"** in the left sidebar → verify navigation to `/ai-brain`.
- [ ] Verify D3 force graph initializes at 60 FPS with fluid pan/zoom.
- [ ] Verify typography inherits `--buzz-type-scale` and `--buzz-type-rem` when user adjusts zoom or font preferences.
- [ ] Click a Project node → verify connected chats and agents highlight correctly.
- [ ] Verify clicking a chat opens the drawer with complete history and jump actions.
- [ ] Run `just desktop-screenshot --name ai-brain-graph` to capture screenshot.
- [ ] Run `just ci`.
