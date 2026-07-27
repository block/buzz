# Pi (pi.dev) First-Class Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make pi (pi.dev, `@earendil-works/pi-coding-agent`) a first-class agent runtime in Buzz's desktop runtime catalog, bridged over ACP via the community `@victor-software-house/pi-acp` adapter.

**Architecture:** Pi becomes the fifth entry in the static `KNOWN_ACP_RUNTIMES` registry, following the Codex precedent (external CLI + npm-installed Node ACP adapter). `buzz-acp` needs zero changes. The one novel piece: since the adapter doesn't forward `mcpServers`, a config bridge writes `.pi/mcp.json` into the shared Buzz nest workdir (`~/.buzz`) at spawn time so pi's MCP extension loads `buzz-dev-mcp`.

**Tech Stack:** Rust (Tauri desktop backend), React/TypeScript (desktop frontend), serde_json.

**Spec:** `docs/superpowers/specs/2026-07-27-pi-harness-design.md`

## Global Constraints

- No `unsafe`; no new `unwrap()`/`expect()` in production paths — use `?` and error types (test code may unwrap).
- New public API needs doc comments.
- Desktop crate is EXCLUDED from the root workspace: run tests with `cargo test --manifest-path desktop/src-tauri/Cargo.toml`.
- Pre-commit gotcha in worktrees: `just desktop-tauri-fmt` fails in git worktrees. If commit is blocked, run `just desktop-tauri-fmt` from the main checkout, re-stage, and commit. Do not rewrite hook commands.
- Activate hermit before git/cargo: `. ./bin/activate-hermit`.
- Adapter version is pinned: `@victor-software-house/pi-acp@0.17.1` (latest as of 2026-07-27).
- Buzz never writes to the user's `~/.pi/agent/` directory. The only file Buzz writes is `<nest>/.pi/mcp.json` (nest = `~/.buzz`, Buzz-owned).
- Model/provider are pi-owned: the catalog entry must NOT set `model_env_var`, `provider_env_var`, or required normalized fields.

## File Map

| File | Change |
|---|---|
| `desktop/src-tauri/src/managed_agents/discovery.rs` | Add `pi` entry to `KNOWN_ACP_RUNTIMES` |
| `desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs` | Extend vendor-metadata test |
| `desktop/src-tauri/src/managed_agents/readiness.rs` | Explicit `"pi"` readiness arm + tests |
| `desktop/src-tauri/src/managed_agents/config_bridge/pi.rs` | **New**: settings.json reader + nest `mcp.json` writer |
| `desktop/src-tauri/src/managed_agents/config_bridge/mod.rs` | Register `mod pi;` |
| `desktop/src-tauri/src/managed_agents/config_bridge/reader.rs` | Dispatch `"pi"` in `read_config_surface` + `mcp_config_file_path_for_runtime` |
| `desktop/src-tauri/src/managed_agents/runtime.rs` | Spawn-time call to nest mcp.json writer (gated on pi) |
| `desktop/src-tauri/src/managed_agents/runtime/process.rs` | Add `pi-acp`/`pi_acp` to `KNOWN_AGENT_BINARIES` |
| `desktop/src/features/onboarding/ui/RuntimeIcon.tsx` | Icon map entry |
| `desktop/src/features/settings/ui/DoctorSettingsPanel.tsx` | Icon map entry |
| `desktop/public/runtime-icons/pi.png`, `desktop/src/features/onboarding/assets/harness-logos/pi.png` | **New** icon assets |
| `README.md`, `ARCHITECTURE.md` | Harness list mentions |

---

### Task 1: Runtime catalog entry

**Files:**
- Modify: `desktop/src-tauri/src/managed_agents/discovery.rs` (constants near line 16-20; `KNOWN_ACP_RUNTIMES` near line 65-197)
- Test: `desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs` (tests module, line 84+)

