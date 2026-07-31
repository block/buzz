# Live per-channel MCP control (`buzz-acp`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a Buzz channel's MCP server set change while the agent keeps the *same* conversation — via `session/resume` with an unchanged sessionId and a new `mcpServers` list — and stop injecting the agent's unused global MCP schemas into every turn.

**Architecture:** The pool holds a per-channel *desired* MCP set; each live session records the set it was *applied* with. At the next turn boundary the two are compared, and on a mismatch the harness sends `session/resume` (same sessionId, new servers). Both flagship ACP adapters implement reconfigure-on-resume by fingerprinting `(cwd, mcpServers)` and recreating their subprocess with `resume`, which reloads the full transcript from disk. Agents that don't advertise `sessionCapabilities.resume` fall back to today's invalidate-and-rotate path.

**Tech Stack:** Rust (tokio, serde, clap), ACP JSON-RPC over stdio. Crate: `crates/buzz-acp`.

**Source spec:** `docs/superpowers/specs/2026-07-31-live-mcp-control-design.md` — every citation in it was re-verified against shipped source before this plan was written (see *Verification log* at the end, including three corrections).

**Scope:** This plan covers **only `crates/buzz-acp`**. The desktop UI (§5.3 of the spec) is a separate plan — this one delivers working, testable software on its own: the harness gains multi-server config, a live control lever, and the token fix, all drivable via env var + control frame without any UI.

## Global Constraints

- No `unsafe` code anywhere.
- No new `unwrap()` / `expect()` in production paths — use `?` and proper error types. (Existing `expect` on bech32 encoding is pre-existing and stays.)
- Every commit uses `git commit -s` (DCO trailer required; CI fails without it).
- New public API must have doc comments.
- Activate the toolchain before any build/test/git command: `. ./bin/activate-hermit`.
- `just ci` must pass before the PR (fmt + clippy + desktop lint + unit tests). Clippy passing does not mean fmt passes — run both.
- **Additive only.** Mirror existing in-crate patterns: `session_resume` mirrors `session_new_full`; the control handler mirrors `handle_switch_model_control`; capability recording mirrors `steering_supported`.
- **MCP changes apply at turn boundaries and must NEVER cancel an in-flight turn.** Unlike `switch_model`, there is no busy-path oneshot.
- **`strictMcpConfig` is opt-in per managed channel.** Defaulting it on would silently strip users' global MCP servers.
- Run tests with `cargo test -p buzz-acp`; the desktop crate is excluded from the root workspace.

---

### Task 1: `session_resume` + resume capability detection

**Files:**
- Modify: `crates/buzz-acp/src/acp.rs:200` (field), `:550` (init), `:603` (initialize), `:655` (new method after `session_new_full`), `:849` (accessor)
- Test: `crates/buzz-acp/src/acp.rs` (in-file `mod tests`, beside `session_new_full_includes_system_prompt_when_some` at :3258)

**Interfaces:**
- Produces: `AcpClient::session_resume(&mut self, session_id: &str, cwd: &str, mcp_servers: Vec<McpServer>) -> Result<serde_json::Value, AcpError>` and `AcpClient::resume_supported(&self) -> bool`. Task 4 calls both.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `acp.rs`:

```rust
#[tokio::test]
async fn session_resume_request_includes_session_id_cwd_and_mcp_servers() {
    let script = r#"
        read -t 2 _init
        echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
        read -t 2 REQ
        echo '{"jsonrpc":"2.0","id":1,"result":{"_receivedRequest":'"$REQ"'}}'
        sleep 1
    "#;
    let mut client = spawn_script(script).await;
    client.initialize().await.expect("initialize should succeed");

    let result = client
        .session_resume(
            "ses_test",
            "/tmp",
            vec![McpServer {
                name: "razorpay".into(),
                command: "/usr/bin/rzp".into(),
                args: vec!["--stdio".into()],
                env: vec![],
            }],
        )
        .await
        .expect("session_resume should succeed");

    let received = &result["_receivedRequest"];
    assert_eq!(received["method"].as_str(), Some("session/resume"));
    assert_eq!(received["params"]["sessionId"].as_str(), Some("ses_test"));
    assert_eq!(received["params"]["cwd"].as_str(), Some("/tmp"));
    assert_eq!(
        received["params"]["mcpServers"][0]["name"].as_str(),
        Some("razorpay"),
        "mcpServers must ride on the resume request — this is the whole mechanism"
    );
}

#[tokio::test]
async fn initialize_records_resume_supported_when_advertised() {
    let script = r#"
        read -t 2 _init
        echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"resume":{}}}}}'
        sleep 1
    "#;
    let mut client = spawn_script(script).await;
    client.initialize().await.expect("initialize should succeed");
    assert!(
        client.resume_supported(),
        "an agent advertising sessionCapabilities.resume must be detected"
    );
}

#[tokio::test]
async fn initialize_records_resume_unsupported_when_absent() {
    let script = r#"
        read -t 2 _init
        echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{}}}}'
        sleep 1
    "#;
    let mut client = spawn_script(script).await;
    client.initialize().await.expect("initialize should succeed");
    assert!(
        !client.resume_supported(),
        "absent resume capability must not be treated as supported"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp session_resume 2>&1 | tail -20
. ./bin/activate-hermit && cargo test -p buzz-acp resume_supported 2>&1 | tail -20
```

Expected: FAIL — `no method named session_resume` / `no method named resume_supported`.

- [ ] **Step 3: Add the capability field**

In `acp.rs`, beside `steering_supported: bool` (~line 200):

```rust
    /// Whether the agent advertised `agentCapabilities.sessionCapabilities.resume`
    /// in its initialize response. Gates the in-place MCP reconfiguration path;
    /// call sites fall back to session invalidation when false.
    resume_supported: bool,
```

In the constructor beside `steering_supported: false` (~line 550):

```rust
            resume_supported: false,
```

- [ ] **Step 4: Record the capability in `initialize()`**

In `initialize()`, immediately after the existing `self.steering_supported = ...` assignment (~line 603):

```rust
        // Session-level resume capability. The value is an empty object (`{}`)
        // when supported, so presence — not truthiness — is the signal.
        self.resume_supported = result
            .pointer("/agentCapabilities/sessionCapabilities/resume")
            .map(|v| !v.is_null())
            .unwrap_or(false);
```

- [ ] **Step 5: Add the accessor**

Beside `steering_supported()` (~line 849):

```rust
    /// Whether the connected agent supports `session/resume`.
    pub fn resume_supported(&self) -> bool {
        self.resume_supported
    }
```

- [ ] **Step 6: Add `session_resume`**

