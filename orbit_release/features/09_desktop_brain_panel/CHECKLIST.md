# Feature 09 — Obsidian-Style Desktop Brain Graph Checklist

> **Directory**: `orbit_release/features/09_desktop_brain_panel/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 5 (Weeks 8–9)

---

## Deliverables & Tasks

### 1. Tauri Backend Graph Queries (`desktop/src-tauri/src/graph.rs`)
- [ ] Implement `fetch_brain_graph` query joining projects, chats, agents, files, entities, and relations
- [ ] Implement time-bounded filtering for temporal playback
- [ ] Implement node classification (Projects, Chats, Agents, Files, Concepts, Decisions)
- [ ] Register `fetch_brain_graph` in `desktop/src-tauri/src/lib.rs`

### 2. D3 Force Graph Canvas Component (`desktop/src/features/memory/BrainGraph.tsx`)
- [ ] Install `d3` and `d3-force` dependencies in `desktop/package.json`
- [ ] Implement responsive SVG/Canvas renderer with zoom and pan behaviors
- [ ] Set up D3 force simulation:
  - [ ] `forceLink` (spring distance)
  - [ ] `forceManyBody` (charge repulsion)
  - [ ] `forceCollide` (radius padding)
  - [ ] `forceCenter`
- [ ] Implement glowing Obsidian-style aesthetics (dark canvas, neon orbs, particle styling)
- [ ] Render node labels on hover and at higher zoom levels

### 3. Interactive Physics & Inspection
- [ ] Implement node click handler: smooth camera zoom & center
- [ ] Dim unrelated nodes on node hover/click
- [ ] Highlight 1-hop and 2-hop edges with animated glowing lines
- [ ] Build slide-out `NodeDetailsDrawer.tsx`:
  - [ ] Show connected Projects, Chats, Agents, and Files
  - [ ] Show associated code diffs and decisions
  - [ ] 1-Click "Open in Chat" and "Open File" actions

### 4. Filters & Controls
- [ ] Build physics controls bar (adjust repulsion, link distance, gravity live)
- [ ] Build node type filter toggles (Projects, Chats, Agents, Files, Concepts, Decisions)
- [ ] Build Agent dropdown filter (filter graph to what Antigravity, Claude, Cursor, etc. touched)
- [ ] Build Project selector
- [ ] Build temporal time slider (scrub through historical graph evolution)
- [ ] Build graph search input with instant node match highlighting

---

## Verification & Sign-off

- [ ] Graph renders smoothly at 60 FPS with 5,000+ nodes
- [ ] Clicking a node smoothly opens drawer and highlights connected neighborhood
- [ ] Agent filter correctly isolates work done by chosen agents
- [ ] `just desktop-screenshot --name memory-graph` captures clean visual
- [ ] `just ci` passes cleanly