**Interfaces:**
- Produces: `known_acp_runtime_exact("pi")` and `known_acp_runtime("pi-acp")` return a `KnownAcpRuntime` with `id == "pi"`. Later tasks (readiness, config bridge, runtime spawn) key off `id == "pi"` and `skill_dir == Some(".pi/skills")`.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs`:

```rust
#[test]
fn pi_metadata_pins_adapter_and_defers_model_to_pi() {
    let pi = known_acp_runtime_exact("pi").unwrap();
    assert_eq!(pi.label, "Pi");
    assert_eq!(pi.commands, &["pi-acp"]);
    assert_eq!(pi.underlying_cli, Some("pi"));
    // Adapter is version-pinned (third-party MVP; bump deliberately).
    assert!(pi
        .adapter_install_commands
        .iter()
        .any(|c| c.contains("@victor-software-house/pi-acp@0.17.1")));
    // MCP extension install so pi loads the nest .pi/mcp.json.
    assert!(pi
        .adapter_install_commands
        .iter()
        .any(|c| c.contains("pi-mcp-extension")));
    assert_eq!(pi.cli_install_instructions_url, "https://pi.dev");
    // Model/provider are pi-owned — Buzz must not inject them.
    assert!(pi.model_env_var.is_none());
    assert!(pi.provider_env_var.is_none());
    assert!(!pi.provider_locked);
    assert!(pi.required_normalized_fields.is_empty());
    assert_eq!(pi.mcp_command, Some("buzz-dev-mcp"));
    assert_eq!(pi.skill_dir, Some(".pi/skills"));
    assert_eq!(pi.config_file_path, Some("~/.pi/agent/settings.json"));
    assert!(pi.auth_probe_args.is_none());
    assert!(pi.login_hint.is_some());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml pi_metadata_pins_adapter -- --nocapture
```

Expected: FAIL — `unwrap()` on `None` (no `pi` runtime yet).

- [ ] **Step 3: Add the catalog entry**

In `desktop/src-tauri/src/managed_agents/discovery.rs`, add near the other avatar constants (line ~16-20):

```rust
const PI_AVATAR_URL: &str = "https://avatars.githubusercontent.com/earendil-works";
```

Append this entry to `KNOWN_ACP_RUNTIMES` after the `codex` entry (keep `buzz-agent` last if ordering elsewhere depends on it — it doesn't, but Doctor sorts by explicit priority map anyway; place `pi` between `codex` and `buzz-agent`):

```rust
    KnownAcpRuntime {
        id: "pi",
        label: "Pi",
        commands: &["pi-acp"],
        aliases: &["pi.dev", "pi-dev"],
        avatar_url: PI_AVATAR_URL,
        // Sent in session/new; a no-op with today's adapter (known upstream
        // gap — mcpServers accepted but not wired into pi). Harmless and
        // future-proof. The working path is the nest .pi/mcp.json bridge.
        mcp_command: Some("buzz-dev-mcp"),
        mcp_hooks: false,
        underlying_cli: Some("pi"),
        cli_install_commands: &["npm install -g @earendil-works/pi-coding-agent"],
        cli_install_commands_windows: &[],
        // Adapter pinned: third-party MVP that self-describes minor breaking
        // changes — bump the pin deliberately after testing.
        adapter_install_commands: &[
            "npm install -g @victor-software-house/pi-acp@0.17.1",
            "pi install npm:pi-mcp-extension",
        ],
        cli_install_instructions_url: "https://pi.dev",
        adapter_install_instructions_url: "https://github.com/victor-software-house/pi-acp",
        cli_install_hint: "Buzz requires the pi CLI (Node.js 24+); install via npm.",
        adapter_install_hint: "Install the pi ACP adapter via npm (Node.js 24+).",
        skill_dir: Some(".pi/skills"),
        supports_acp_model_switching: false,
        // Model/provider are pi-owned (Claude Code pattern): pi's own config
        // and /login flow decide the model; Buzz does not inject either.
        model_env_var: None,
        provider_env_var: None,
        provider_locked: false,
        default_env: &[],
        config_file_path: Some("~/.pi/agent/settings.json"),
        config_file_format: Some("json"),
        supports_acp_native_config: false,
        thinking_env_var: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        required_normalized_fields: &[],
        login_hint: Some(
            "Run `pi` and use /login, or set a provider API key (e.g. ANTHROPIC_API_KEY).",
        ),
        // Pi has no `auth status` CLI subcommand — no probe (same as Goose).
        // Auth failures surface as runtime errors in agent output.
        auth_probe_args: None,
    },
```

- [ ] **Step 4: Run tests to verify pass**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml pi_metadata_pins_adapter -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml discovery
```

Expected: PASS (all discovery tests — the new entry must not break alias/normalization tests).

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/managed_agents/discovery.rs desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs
git commit -m "feat(desktop): add pi (pi.dev) to the ACP runtime catalog"
```

---

### Task 2: Readiness policy

**Files:**
- Modify: `desktop/src-tauri/src/managed_agents/readiness.rs` (`collect_missing_requirements`, line ~268-293; doc comment on `agent_readiness`, line ~234-254; tests module, line ~489+)

**Interfaces:**
- Consumes: catalog entry from Task 1 (`known_acp_runtime("pi-acp")` resolves).
- Produces: `agent_readiness` returns `Ready` for pi with an empty env.

- [ ] **Step 1: Write the failing-by-omission test**

Append to the `tests` module in `readiness.rs` (uses the existing `make_env`/`env_with` helpers at line ~496-510):

```rust
// ── pi tests ──────────────────────────────────────────────────────────

#[test]
fn pi_is_ready_with_no_buzz_side_config() {
    // Pi owns model/provider/auth via its own config and /login flow —
    // Buzz has no requirements to enforce (deliberate policy, not fallthrough).
    let env = make_env("pi-acp", env_with(&[]));
    assert!(agent_readiness(&env).is_ready());
}
```

- [ ] **Step 2: Run the test**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml pi_is_ready_with_no_buzz_side -- --nocapture
```

Expected: PASS already (unknown-runtime fallthrough) — that's precisely why the explicit arm matters: it turns accident into policy. Verify it passes, then make the policy explicit.

- [ ] **Step 3: Add the explicit arm**

In `collect_missing_requirements` (line ~277-292), add before the `_ => vec![]` fallthrough:

```rust
        // Pi owns model, provider, and auth via its own config (~/.pi/agent)
        // and /login flow — deliberately no Buzz-side requirements. Auth
        // failures surface as runtime errors in agent output; Doctor shows
        // the catalog login_hint.
        "pi" => vec![],
```

Also extend the `agent_readiness` doc comment's runtime list (line ~239-249) with:

```rust
/// * **pi**: no Buzz-side requirements — pi's own config owns model,
///   provider, and auth.
```

- [ ] **Step 4: Run the readiness suite**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml readiness
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/managed_agents/readiness.rs
git commit -m "feat(desktop): explicit no-requirements readiness policy for pi"
```

---

### Task 3: Config bridge — read side

**Files:**
- Create: `desktop/src-tauri/src/managed_agents/config_bridge/pi.rs`
- Modify: `desktop/src-tauri/src/managed_agents/config_bridge/mod.rs` (add `mod pi;`)
- Modify: `desktop/src-tauri/src/managed_agents/config_bridge/reader.rs` (dispatch at line ~21-27; `mcp_config_file_path_for_runtime` at line ~216-227)

**Interfaces:**
- Consumes: `RuntimeFileConfig` (has `#[derive(Default)]`), `schema_walker::extract_config_fields(&serde_json::Value, skip: &[&str]) -> BTreeMap<String, String>` — both already exist in `config_bridge`.
- Produces: `pi::read_config_file() -> Option<RuntimeFileConfig>` and `pi::pi_agent_dir() -> Option<PathBuf>`. Task 4 adds the writer to this same file.

Pi's `settings.json` holds harness behavior settings (steeringMode, transport, defaultProjectTrust, …), not model/provider — those live in pi's credential store and models.json. So the reader is deliberately minimal: parse JSON, surface everything via `extra` (config-driven), leave all normalized fields `None`. This matches the "pi config owns it" decision — Buzz surfaces, never interprets.

- [ ] **Step 1: Write the failing tests**

Create `desktop/src-tauri/src/managed_agents/config_bridge/pi.rs` with tests first (module skeleton + tests, no impl yet — or write it all at once and rely on test-first for the parse function; keep the cycle honest by writing tests before the parse body):

```rust
use super::types::RuntimeFileConfig;
use std::path::PathBuf;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_settings_surfaces_fields_as_extra() {
        let json = r#"{
            "steeringMode": "one-at-a-time",
            "transport": "auto",
            "defaultProjectTrust": "ask"
        }"#;
        let cfg = parse_pi_settings(json).unwrap();
        assert_eq!(
            cfg.extra.get("steeringMode").map(String::as_str),
            Some("one-at-a-time")
        );
        assert_eq!(cfg.extra.get("transport").map(String::as_str), Some("auto"));
        // Pi settings.json carries no model/provider — normalized fields stay None.
        assert!(cfg.model.is_none());
        assert!(cfg.provider.is_none());
        assert!(cfg.system_prompt.is_none());
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(parse_pi_settings("{{{{not json").is_none());
    }

    #[test]
    fn pi_agent_dir_honors_env_override() {
        // PI_CODING_AGENT_DIR overrides ~/.pi/agent (pi's own convention).
        // Serialize env mutation isn't needed: this test sets and removes
        // within one test; the suite has no other reader of this var.
        std::env::set_var("PI_CODING_AGENT_DIR", "/tmp/pi-test-agent-dir");
        let dir = pi_agent_dir();
        std::env::remove_var("PI_CODING_AGENT_DIR");
        assert_eq!(dir, Some(PathBuf::from("/tmp/pi-test-agent-dir")));
    }
}
```

- [ ] **Step 2: Run to verify compile failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml config_bridge::pi
```

Expected: FAIL — `parse_pi_settings` / `pi_agent_dir` not defined. (First add `mod pi;` to `config_bridge/mod.rs` alphabetically — after `mod goose;` — or the module won't compile at all.)

- [ ] **Step 3: Implement the read side**

Add above the tests in `pi.rs`:

```rust
/// Read pi's harness settings from `~/.pi/agent/settings.json`
/// (or `$PI_CODING_AGENT_DIR/settings.json`).
///
/// Pi's settings.json holds harness behavior (steering, transport, trust) —
/// not model/provider, which live in pi's credential store and models.json.
/// Everything is surfaced read-only via `extra`; normalized fields stay None.
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let path = pi_settings_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    parse_pi_settings(&raw)
}