Immediately after `session_new` (~line 672):

```rust
    /// Send `session/resume` to reconfigure a live session in place.
    ///
    /// The session ID is unchanged. Adapters that fingerprint the
    /// session-defining params (`cwd` + `mcpServers`) tear down and recreate
    /// the underlying agent process with `resume`, restoring the full
    /// conversation transcript from disk — this is how an MCP grant lands
    /// without losing the conversation.
    ///
    /// `cwd` must be an absolute path. `mcp_servers` may be empty. Gated on
    /// [`resume_supported`](Self::resume_supported): callers must fall back to
    /// session invalidation when the agent does not advertise the capability.
    ///
    /// Prefer this over `session/load` — `load` replays the whole history as
    /// `session/update` notifications before responding, which risks the
    /// request timeout on long conversations. Reconfiguration semantics are
    /// identical.
    pub async fn session_resume(
        &mut self,
        session_id: &str,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
    ) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "cwd": cwd,
            "mcpServers": mcp_servers,
        });
        self.send_request("session/resume", params).await
    }
```

- [ ] **Step 7: Run tests to verify they pass**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp session_resume resume_supported 2>&1 | tail -20
```

Expected: 3 passed.

- [ ] **Step 8: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-acp/src/acp.rs
git commit -s -m "feat(acp): session/resume request + resume capability detection"
```

---

### Task 2: Multi-server MCP configuration

**Files:**
- Modify: `crates/buzz-acp/src/acp.rs:27-40` (derives on `McpServer` / `EnvVar`)
- Modify: `crates/buzz-acp/src/config.rs:261` (clap arg), `:497` (Config field), `:1061` (mapping)
- Modify: `crates/buzz-acp/src/lib.rs:4179` (`build_mcp_servers`)
- Test: `crates/buzz-acp/src/lib.rs` (`mod build_mcp_servers_tests` at :4989)

**Interfaces:**
- Consumes: `McpServer` from Task 1's test usage (unchanged shape).
- Produces: `McpServer: Deserialize + PartialEq`; `Config.mcp_servers_json: String`; `build_mcp_servers(config) -> Vec<McpServer>` now returns the dev-MCP server **plus** any servers parsed from `BUZZ_ACP_MCP_SERVERS`. Tasks 3–5 rely on `PartialEq` for the desired-vs-applied comparison and on `Deserialize` for control-frame parsing.

**Why `Deserialize` is needed:** `McpServer` is currently `Serialize`-only. Both the new env-var config and the Task 5 control frame arrive as JSON that must be parsed *into* it.

- [ ] **Step 1: Write the failing test**

Add to `mod build_mcp_servers_tests` in `lib.rs`:

```rust
    #[test]
    fn build_mcp_servers_appends_json_configured_servers() {
        let mut config = test_config();
        config.mcp_command = String::new(); // isolate the JSON path
        config.mcp_servers_json = r#"[
            {"name":"razorpay","command":"/usr/bin/rzp","args":["--stdio"],"env":[{"name":"RZP_KEY","value":"secret"}]}
        ]"#
        .to_string();

        let servers = build_mcp_servers(&config);

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "razorpay");
        assert_eq!(servers[0].command, "/usr/bin/rzp");
        assert_eq!(servers[0].args, vec!["--stdio".to_string()]);
        assert_eq!(servers[0].env[0].name, "RZP_KEY");
    }

    #[test]
    fn build_mcp_servers_keeps_dev_mcp_alongside_json_servers() {
        let mut config = test_config();
        config.mcp_command = "/usr/local/bin/buzz-dev-mcp".to_string();
        config.mcp_servers_json =
            r#"[{"name":"razorpay","command":"/usr/bin/rzp","args":[],"env":[]}]"#.to_string();

        let servers = build_mcp_servers(&config);

        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"buzz-dev-mcp"), "dev MCP must survive: {names:?}");
        assert!(names.contains(&"razorpay"), "json server must be added: {names:?}");
    }

    #[test]
    fn build_mcp_servers_ignores_malformed_json_rather_than_dropping_dev_mcp() {
        let mut config = test_config();
        config.mcp_command = "/usr/local/bin/buzz-dev-mcp".to_string();
        config.mcp_servers_json = "{ not valid json".to_string();

        let servers = build_mcp_servers(&config);

        assert_eq!(
            servers.len(),
            1,
            "malformed config must fail open to the dev MCP, not panic or wipe the list"
        );
        assert_eq!(servers[0].name, "buzz-dev-mcp");
    }
```

> Use the module's existing `test_config()` helper — if it is named differently in `mod build_mcp_servers_tests`, use whatever helper the neighbouring tests (`lib.rs:5044`, `:5065`) already call to build a `Config`.

