# Feature 07 — MCP Server Tools & Settings Configuration (orbit.* Tools, Custom MCP Servers & Plugins)

> **Priority**: P0 — Integration point. Exposes Orbit intelligence to AI agents and provides user configuration in Desktop Settings.  
> **Sprint**: Sprint 4 (Week 7)  
> **Dependencies**: Feature 01, 02, 04, 05, 06  
> **Crates**: `buzz-mcp` (server & client implementation), `buzz-dev-mcp` (developer tools), `buzz-core`, `buzz-db`, `buzz-search`, `buzz-ai`  
> **Desktop Surface**: Settings Panel (`desktop/src/features/settings/ui/PluginsMcpSettingsPanel.tsx`), Tauri IPC (`desktop/src-tauri/src/commands/mcp_config.rs`)  
> **Environment Variables**: `BUZZ_DATABASE_URL`, `BUZZ_DATA_DIR`

---

## Overview

Feature 07 delivers the comprehensive **Model Context Protocol (MCP) Fabric & Settings Management** for Orbit:
1. Exposes the 8 standardized `orbit.*` MCP tools to AI coding agents (Antigravity, Claude Code, Cursor, Codex, Goose, OpenCode, ZCode, AGY, Kimi).
2. Provides a first-class **Plugins & MCP Tools Settings Panel** in the Orbit desktop application, enabling users to toggle individual tools, register custom MCP servers (stdio & SSE), install plugins, and manage security execution policies.
3. Implements the complete 4-tier layer architecture across Frontend UI, Desktop Backend IPC, Core Crates, and Native Bundling.

---

## 4-Tier Implementation Layer Architecture

### 1. Frontend Tier: Desktop App Settings UI
- **Location**: `desktop/src/features/settings/ui/PluginsMcpSettingsPanel.tsx`
- **Registration**: Registered in `desktop/src/features/settings/ui/SettingsPanels.tsx` and `SettingsView.tsx` under the navigation group **"Plugins & MCP Tools"**.
- **Interactive UI Capabilities**:
  - **Built-in Orbit & Developer Tools Table**:
    - Displays all pre-packaged tools: `orbit.search_context`, `orbit.store_memory`, `orbit.get_project_context`, `orbit.recall_session`, `orbit.get_file_history`, `orbit.mark_decision`, `orbit.get_index_stats`, `orbit.delete_memory`, plus developer tools (`buzz-dev-mcp:shell`, `buzz-dev-mcp:read_file`, `buzz-dev-mcp:edit_file`).
    - Per-tool Enable/Disable toggle switch.
    - Per-tool Execution Policy selector:
      - `Auto-Approve`: Allowed to execute without prompting (default for read-only tools).
      - `Confirm-on-Execute`: Prompts developer for explicit approval before running (recommended for write tools, file edits, shell commands).
      - `Disabled`: Suppressed from agent discovery.
  - **Custom MCP Servers Management**:
    - List of active and configured external MCP servers.
    - `+ Add Custom MCP Server` modal:
      - Name & Identifier.
      - Transport: `stdio` (Executable Path, CLI Arguments, Working Directory, Custom Environment Variables) or `sse` / `http` (Server Endpoint URL, Custom HTTP Headers, Bearer Token).
      - Test Connection button: Performs an immediate live handshake, pings `tools/list`, displays response latency, and renders discovered tool schemas.
      - Delete / Edit actions.
  - **Plugins Marketplace & Local Importer**:
    - Directory of installable plugins (GitHub Connector, Slack Reader, PostgreSQL Schema Explorer, Jira Issue Tracker, Web Search).
    - Custom Plugin Installer: Import via Git URL, local folder path, or manifest JSON.
    - Plugin Enable/Disable toggle and configuration card.
  - **Security & Redaction Banner**:
    - Secret redaction preview and toggle.
    - Direct link to `buzz-audit` execution log.

### 2. Desktop Backend Tier: Tauri Rust IPC & Process Supervision
- **Location**: `desktop/src-tauri/src/commands/mcp_config.rs`
- **Tauri IPC Command Handlers**:
  ```rust
  #[tauri::command]
  pub async fn list_mcp_servers() -> Result<Vec<McpServerConfig>, String>;

  #[tauri::command]
  pub async fn save_mcp_server(server: McpServerConfig) -> Result<(), String>;

  #[tauri::command]
  pub async fn delete_mcp_server(id: String) -> Result<(), String>;

  #[tauri::command]
  pub async fn test_mcp_connection(config: McpServerConfig) -> Result<McpTestResult, String>;

  #[tauri::command]
  pub async fn list_plugins() -> Result<Vec<PluginConfig>, String>;

  #[tauri::command]
  pub async fn toggle_plugin(id: String, enabled: bool) -> Result<(), String>;

  #[tauri::command]
  pub async fn get_tool_policies() -> Result<HashMap<String, ToolPolicy>, String>;

  #[tauri::command]
  pub async fn set_tool_policy(tool_name: String, policy: ToolPolicy) -> Result<(), String>;
  ```