fn parse_pi_settings(json_str: &str) -> Option<RuntimeFileConfig> {
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let extra = super::schema_walker::extract_config_fields(&value, &[]);
    Some(RuntimeFileConfig {
        extra,
        ..Default::default()
    })
}

/// Pi's config directory: `$PI_CODING_AGENT_DIR` if set, else `~/.pi/agent`.
pub(crate) fn pi_agent_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::home_dir().map(|home| home.join(".pi").join("agent"))
}

fn pi_settings_path() -> Option<PathBuf> {
    pi_agent_dir().map(|dir| dir.join("settings.json"))
}
```

Check `schema_walker::extract_config_fields`'s exact signature before use (`desktop/src-tauri/src/managed_agents/config_bridge/schema_walker.rs`) — the codex reader at `codex.rs:45` is the reference call site; mirror it exactly.

- [ ] **Step 4: Wire the dispatch**

In `reader.rs` `read_config_surface` (line ~21-27), add to the match:

```rust
            "pi" => super::pi::read_config_file().map(|c| (c, true)),
```

In `mcp_config_file_path_for_runtime` (line ~216-227), add:

```rust
        // Pi's MCP servers come from the Buzz-owned nest project file
        // (written at spawn time), not from ~/.pi/agent.
        "pi" => crate::managed_agents::default_agent_workdir()
            .map(|dir| dir.join(".pi/mcp.json").to_string_lossy().into_owned()),
