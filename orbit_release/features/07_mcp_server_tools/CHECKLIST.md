# Feature 07 — MCP Server Tools & Settings Configuration Checklist

> **Directory**: `orbit_release/features/07_mcp_server_tools/`  
> **Status**: `[x] Complete / Verified`  
> **Estimated Effort**: Sprint 4 (Week 7)

---

## Deliverables & Tasks

### 1. Frontend: Desktop Settings UI (`desktop/src/features/settings/`)
- [x] Create `desktop/src/features/settings/ui/PluginsMcpSettingsPanel.tsx`
- [x] Register panel in `SettingsPanels.tsx` and `SettingsView.tsx` under `"Plugins & MCP Tools"`
- [x] Implement Built-in Tools Table with individual Enable/Disable toggle switches
- [x] Implement per-tool Execution Policy selector (`Auto-Approve`, `Confirm-on-Execute`, `Disabled`)
- [x] Build `AddCustomMcpServerModal.tsx` supporting `stdio` and `sse` transports
- [x] Implement "Test Connection" handshake button that pings JSON-RPC `tools/list` and renders tool schema preview
- [x] Build Plugins section for installing, configuring, and toggling community/custom plugins
- [x] Add Secret Redaction preview and link to audit logs

### 2. Desktop Backend: Tauri Rust IPC (`desktop/src-tauri/src/commands/mcp_config.rs`)
- [x] Implement `list_mcp_servers` IPC command
- [x] Implement `save_mcp_server` IPC command persisting to `~/.orbit/mcp_servers.json`
- [x] Implement `delete_mcp_server` IPC command
- [x] Implement `test_mcp_connection` IPC command executing live stdio/SSE probe
- [x] Implement `list_plugins` and `toggle_plugin` IPC commands
- [x] Implement `get_tool_policies` and `set_tool_policy` IPC commands
- [x] Register all commands in `desktop/src-tauri/src/lib.rs` inside `generate_handler![]`
- [x] Implement child process supervisor with graceful termination and buffer limits

### 3. Core Workspace Crates (`crates/buzz-mcp` & `crates/buzz-dev-mcp`)
- [x] Add `rmcp` (Rust MCP protocol library) to `crates/buzz-mcp/Cargo.toml`
- [x] Wire dependencies to `buzz-db`, `buzz-search`, `buzz-core`, `buzz-ai`
- [x] Register developer tools in `crates/buzz-dev-mcp/src/lib.rs` (`shell`, `read_file`, `edit_file`)
- [x] Implement the 8 standardized Orbit MCP tools:
  - [x] `orbit.search_context`
  - [x] `orbit.store_memory`
  - [x] `orbit.get_project_context`
  - [x] `orbit.recall_session`
  - [x] `orbit.get_file_history`
  - [x] `orbit.mark_decision`
  - [x] `orbit.get_index_stats`
  - [x] `orbit.delete_memory`
- [x] Implement Context Arbiter security layer:
  - [x] `<orbit_untrusted_context>` delimiter wrapper
  - [x] Outbound secret redaction filter (`buzz-db/src/redactor.rs`)
  - [x] Audit logging integration with `buzz-audit`
- [x] Transports:
  - [x] Stdio transport for local agent subprocessing
  - [x] SSE / HTTP transport for network-accessible agents

### 4. Packaging & Bundling (`desktop/src-tauri/tauri.conf.json`)
- [x] Add `"binaries/buzz-mcp"` to `bundle.externalBin`
- [x] Add `"binaries/buzz-dev-mcp"` to `bundle.externalBin`
- [x] Bundle default plugin configurations in `resources/plugins/`

---

## Verification & Sign-off

- [x] `cargo test -p buzz-mcp` passes cleanly
- [x] Desktop Settings UI renders "Plugins & MCP Tools" panel cleanly under Catppuccin theme
- [x] Custom stdio server can be added, tested, and discovered from the Settings UI
- [x] Tool execution policies (`Auto-Approve` vs `Confirm-on-Execute`) properly gate execution
- [x] JSON-RPC stdio handshake and tool invocation verified with mock client
- [x] All 8 tools return valid JSON conforming to MCP schema
- [x] `just ci` passes cleanly