- **Registration**: Bound into `desktop/src-tauri/src/lib.rs` inside `tauri::generate_handler![]`.
- **Config Storage**: Stored locally in `~/.orbit/mcp_servers.json` and `~/.orbit/plugins/config.json`.
- **Process Supervisor**: Manages child process lifecycles for stdio servers, kills orphaned processes on app shutdown, and enforces stdout/stderr buffer limits.

### 3. Core Workspace Crates Tier (`crates/buzz-mcp` & `crates/buzz-dev-mcp`)
- **Protocol**: JSON-RPC 2.0 over `stdio` and `SSE` using `rmcp`.
- **Tool Registry**: Dynamic registry in `buzz-mcp` aggregating active built-in tools and registered plugins.
- **Context Arbiter & Safety Fencing**:
  - Outbound delimiter fencing: Wraps retrieved memory in `<orbit_untrusted_context source="...">`.
  - Secret redaction: Strips credentials matching regex patterns via `buzz-db/src/redactor.rs`.
  - Audit logging: Records every tool invocation in `buzz-audit`.

### 4. Packaging & Bundling Tier
- **External Binaries**: `desktop/src-tauri/tauri.conf.json` bundles:
  - `"binaries/buzz-mcp"`
  - `"binaries/buzz-dev-mcp"`
- **Resources**: Default plugin manifests packaged in `resources/plugins/`.

---

## The 8 Standardized Orbit MCP Tools

| Tool Name | Type | Signature | Description | Backend Feature |
|-----------|------|-----------|-------------|-----------------|
| `orbit.search_context` | Read | `(query: string, limit?: number, workspace?: string)` | Hybrid Multi-RAG search over code, docs, and memories | Feature 04 |
| `orbit.store_memory` | Write | `(content: string, tags?: string[], scope?: string)` | Explicitly writes a new architectural decision or fact | Feature 01, 02 |
| `orbit.get_project_context` | Read | `(path: string, max_tokens?: number)` | Generates full architectural summary for a workspace | Feature 03, 04 |
| `orbit.recall_session` | Read | `(agent?: string, query?: string, days?: number)` | Recalls past conversations and decisions from agents | Feature 06 |
| `orbit.get_file_history` | Read | `(file_path: string)` | Retrieves decisions and changes affecting a specific file | Feature 05 |
| `orbit.mark_decision` | Write | `(id: string, state: string, note?: string)` | Updates or invalidates a decision in the knowledge graph | Feature 05 |
| `orbit.get_index_stats` | Read | `()` | Returns chunk counts, index health, and sync status | Feature 01 |
| `orbit.delete_memory` | Write | `(id: string)` | Deletes a chunk/memory (GDPR compliance) | Feature 01 |

---

## Local versus Hosted MCP Policy

The MCP surface is local-first in both product editions:
- A local agent call executes directly against the local Orbit engine and local indexes (`~/.orbit/brain/orbit.db`), even when the user is signed in.
- Remote retrieval is an explicit capability controlled by workspace policy and user settings.
- Cloud sync status tools:
  - `orbit.sync_status` — pending changes, last successful sync, device identifier.
  - `orbit.sync_now` — request an immediate logical-state sync.
  - `orbit.list_devices` — show registered devices for the signed-in account/workspace.
  - `orbit.cloud_policy` — report whether the current workspace permits cloud storage or hosted processing.
- No MCP tool ever returns access tokens, subscription secrets, raw credentials, or another device's local-only data.

---

## Verification & Quality Gates

- [ ] Open Settings → **Plugins & MCP Tools** panel in Desktop app.
- [ ] Toggle built-in tools ON/OFF and verify agent discovery updates.
- [ ] Register a custom MCP server via stdio → verify "Test Connection" pings tools and displays schemas.
- [ ] Invoke `orbit.store_memory` and `orbit.search_context` over JSON-RPC stdio → verify ranked results.
- [ ] Verify secret redaction strips API keys from returned context.
- [ ] Run `cargo test -p buzz-mcp`.
- [ ] Run `just ci`.