```

- [ ] **Step 5: Run tests and commit**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml config_bridge
git add desktop/src-tauri/src/managed_agents/config_bridge/
git commit -m "feat(desktop): pi config-bridge reader for ~/.pi/agent/settings.json"
```

Expected: PASS.

---

### Task 4: Config bridge — nest `.pi/mcp.json` writer + spawn hook

**Files:**
- Modify: `desktop/src-tauri/src/managed_agents/config_bridge/pi.rs` (add writer + tests)
- Modify: `desktop/src-tauri/src/managed_agents/runtime.rs` (spawn path, near the `runtime_meta` lookup at line ~549-553)

**Interfaces:**
- Consumes: `known_acp_runtime(&effective_command)` (already called in the spawn path — variable `runtime_meta`), `super::default_agent_workdir()`.
- Produces: `pi::ensure_workdir_mcp_json(workdir: &Path) -> Result<(), String>` — public within `managed_agents` (needs `pub(crate)` and a re-export or direct path `config_bridge::pi::ensure_workdir_mcp_json`; check `config_bridge/mod.rs` visibility — `mod pi;` is private, so either make it `pub(crate) mod pi;` or add a re-export fn in `mod.rs` like the existing `read_goose_file_config` precedent. Use the re-export precedent: add `pub(crate) fn ensure_pi_workdir_mcp_json(workdir: &std::path::Path) -> Result<(), String> { pi::ensure_workdir_mcp_json(workdir) }` to `mod.rs`.)