- [ ] **Step 2: Run tests to verify they fail**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp build_mcp_servers 2>&1 | tail -20
```

Expected: FAIL — `no field mcp_servers_json on type Config`.

- [ ] **Step 3: Add the derives**

In `acp.rs`, replace the derive lines on `McpServer` (line 27) and `EnvVar` (line 36):

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct McpServer {
```

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EnvVar {
```

- [ ] **Step 4: Add the config field**

In `config.rs`, after the `mcp_command` arg (line 261-262):

```rust
    /// Additional MCP servers as a JSON array of
    /// `{name, command, args, env:[{name,value}]}` objects. Merged with the
    /// dev-MCP server derived from `--mcp-command`. Malformed JSON is logged
    /// and ignored (fail open) rather than dropping the dev MCP.
    #[arg(long, env = "BUZZ_ACP_MCP_SERVERS", default_value = "")]
    pub mcp_servers_json: String,
```

In the `Config` struct after `pub mcp_command: String,` (line 497):

```rust
    pub mcp_servers_json: String,
```

In the `Config { .. }` construction after `mcp_command: args.mcp_command,` (line 1061):

```rust
            mcp_servers_json: args.mcp_servers_json,
```

- [ ] **Step 5: Merge the JSON servers in `build_mcp_servers`**

In `lib.rs`, restructure `build_mcp_servers` (line 4179). Keep the entire existing dev-MCP body; change only the early return and the tail:

```rust
fn build_mcp_servers(config: &Config) -> Vec<McpServer> {
    let mut servers: Vec<McpServer> = Vec::new();

    if !config.mcp_command.is_empty() {
        servers.push(McpServer {
            // ... existing dev-MCP construction, unchanged ...
        });
    }

    servers.extend(parse_extra_mcp_servers(&config.mcp_servers_json));
    servers
}

/// Parse the `BUZZ_ACP_MCP_SERVERS` JSON array.
///
/// Fails open: a malformed value is logged and treated as "no extra servers",
/// so a bad env var degrades to legacy behaviour instead of taking the agent's
/// dev MCP down with it.
fn parse_extra_mcp_servers(raw: &str) -> Vec<McpServer> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Vec<McpServer>>(raw) {
        Ok(servers) => servers,
        Err(error) => {
            tracing::warn!("BUZZ_ACP_MCP_SERVERS is not a valid MCP server array: {error}");
            Vec::new()
        }
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp build_mcp_servers 2>&1 | tail -20
```

Expected: all `build_mcp_servers_*` tests pass, including the pre-existing ones.

- [ ] **Step 7: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-acp/src/acp.rs crates/buzz-acp/src/config.rs crates/buzz-acp/src/lib.rs
git commit -s -m "feat(acp): configure multiple MCP servers via BUZZ_ACP_MCP_SERVERS"
```

---

### Task 3: Per-channel desired/applied MCP state

**Files:**
- Modify: `crates/buzz-acp/src/pool.rs:~105` (`SessionState.applied_mcp`), `:124` (`invalidate_channel`), `:137` (`invalidate_all`), `:~170` (`OwnedAgent.desired_mcp`), `:599` (`return_agent`)
- Modify: `crates/buzz-acp/src/lib.rs:2928` (stamp at dispatch), `:1793` + `:3838` + `:5289` (struct literals gain the new field)
- Test: `crates/buzz-acp/src/pool.rs` (in-file `mod tests`)

**Interfaces:**
- Consumes: `McpServer: PartialEq` from Task 2.
- Produces:
  - `AgentPool.desired_mcp: HashMap<Uuid, Vec<McpServer>>` (pool-owned authority, runtime-only) with `AgentPool::set_desired_mcp(&mut self, channel_id: Uuid, servers: Vec<McpServer>)` and `AgentPool::desired_mcp_for(&self, channel_id: &Uuid) -> Option<&Vec<McpServer>>`.
  - `OwnedAgent.desired_mcp: Option<Vec<McpServer>>` — the set stamped onto the agent for the channel it is about to serve.
  - `SessionState.applied_mcp: HashMap<Uuid, Vec<McpServer>>` — what each live channel session was actually built with.

**Design note (deviation from spec §5.1/§5.2, deliberate):** the spec proposes a `mcp_dirty: HashSet<Uuid>` plus an eager "apply NOW if idle" path. This plan instead compares *desired vs applied* at the turn boundary. That is strictly less state (no dirty set to set, clear, or leak), and it is more correct: a dirty flag set on agent A is invisible when agent B picks the channel up, whereas the comparison is self-healing across agent swaps. It also removes the need to `await` a resume from the synchronous control handler. User-visible outcome is unchanged — same session, full transcript, new tools on the next turn.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `pool.rs`:

```rust
    #[test]
    fn invalidate_channel_clears_applied_mcp() {
        let cid = Uuid::new_v4();
        let mut state = SessionState::default();
        state.sessions.insert(cid, "ses_1".to_string());
        state.applied_mcp.insert(
            cid,
            vec![McpServer {
                name: "razorpay".into(),
                command: "/usr/bin/rzp".into(),
                args: vec![],
                env: vec![],
            }],
        );

        assert!(state.invalidate_channel(&cid));

        assert!(
            !state.applied_mcp.contains_key(&cid),
            "a dropped session must not leave a stale applied-MCP record behind"
        );
    }

    #[test]
    fn set_desired_mcp_is_readable_per_channel() {
        let mut pool = test_pool();
        let cid = Uuid::new_v4();
        let servers = vec![McpServer {
            name: "razorpay".into(),
            command: "/usr/bin/rzp".into(),
            args: vec![],
            env: vec![],
        }];

        pool.set_desired_mcp(cid, servers.clone());

        assert_eq!(pool.desired_mcp_for(&cid), Some(&servers));
        assert_eq!(pool.desired_mcp_for(&Uuid::new_v4()), None);
    }
```

> `test_pool()` — use whichever pool constructor the neighbouring tests in `pool.rs` already use (see the `desired_model: None` struct literals at `pool.rs:5877` / `:5935` for the shape).

- [ ] **Step 2: Run tests to verify they fail**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp applied_mcp desired_mcp 2>&1 | tail -20
```

Expected: FAIL — `no field applied_mcp` / `no method set_desired_mcp`.

- [ ] **Step 3: Add `applied_mcp` to `SessionState`**

In `pool.rs`, after the `canvas_sections` field (~line 105):

```rust
    /// channel_id → the MCP server set the live session for that channel was
    /// actually created or resumed with. Compared against the desired set at
    /// each turn boundary; a mismatch triggers an in-place `session/resume`.
    /// Cleared with the session so a rotated session re-applies from scratch.
    pub applied_mcp: HashMap<Uuid, Vec<McpServer>>,
```

In `invalidate_channel` (line 124), beside the other `remove` calls:

```rust
        self.applied_mcp.remove(channel_id);
```

In `invalidate_all` (line 137), beside the other `clear` calls:

```rust
        self.applied_mcp.clear();
```

In the `#[cfg(test)] fn has_channel_state`, add the new map so the existing invariant test keeps covering it:

```rust
            || self.applied_mcp.contains_key(channel_id)
```

- [ ] **Step 4: Add `desired_mcp` to `OwnedAgent`**

In `pool.rs`, after the `model_overridden` field (~line 170):

```rust
    /// MCP server set desired for the channel this agent is about to serve,
    /// stamped by `dispatch_pending` at claim time. `None` means "no managed
    /// set for this channel" — the agent uses `PromptContext.mcp_servers`.
    /// Runtime-only, re-stamped on every dispatch.
    pub desired_mcp: Option<Vec<McpServer>>,
```

Add `desired_mcp: None,` to every `OwnedAgent { .. }` literal. Compiler will point at each; the known sites are `lib.rs:1793`, `lib.rs:3838`, `lib.rs:5289`, `pool.rs:5877`, `pool.rs:5935`.

- [ ] **Step 5: Add the pool map and accessors**

In `pool.rs`, add to the `AgentPool` struct:

```rust
    /// channel_id → desired MCP server set, the authority for what a channel's
    /// sessions should run with. Runtime-only (the desktop re-sends on
    /// reconnect), matching `desired_model` semantics.
    desired_mcp: HashMap<Uuid, Vec<McpServer>>,
```

Initialise it to `HashMap::new()` in the pool constructor, and add the accessors near `switch_idle_agent_model` (line 741):

```rust
    /// Record the desired MCP server set for a channel.
    ///
    /// Takes effect at the channel's next turn boundary: `dispatch_pending`
    /// stamps it onto the claimed agent, and the session lookup resumes the
    /// live session in place if it differs from what was applied. Never
    /// disturbs an in-flight turn.
    pub fn set_desired_mcp(&mut self, channel_id: Uuid, servers: Vec<McpServer>) {
        self.desired_mcp.insert(channel_id, servers);
    }

    /// The desired MCP server set for a channel, if one has been recorded.
    pub fn desired_mcp_for(&self, channel_id: &Uuid) -> Option<&Vec<McpServer>> {
        self.desired_mcp.get(channel_id)
    }
```

- [ ] **Step 6: Stamp the agent at dispatch**

In `lib.rs` `dispatch_pending`, immediately after the `tracing::debug!(agent = agent.index, ... "agent_claimed");` line (~2928):

```rust
        // Turn-boundary MCP application: stamp the channel's desired set onto
        // the agent before it is moved into the task. The task compares this
        // against `SessionState.applied_mcp` and resumes in place on a
        // mismatch. Stamping here — not in the control handler — is what makes
        // a live toggle land without cancelling an in-flight turn.
        agent.desired_mcp = pool.desired_mcp_for(&channel_id).cloned();
```

- [ ] **Step 7: Run tests to verify they pass**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp 2>&1 | tail -20
```

Expected: new tests pass; whole crate still green.

- [ ] **Step 8: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-acp/src/pool.rs crates/buzz-acp/src/lib.rs
git commit -s -m "feat(acp): track desired and applied MCP sets per channel"
```

---

### Task 4: Resume-swap at the turn boundary

**Files:**
- Modify: `crates/buzz-acp/src/pool.rs:1546-1556` (existing-session branch of the session lookup), `:872-910` (`create_session_and_apply_model` records applied set)
- Test: `crates/buzz-acp/src/pool.rs` (in-file `mod tests`)

**Interfaces:**
- Consumes: `AcpClient::session_resume` + `AcpClient::resume_supported` (Task 1); `OwnedAgent.desired_mcp`, `SessionState.applied_mcp` (Task 3).
- Produces: `fn effective_mcp_servers<'a>(agent: &'a OwnedAgent, ctx: &'a PromptContext) -> &'a Vec<McpServer>` — the single place that resolves "which servers should this channel run with".

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `pool.rs`:

```rust
    #[test]
    fn effective_mcp_servers_prefers_the_desired_set() {
        let ctx_servers = vec![McpServer {
            name: "buzz-dev-mcp".into(),
            command: "/usr/local/bin/buzz-dev-mcp".into(),
            args: vec![],
            env: vec![],
        }];
        let desired = vec![McpServer {
            name: "razorpay".into(),
            command: "/usr/bin/rzp".into(),
            args: vec![],
            env: vec![],
        }];

        let ctx = test_prompt_context(ctx_servers.clone());
        let mut agent = test_owned_agent();

        agent.desired_mcp = None;
        assert_eq!(
            effective_mcp_servers(&agent, &ctx),
            &ctx_servers,
            "unmanaged channel must keep legacy behaviour"
        );

        agent.desired_mcp = Some(desired.clone());
        assert_eq!(
            effective_mcp_servers(&agent, &ctx),
            &desired,
            "a managed channel must use exactly the desired set"
        );
    }

    #[test]
    fn identical_sets_do_not_trigger_a_resume() {
        // Guards against gratuitous subprocess respawns: the desired set
        // equalling the applied set must be a no-op, not a resume.
        let servers = vec![McpServer {
            name: "razorpay".into(),
            command: "/usr/bin/rzp".into(),
            args: vec!["--stdio".into()],
            env: vec![],
        }];
        let cid = Uuid::new_v4();
        let mut state = SessionState::default();
        state.applied_mcp.insert(cid, servers.clone());

        assert_eq!(
            state.applied_mcp.get(&cid),
            Some(&servers),
            "equality must hold for identical sets so no resume is issued"
        );

        let reordered_args = vec![McpServer {
            name: "razorpay".into(),
            command: "/usr/bin/rzp".into(),
            args: vec!["--other".into()],
            env: vec![],
        }];
        assert_ne!(
            state.applied_mcp.get(&cid),
            Some(&reordered_args),
            "an args change must be detected — the adapter fingerprints args too"
        );
    }
```

> `test_prompt_context(..)` / `test_owned_agent()` — reuse or extend whatever helpers `mod tests` already has for building a `PromptContext` and `OwnedAgent`; do not invent new infrastructure.

- [ ] **Step 2: Run tests to verify they fail**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp effective_mcp_servers identical_sets 2>&1 | tail -20
```

Expected: FAIL — `cannot find function effective_mcp_servers`.

- [ ] **Step 3: Add the resolver and record the applied set on creation**

In `pool.rs`, add near `create_session_and_apply_model` (line 867):

```rust
/// The MCP server set a channel should run with: the desired set when the
/// channel is managed, otherwise the harness-wide set from the prompt context.
fn effective_mcp_servers<'a>(agent: &'a OwnedAgent, ctx: &'a PromptContext) -> &'a Vec<McpServer> {
    agent.desired_mcp.as_ref().unwrap_or(&ctx.mcp_servers)
}
```

In `create_session_and_apply_model`, replace `ctx.mcp_servers.clone()` (line 906) with:

```rust
            effective_mcp_servers(agent, ctx).clone(),
```

and, after the session is successfully created, record what was applied so the
next turn's comparison has a baseline. (`create_session_and_apply_model`
returns the session id; the caller inserts into `agent.state.sessions`, so
record alongside that insert in Step 4.)

- [ ] **Step 4: Resume in place when the sets differ**

In `pool.rs` `run_prompt_task`, replace the existing-session arm of the channel branch (line ~1546):

```rust
        PromptSource::Channel(cid) => {
            if let Some(sid) = agent.state.sessions.get(cid) {
                (sid.clone(), false)
            } else {
```

with:

```rust
        PromptSource::Channel(cid) => {
            if let Some(sid) = agent.state.sessions.get(cid).cloned() {
                // Turn-boundary MCP reconciliation. The desired set is stamped
                // by `dispatch_pending`; `applied_mcp` is what this session was
                // actually built with. On a mismatch, reconfigure the live
                // session in place: same sessionId, full transcript preserved
                // by the adapter's resume, new tool set.
                let desired = effective_mcp_servers(&agent, &ctx).clone();
                if agent.state.applied_mcp.get(cid) != Some(&desired) {
                    if agent.acp.resume_supported() {
                        match agent.acp.session_resume(&sid, &ctx.cwd, desired.clone()).await {
                            Ok(_) => {
                                agent.state.applied_mcp.insert(*cid, desired);
                                tracing::info!(
                                    target: "pool::session",
                                    "resumed session {sid} for channel {cid} with a new MCP set"
                                );
                            }
                            Err(error) => {
                                // Never fail the turn over a tool grant: drop the
                                // session so the code below creates a fresh one
                                // with the desired set. Conversation continuity is
                                // lost, which is strictly better than a lost turn.
                                tracing::warn!(
                                    target: "pool::session",
                                    "session/resume failed for channel {cid} ({error}); rotating instead"
                                );
                                agent.state.invalidate_channel(cid);
                            }
                        }
                    } else {
                        tracing::info!(
                            target: "pool::session",
                            "agent does not support session/resume; rotating channel {cid} to apply its MCP set"
                        );
                        agent.state.invalidate_channel(cid);
                    }
                }

                match agent.state.sessions.get(cid) {
                    Some(sid) => (sid.clone(), false),
                    None => {
                        // Rotated above — fall through to creation.
                        match create_session_and_apply_model(
                            &mut agent,
                            &ctx,
                            agent_core.as_deref(),
                            agent_canvas.as_deref(),
                            title_channel.as_deref(),
                        )
                        .await
                        {
                            Ok(sid) => {
                                // Hoist the resolve before the mutable borrow: calling
                                // `effective_mcp_servers(&agent, ..)` inline as an argument to
                                // `agent.state.applied_mcp.insert(..)` mixes an immutable and a
                                // mutable borrow of `agent` in one expression. Do not inline it.
                                let applied = effective_mcp_servers(&agent, &ctx).clone();
                                agent.state.sessions.insert(*cid, sid.clone());
                                agent.state.applied_mcp.insert(*cid, applied);
                                (sid, true)
                            }
                            Err(AcpError::AgentExited) => {
                                agent.state.invalidate_all();
                                send_prompt_result(
                                    &result_tx,
                                    &turn_id,
                                    agent,
                                    source,
                                    PromptOutcome::AgentExited,
                                    requeue_batch_if_queue(&ctx, batch),
                                );
                                return;
                            }
                            Err(e) => {
                                send_prompt_result(
                                    &result_tx,
                                    &turn_id,
                                    agent,
                                    source,
                                    PromptOutcome::Error(e),
                                    requeue_batch_if_queue(&ctx, batch),
                                );
                                return;
                            }
                        }
                    }
                }
            } else {
```

In the pre-existing "no session yet" arm below, add the same `applied_mcp` record around `agent.state.sessions.insert(*cid, sid.clone());` — again resolving *before* the mutable borrow:

```rust
                        let applied = effective_mcp_servers(&agent, &ctx).clone();
                        agent.state.sessions.insert(*cid, sid.clone());
                        agent.state.applied_mcp.insert(*cid, applied);
```

> If the duplicated creation block reads badly once written, extract both call sites into a single local `async fn create_and_record(...)` helper — but only after the tests are green.

- [ ] **Step 5: Run tests to verify they pass**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp 2>&1 | tail -20
```

Expected: new tests pass; whole crate green.

- [ ] **Step 6: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-acp/src/pool.rs
git commit -s -m "feat(acp): apply MCP changes via in-place session/resume at turn boundaries"
```

---

### Task 5: `update_mcp_servers` control frame

**Files:**
- Modify: `crates/buzz-acp/src/lib.rs:883-890` (dispatch arm), `:~1005` (new handler after `handle_switch_model_control`)
- Test: `crates/buzz-acp/src/lib.rs` (in-file tests, beside the existing `switch_model` control tests)

**Interfaces:**
- Consumes: `AgentPool::set_desired_mcp` (Task 3); `McpServer: Deserialize` (Task 2).
- Produces: control frame `{"type":"update_mcp_servers","channelId":"<uuid>","mcpServers":[...]}` → `control_result` with `status ∈ {"pending_next_turn", "invalid_servers", "unchanged"}`.

**Security:** this frame names a command to execute — RCE by design. It rides the existing owner-signed, encrypted, ±5-minute-freshness envelope (`handle_relay_observer_control_event`, `lib.rs:837`). Do **not** add any path that accepts it outside that envelope.

- [ ] **Step 1: Write the failing test**

Add beside the existing `switch_model` control tests in `lib.rs`:

```rust
    #[test]
    fn update_mcp_servers_control_records_the_desired_set() {
        let mut pool = test_pool();
        let cid = Uuid::new_v4();
        let payload = serde_json::json!({
            "type": "update_mcp_servers",
            "channelId": cid.to_string(),
            "mcpServers": [
                {"name": "razorpay", "command": "/usr/bin/rzp", "args": [], "env": []}
            ]
        });

        handle_update_mcp_servers_control(&payload, &mut pool, None);

        let recorded = pool.desired_mcp_for(&cid).expect("desired set recorded");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].name, "razorpay");
    }

    #[test]
    fn update_mcp_servers_control_rejects_a_malformed_server_list() {
        let mut pool = test_pool();
        let cid = Uuid::new_v4();
        let payload = serde_json::json!({
            "type": "update_mcp_servers",
            "channelId": cid.to_string(),
            "mcpServers": [{"name": "missing-command"}]
        });

        handle_update_mcp_servers_control(&payload, &mut pool, None);

        assert!(
            pool.desired_mcp_for(&cid).is_none(),
            "a malformed grant must be rejected outright, never partially applied"
        );
    }

    #[test]
    fn update_mcp_servers_control_ignores_a_bad_channel_id() {
        let mut pool = test_pool();
        let payload = serde_json::json!({
            "type": "update_mcp_servers",
            "channelId": "not-a-uuid",
            "mcpServers": []
        });

        handle_update_mcp_servers_control(&payload, &mut pool, None);
        // No panic, nothing recorded.
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp update_mcp_servers 2>&1 | tail -20
```

Expected: FAIL — `cannot find function handle_update_mcp_servers_control`.

- [ ] **Step 3: Add the dispatch arm**

In `handle_relay_observer_control_event` (`lib.rs:885`), after the `switch_model` arm:

```rust
        Some("update_mcp_servers") => {
            handle_update_mcp_servers_control(&payload, pool, observer);
        }
```

- [ ] **Step 4: Add the handler**

After `handle_switch_model_control` (~line 1005):

```rust
/// Handle an `update_mcp_servers` control frame.
///
/// Records the channel's desired MCP server set. Unlike `switch_model`, this
/// never cancels an in-flight turn: the set is stamped onto the agent by
/// `dispatch_pending` at the next turn boundary, and the session is resumed in
/// place there. The status is therefore always forward-looking.
///
/// The payload names a command to execute, so this must only ever be reached
/// through the owner-signed, encrypted, freshness-checked observer path.
fn handle_update_mcp_servers_control(
    payload: &serde_json::Value,
    pool: &mut AgentPool,
    observer: Option<&observer::ObserverHandle>,
) {
    let Some(channel_id) = payload
        .get("channelId")
        .and_then(|value| value.as_str())
        .and_then(|value| value.parse::<Uuid>().ok())
    else {
        tracing::warn!("observer update_mcp_servers control frame missing valid channelId");
        return;
    };

    let Some(raw_servers) = payload.get("mcpServers") else {
        tracing::warn!("observer update_mcp_servers control frame missing mcpServers");
        return;
    };

    // Reject the whole grant on a malformed entry — a partially applied tool
    // set is worse than none, and the desktop can re-send.
    let servers: Vec<McpServer> = match serde_json::from_value(raw_servers.clone()) {
        Ok(servers) => servers,
        Err(error) => {
            tracing::warn!("observer update_mcp_servers control frame has invalid mcpServers: {error}");
            if let Some(observer) = observer {
                emit_mcp_control_result(observer, channel_id, "invalid_servers");
            }
            return;
        }
    };

    let status = if pool.desired_mcp_for(&channel_id) == Some(&servers) {
        "unchanged"
    } else {
        pool.set_desired_mcp(channel_id, servers);
        "pending_next_turn"
    };

    if let Some(observer) = observer {
        emit_mcp_control_result(observer, channel_id, status);
    }
}

fn emit_mcp_control_result(
    observer: &observer::ObserverHandle,
    channel_id: Uuid,
    status: &str,
) {
    observer.emit(
        "control_result",
        None,
        &observer::ObserverContext {
            channel_id: Some(channel_id.to_string()),
            session_id: None,
            turn_id: None,
            started_at: None,
        },
        serde_json::json!({
            "type": "update_mcp_servers",
            "status": status,
        }),
    );
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp update_mcp_servers 2>&1 | tail -20
```

Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-acp/src/lib.rs
git commit -s -m "feat(acp): update_mcp_servers control frame for live per-channel grants"
```

---

### Task 6: `strictMcpConfig` opt-in (the ~64k-token fix)

**Files:**
- Modify: `crates/buzz-acp/src/acp.rs:629-655` (`session_new_full` gains a strict flag)
- Modify: `crates/buzz-acp/src/pool.rs:~906` (call site passes it)
- Test: `crates/buzz-acp/src/acp.rs` (in-file tests)

**Interfaces:**
- Consumes: `effective_mcp_servers` / `OwnedAgent.desired_mcp` (Tasks 3–4) to decide *managed vs unmanaged*.
- Produces: `session_new_full(..., strict_mcp: bool)` — when true, sends `_meta.claudeCode.options = {"strictMcpConfig": true, "settingSources": ["project"]}`.

**Verified behaviour this depends on:** the Claude adapter spreads `_meta.claudeCode.options` straight into the SDK options object (`acp-agent.js:4103` reads it, `:4157` spreads it). `strictMcpConfig` is a real SDK option (`sdk.d.ts:1959`, "Maps to the CLI `--strict-mcp-config` flag") that suppresses project `.mcp.json`, user settings, plugin, and agent-frontmatter MCP.

**Review correction — do NOT send `settingSources`.** The spec pairs `strictMcpConfig` with `settingSources: ["project"]`. That is unnecessary and actively risky. `strictMcpConfig` alone already suppresses *every* other MCP source (`sdk.d.ts:1955-1957` enumerates them). `settingSources` is a much broader lever governing which settings files load at all — narrowing the adapter's default `["user","project","local"]` (`acp-agent.js:4156`) down to `["project"]` would silently drop the user's permission defaults and local settings for managed channels, changing behaviour far beyond MCP. Send `strictMcpConfig` only; leave `settingSources` at the adapter default.

**The adapter never names `strictMcpConfig`** — it rides the generic spread, so a typo silently no-ops with no error. The assertion in Step 1 is the only thing standing between a typo and a silent 64k-token regression.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn session_new_full_sends_strict_mcp_config_when_managed() {
    let script = r#"
        read -t 2 _init
        echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
        read -t 2 REQ
        echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
        sleep 1
    "#;
    let mut client = spawn_script(script).await;
    client.initialize().await.expect("initialize should succeed");

    let resp = client
        .session_new_full("/tmp", vec![], None, None, true)
        .await
        .expect("session_new_full should succeed");

    let opts = &resp.raw["_receivedRequest"]["params"]["_meta"]["claudeCode"]["options"];
    assert_eq!(
        opts["strictMcpConfig"].as_bool(),
        Some(true),
        "managed channels must suppress the agent's global MCP config"
    );
    assert!(
        opts["settingSources"].is_null(),
        "settingSources must be left alone — narrowing it would drop the user's \
         permission defaults for managed channels, which is not what this flag is for"
    );
}

#[tokio::test]
async fn session_new_full_omits_strict_mcp_config_when_unmanaged() {
    let script = r#"
        read -t 2 _init
        echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
        read -t 2 REQ
        echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
        sleep 1
    "#;
    let mut client = spawn_script(script).await;
    client.initialize().await.expect("initialize should succeed");

    let resp = client
        .session_new_full("/tmp", vec![], None, None, false)
        .await
        .expect("session_new_full should succeed");

    let received = &resp.raw["_receivedRequest"];
    assert!(
        received["params"]["_meta"]["claudeCode"].is_null(),
        "unmanaged channels must keep legacy behaviour — never silently strip a user's global MCP servers"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp strict_mcp 2>&1 | tail -20
```

Expected: FAIL — `session_new_full` takes 4 arguments, 5 supplied.

- [ ] **Step 3: Extend `session_new_full`**

Add the parameter and merge into `_meta` without clobbering `sessionTitle`:

```rust
    pub async fn session_new_full(
        &mut self,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
        system_prompt: Option<&str>,
        session_title: Option<&str>,
        strict_mcp: bool,
    ) -> Result<SessionNewResponse, AcpError> {
        let mut params = serde_json::json!({
            "cwd": cwd,
            "mcpServers": mcp_servers,
        });
        if let Some(sp) = system_prompt {
            params["systemPrompt"] = serde_json::Value::String(sp.to_owned());
        }
        let mut meta = serde_json::Map::new();
        if let Some(title) = session_title {
            meta.insert("sessionTitle".into(), serde_json::Value::String(title.to_owned()));
        }
        if strict_mcp {
            // Managed channel: run exactly the servers the panel shows. Without
            // this the agent additionally loads its own global MCP config,
            // injecting tool schemas nobody asked for into every turn.
            //
            // Deliberately does NOT touch `settingSources`: `strictMcpConfig`
            // already suppresses every other MCP source, and narrowing settings
            // loading would drop the user's permission defaults as a side
            // effect — a much wider blast radius than this flag is meant to have.
            meta.insert(
                "claudeCode".into(),
                serde_json::json!({
                    "options": { "strictMcpConfig": true }
                }),
            );
        }
        if !meta.is_empty() {
            params["_meta"] = serde_json::Value::Object(meta);
        }
        // ... rest unchanged ...
```

Update the `session_new` convenience wrapper to pass `false`, and update the doc comment to describe the new parameter.

- [ ] **Step 4: Update the call site**

In `pool.rs` `create_session_and_apply_model` (~line 906), a channel is *managed* exactly when it has a desired set:

```rust
            agent.desired_mcp.is_some(),
```

- [ ] **Step 5: Fix the other call sites**

```bash
. ./bin/activate-hermit && cargo build -p buzz-acp 2>&1 | grep -A3 'this function takes' | head -30
```

Pass `false` at every pre-existing `session_new_full` call and in the existing tests at `acp.rs:3274`, `:3359`, `:3387`, `:3415` — legacy behaviour is the default everywhere.

- [ ] **Step 6: Run tests to verify they pass**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp 2>&1 | tail -20
```

Expected: whole crate green.

- [ ] **Step 7: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-acp/src/acp.rs crates/buzz-acp/src/pool.rs
git commit -s -m "feat(acp): opt-in strictMcpConfig for managed channels"
```

---

### Task 7: Grant-landed verification

> **✅ DECIDED 2026-07-31 (Moni): option 1 below — honest status + opportunistic verification.** Build this task as written; no upstream dependency, no blocking.
>
> Background. The spec (§5.4) and the kickoff both make this non-negotiable: *"verify the grant actually landed (observe the next turn's advertised tool list); do not trust the resume success response."* Verification of the shipped adapter found **that surface does not exist**. `acp-agent.js:1601-1614` consumes the SDK's `system`/`init` message internally (it latches `msgLifecycleV1` and syncs fast-mode state) and never forwards `tools[]` or `mcp_servers[{name,status}]` to the ACP client, even though the SDK carries both (`sdk.d.ts:4418-4424`). `session/resume` returns only `{sessionId, modes, configOptions}`. So there is nothing on the ACP wire to observe today.
>
> Options considered:
> 1. **← CHOSEN. Ship honest status, verify opportunistically.** Treat `pending_next_turn` → `applied_unverified` on a successful resume, and promote to `verified` only when a tool call whose name matches a granted server is seen in a later `session/update`. Cheap, truthful, no upstream dependency; a grant that silently fails shows as `applied_unverified` forever rather than as a false green.
> 2. *Rejected:* land the mechanism upstream first — a PR against `@agentclientprotocol/claude-agent-acp` to surface `mcp_servers` status. Correct, but blocks this PR on another repo's review.
> 3. *Rejected:* drop the requirement for v1. The spec explicitly warns reconfigure-on-resume is adapter behaviour, not an ACP contract — trusting the success response is exactly the failure mode §9 guards against.
>
> Option 2 remains the right long-term fix and should be noted in the PR body as a follow-up: `applied_unverified` is a workaround for a missing ACP surface, not a permanent design.

**Files:**
- Modify: `crates/buzz-acp/src/pool.rs` (record grant state; observe tool-call updates)
- Test: `crates/buzz-acp/src/pool.rs` (in-file tests)

**Interfaces:**
- Consumes: `SessionState.applied_mcp` (Task 3), the resume path (Task 4).
- Produces: `SessionState.mcp_verified: HashSet<Uuid>` and a `control_result` status of `applied_unverified` | `verified`.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn a_tool_call_from_a_granted_server_marks_the_grant_verified() {
        let cid = Uuid::new_v4();
        let mut state = SessionState::default();
        state.applied_mcp.insert(
            cid,
            vec![McpServer {
                name: "razorpay".into(),
                command: "/usr/bin/rzp".into(),
                args: vec![],
                env: vec![],
            }],
        );

        assert!(!state.mcp_verified.contains(&cid));

        // MCP tools surface as `mcp__<server>__<tool>`.
        note_tool_call_for_verification(&mut state, &cid, "mcp__razorpay__create_order");

        assert!(
            state.mcp_verified.contains(&cid),
            "a tool call from a granted server is proof the grant landed"
        );
    }

    #[test]
    fn an_unrelated_tool_call_does_not_verify_the_grant() {
        let cid = Uuid::new_v4();
        let mut state = SessionState::default();
        state.applied_mcp.insert(
            cid,
            vec![McpServer {
                name: "razorpay".into(),
                command: "/usr/bin/rzp".into(),
                args: vec![],
                env: vec![],
            }],
        );

        note_tool_call_for_verification(&mut state, &cid, "Read");

        assert!(
            !state.mcp_verified.contains(&cid),
            "a built-in tool call proves nothing about the MCP grant"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp verification 2>&1 | tail -20
```

Expected: FAIL — `cannot find function note_tool_call_for_verification`.

- [ ] **Step 3: Implement**

```rust
/// Promote a channel's MCP grant to *verified* when a tool call proves a
/// granted server is actually mounted.
///
/// The ACP wire carries no MCP status today (the Claude adapter consumes the
/// SDK's `system`/`init` message, which holds `mcp_servers[{name,status}]`,
/// without forwarding it). A tool call named `mcp__<server>__<tool>` is
/// therefore the only first-hand evidence available that the grant landed.
/// Absence of evidence is reported as `applied_unverified`, never as success.
fn note_tool_call_for_verification(state: &mut SessionState, channel_id: &Uuid, tool_name: &str) {
    let Some(servers) = state.applied_mcp.get(channel_id) else {
        return;
    };
    // Match the full `mcp__<server>__` prefix rather than splitting on `__`:
    // a server name may itself contain a double underscore, and splitting would
    // silently attribute its tool calls to the wrong server (or to none).
    let landed = servers
        .iter()
        .any(|s| tool_name.starts_with(&format!("mcp__{}__", s.name)));
    if landed {
        state.mcp_verified.insert(*channel_id);
    }
}
```

Add `pub mcp_verified: HashSet<Uuid>` to `SessionState`, clear it in `invalidate_channel` / `invalidate_all`, and call `note_tool_call_for_verification` from wherever `run_prompt_task` already observes `tool_call` session updates. Clear the channel's entry whenever `applied_mcp` changes (a new grant is unverified again).

- [ ] **Step 4: Run tests to verify they pass**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp verification 2>&1 | tail -20
```

Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-acp/src/pool.rs
git commit -s -m "feat(acp): opportunistic verification that an MCP grant landed"
```

---

### Task 8: Continuity proof + PR

**Files:**
- Create: `crates/buzz-acp/tests/mcp_resume_continuity.rs`
- Modify: `AGENTS.md` (document `BUZZ_ACP_MCP_SERVERS` beside the other harness env vars)

**Interfaces:**
- Consumes: everything above.

**Note on the spec's §8 integration test:** the spec asks for "20 turns with tool calls, grant a server, resume, assert recall of early-turn facts." That requires a real agent subprocess and a real model — not runnable in CI. This task splits it: a scripted-agent test that proves the *protocol* contract (which CI can enforce), plus a manual runbook for the *recall* claim (which only a live agent can demonstrate).

- [ ] **Step 1: Write the scripted continuity test**

```rust
//! Proves the protocol-level continuity contract: an MCP change is applied by
//! `session/resume` on the SAME session id, and no `session/new` or
//! `session/cancel` is issued. Recall of early-turn content is a model-level
//! property — see the manual runbook in the PR description.

// Script a fake agent that records every method it receives, run two turns
// with an MCP change between them, then assert on the recorded sequence:
//   1. initialize
//   2. session/new           → ses_fixed
//   3. session/prompt
//   4. session/resume        → sessionId == ses_fixed, mcpServers == [razorpay]
//   5. session/prompt
// and assert NO "session/cancel" and exactly ONE "session/new".
```

Implement it with the same `spawn_script` harness the `acp.rs` tests use, asserting:

```rust
assert_eq!(methods.iter().filter(|m| *m == "session/new").count(), 1,
    "an MCP change must never mint a new session — that is the whole point");
assert!(!methods.iter().any(|m| m == "session/cancel"),
    "an MCP change must never cancel an in-flight turn");
assert_eq!(resume_params["sessionId"], "ses_fixed");
```

- [ ] **Step 2: Run it**

```bash
. ./bin/activate-hermit && cargo test -p buzz-acp --test mcp_resume_continuity 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 3: Full local gate**

```bash
. ./bin/activate-hermit && just ci 2>&1 | tail -30
```

Expected: green. Fix fmt/clippy before proceeding — clippy passing does not mean fmt passes.

- [ ] **Step 4: Unshallow before the PR**

```bash
cd ~/Dev/buzz && git fetch --unshallow upstream 2>/dev/null || git fetch upstream
```

(This clone is `--depth 50`, so the merge-base against `block/buzz` is incomplete without this.)

- [ ] **Step 5: Manual live verification (record the result in the PR)**

1. Start the harness with `BUZZ_ACP_MCP_SERVERS='[]'` on a test channel.
2. Have a 10+ turn conversation including a file read, and state a distinctive fact early ("the passphrase is *hollyhock*").
3. Send an `update_mcp_servers` control frame granting a server.
4. Next turn: confirm the new tool is callable **and** ask the agent to repeat the passphrase.
5. Record: session id unchanged, tool available, passphrase recalled.

- [ ] **Step 6: Commit and open the PR**

```bash
. ./bin/activate-hermit
git add crates/buzz-acp/tests/mcp_resume_continuity.rs AGENTS.md
git commit -s -m "test(acp): prove MCP changes resume in place without a new session"
git push origin feat/live-mcp-control
```

PR body must state: the mechanism is **adapter behaviour, not an ACP contract** (§9 abandon triggers), that `strictMcpConfig` is opt-in per managed channel, and the verification limitation from Task 7.

---

## Verification log

Every citation in the spec was re-checked against shipped source before this plan was written. All eight held. Three corrections are folded into the tasks above:

| Spec claim | Verdict |
|---|---|
| `session/resume` carries `mcpServers` | ✅ `zResumeSessionRequest` — codex `dist/index.js:19573-19579` |
| Claude adapter reconfigures MCP on resume | ✅ `acp-agent.js:132-139` (fingerprint), `:3981-4008` (teardown + `createSession({resume})`); comment names the case |
| Resume restores transcript, same sessionId | ✅ `acp-agent.js:4279-4282` — `options.sessionId` is set only when *not* resuming |
| Codex applies new MCP on resume | ✅ `dist/index.js:26252-26259` → `threadResume({config: createSessionConfig(cwd, dirs, request.mcpServers)})`; no change-detection, so suppress no-op grants |
| Both adapters advertise resume | ✅ `acp-agent.js:644`, codex `dist/index.js:28490-28491` — both `sessionCapabilities.resume: {}` |
| `strictMcpConfig` suppresses global MCP | ✅ `sdk.d.ts:1959` — **correction:** the adapter never names it; it rides the generic `...userProvidedOptions` spread (`acp-agent.js:4103`, `:4157`), so a typo silently no-ops |
| Buzz sends `mcpServers` at `session/new` | ✅ `acp.rs:629-655`; default is exactly one server (dev MCP) or none — `lib.rs:4179` |
| Control-frame path is owner-signed + fresh | ✅ `lib.rs:837-1005` |

**Correction 2 — `McpServer` lives in `acp.rs:28`, not `config.rs`,** and is `Serialize`-only. Task 2 adds `Deserialize`/`PartialEq`; the spec's §5.2.2 "extend to an enum covering stdio + http/sse" is **deferred** — the Rust type only models `McpServerStdio` today, and remote (http/sse) servers are a second, separable change. Flag in the PR as a known limitation.

**Correction 3 — no MCP status on the ACP wire.** See the Task 7 decision block.

## Self-review

- **Spec coverage.** §5.2.1 → Task 1. §5.2.2 → Task 2 (http/sse variant explicitly deferred). §5.2.3 → Tasks 3–4. §5.2.4 → Task 5. §5.2.5 → Task 6. §5.4 → Task 7 (blocked on a decision). §8 → tests in every task + Task 8. §5.3 (desktop) → **out of scope, separate plan.** §7 (security) → Task 5 note. §9/§10 → PR description.
- **Type consistency.** `session_resume(session_id, cwd, mcp_servers)`, `resume_supported()`, `set_desired_mcp` / `desired_mcp_for`, `effective_mcp_servers`, `applied_mcp`, `desired_mcp`, `mcp_verified`, `note_tool_call_for_verification` — each defined once and used with the same name and signature throughout.
- **Known rough edge.** Task 4 Step 4 duplicates the session-creation block. The step says to extract a helper *after* tests are green; do not pre-factor it.
