# Feature 07 — MCP Server Tools & Settings Configuration Checklist

> **Directory**: `orbit_release/features/07_mcp_server_tools/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 4 (Week 7)

---

## Deliverables & Tasks

### 1. Frontend: Desktop Settings UI (`desktop/src/features/settings/`)
- [ ] Create `desktop/src/features/settings/ui/PluginsMcpSettingsPanel.tsx`
- [ ] Register panel in `SettingsPanels.tsx` and `SettingsView.tsx` under `"Plugins & MCP Tools"`
- [ ] Implement Built-in Tools Table with individual Enable/Disable toggle switches
- [ ] Implement per-tool Execution Policy selector (`Auto-Approve`, `Confirm-on-Execute`, `Disabled`)
- [ ] Build `AddCustomMcpServerModal.tsx` supporting `stdio` and `sse` transports
- [ ] Implement "Test Connection" handshake button that pings JSON-RPC `tools/list` and renders tool schema preview
- [ ] Build Plugins section for installing, configuring, and toggling community/custom plugins
- [ ] Add Secret Redaction preview and link to audit logs

### 2. Desktop Backend: Tauri Rust IPC (`desktop/src-tauri/src/commands/mcp_config.rs`)
- [ ] Implement `list_mcp_servers` IPC command
- [ ] Implement `save_mcp_server` IPC command persisting to `~/.orbit/mcp_servers.json`
- [ ] Implement `delete_mcp_server` IPC command
- [ ] Implement `test_mcp_connection` IPC command executing live stdio/SSE probe
- [ ] Implement `list_plugins` and `toggle_plugin` IPC commands
- [ ] Implement `get_tool_policies` and `set_tool_policy` IPC commands
- [ ] Register all commands in `desktop/src-tauri/src/lib.rs` inside `generate_handler![]`
- [ ] Implement child process supervisor with graceful termination and buffer limits

### 3. Core Workspace Crates (`crates/buzz-mcp` & `crates/buzz-dev-mcp`)
- [ ] Add `rmcp` (Rust MCP protocol library) to `crates/buzz-mcp/Cargo.toml`
- [ ] Wire dependencies to `buzz-db`, `buzz-search`, `buzz-core`, `buzz-ai`
- [ ] Register developer tools in `crates/buzz-dev-mcp/src/lib.rs` (`shell`, `read_file`, `edit_file`)
- [ ] Implement the 8 standardized Orbit MCP tools:
  - [ ] `orbit.search_context`
  - [ ] `orbit.store_memory`
  - [ ] `orbit.get_project_context`
  - [ ] `orbit.recall_session`
  - [ ] `orbit.get_file_history`
  - [ ] `orbit.mark_decision`
  - [ ] `orbit.get_index_stats`
  - [ ] `orbit.delete_memory`
- [ ] Implement Context Arbiter security layer:
  - [ ] `<orbit_untrusted_context>` delimiter wrapper
  - [ ] Outbound secret redaction filter (`buzz-db/src/redactor.rs`)
  - [ ] Audit logging integration with `buzz-audit`
- [ ] Transports:
  - [ ] Stdio transport for local agent subprocessing
  - [ ] SSE / HTTP transport for network-accessible agents

### 4. Packaging & Bundling (`desktop/src-tauri/tauri.conf.json`)
- [ ] Add `"binaries/buzz-mcp"` to `bundle.externalBin`
- [ ] Add `"binaries/buzz-dev-mcp"` to `bundle.externalBin`
- [ ] Bundle default plugin configurations in `resources/plugins/`

---

## Verification & Sign-off

- [ ] `cargo test -p buzz-mcp` passes cleanly
- [ ] Desktop Settings UI renders "Plugins & MCP Tools" panel cleanly under Catppuccin theme
- [ ] Custom stdio server can be added, tested, and discovered from the Settings UI
- [ ] Tool execution policies (`Auto-Approve` vs `Confirm-on-Execute`) properly gate execution
- [ ] JSON-RPC stdio handshake and tool invocation verified with mock client
- [ ] All 8 tools return valid JSON conforming to MCP schema
- [ ] `just ci` passes cleanly