Behavior contract:
- Creates `<workdir>/.pi/mcp.json` with `{"mcpServers": {"buzz": {"command": "buzz-dev-mcp"}}}` if absent.
- Merge-preserving: an existing file with foreign servers keeps them; only the `buzz` key is inserted/updated.
- Malformed existing JSON → `Err` (never clobber a file we can't parse).
- Idempotent: second call with correct content is a no-op (no rewrite).
- No secrets in the file: `buzz-dev-mcp` reads `BUZZ_RELAY_URL`/`BUZZ_PRIVATE_KEY`/`BUZZ_AUTH_TAG` from process env, and is resolved via the augmented child `PATH`.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `pi.rs`:

```rust
    #[test]
    fn ensure_mcp_json_creates_file_with_buzz_server() {
        let dir = tempfile::tempdir().unwrap();
        ensure_workdir_mcp_json(dir.path()).unwrap();
        let raw = std::fs::read_to_string(dir.path().join(".pi/mcp.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            json["mcpServers"]["buzz"]["command"],
            serde_json::json!("buzz-dev-mcp")
        );
    }

    #[test]
    fn ensure_mcp_json_preserves_foreign_servers() {
        let dir = tempfile::tempdir().unwrap();
        let pi_dir = dir.path().join(".pi");
        std::fs::create_dir_all(&pi_dir).unwrap();
        std::fs::write(
            pi_dir.join("mcp.json"),
            r#"{"mcpServers": {"github": {"command": "gh-mcp"}}, "otherKey": 1}"#,
        )
        .unwrap();
        ensure_workdir_mcp_json(dir.path()).unwrap();
        let raw = std::fs::read_to_string(pi_dir.join("mcp.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            json["mcpServers"]["github"]["command"],
            serde_json::json!("gh-mcp"),
            "foreign server must survive the merge"
        );
        assert_eq!(json["otherKey"], serde_json::json!(1));
        assert_eq!(
            json["mcpServers"]["buzz"]["command"],
            serde_json::json!("buzz-dev-mcp")
        );
    }

    #[test]
    fn ensure_mcp_json_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        ensure_workdir_mcp_json(dir.path()).unwrap();
        let path = dir.path().join(".pi/mcp.json");
        let first_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        ensure_workdir_mcp_json(dir.path()).unwrap();
        let second_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(first_mtime, second_mtime, "no rewrite when content is correct");
    }

    #[test]
    fn ensure_mcp_json_refuses_to_clobber_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let pi_dir = dir.path().join(".pi");
        std::fs::create_dir_all(&pi_dir).unwrap();
        std::fs::write(pi_dir.join("mcp.json"), "{{{{not json").unwrap();
        assert!(ensure_workdir_mcp_json(dir.path()).is_err());
        // Original content untouched.
        assert_eq!(
            std::fs::read_to_string(pi_dir.join("mcp.json")).unwrap(),
            "{{{{not json"
        );
    }
```

(`tempfile` is already a dependency — `nest.rs` uses `tempfile::NamedTempFile`.)

- [ ] **Step 2: Run to verify failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml config_bridge::pi
```

Expected: FAIL — `ensure_workdir_mcp_json` not defined.

- [ ] **Step 3: Implement the writer**

Add to `pi.rs`:

```rust
/// Ensure `<workdir>/.pi/mcp.json` registers `buzz-dev-mcp` for pi's MCP
/// extension. Merge-preserving (foreign servers and keys survive), idempotent
/// (no rewrite when already correct), and refuses to clobber malformed JSON.
///
/// The file contains no secrets: `buzz-dev-mcp` reads its relay URL and key
/// from the process environment injected by `buzz-acp`, and is resolved via
/// the augmented child PATH.
pub(super) fn ensure_workdir_mcp_json(workdir: &std::path::Path) -> Result<(), String> {
    let pi_dir = workdir.join(".pi");
    std::fs::create_dir_all(&pi_dir).map_err(|e| format!("create {}: {e}", pi_dir.display()))?;
    let path = pi_dir.join("mcp.json");

    let mut root: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| format!("existing {} is not valid JSON ({e}); refusing to overwrite", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(err) => return Err(format!("read {}: {err}", path.display())),
    };

    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| format!("{} root is not a JSON object", path.display()))?;
    let servers = root_obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let servers_obj = servers
        .as_object_mut()
        .ok_or_else(|| format!("{} mcpServers is not a JSON object", path.display()))?;

    let desired = serde_json::json!({ "command": "buzz-dev-mcp" });
    if servers_obj.get("buzz") == Some(&desired) {
        return Ok(()); // already correct — no rewrite
    }
    servers_obj.insert("buzz".to_string(), desired);

    let serialized = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("serialize {}: {e}", path.display()))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))
}
```

And the re-export in `config_bridge/mod.rs` (following the `read_goose_file_config` precedent at line 16-18):

```rust
/// Ensure the Buzz nest workdir has a `.pi/mcp.json` registering
/// `buzz-dev-mcp` for pi's MCP extension. Called from the spawn path when
/// launching a pi agent. See `pi::ensure_workdir_mcp_json`.
pub(crate) fn ensure_pi_workdir_mcp_json(workdir: &std::path::Path) -> Result<(), String> {
    pi::ensure_workdir_mcp_json(workdir)
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml config_bridge::pi
```

Expected: PASS (all pi bridge tests).

- [ ] **Step 5: Hook into the spawn path**

In `runtime.rs`, the spawn function already computes `let runtime_meta = known_acp_runtime(&effective_command);` (line ~551). Immediately after that lookup (before the process is spawned), add:

```rust
    // Pi loads MCP servers from a project-level .pi/mcp.json (via its MCP
    // extension) — the ACP session/new mcpServers field is not yet wired
    // through the adapter. Write the nest file so buzz-dev-mcp tools are
    // available. Non-fatal: pi degrades to its native tools on failure.
    if runtime_meta.is_some_and(|r| r.id == "pi") {
        if let Some(workdir) = super::default_agent_workdir() {
            if let Err(error) = super::config_bridge::ensure_pi_workdir_mcp_json(&workdir) {
                log::warn!("failed to write pi mcp.json in nest: {error}");
            }
        }
    }
```

Match the file's existing logging idiom: check how `runtime.rs` reports non-fatal errors (search for `warn!`/`log::` at the top of the file); if it uses a different macro or `eprintln!`, use that instead of `log::warn!`. Also verify `config_bridge` is importable from `runtime.rs` (both are `managed_agents` submodules — `super::config_bridge::...` should resolve; adjust the path if the compiler disagrees).

- [ ] **Step 6: Build + commit**

```bash
cargo build --manifest-path desktop/src-tauri/Cargo.toml
git add desktop/src-tauri/src/managed_agents/config_bridge/ desktop/src-tauri/src/managed_agents/runtime.rs
git commit -m "feat(desktop): write nest .pi/mcp.json at pi spawn so buzz-dev-mcp loads"
```

Expected: clean build.

---

### Task 5: Process cleanup recognition

**Files:**
- Modify: `desktop/src-tauri/src/managed_agents/runtime/process.rs` (`KNOWN_AGENT_BINARIES`, line 8-25)

**Interfaces:**
- Consumes: nothing new. Produces: orphan sweep recognizes `pi-acp` processes.

Deliberate deviation from spec §4 (which said add `pi-acp` **and** `pi`): add only `pi-acp`/`pi_acp`. Rationale: pi and pi-acp are Node-hosted (their processes appear as `node` and are claimed via the `KNOWN_SCRIPT_INTERPRETERS` + `BUZZ_MANAGED_AGENT` marker path, same as codex-acp's npm shim). Listing bare `pi` in `KNOWN_AGENT_BINARIES` would prefix-match unrelated processes like `pi-hole` (the matcher accepts `pi-*`), risking sweeping processes Buzz doesn't own. `pi-acp` is listed for the case where it's installed as a standalone binary (matching the codex-acp precedent at line 17-18).

- [ ] **Step 1: Write the failing test**

Find the existing tests for `name_matches_known_binary` (grep `name_matches_known_binary` in `process.rs` / sibling test files) and add alongside them (or create a `tests` module in `process.rs` if none exists):

```rust
    #[test]
    fn pi_acp_is_a_known_binary_but_bare_pi_is_not() {
        assert!(name_matches_known_binary("pi-acp"));
        assert!(name_matches_known_binary("pi_acp"));
        // Bare "pi" is Node-hosted (covered by the interpreter+marker path);
        // listing it would prefix-match unrelated processes like "pi-hole".
        assert!(!name_matches_known_binary("pi"));
        assert!(!name_matches_known_binary("pi-hole"));
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml pi_acp_is_a_known_binary
```

Expected: FAIL on the first assertion.

- [ ] **Step 3: Add the entries**

In `KNOWN_AGENT_BINARIES` (after the `codex_acp` entries, line ~18):

```rust
    "pi-acp",
    "pi_acp",
```

- [ ] **Step 4: Run tests, commit**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml process
git add desktop/src-tauri/src/managed_agents/runtime/process.rs
git commit -m "feat(desktop): recognize pi-acp in orphan-sweep known binaries"
```

Expected: PASS.

---

### Task 6: Frontend icons + assets

**Files:**
- Create: `desktop/public/runtime-icons/pi.png`
- Create: `desktop/src/features/onboarding/assets/harness-logos/pi.png`
- Modify: `desktop/src/features/onboarding/ui/RuntimeIcon.tsx` (imports line 8-10, `RUNTIME_LOGOS` line 12-16)
- Modify: `desktop/src/features/settings/ui/DoctorSettingsPanel.tsx` (`RUNTIME_LOGO_URLS` line ~38-43, `RUNTIME_LOGO_SCALE` line ~45-50)

**Interfaces:** none — pure presentation. Everything else (definition dialog picker, onboarding list, install offers) is data-driven from the Rust catalog via `AcpRuntimeCatalogEntry`; per `desktop/src/features/agents/AGENTS.md`, do NOT create a parallel TS registry.

- [ ] **Step 1: Fetch the icon asset**

pi.dev serves no favicon/logo at conventional paths (verified 404); use the earendil-works GitHub org avatar (verified 200):

```bash
curl -fsSL -o desktop/public/runtime-icons/pi.png "https://avatars.githubusercontent.com/earendil-works?size=128"
cp desktop/public/runtime-icons/pi.png desktop/src/features/onboarding/assets/harness-logos/pi.png
file desktop/public/runtime-icons/pi.png
```

Expected: `file` reports PNG image data. If the avatar downloads as JPEG (GitHub sometimes serves JPEG), convert: `sips -s format png <in> --out <out>` on macOS — the `?inline` Vite import and `<img>` both tolerate either, but keep the `.png` name consistent with the map entries.

- [ ] **Step 2: Wire RuntimeIcon.tsx**

```tsx
import piLogoUrl from "../assets/harness-logos/pi.png?inline";
```

and in `RUNTIME_LOGOS`:

```tsx
  pi: piLogoUrl,
```

- [ ] **Step 3: Wire DoctorSettingsPanel.tsx**

```tsx
const RUNTIME_LOGO_URLS: Record<string, string> = {
  "buzz-agent": "/app-icon@2x.png",
  claude: "/runtime-icons/claude.png",
  codex: "/runtime-icons/codex.png",
  goose: "/runtime-icons/goose.svg",
  pi: "/runtime-icons/pi.png",
};
```

and in `RUNTIME_LOGO_SCALE` add `pi: "scale-110",`.

- [ ] **Step 4: Lint + typecheck**

```bash
cd desktop && pnpm exec biome check src/features/onboarding/ui/RuntimeIcon.tsx src/features/settings/ui/DoctorSettingsPanel.tsx && pnpm exec tsc --noEmit -p . ; cd ..
```

Expected: clean. (If the project has a `just` recipe for desktop lint, prefer it — check `just --list | grep desktop`.)

- [ ] **Step 5: Commit**

```bash
git add desktop/public/runtime-icons/pi.png desktop/src/features/onboarding/assets/harness-logos/pi.png desktop/src/features/onboarding/ui/RuntimeIcon.tsx desktop/src/features/settings/ui/DoctorSettingsPanel.tsx
git commit -m "feat(desktop): pi runtime icons in onboarding and Doctor"
```

---

### Task 7: Docs + full CI gate

**Files:**
- Modify: `README.md` (lines 103, 179, 209 — harness enumerations)
- Modify: `ARCHITECTURE.md` (line 651 — harness diagram label)

**Interfaces:** none.

- [ ] **Step 1: Update harness mentions**

- `README.md:103`: `ACP harness (Goose, Codex, Claude Code)` → `ACP harness (Goose, Codex, Claude Code, Pi)`
- `README.md:179`: `(Goose, Codex, ...)` — already elided, leave as-is.
- `README.md:209`: `buzz-acp (ACP harness for Goose/Codex/Claude Code)` → `buzz-acp (ACP harness for Goose/Codex/Claude Code/Pi)`
- `ARCHITECTURE.md:651`: `Agent (goose/codex/claude)` → `Agent (goose/codex/claude/pi)`

Line numbers may have drifted — locate by grepping for the quoted text, not by line.

- [ ] **Step 2: Run the full local gate**

```bash
. ./bin/activate-hermit
just ci
```

Expected: PASS (fmt + clippy + desktop lint + unit tests + builds). Fix any clippy/fmt fallout from Tasks 1-6 before committing.

- [ ] **Step 3: Commit**

```bash
git add README.md ARCHITECTURE.md
git commit -m "docs: list pi among supported agent harnesses"
```

---

### Task 8: Manual E2E verification (human-in-the-loop)

No code. Verify on a real machine before PR:

- [ ] Install via the guided flow (or manually): `npm i -g @earendil-works/pi-coding-agent @victor-software-house/pi-acp@0.17.1`, then `pi install npm:pi-mcp-extension`. Requires Node 24+ — if Buzz's managed node is older, note it in the PR (spec flags this as an implementation-time checkpoint; the install hints already mention Node 24+).
- [ ] Authenticate pi (`pi` → `/login`, or export `ANTHROPIC_API_KEY`).
- [ ] Desktop: create an agent definition with runtime **Pi**; confirm discovery shows it available and readiness is Ready.
- [ ] Spawn; confirm `~/.buzz/.pi/mcp.json` exists with the `buzz` server entry.
- [ ] Mention the agent in a channel; confirm a response (system prompt arrives via buzz-acp's first-user-message framing — verify persona traits show through).
- [ ] Ask the agent to use a buzz-dev-mcp tool (e.g. post a message via the buzz CLI/MCP); confirm tool availability. If the MCP extension didn't load the nest file (extension config-path mismatch), file the finding — pi still works with native tools (accepted degradation), but record the actual config path the extension reads and adjust `ensure_workdir_mcp_json`'s target path in a follow-up.
- [ ] Stop the agent; confirm no orphaned `node`/`pi` processes remain (`ps aux | grep -i pi-acp`).
- [ ] Config panel: confirm pi's model/thinking appear post-spawn via ACP configOptions, and `~/.pi/agent/settings.json` fields surface read-only.

---

## Self-review notes

- **Spec coverage:** catalog entry (T1), readiness (T2), config bridge read (T3), MCP write + spawn hook (T4), process cleanup (T5), UI/icons (T6), docs (T7), manual E2E incl. Node-24 checkpoint (T8). Spec's "per-agent nest" corrected to the shared Buzz nest (`~/.buzz`) — the workdir all agents share; the invariant that matters (never touching `~/.pi/agent/`) holds. Spec §4's "add `pi` to known binaries" deliberately narrowed in T5 with rationale.
- **Type consistency:** `ensure_workdir_mcp_json(&Path) -> Result<(), String>` defined in T4 matches the T4 spawn-hook call via the `mod.rs` re-export `ensure_pi_workdir_mcp_json`; `pi_agent_dir()` defined and used only in T3.
- **Placeholders:** none — every code step carries full code; the two "verify before use" instructions (schema_walker signature, logging macro) name the exact reference file and a concrete default.
