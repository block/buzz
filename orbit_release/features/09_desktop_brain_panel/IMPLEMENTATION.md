# Feature 09 — Obsidian-Style Desktop Brain Graph (D3 Force Graph & Memory Explorer)

> **Priority**: P1 — Visual, interactive knowledge universe for developers.  
> **Sprint**: Sprint 5 (Weeks 8–9)  
> **Dependencies**: Feature 04 (SuperRAG Search), Feature 05 (Knowledge Graph), Feature 07 (MCP)  
> **Components**: `desktop/src/features/memory/` (React 19), `desktop/src-tauri/src/graph.rs` (Tauri IPC)  
> **Aesthetic Reference**: Obsidian Graph View (dark universe, glowing nodes, fluid physics)  
> **Codebase Design System**: Catppuccin Theme & Buzz Typography contract (`desktop/src/shared/styles/globals/`)

---

## Overview

Feature 09 implements a **dynamic, interactive Obsidian-style Brain Graph** within the Orbit React 19 desktop application. Rather than navigating a static table of memories, developers explore their entire ecosystem as a living, interconnected universe of:
- **Projects & Workspaces** (Layer 1)
- **Chats & Conversation Threads** (Layer 1 & 3)
- **AI Agents** working across tasks (Layer 3 & 5)
- **Files & Source Code** (Layer 1 & 2)
- **Entities, Concepts & Architectural Decisions** (Layer 3)

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
             (Entity: NIP-42) 🟡 ─────────► [Project: Buzz Relay] 🟢
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

## Interactive Capabilities

### 1. Physics Engine (D3 Force Simulation)
- **`forceLink`**: Pulls related nodes together with spring tension.
- **`forceManyBody`**: Repels unlinked nodes to create clean clusters by project and agent.
- **`forceCollide`**: Prevents node overlaps.
- **`forceCenter`**: Centers the visual universe on the viewport.
- **Dynamic Physics Sliders**: Developers can adjust Gravity, Charge Repulsion, and Link Distance live.

### 2. Inspection Drawer
Clicking any node:
1. Centers and smoothly zooms the camera onto the node.
2. Dims all unrelated nodes in the background.
3. Highlights 1-hop and 2-hop connected edges with luminous energy lines.
4. Opens a slide-out drawer on the right displaying:
   - **Node Details**: Type, timestamps, author/agent.
   - **Connected Chats**: All discussions where this node was mentioned.
   - **Associated Code Diffs**: Recent commits or edits.
   - **Related Decisions**: Active ADRs and historical invalidations.
   - **Jump Button**: 1-click jump to open the chat thread or file in editor.

### 3. Filters & Temporal Playback
- **Agent Filter**: Show only nodes created or touched by specific agents (e.g. filter by "Claude" or "Antigravity").
- **Project Filter**: Scope the view to a single repository or crate.
- **Type Toggles**: Show/hide Chats, Files, or Decisions.
- **Temporal Time Slider**: Scrub back through time to visualize how knowledge grew across sprints!
- **Real-Time Search**: Types directly into graph search to highlight matching nodes instantly.

---

## Tauri IPC Bridge: `desktop/src-tauri/src/graph.rs`

```rust
#[tauri::command]
pub async fn fetch_brain_graph(
    workspace_path: Option<String>,
    filter_agent: Option<String>,
    start_time: Option<DateTime<Utc>>,
    end_time: Option<DateTime<Utc>>,
) -> Result<BrainGraphPayload, String> {
    // Queries buzz_documents, buzz_chunks, buzz_entities, buzz_relations
    // Returns nodes & links ready for D3 simulation
}
```

---

## Verification & Quality Gates

- Open `/memory/graph` in Desktop preview.
- Verify D3 force graph initializes at 60 FPS.
- Verify typography inherits `--buzz-type-scale` and `--buzz-type-rem` when user adjusts zoom or font preferences.
- Click a Project node → verify connected chats and agents highlight correctly.
- Verify clicking a chat opens the drawer with complete history.
- Run `just desktop-screenshot --name memory-graph` to capture screenshot.
- Run `just ci`.

## Local / cloud state surface

The Brain panel should show the storage mode without making cloud mode feel like a different brain:

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

The UI should expose `Sync now`, sync health, pending changes and connected devices. It must not display access tokens, secret material or raw remote payloads. A signed-in account alone is not evidence that memory is currently stored in the cloud; the user/workspace sync state must be shown explicitly.
