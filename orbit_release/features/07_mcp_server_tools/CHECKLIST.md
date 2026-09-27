# Feature 07 — MCP Server Tools Checklist

> **Directory**: `orbit_release/features/07_mcp_server_tools/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 4 (Week 7)

---

## Deliverables & Tasks

### 1. `orbit-mcp` Crate Setup
- [ ] Add `rmcp` (Rust MCP protocol library) to `crates/orbit-mcp/Cargo.toml`
- [ ] Wire dependencies to `buzz-db`, `buzz-search`, `orbit-core`, `orbit-ai`
- [ ] Register tools in `crates/buzz-dev-mcp/src/lib.rs`

### 2. 8 Orbit MCP Tools
- [ ] Implement `orbit.search_context`
- [ ] Implement `orbit.store_memory`
- [ ] Implement `orbit.get_project_context`
- [ ] Implement `orbit.recall_session`
- [ ] Implement `orbit.get_file_history`
- [ ] Implement `orbit.mark_decision`
- [ ] Implement `orbit.get_index_stats`
- [ ] Implement `orbit.delete_memory`

### 3. Context Arbiter Security
- [ ] Implement `<orbit_untrusted_context>` delimiter wrapper
- [ ] Implement outbound secret redaction filter
- [ ] Add audit logging integration with `buzz-audit`

### 4. Transports
- [ ] Support `stdio` transport for standard agent subprocessing
- [ ] Support named pipe / local domain socket transport

---

## Verification & Sign-off

- [ ] `cargo test -p orbit-mcp` passes
- [ ] JSON-RPC stdio handshake and tool invocation verified with mock client
- [ ] All 8 tools return valid JSON conforming to MCP schema
- [ ] `just ci` passes cleanly
