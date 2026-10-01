# Feature 09 — Obsidian-Style Desktop Brain Graph Checklist

> **Directory**: `orbit_release/features/09_desktop_brain_panel/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 5 (Weeks 8–9)

---

## Deliverables & Tasks

### 1. Frontend: Primary "AI Brain" Navigation & Route Setup
- [ ] Add prominent navigation item **"AI Brain"** in `desktop/src/features/sidebar/ui/AppSidebar.tsx` and `CommunityRail.tsx`
- [ ] Bind keyboard shortcut `Cmd+B` / `Ctrl+B` for rapid toggle to AI Brain
- [ ] Create route screen `desktop/src/app/routes/ai-brain.tsx`
- [ ] Build Top HUD banner (`desktop/src/features/memory/BrainHUD.tsx`):
  - [ ] Monitored workspace directories counter
  - [ ] Total indexed files & chunks count
  - [ ] Vector index health indicator
  - [ ] Active connected agents pill (Claude Code, Antigravity, Cursor, etc.)
  - [ ] Cloud sync status pill (`Local` / `Synced` / `Syncing...`)
- [ ] Build SuperRAG Omnibar Search Input (`desktop/src/features/memory/SuperRagSearchBar.tsx`):
  - [ ] Real-time hybrid search query execution
  - [ ] Candidate confidence score gauge
  - [ ] Instant matching node pulse/highlight on the canvas

### 2. Frontend: D3 Force Graph Canvas (`desktop/src/features/memory/BrainGraph.tsx`)
- [ ] Install `d3` and `d3-force` dependencies in `desktop/package.json`
- [ ] Implement responsive SVG/Canvas renderer with zoom and pan behaviors
- [ ] Set up D3 force simulation:
  - [ ] `forceLink` (spring distance)
  - [ ] `forceManyBody` (charge repulsion)
  - [ ] `forceCollide` (radius padding)
  - [ ] `forceCenter`
- [ ] Implement glowing Obsidian-style aesthetics (dark canvas, neon orbs, particle styling)
- [ ] Render node labels on hover and at higher zoom levels
- [ ] Render 6 node types with distinct visuals:
  - [ ] 🟢 Projects & Workspaces
  - [ ] 🔵 Chats & Threads
  - [ ] 🟣 Agents
  - [ ] 🟠 Files & Code
  - [ ] 🟡 Concepts & Entities
  - [ ] ⚪ Decisions & Memories

### 3. Frontend: Interactive Physics & Inspection
- [ ] Implement node click handler: smooth camera zoom & center
- [ ] Dim unrelated nodes on node hover/click
- [ ] Highlight 1-hop and 2-hop edges with animated glowing lines
- [ ] Build slide-out `NodeDetailsDrawer.tsx`:
  - [ ] Show connected Projects, Chats, Agents, and Files
  - [ ] Show associated code diffs and decisions
  - [ ] 1-Click "Open in Chat" and "Open File" actions
  - [ ] 1-Click "Invalidate Decision" action

### 4. Frontend: Filters & Controls
- [ ] Build physics controls bar (adjust repulsion, link distance, gravity live)
- [ ] Build node type filter toggles (Projects, Chats, Agents, Files, Concepts, Decisions)
- [ ] Build Agent dropdown filter (filter graph to what Antigravity, Claude, Cursor, etc. touched)
- [ ] Build Project selector
- [ ] Build temporal time slider (scrub through historical graph evolution)
- [ ] Build graph search input with instant node match highlighting

### 5. Desktop Backend: Tauri Rust IPC (`desktop/src-tauri/src/commands/brain_graph.rs`)
- [ ] Implement `fetch_brain_graph` query joining `orbit_documents`, `orbit_chunks`, `orbit_entities`, and `orbit_relations`
- [ ] Implement `query_superrag` IPC command executing hybrid retrieval
- [ ] Implement `get_brain_stats` IPC command returning live ingestion metrics
- [ ] Implement `delete_memory_node` IPC command
- [ ] Implement time-bounded filtering for temporal playback
- [ ] Register all commands in `desktop/src-tauri/src/lib.rs` inside `generate_handler![]`

---

## Verification & Sign-off

- [ ] Click **"AI Brain"** in sidebar → opens `/ai-brain` view cleanly
- [ ] Graph renders smoothly at 60 FPS with 5,000+ nodes
- [ ] Clicking a node smoothly opens drawer and highlights connected neighborhood
- [ ] Agent filter correctly isolates work done by chosen agents
- [ ] `just desktop-screenshot --name ai-brain-graph` captures clean visual
- [ ] `just ci` passes cleanly
