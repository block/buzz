# Per-owner Server Agent Credentials Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each user provide their own Claude OAuth token (or API key) from the desktop Server Agents settings, delivered encrypted to the spawner, which provisions it into that owner's agent containers and refuses to run agents whose owner has no token.

**Architecture:** Two new NIP-44-encrypted frame variants on the existing kind:24201 attestation channel (`CredentialUpdate` owner→spawner, `CredentialAck` spawner→owner). The spawner stores tokens in a new `credentials.json` (0600) keyed by owner pubkey, injects the prefix-classified env var (`sk-ant-oat*` → `CLAUDE_CODE_OAUTH_TOKEN`, else `ANTHROPIC_API_KEY`) at container start, and restarts an owner's agents when their token changes. **No fallback:** an owner without a token gets their agents held stopped with a new `needs_credential: true` field on kind:30179 status. Desktop adds a write-only credential card per connected spawner and a "Needs credential" badge.

**Tech Stack:** Rust (buzz-sdk, buzz-spawner, Tauri commands), TypeScript/React 19 (desktop), Playwright (screenshot spec).

**Spec:** `docs/superpowers/specs/2026-07-27-per-owner-server-agent-credentials-design.md`

## Global Constraints

- No `unsafe`; no new `unwrap()`/`expect()` in production paths (tests are fine — existing tests use them).
- Credentials must NEVER appear in: kind:30178 specs, kind:30179 status bodies or hashes, `PromptMaterial`, announcements, logs, or `agents.json`. Only the encrypted 24201 channel and `credentials.json` (0600) may hold them.
- Run `. ./bin/activate-hermit` before any cargo/git command (repo root: `/Users/sid/Developer/buzz`).
- Rust checks: `cargo test -p buzz-sdk -p buzz-spawner` and `cargo test --manifest-path desktop/src-tauri/Cargo.toml` (desktop crate is NOT in the root workspace). `cargo fmt --all` + clippy run in pre-commit/pre-push hooks.
- Desktop checks: `cd desktop && pnpm test` (vitest), `pnpm exec biome check src`.
- Commit after every task.

---

### Task 1: SDK — credential frames and `needs_credential` status field

**Files:**
- Modify: `crates/buzz-sdk/src/spawner.rs`

**Interfaces:**
- Produces: `AttestationFrame::CredentialUpdate { credential: String }`, `AttestationFrame::CredentialAck { accepted: bool, message: Option<String> }`, `SpawnerAgentStatus.needs_credential: bool`, `pub const MAX_CREDENTIAL_BYTES: usize = 512`.
- Frame helpers: `agent_pubkey()` and `spec_slug()` return `""` for both credential variants (they are owner-scoped, not agent-scoped); `nonce()` returns `""` (no handshake round).

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/buzz-sdk/src/spawner.rs`:

```rust
#[test]
fn credential_update_round_trips_and_is_owner_scoped() {
    let frame = AttestationFrame::CredentialUpdate {
        credential: "sk-ant-oat01-abc".into(),
    };
    assert!(frame.validate().is_ok());
    // Owner-scoped: no agent, no slug, no nonce.
    assert_eq!(frame.agent_pubkey(), "");
    assert_eq!(frame.spec_slug(), "");
    assert_eq!(frame.nonce(), "");
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains(r#""type":"credential_update""#));
    assert_eq!(
        serde_json::from_str::<AttestationFrame>(&json).unwrap(),
        frame
    );
}

#[test]
fn credential_update_may_be_empty_to_clear() {
    let frame = AttestationFrame::CredentialUpdate {
        credential: String::new(),
    };
    assert!(frame.validate().is_ok());
}

#[test]
fn credential_update_rejects_oversized_tokens() {
    let frame = AttestationFrame::CredentialUpdate {
        credential: "x".repeat(MAX_CREDENTIAL_BYTES + 1),
    };
    assert!(frame.validate().is_err());
}

#[test]
fn credential_ack_round_trips() {
    let frame = AttestationFrame::CredentialAck {
        accepted: true,
        message: None,
    };
    assert!(frame.validate().is_ok());
    let json = serde_json::to_string(&frame).unwrap();
    assert!(json.contains(r#""type":"credential_ack""#));
    assert_eq!(
        serde_json::from_str::<AttestationFrame>(&json).unwrap(),
        frame
    );
}

#[test]
fn status_needs_credential_round_trips_and_defaults_false() {
    let s = SpawnerAgentStatus {
        phase: SpawnPhase::Stopped,
        agent_pubkey: None,
        spec_hash: None,
        error: None,
        restart_count: 0,
        prompt_hash: None,
        needs_credential: true,
    };
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("needs_credential"));
    let back: SpawnerAgentStatus = serde_json::from_str(&json).unwrap();
    assert!(back.needs_credential);
    // Old events without the field still parse, and false is omitted.
    let legacy: SpawnerAgentStatus =
        serde_json::from_str(r#"{"phase":"running"}"#).unwrap();
    assert!(!legacy.needs_credential);
    let quiet = SpawnerAgentStatus { needs_credential: false, ..s };
    assert!(!serde_json::to_string(&quiet).unwrap().contains("needs_credential"));
}
```

Existing tests constructing `SpawnerAgentStatus` literally (`status_failed_requires_an_error`, `status_round_trips_and_tags_the_agent`, `status_prompt_hash_round_trips`) must gain `needs_credential: false`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p buzz-sdk spawner`
Expected: FAIL — no `CredentialUpdate` variant, no `needs_credential` field, no `MAX_CREDENTIAL_BYTES`.

- [ ] **Step 3: Implement**

In `crates/buzz-sdk/src/spawner.rs`:

1. New constant next to `ATTESTATION_NONCE_BYTES`:

```rust
/// Maximum byte length of an owner credential in a `CredentialUpdate` frame.
pub const MAX_CREDENTIAL_BYTES: usize = 512;
```

2. Add field to `SpawnerAgentStatus` (after `prompt_hash`):

```rust
    /// True when the agent is held stopped because its owner has not delivered
    /// a provider credential (see [`AttestationFrame::CredentialUpdate`]).
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_credential: bool,
```

with helper `fn is_false(b: &bool) -> bool { !*b }` next to `is_zero`.

3. Add two variants at the end of `AttestationFrame`:

```rust
    /// Owner → spawner: set or replace the owner's provider credential.
    ///
    /// Owner-scoped, not agent-scoped: one token covers every agent this owner
    /// runs on the spawner. An empty string clears it. The token never appears
    /// in any hash or public event — unlike prompt updates there is no
    /// world-readable echo; delivery is confirmed by [`Self::CredentialAck`].
    CredentialUpdate {
        /// The raw token (`sk-ant-oat…` OAuth token or `sk-ant-api…` API key).
        /// Empty clears the stored credential.
        credential: String,
    },
    /// Spawner → owner: delivery confirmation for a `CredentialUpdate`.
    CredentialAck {
        /// Whether the update was stored.
        accepted: bool,
        /// Human-readable detail when `accepted` is false.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
```

4. Update the helper methods. `agent_pubkey()` and `spec_slug()` return `""` for the credential variants (match arm `Self::CredentialUpdate { .. } | Self::CredentialAck { .. } => ""`); `nonce()` adds them to the `PromptUpdate` empty arm. `validate()` gets an early arm **before** the slug/pubkey checks:

```rust
        match self {
            Self::CredentialUpdate { credential } => {
                if credential.len() > MAX_CREDENTIAL_BYTES {
                    return Err(SdkError::InvalidInput(format!(
                        "credential exceeds {MAX_CREDENTIAL_BYTES} bytes"
                    )));
                }
                return Ok(());
            }
            Self::CredentialAck { message, .. } => {
                if message.as_ref().is_some_and(|m| m.len() > 2048) {
                    return Err(SdkError::InvalidInput(
                        "credential ack message exceeds 2048 bytes".into(),
                    ));
                }
                return Ok(());
            }
            _ => {}
        }
```

(The error message deliberately does not echo the credential.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p buzz-sdk spawner`
Expected: PASS (all, including pre-existing).

- [ ] **Step 5: Commit**

```bash
git add crates/buzz-sdk/src/spawner.rs
git commit -m "feat(sdk): credential update/ack attestation frames and needs_credential status"
```

---

### Task 2: Spawner — credential store and token classification

**Files:**
- Create: `crates/buzz-spawner/src/credentials.rs`
- Modify: `crates/buzz-spawner/src/lib.rs` (add `pub mod credentials;` alongside the existing module list)

**Interfaces:**
- Produces: `pub fn credential_env_key(token: &str) -> &'static str`; `pub struct CredentialStore` with `open(state_dir: &Path) -> Result<Self>`, `get(&self, owner_pubkey: &str) -> Option<&str>`, `set(&mut self, owner_pubkey: &str, token: String) -> Result<()>`, `remove(&mut self, owner_pubkey: &str) -> Result<bool>`, `owners(&self) -> impl Iterator<Item = &String>`.
- Consumes: nothing new. File is `<state_dir>/credentials.json`, 0600, atomic write-then-rename — copy the exact pattern from `store.rs::flush`/`restrict_permissions`.

- [ ] **Step 1: Write the module with failing tests**

```rust
//! Per-owner provider credentials, delivered over the encrypted kind:24201
//! channel and held only on this host.
//!
//! Kept in a separate file from `agents.json` so agent records stay
//! credential-free: the two files have different lifecycles (a credential
//! outlives any one agent) and different blast radii when read or logged.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Which env var a token should be injected as, by prefix.
///
/// Claude Code OAuth tokens are `sk-ant-oat…`; anything else is treated as an
/// Anthropic API key. Misclassifying is harmless-but-broken (the harness fails
/// to authenticate), never a leak.
pub fn credential_env_key(token: &str) -> &'static str {
    if token.starts_with("sk-ant-oat") {
        "CLAUDE_CODE_OAUTH_TOKEN"
    } else {
        "ANTHROPIC_API_KEY"
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CredentialFile {
    /// Keyed by owner pubkey, hex.
    #[serde(default)]
    credentials: HashMap<String, String>,
}

/// Persistent owner-pubkey → token store over `<state_dir>/credentials.json`.
pub struct CredentialStore {
    path: PathBuf,
    state: CredentialFile,
}

impl CredentialStore {
    /// Open the store, creating the directory if needed.
    pub fn open(state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("failed to create state dir {}", state_dir.display()))?;
        let path = state_dir.join("credentials.json");
        let state = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => CredentialFile::default(),
            Err(e) => return Err(e).with_context(|| format!("failed to read {}", path.display())),
        };
        Ok(Self { path, state })
    }

    /// The stored token for an owner, if any.
    pub fn get(&self, owner_pubkey: &str) -> Option<&str> {
        self.state.credentials.get(owner_pubkey).map(String::as_str)
    }

    /// Store or replace an owner's token and persist.
    pub fn set(&mut self, owner_pubkey: &str, token: String) -> Result<()> {
        self.state
            .credentials
            .insert(owner_pubkey.to_string(), token);
        self.flush()
    }

    /// Remove an owner's token and persist. Returns whether one existed.
    pub fn remove(&mut self, owner_pubkey: &str) -> Result<bool> {
        let existed = self.state.credentials.remove(owner_pubkey).is_some();
        if existed {
            self.flush()?;
        }
        Ok(existed)
    }

    /// Owners that currently have a token.
    pub fn owners(&self) -> impl Iterator<Item = &String> {
        self.state.credentials.keys()
    }

    /// Atomic 0600 write, same crash-safety rationale as `Store::flush`.
    fn flush(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.state)
            .context("failed to serialize credential store")?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        crate::store::restrict_permissions(&tmp)?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("failed to rename into {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_tokens_by_prefix() {
        assert_eq!(credential_env_key("sk-ant-oat01-xyz"), "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(credential_env_key("sk-ant-api03-xyz"), "ANTHROPIC_API_KEY");
        assert_eq!(credential_env_key("something-else"), "ANTHROPIC_API_KEY");
    }

    #[test]
    fn round_trips_through_the_file_with_owner_only_permissions() {
        let dir = std::env::temp_dir().join(format!("buzz-cred-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let owner = "b".repeat(64);
        {
            let mut store = CredentialStore::open(&dir).unwrap();
            store.set(&owner, "sk-ant-oat01-abc".into()).unwrap();
        }
        let mut store = CredentialStore::open(&dir).unwrap();
        assert_eq!(store.get(&owner), Some("sk-ant-oat01-abc"));
        assert!(store.get("missing").is_none());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("credentials.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "credential file holds tokens");
        }

        assert!(store.remove(&owner).unwrap());
        assert!(!store.remove(&owner).unwrap());
        assert!(store.get(&owner).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
```

`restrict_permissions` in `store.rs` is currently private — change it to `pub(crate) fn restrict_permissions` (both the unix and non-unix versions).

- [ ] **Step 2: Run tests**

Run: `cargo test -p buzz-spawner credentials`
Expected: PASS (write module + lib.rs registration + store.rs visibility together; the test-first cycle here is the file itself).

- [ ] **Step 3: Commit**

```bash
git add crates/buzz-spawner/src/credentials.rs crates/buzz-spawner/src/lib.rs crates/buzz-spawner/src/store.rs
git commit -m "feat(spawner): per-owner credential store with prefix classification"
```

---

### Task 3: Spawner — inject owner credential in `build_agent_env`

**Files:**
- Modify: `crates/buzz-spawner/src/env.rs`
- Modify: `crates/buzz-spawner/src/daemon.rs` (call site only — pass `None` for now to keep it compiling; Task 5 wires the real value)

**Interfaces:**
- Changes signature: `build_agent_env(record, spec, prompt, relay_url, passthrough, runtime, owner_credential: Option<&str>)`.
- When `owner_credential` is `Some`, `ANTHROPIC_API_KEY` and `CLAUDE_CODE_OAUTH_TOKEN` are stripped from passthrough and the classified var is set from the owner token — the owner token always wins over host-global credentials. When `None`, passthrough behaves exactly as today (host-global vars still flow; the *enforcement* that agents don't start without an owner token lives in reconcile, Task 4 — env assembly stays permissive so nothing else breaks).

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `env.rs`:

```rust
#[test]
fn owner_credential_wins_over_host_global_passthrough() {
    let passthrough = vec![
        ("ANTHROPIC_API_KEY".to_string(), "sk-host-global".to_string()),
        ("CLAUDE_CODE_OAUTH_TOKEN".to_string(), "sk-host-oauth".to_string()),
        ("OTHER_VAR".to_string(), "kept".to_string()),
    ];
    let env = build_agent_env(
        &record(),
        &spec(),
        &ResolvedPrompt::default(),
        "wss://r",
        &passthrough,
        &DEFAULT_RUNTIME,
        Some("sk-ant-oat01-owner"),
    );
    assert_eq!(
        lookup(&env, "CLAUDE_CODE_OAUTH_TOKEN").as_deref(),
        Some("sk-ant-oat01-owner")
    );
    // Host-global Anthropic credentials are fully displaced, not just shadowed.
    assert!(lookup(&env, "ANTHROPIC_API_KEY").is_none());
    assert!(!env.iter().any(|(_, v)| v.starts_with("sk-host")));
    assert_eq!(lookup(&env, "OTHER_VAR").as_deref(), Some("kept"));
}

#[test]
fn owner_api_key_is_injected_under_the_api_key_var() {
    let env = build_agent_env(
        &record(),
        &spec(),
        &ResolvedPrompt::default(),
        "wss://r",
        &[],
        &DEFAULT_RUNTIME,
        Some("sk-ant-api03-owner"),
    );
    assert_eq!(
        lookup(&env, "ANTHROPIC_API_KEY").as_deref(),
        Some("sk-ant-api03-owner")
    );
    assert!(lookup(&env, "CLAUDE_CODE_OAUTH_TOKEN").is_none());
}

#[test]
fn without_an_owner_credential_passthrough_flows_unchanged() {
    let passthrough = vec![("ANTHROPIC_API_KEY".to_string(), "sk-host".to_string())];
    let env = build_agent_env(
        &record(),
        &spec(),
        &ResolvedPrompt::default(),
        "wss://r",
        &passthrough,
        &DEFAULT_RUNTIME,
        None,
    );
    assert_eq!(lookup(&env, "ANTHROPIC_API_KEY").as_deref(), Some("sk-host"));
}
```

Update every existing `build_agent_env` call in this test module to pass `None` as the new final argument.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p buzz-spawner env`
Expected: compile error (wrong arity) — that is the failure signal for a signature change.

- [ ] **Step 3: Implement**

In `env.rs`:

```rust
/// Env var names an owner-delivered credential displaces.
const OWNER_CREDENTIAL_KEYS: &[&str] = &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"];
```

Add `owner_credential: Option<&str>` as the last parameter of `build_agent_env` (doc it: "Per-owner provider credential from the encrypted kind:24201 channel. When present it displaces any host-global Anthropic credential in `passthrough` — each owner's agents bill against their own token, never the operator's."). Change the passthrough filter to:

```rust
    let mut env: Vec<(String, String)> = passthrough
        .iter()
        .filter(|(k, _)| !RESERVED_KEYS.contains(&k.as_str()))
        .filter(|(k, _)| {
            owner_credential.is_none() || !OWNER_CREDENTIAL_KEYS.contains(&k.as_str())
        })
        .cloned()
        .collect();
```

And immediately after the filter block (before runtime selection):

```rust
    if let Some(token) = owner_credential {
        env.push((
            crate::credentials::credential_env_key(token).to_string(),
            token.to_string(),
        ));
    }
```

In `daemon.rs::start_agent`, add `None,` as the last argument of the `build_agent_env` call (Task 5 replaces it).

- [ ] **Step 4: Run tests**

Run: `cargo test -p buzz-spawner`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/buzz-spawner/src/env.rs crates/buzz-spawner/src/daemon.rs
git commit -m "feat(spawner): inject per-owner credential into agent env, displacing host-global keys"
```

---

### Task 4: Spawner — reconcile holds agents whose owner has no credential

**Files:**
- Modify: `crates/buzz-spawner/src/reconcile.rs`

**Interfaces:**
- `ReconcileInput` gains `pub credentialed_owners: &'a HashSet<String>` (owner pubkeys that have a stored token).
- New `Action::HoldForCredential { owner_pubkey: String, slug: String, container_id: Option<String> }` — emitted instead of `Start`/`Restart` for an attested, enabled agent whose owner is not in `credentialed_owners`; carries the container id when one is running so the daemon can tear it down. Emitted **only when a container exists or the record's `spec_hash` is `Some`** — i.e. exactly once per transition into the held state, so status isn't republished on every reconcile tick. (`spec_hash: None` + no container = already held.)
- Attestation chasing, Stop (disabled), Delete, and orphan handling are unaffected — an owner can still deploy/approve an agent before adding a token.

- [ ] **Step 1: Write the failing tests**

In `reconcile.rs` tests: the `input()` helper gains a `credentialed_owners` field. Add a module-level helper and update `input()`:

```rust
    fn all_owners() -> HashSet<String> {
        HashSet::from(["b".repeat(64)])
    }
```

`input()` cannot return a struct borrowing a local, so change tests to construct with a named binding. Simplest mechanical change: make `input()` take the set by reference — add a `static`-like leaked set is overkill; instead give `input()` a third-arg default by defining:

```rust
    fn input<'a>(
        desired: &'a [DesiredAgent],
        records: &'a [AgentRecord],
        containers: &'a [ManagedContainer],
        credentialed_owners: &'a HashSet<String>,
    ) -> ReconcileInput<'a> {
        ReconcileInput {
            desired,
            records,
            containers,
            now: 1_100,
            attestation_timeout_secs: 600,
            max_agents: 16,
            desired_hydrated: true,
            credentialed_owners,
        }
    }
```

and update every existing call site to `let owners = all_owners();` … `input(&d, &r, &c, &owners)`. New tests:

```rust
    #[test]
    fn holds_an_attested_agent_whose_owner_has_no_credential() {
        let d = vec![desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let none = HashSet::new();
        let actions = plan(input(&d, &r, &[], &none));
        assert_eq!(
            actions,
            [Action::HoldForCredential {
                owner_pubkey: "b".repeat(64),
                slug: "fizz".into(),
                container_id: None,
            }]
        );
    }

    #[test]
    fn stops_a_running_agent_when_its_owner_credential_is_cleared() {
        let d = vec![desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let c = vec![container("agent1", true)];
        let none = HashSet::new();
        assert_eq!(
            plan(input(&d, &r, &c, &none)),
            [Action::HoldForCredential {
                owner_pubkey: "b".repeat(64),
                slug: "fizz".into(),
                container_id: Some("ctr-agent1".into()),
            }]
        );
    }

    #[test]
    fn an_already_held_agent_is_not_re_held_every_pass() {
        // spec_hash None + no container = the hold already happened; a second
        // HoldForCredential would republish status on every reconcile tick.
        let d = vec![desired("fizz", true, 1)];
        let r = vec![record("fizz", "agent1", true, None)];
        let none = HashSet::new();
        assert!(plan(input(&d, &r, &[], &none)).is_empty());
    }

    #[test]
    fn credential_gate_does_not_block_attestation_or_teardown() {
        // Attestation chasing still runs without a credential…
        let d = vec![desired("fizz", true, 1)];
        let r = vec![record("fizz", "agent1", false, None)];
        let none = HashSet::new();
        let mut late = input(&d, &r, &[], &none);
        late.now = 1_000 + 601;
        assert!(matches!(
            plan(late).as_slice(),
            [Action::ReRequestAttestation { .. }]
        ));
        // …and so does deletion of a removed spec.
        let hash = desired("fizz", true, 1).spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        assert!(plan(input(&[], &r, &[], &none))
            .iter()
            .any(|a| matches!(a, Action::Delete { .. })));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p buzz-spawner reconcile`
Expected: compile error (missing field/variant).

- [ ] **Step 3: Implement**

Add the variant to `Action` (documented: "Tear down / decline to start an agent whose owner has delivered no credential, and report `needs_credential`. Identity, record, and volume are all preserved — this is a hold, not a delete."). Add the field to `ReconcileInput` (documented: "Owner pubkeys with a stored provider credential. An attested, enabled agent whose owner is absent here is held stopped instead of started — server agents run on their owner's token, never the operator's."). In `plan()`, insert **after** the `is_attested` check (step 5) and **before** `let hash = desired.spec_hash();`:

```rust
        // 5b. Attested but the owner has delivered no credential — hold rather
        // than start. Emitted only on the transition (running container, or a
        // record that still thinks it started something) so status is not
        // republished on every pass.
        if !input.credentialed_owners.contains(&desired.owner_pubkey) {
            let container_id = container.map(|c| c.id.clone());
            if container_id.is_some() || record.spec_hash.is_some() {
                actions.push(Action::HoldForCredential {
                    owner_pubkey: desired.owner_pubkey.clone(),
                    slug: desired.slug.clone(),
                    container_id,
                });
            }
            continue;
        }
```

Note: `container` is currently bound *after* the enabled check as `let container = containers_by_agent.get(record.agent_pubkey.as_str());` — the new block goes after that binding (reorder if needed so `container` is in scope; it already is, at line ~240).

Wait — the enabled check (step 4) uses `container` too, and it is bound before step 4. Confirm ordering: binding at step ~4, enabled check, attestation check, then this new block. Correct as written.

Also handle the new variant in `daemon.rs::apply` temporarily so the crate compiles: add an arm that just logs (`Action::HoldForCredential { .. } => Ok(())`); Task 5 fills it in.

- [ ] **Step 4: Run tests**

Run: `cargo test -p buzz-spawner`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/buzz-spawner/src/reconcile.rs crates/buzz-spawner/src/daemon.rs
git commit -m "feat(spawner): hold agents whose owner has no stored credential"
```

---

### Task 5: Spawner daemon — apply credential updates, ack, restart, publish `needs_credential`

**Files:**
- Modify: `crates/buzz-spawner/src/daemon.rs`

**Interfaces:**
- Consumes: `CredentialStore` (Task 2), `Action::HoldForCredential` + `ReconcileInput.credentialed_owners` (Task 4), `AttestationFrame::{CredentialUpdate, CredentialAck}` (Task 1).
- `Daemon` gains field `credentials: CredentialStore`, opened in `start()` next to `Store::open`.

- [ ] **Step 1: Write the failing unit tests**

The daemon's I/O paths are integration-shaped; follow the existing pattern of extracting pure helpers. Add to `daemon.rs`:

```rust
/// Whether a credential update clears (empty/whitespace) or sets a token.
/// Split out so the trim rule is testable without a daemon.
fn normalized_credential(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
```

Tests:

```rust
    #[test]
    fn credential_normalization_trims_and_treats_blank_as_clear() {
        assert_eq!(
            normalized_credential("  sk-ant-oat01-x \n"),
            Some("sk-ant-oat01-x".into())
        );
        assert_eq!(normalized_credential(""), None);
        assert_eq!(normalized_credential("   "), None);
    }
```

- [ ] **Step 2: Run to verify failure, then implement**

Run: `cargo test -p buzz-spawner daemon` → FAIL (missing fn). Then implement all of the following:

1. **Field + open.** In `Daemon`: `credentials: crate::credentials::CredentialStore,`. In `start()`: `let credentials = crate::credentials::CredentialStore::open(&config.state_dir)?;` and include in the struct literal.

2. **Route the frame.** In `handle_inbound`, extend the `Inbound::Attestation` arm before the `apply_attestation` fallthrough:

```rust
                if let buzz_sdk::spawner::AttestationFrame::CredentialUpdate { credential } =
                    &frame
                {
                    return self.apply_credential_update(&sender, credential).await;
                }
                if matches!(
                    &frame,
                    buzz_sdk::spawner::AttestationFrame::CredentialAck { .. }
                ) {
                    // Our own outbound ack echoed off the ephemeral stream.
                    return Ok(());
                }
```

3. **The handler.** New method on `Daemon`:

```rust
    /// Store, replace, or clear the sender's provider credential, then restart
    /// every agent that owner runs here so the change takes effect immediately.
    ///
    /// The sender IS the authorization: a kind:24201 frame is NIP-44 encrypted
    /// to this spawner and the event signature proves who sent it, so the token
    /// is filed under that verified pubkey and can only ever affect that
    /// owner's own agents.
    async fn apply_credential_update(
        &mut self,
        sender: &PublicKey,
        credential: &str,
    ) -> Result<()> {
        let owner = sender.to_hex();
        let outcome: Result<()> = match normalized_credential(credential) {
            Some(token) => self.credentials.set(&owner, token),
            None => self.credentials.remove(&owner).map(|_| ()),
        };

        let ack = buzz_sdk::spawner::AttestationFrame::CredentialAck {
            accepted: outcome.is_ok(),
            message: outcome.as_ref().err().map(|e| format!("{e:#}")),
        };
        if let Err(e) = self.relay.send_attestation(sender, &ack).await {
            warn!(owner = %owner, "failed to send credential ack: {e:#}");
        }
        outcome?;

        // Deliberately no token material in the log line.
        info!(owner = %owner, "owner credential updated");

        // Force-restart this owner's agents: clearing spec_hash makes the next
        // reconcile pass see drift and replace each container (same mechanism
        // as apply_prompt_update). With the credential removed, the same clear
        // lets reconcile emit exactly one HoldForCredential per agent.
        let slugs: Vec<String> = self
            .store
            .agents()
            .filter(|r| r.owner_pubkey == owner)
            .map(|r| r.slug.clone())
            .collect();
        for slug in slugs {
            self.store.update(&owner, &slug, |r| {
                r.spec_hash = None;
            })?;
        }
        self.reconcile().await
    }
```

Wait — clearing `spec_hash` makes an agent *without* a container fall into reconcile's "attested but not running → Start" path, which is what we want for agents previously held. And for the clear-credential case, reconcile's hold gate fires before Start. But note the hold-transition condition (Task 4) requires `container_id.is_some() || record.spec_hash.is_some()` — after this clear, a held agent with a running container still matches via `container_id`. Correct. However there is one wrinkle: clearing `spec_hash` on a **running** agent whose credential was just *set* triggers a `Restart` with `crashed: false` — correct, no backoff. Good.

4. **Apply the hold.** Replace the Task-4 stub arm in `apply()`:

```rust
            Action::HoldForCredential {
                owner_pubkey,
                slug,
                container_id,
            } => {
                if let Some(id) = container_id {
                    // Volume preserved, identity preserved: this is a hold.
                    self.containers.remove(&id, None).await?;
                }
                self.store.update(&owner_pubkey, &slug, |r| {
                    r.spec_hash = None;
                })?;
                let agent_pubkey = self
                    .store
                    .get(&owner_pubkey, &slug)
                    .map(|r| r.agent_pubkey.clone());
                info!(slug = %slug, "agent held: owner has no credential");
                self.publish_needs_credential(&slug, &owner_pubkey, agent_pubkey.as_deref())
                    .await
            }
```

5. **Status with the flag.** `publish_status` builds the struct literally; add `needs_credential: false` there, and add a sibling:

```rust
    /// Publish a `stopped` status flagged `needs_credential`, so clients can
    /// say *why* the agent is not running instead of showing a plain Stopped.
    async fn publish_needs_credential(
        &mut self,
        slug: &str,
        owner_pubkey: &str,
        agent_pubkey: Option<&str>,
    ) -> Result<()> {
        let status = SpawnerAgentStatus {
            phase: SpawnPhase::Stopped,
            agent_pubkey: agent_pubkey.map(str::to_string),
            spec_hash: None,
            error: None,
            restart_count: 0,
            prompt_hash: prompt_hash_for(self.store.get(owner_pubkey, slug)),
            needs_credential: true,
        };
        self.relay.publish_status(slug, owner_pubkey, &status).await
    }
```

6. **Wire reconcile.** In `reconcile()`:

```rust
        let credentialed_owners: std::collections::HashSet<String> =
            self.credentials.owners().cloned().collect();
```

and pass `credentialed_owners: &credentialed_owners` in `ReconcileInput`.

7. **Wire env.** In `start_agent`, replace the Task-3 `None` with a lookup **before** the `ContainerSpec` literal (borrow rules: `self.credentials.get` borrows `self` immutably while `build_agent_env` only needs the `&str` — clone to a local first):

```rust
        let owner_credential = self
            .credentials
            .get(&desired.owner_pubkey)
            .map(str::to_string);
```

then pass `owner_credential.as_deref()`.

- [ ] **Step 3: Run the full spawner + sdk test suites**

Run: `cargo test -p buzz-spawner -p buzz-sdk`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/buzz-spawner/src/daemon.rs
git commit -m "feat(spawner): apply owner credential updates with ack, restart, and needs_credential status"
```

---

### Task 6: Desktop Tauri — build credential-update events, decode acks

**Files:**
- Modify: `desktop/src-tauri/src/commands/spawner.rs`
- Modify: `desktop/src-tauri/src/lib.rs` (register the two new commands next to `send_spawner_prompt_update`, line ~706)

**Interfaces:**
- Produces command `send_spawner_credential_update(spawner_pubkey: String, credential: String) -> Result<SpawnerCredentialUpdateOut, String>` where `SpawnerCredentialUpdateOut { event_json: String }` (camelCase wire: `eventJson`). Empty `credential` clears.
- Produces command `decode_spawner_credential_ack(spawner_pubkey: String, encrypted_content: String) -> Result<Option<SpawnerCredentialAck>, String>` where `SpawnerCredentialAck { accepted: bool, message: Option<String> }` — returns `Ok(None)` for any frame that is not a `CredentialAck` (mirrors `decode_spawner_attestation`'s non-request handling).

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `commands/spawner.rs`:

```rust
    #[test]
    fn credential_update_round_trips_to_the_spawner() {
        let owner = Keys::generate();
        let spawner = Keys::generate();
        let out = build_credential_update_event(
            &owner,
            &spawner.public_key().to_hex(),
            "sk-ant-oat01-abc",
        )
        .unwrap();
        let event: nostr::Event = serde_json::from_str(&out.event_json).unwrap();
        // Ciphertext on the wire — the token must not be readable.
        assert!(!event.content.as_str().contains("sk-ant-oat01-abc"));
        let plain = nostr::nips::nip44::decrypt(
            spawner.secret_key(),
            &owner.public_key(),
            event.content.as_str(),
        )
        .unwrap();
        match serde_json::from_str::<AttestationFrame>(&plain).unwrap() {
            AttestationFrame::CredentialUpdate { credential } => {
                assert_eq!(credential, "sk-ant-oat01-abc");
            }
            other => panic!("wrong frame: {other:?}"),
        }
    }

    #[test]
    fn decoding_an_ack_ignores_other_frames() {
        let owner = Keys::generate();
        let spawner = Keys::generate();
        let encrypt = |frame: &AttestationFrame| {
            nostr::nips::nip44::encrypt(
                spawner.secret_key(),
                &owner.public_key(),
                serde_json::to_string(frame).unwrap(),
                nostr::nips::nip44::Version::V2,
            )
            .unwrap()
        };
        let ack = encrypt(&AttestationFrame::CredentialAck {
            accepted: true,
            message: None,
        });
        let decoded =
            decode_credential_ack_frame(&owner, &spawner.public_key(), &ack).unwrap();
        assert_eq!(
            decoded,
            Some(SpawnerCredentialAck { accepted: true, message: None })
        );

        let request = encrypted_request(
            &spawner,
            &owner,
            &Keys::generate().public_key().to_hex(),
        );
        assert_eq!(
            decode_credential_ack_frame(&owner, &spawner.public_key(), &request).unwrap(),
            None
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml spawner`
Expected: compile error (missing functions/types).

- [ ] **Step 3: Implement**

In `commands/spawner.rs`:

```rust
/// Output of [`send_spawner_credential_update`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnerCredentialUpdateOut {
    /// The signed kind:24201 event to publish over the WebSocket.
    pub event_json: String,
}

/// A decoded `CredentialAck` frame from a spawner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnerCredentialAck {
    /// Whether the spawner stored the update.
    pub accepted: bool,
    /// Detail when it did not.
    pub message: Option<String>,
}

/// Build a signed, NIP-44-encrypted `CredentialUpdate` frame.
///
/// The token is encrypted here, in Rust, and the renderer only ever handles
/// the resulting ciphertext event — nothing persists it on this device. An
/// empty `credential` clears the spawner-side token.
fn build_credential_update_event(
    owner: &nostr::Keys,
    spawner_pubkey: &str,
    credential: &str,
) -> Result<SpawnerCredentialUpdateOut, String> {
    let spawner = nostr::PublicKey::from_hex(spawner_pubkey)
        .map_err(|e| format!("invalid spawner pubkey: {e}"))?;

    let frame = AttestationFrame::CredentialUpdate {
        credential: credential.to_string(),
    };
    frame
        .validate()
        .map_err(|e| format!("invalid credential update: {e}"))?;

    let plaintext = serde_json::to_string(&frame)
        .map_err(|e| format!("failed to serialize credential frame: {e}"))?;
    let ciphertext = nostr::nips::nip44::encrypt(
        owner.secret_key(),
        &spawner,
        plaintext,
        nostr::nips::nip44::Version::V2,
    )
    .map_err(|e| format!("failed to encrypt credential frame: {e}"))?;

    let event = build_spawner_attestation(&spawner.to_hex(), &ciphertext)
        .map_err(|e| format!("failed to build credential event: {e}"))?
        .sign_with_keys(owner)
        .map_err(|e| format!("failed to sign credential event: {e}"))?;

    Ok(SpawnerCredentialUpdateOut {
        event_json: event.as_json(),
    })
}

/// Decrypt a frame and return it only when it is a `CredentialAck`.
fn decode_credential_ack_frame(
    keys: &nostr::Keys,
    spawner: &nostr::PublicKey,
    encrypted_content: &str,
) -> Result<Option<SpawnerCredentialAck>, String> {
    match decrypt_frame(keys, spawner, encrypted_content)? {
        AttestationFrame::CredentialAck { accepted, message } => {
            Ok(Some(SpawnerCredentialAck { accepted, message }))
        }
        _ => Ok(None),
    }
}

/// Sign an owner credential update for a spawner. See
/// [`respond_to_spawner_attestation`] for why the event is returned rather
/// than published: kind 24201 is ephemeral and must go over the WebSocket.
#[tauri::command]
pub async fn send_spawner_credential_update(
    state: tauri::State<'_, AppState>,
    spawner_pubkey: String,
    credential: String,
) -> Result<SpawnerCredentialUpdateOut, String> {
    let keys = state.signing_keys()?;
    build_credential_update_event(&keys, &spawner_pubkey, &credential)
}

/// Decode an inbound frame as a credential ack, `Ok(None)` for anything else.
#[tauri::command]
pub async fn decode_spawner_credential_ack(
    state: tauri::State<'_, AppState>,
    spawner_pubkey: String,
    encrypted_content: String,
) -> Result<Option<SpawnerCredentialAck>, String> {
    let keys = state.signing_keys()?;
    let spawner = nostr::PublicKey::from_hex(&spawner_pubkey)
        .map_err(|e| format!("invalid spawner pubkey: {e}"))?;
    decode_credential_ack_frame(&keys, &spawner, &encrypted_content)
}
```

Register both in `lib.rs` after `send_spawner_prompt_update,`:

```rust
            send_spawner_credential_update,
            decode_spawner_credential_ack,
```

(and add them to the `use`/module import if commands are imported by name there — follow how `send_spawner_prompt_update` is brought in).

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path desktop/src-tauri/Cargo.toml spawner`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/commands/spawner.rs desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): tauri commands to sign credential updates and decode acks"
```

---

### Task 7: Desktop — JS transport, ack waiter, and status parsing

**Files:**
- Modify: `desktop/src/shared/api/tauriSpawner.ts`
- Modify: `desktop/src/shared/api/spawnerRelay.ts`
- Create: `desktop/src/features/agents/spawnerCredentialAcks.ts`
- Modify: `desktop/src/features/agents/spawnerAttestationStore.ts` (deliver acks from its 24201 handler)
- Test: `desktop/src/features/agents/spawnerCredentialAcks.test.ts`

**Interfaces:**
- `tauriSpawner.ts` produces: `buildSpawnerCredentialUpdate(input: { spawnerPubkey: string; credential: string }): Promise<{ event: RelayEvent }>` and `decodeSpawnerCredentialAck(spawnerPubkey: string, encryptedContent: string): Promise<SpawnerCredentialAck | null>` with `export type SpawnerCredentialAck = { accepted: boolean; message?: string | null }`.
- `spawnerRelay.ts` produces: `sendSpawnerCredentialUpdate(input: { spawnerPubkey: string; credential: string }): Promise<void>` — builds via Rust, publishes over the WebSocket (same pattern as `sendSpawnerPromptUpdate`). Also: `SpawnerAgentStatus` gains `needsCredential: boolean`, parsed in `parseSpawnerStatus` as `needsCredential: raw.needs_credential === true`.
- `spawnerCredentialAcks.ts` produces:
  - `deliverSpawnerCredentialAck(spawnerPubkey: string, ack: SpawnerCredentialAck): void`
  - `waitForSpawnerCredentialAck(spawnerPubkey: string, timeoutMs: number): Promise<SpawnerCredentialAck>` — resolves with the next ack from that spawner, rejects with `new Error("The server did not confirm the credential in time.")` on timeout.
  - `resetSpawnerCredentialAcks(): void` — clears waiters (wired into `resetCommunityState()` in `desktop/src/features/communities/useCommunityInit.ts`, per the CLAUDE.md singleton rule).

- [ ] **Step 1: Write the failing test**

`desktop/src/features/agents/spawnerCredentialAcks.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  deliverSpawnerCredentialAck,
  resetSpawnerCredentialAcks,
  waitForSpawnerCredentialAck,
} from "./spawnerCredentialAcks";

const SPAWNER = "a".repeat(64);

afterEach(() => {
  resetSpawnerCredentialAcks();
  vi.useRealTimers();
});

describe("spawnerCredentialAcks", () => {
  it("resolves a waiter with the next ack from that spawner", async () => {
    const waiting = waitForSpawnerCredentialAck(SPAWNER, 1_000);
    deliverSpawnerCredentialAck(SPAWNER, { accepted: true });
    await expect(waiting).resolves.toEqual({ accepted: true });
  });

  it("ignores acks from a different spawner", async () => {
    vi.useFakeTimers();
    const waiting = waitForSpawnerCredentialAck(SPAWNER, 1_000);
    // Attach the rejection expectation before advancing timers.
    const assertion = expect(waiting).rejects.toThrow(/did not confirm/);
    deliverSpawnerCredentialAck("b".repeat(64), { accepted: true });
    vi.advanceTimersByTime(1_001);
    await assertion;
  });

  it("rejects on timeout", async () => {
    vi.useFakeTimers();
    const waiting = waitForSpawnerCredentialAck(SPAWNER, 500);
    const assertion = expect(waiting).rejects.toThrow(/did not confirm/);
    vi.advanceTimersByTime(501);
    await assertion;
  });

  it("reset drops pending waiters by rejecting them", async () => {
    const waiting = waitForSpawnerCredentialAck(SPAWNER, 10_000);
    const assertion = expect(waiting).rejects.toThrow();
    resetSpawnerCredentialAcks();
    await assertion;
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd desktop && pnpm vitest run src/features/agents/spawnerCredentialAcks.test.ts`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement**

`spawnerCredentialAcks.ts`:

```ts
import type { SpawnerCredentialAck } from "@/shared/api/tauriSpawner";

/**
 * One-shot waiters for spawner credential acks, keyed by spawner pubkey.
 *
 * A `CredentialUpdate` has no world-readable echo to poll (deliberately —
 * credentials never appear in any hash), so confirmation arrives as an
 * encrypted `CredentialAck` frame on the same kind:24201 stream the
 * attestation store already subscribes to. The store delivers decoded acks
 * here; the credential card awaits one with a timeout.
 *
 * Module-level singleton for the same reason as the attestation store — the
 * subscription outlives components — so it is reset in `resetCommunityState()`.
 */

type Waiter = {
  resolve: (ack: SpawnerCredentialAck) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};

const waiters = new Map<string, Waiter[]>();

/** Deliver a decoded ack to whoever is waiting on this spawner. */
export function deliverSpawnerCredentialAck(
  spawnerPubkey: string,
  ack: SpawnerCredentialAck,
): void {
  const queue = waiters.get(spawnerPubkey);
  const waiter = queue?.shift();
  if (!waiter) return;
  if (queue && queue.length === 0) waiters.delete(spawnerPubkey);
  clearTimeout(waiter.timer);
  waiter.resolve(ack);
}

/** Await the next ack from `spawnerPubkey`, rejecting after `timeoutMs`. */
export function waitForSpawnerCredentialAck(
  spawnerPubkey: string,
  timeoutMs: number,
): Promise<SpawnerCredentialAck> {
  return new Promise((resolve, reject) => {
    const waiter: Waiter = {
      resolve,
      reject,
      timer: setTimeout(() => {
        remove(spawnerPubkey, waiter);
        reject(new Error("The server did not confirm the credential in time."));
      }, timeoutMs),
    };
    const queue = waiters.get(spawnerPubkey) ?? [];
    queue.push(waiter);
    waiters.set(spawnerPubkey, queue);
  });
}

function remove(spawnerPubkey: string, waiter: Waiter): void {
  const queue = waiters.get(spawnerPubkey);
  if (!queue) return;
  const index = queue.indexOf(waiter);
  if (index >= 0) queue.splice(index, 1);
  if (queue.length === 0) waiters.delete(spawnerPubkey);
}

/** Reject and drop every pending waiter. Wired into `resetCommunityState()`. */
export function resetSpawnerCredentialAcks(): void {
  for (const queue of waiters.values()) {
    for (const waiter of queue) {
      clearTimeout(waiter.timer);
      waiter.reject(new Error("Community changed before the server confirmed."));
    }
  }
  waiters.clear();
}
```

`tauriSpawner.ts` additions (follow the `buildSpawnerPromptUpdate` shape):

```ts
/** A decoded credential ack from a spawner. */
export type SpawnerCredentialAck = {
  accepted: boolean;
  message?: string | null;
};

/**
 * Build a signed credential update for a spawner. The token goes straight to
 * Rust, which encrypts it into the returned event — it is never persisted on
 * this device. An empty `credential` clears the spawner-side token.
 */
export async function buildSpawnerCredentialUpdate(input: {
  spawnerPubkey: string;
  credential: string;
}): Promise<{ event: RelayEvent }> {
  const raw = await invokeTauri<{ eventJson: string }>(
    "send_spawner_credential_update",
    { spawnerPubkey: input.spawnerPubkey, credential: input.credential },
  );
  return { event: JSON.parse(raw.eventJson) as RelayEvent };
}

/** Decode an inbound 24201 frame as a credential ack; null for other frames. */
export async function decodeSpawnerCredentialAck(
  spawnerPubkey: string,
  encryptedContent: string,
): Promise<SpawnerCredentialAck | null> {
  const result = await invokeTauri<SpawnerCredentialAck | null>(
    "decode_spawner_credential_ack",
    { spawnerPubkey, encryptedContent },
  );
  return result ?? null;
}
```

`spawnerRelay.ts` additions:

```ts
/**
 * Build and publish an owner credential update over the WebSocket — same
 * ephemeral-kind routing rationale as `sendSpawnerPromptUpdate`. Deliberately
 * no persistent queue: a queued plaintext credential on disk is exactly what
 * this feature exists to avoid. Confirmation arrives as an encrypted ack; see
 * `waitForSpawnerCredentialAck`.
 */
export async function sendSpawnerCredentialUpdate(input: {
  spawnerPubkey: string;
  credential: string;
}): Promise<void> {
  const { event } = await buildSpawnerCredentialUpdate(input);
  await relayClient.preconnect();
  await relayClient.publishEvent(
    event,
    "Timed out sending the credential.",
    "Failed to send the credential.",
  );
}
```

(import `buildSpawnerCredentialUpdate` at the top next to `buildSpawnerPromptUpdate`). In `SpawnerAgentStatus` add `needsCredential: boolean;` with doc `/** True when the spawner holds this agent stopped awaiting an owner credential. */`, and in `parseSpawnerStatus` add `needsCredential: raw.needs_credential === true,`.

`spawnerAttestationStore.ts`: in `handleAttestationEvent`, the current flow decodes as a request and returns early when `!request`. Change the `if (!request) return;` line to also try the ack decode:

```ts
  if (!request) {
    try {
      const ack = await decodeSpawnerCredentialAck(event.pubkey, event.content);
      if (ack) deliverSpawnerCredentialAck(event.pubkey, ack);
    } catch {
      // Same expected-traffic rationale as the request decode above.
    }
    return;
  }
```

with imports `decodeSpawnerCredentialAck` from `@/shared/api/tauriSpawner` and `deliverSpawnerCredentialAck` from `./spawnerCredentialAcks`.

`useCommunityInit.ts`: add `resetSpawnerCredentialAcks()` inside `resetCommunityState()` next to the other agent resets (import from `@/features/agents/spawnerCredentialAcks`) — also add it to the reset list in the CLAUDE.md "Community Switching" section? **No** — CLAUDE.md lists examples, do not edit it; just wire the reset.

- [ ] **Step 4: Run tests + typecheck**

Run: `cd desktop && pnpm vitest run src/features/agents && pnpm exec tsc --noEmit -p tsconfig.json`
(if the repo has no direct tsc script, `pnpm run build` typechecks; use whichever `package.json` exposes)
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src/shared/api/tauriSpawner.ts desktop/src/shared/api/spawnerRelay.ts desktop/src/features/agents/spawnerCredentialAcks.ts desktop/src/features/agents/spawnerCredentialAcks.test.ts desktop/src/features/agents/spawnerAttestationStore.ts desktop/src/features/communities/useCommunityInit.ts
git commit -m "feat(desktop): credential update transport with encrypted ack waiter"
```

---

### Task 8: Desktop UI — credential card and needs-credential badge

**Files:**
- Create: `desktop/src/features/agents/ui/SpawnerCredentialCard.tsx`
- Modify: `desktop/src/features/agents/ui/ServerAgentsSection.tsx`
- Test: `desktop/src/features/agents/ui/SpawnerCredentialCard.test.tsx`

**Interfaces:**
- Consumes: `sendSpawnerCredentialUpdate` (Task 7), `waitForSpawnerCredentialAck` (Task 7), `SpawnerAgentStatus.needsCredential` (Task 7).
- Produces: `<SpawnerCredentialCard spawnerPubkey={string} spawnerName={string} />`, rendered inside each spawner's block in `ServerAgentsSection`. `ServerAgentRow` shows a warning badge when `agent.status.needsCredential`.

- [ ] **Step 1: Write the failing component test**

`SpawnerCredentialCard.test.tsx` (mock the API layer; follow whatever render-helper convention neighboring `*.test.tsx` files in `desktop/src` use — if none exists nearby, plain `@testing-library/react` `render` is fine):

```tsx
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SpawnerCredentialCard } from "./SpawnerCredentialCard";

const sendSpawnerCredentialUpdate = vi.fn();
const waitForSpawnerCredentialAck = vi.fn();

vi.mock("@/shared/api/spawnerRelay", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  sendSpawnerCredentialUpdate: (...args: unknown[]) =>
    sendSpawnerCredentialUpdate(...args),
}));
vi.mock("../spawnerCredentialAcks", () => ({
  waitForSpawnerCredentialAck: (...args: unknown[]) =>
    waitForSpawnerCredentialAck(...args),
}));

const SPAWNER = "a".repeat(64);

describe("SpawnerCredentialCard", () => {
  it("sends the token and shows success on an accepting ack", async () => {
    sendSpawnerCredentialUpdate.mockResolvedValue(undefined);
    waitForSpawnerCredentialAck.mockResolvedValue({ accepted: true });

    render(<SpawnerCredentialCard spawnerName="prod-vps" spawnerPubkey={SPAWNER} />);
    const input = screen.getByLabelText(/claude credential/i);
    expect(input).toHaveAttribute("type", "password");
    fireEvent.change(input, { target: { value: "sk-ant-oat01-x" } });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() =>
      expect(screen.getByText(/provisioned/i)).toBeInTheDocument(),
    );
    expect(sendSpawnerCredentialUpdate).toHaveBeenCalledWith({
      spawnerPubkey: SPAWNER,
      credential: "sk-ant-oat01-x",
    });
    // Write-only: the field empties after a confirmed save.
    expect(input).toHaveValue("");
  });

  it("shows the failure and keeps the input on a rejected ack", async () => {
    sendSpawnerCredentialUpdate.mockResolvedValue(undefined);
    waitForSpawnerCredentialAck.mockRejectedValue(
      new Error("The server did not confirm the credential in time."),
    );

    render(<SpawnerCredentialCard spawnerName="prod-vps" spawnerPubkey={SPAWNER} />);
    fireEvent.change(screen.getByLabelText(/claude credential/i), {
      target: { value: "sk-ant-oat01-x" },
    });
    fireEvent.click(screen.getByRole("button", { name: /save/i }));

    await waitFor(() =>
      expect(screen.getByText(/did not confirm/i)).toBeInTheDocument(),
    );
    expect(screen.getByLabelText(/claude credential/i)).toHaveValue(
      "sk-ant-oat01-x",
    );
  });

  it("clears the credential with an empty update", async () => {
    sendSpawnerCredentialUpdate.mockResolvedValue(undefined);
    waitForSpawnerCredentialAck.mockResolvedValue({ accepted: true });

    render(<SpawnerCredentialCard spawnerName="prod-vps" spawnerPubkey={SPAWNER} />);
    fireEvent.click(screen.getByRole("button", { name: /clear/i }));

    await waitFor(() =>
      expect(sendSpawnerCredentialUpdate).toHaveBeenCalledWith({
        spawnerPubkey: SPAWNER,
        credential: "",
      }),
    );
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `cd desktop && pnpm vitest run src/features/agents/ui/SpawnerCredentialCard.test.tsx`
Expected: FAIL — component does not exist.

- [ ] **Step 3: Implement the card**

`SpawnerCredentialCard.tsx`:

```tsx
import { Check, CircleAlert } from "lucide-react";
import React from "react";

import { sendSpawnerCredentialUpdate } from "@/shared/api/spawnerRelay";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { waitForSpawnerCredentialAck } from "../spawnerCredentialAcks";

/** How long to wait for the spawner's encrypted ack before reporting failure. */
const ACK_TIMEOUT_MS = 15_000;

type Status =
  | { kind: "idle" }
  | { kind: "sending" }
  | { kind: "saved"; cleared: boolean }
  | { kind: "error"; message: string };

/**
 * Write-only entry for the owner's Claude credential on one spawner.
 *
 * The token is sent straight to Rust for encryption and never stored on this
 * device — the spawner is the source of truth, which is why nothing here reads
 * a saved value back. Server agents on this spawner do not run until the owner
 * provisions a token (the spawner reports `needs_credential` on their status).
 */
export function SpawnerCredentialCard({
  spawnerPubkey,
  spawnerName,
}: {
  spawnerPubkey: string;
  spawnerName: string;
}) {
  const [value, setValue] = React.useState("");
  const [status, setStatus] = React.useState<Status>({ kind: "idle" });
  const inputId = React.useId();

  const submit = async (credential: string) => {
    setStatus({ kind: "sending" });
    try {
      // Await the ack that the send provokes; start listening before sending
      // would be racier to read and the relay round-trip dwarfs the gap.
      const acked = waitForSpawnerCredentialAck(spawnerPubkey, ACK_TIMEOUT_MS);
      await sendSpawnerCredentialUpdate({ spawnerPubkey, credential });
      const ack = await acked;
      if (!ack.accepted) {
        setStatus({
          kind: "error",
          message: ack.message || "The server rejected the credential.",
        });
        return;
      }
      setStatus({ kind: "saved", cleared: credential === "" });
      if (credential !== "") setValue("");
    } catch (error) {
      setStatus({
        kind: "error",
        message:
          error instanceof Error ? error.message : "Failed to send the credential.",
      });
    }
  };

  const sending = status.kind === "sending";

  return (
    <div className="space-y-2 rounded-md border border-border p-3">
      <label className="text-sm font-medium" htmlFor={inputId}>
        Your Claude credential
      </label>
      <p className="text-2xs text-muted-foreground">
        Agents you run on {spawnerName} use your own token. Paste a Claude Code
        OAuth token (sk-ant-oat…) or an Anthropic API key. It is sent encrypted
        to the server and never stored on this device.
      </p>
      <div className="flex gap-2">
        <Input
          autoComplete="off"
          data-testid="spawner-credential-input"
          disabled={sending}
          id={inputId}
          onChange={(event) => setValue(event.target.value)}
          placeholder="sk-ant-…"
          type="password"
          value={value}
        />
        <Button
          disabled={sending || value.trim().length === 0}
          onClick={() => void submit(value.trim())}
          type="button"
          variant="outline"
        >
          Save
        </Button>
        <Button
          disabled={sending}
          onClick={() => void submit("")}
          type="button"
          variant="ghost"
        >
          Clear
        </Button>
      </div>
      {status.kind === "saved" ? (
        <p className="flex items-center gap-1 text-xs text-muted-foreground">
          <Check aria-hidden className="size-3" />
          {status.cleared
            ? "Credential cleared. Your agents here will stop."
            : "Provisioned. Your agents here are restarting with it."}
        </p>
      ) : null}
      {status.kind === "error" ? (
        <p className="flex items-start gap-1 text-xs text-destructive">
          <CircleAlert aria-hidden className="mt-0.5 size-3 shrink-0" />
          <span className="min-w-0 break-words">{status.message}</span>
        </p>
      ) : null}
      {status.kind === "sending" ? (
        <p className="text-xs text-muted-foreground">Waiting for the server…</p>
      ) : null}
    </div>
  );
}
```

In `ServerAgentsSection.tsx`:
1. Render the card inside each connected spawner's block, after the agent list (i.e. just before the closing `</div>` of the `spawners.map` block):

```tsx
            <SpawnerCredentialCard
              spawnerName={spawnerLabel(spawner, directory)}
              spawnerPubkey={spawner}
            />
```

with the import added.
2. In `ServerAgentRow`, derive the badge from `needsCredential` — when set, override the phase badge (the phase is `stopped`, but *why* is the useful part):

```tsx
  const { label, variant } = agent.status.needsCredential
    ? { label: "Needs credential", variant: "warning" as const }
    : phaseLabel(agent.status.phase);
```

and under the error row add:

```tsx
        {agent.status.needsCredential ? (
          <p className="text-2xs text-muted-foreground">
            Add your Claude credential below to start this agent.
          </p>
        ) : null}
```

- [ ] **Step 4: Run tests + lint**

Run: `cd desktop && pnpm vitest run src/features/agents/ui/SpawnerCredentialCard.test.tsx && pnpm exec biome check src/features/agents`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add desktop/src/features/agents/ui/SpawnerCredentialCard.tsx desktop/src/features/agents/ui/SpawnerCredentialCard.test.tsx desktop/src/features/agents/ui/ServerAgentsSection.tsx
git commit -m "feat(desktop): spawner credential card and needs-credential badge"
```

---

### Task 9: Screenshot spec

**Files:**
- Modify: `desktop/tests/e2e/server-agent-editing-screenshots.spec.ts` (extend; read it first and follow its exact seeding/bridge conventions — it already renders the Server Agents section with mocked spawner state)

**Interfaces:**
- Consumes: `installMockBridge(page)` from `desktop/tests/helpers/bridge.ts`, `waitForAnimations` from `desktop/tests/helpers/animations`, mock constants from `desktop/src/testing/e2eBridge.ts`.

- [ ] **Step 1: Read the existing spec and add two screenshots**

Add tests capturing:
1. `credential-card` — the Server Agents section with the credential card visible (scope with `locator.screenshot()` on the card's container or a clip on the section; full-page shots of one grid are the known byte-identical-PNG trap).
2. `needs-credential-badge` — a server agent row whose mocked kind:30179 status body includes `"needs_credential": true` with phase `stopped`, showing the warning badge. Reuse however the existing spec injects status events (it exercises `serverAgentEditPolicy`; if it seeds statuses via the bridge, add the field to that seed).

Follow the mandatory rules from CLAUDE.md: `waitForAnimations(page)` before every capture; `addInitScript` before `installMockBridge`; kill port 4173 + `pnpm run build` if the preview server predates your changes.

- [ ] **Step 2: Run the spec**

Run: `cd desktop && lsof -ti:4173 | xargs kill -9 2>/dev/null; pnpm run build && pnpm exec playwright test server-agent-editing-screenshots`
Expected: PASS, PNGs under `test-results/`.

- [ ] **Step 3: Verify distinctness**

Run: `shasum -a 256 desktop/test-results/**/*.png` — every hash unique. Identical hashes = same pixels captured; fix the spec, do not proceed.

- [ ] **Step 4: Commit**

```bash
git add desktop/tests/e2e/server-agent-editing-screenshots.spec.ts
git commit -m "test(desktop): screenshots for spawner credential card and needs-credential badge"
```

---

### Task 10: Deployment note + full gate

**Files:**
- Modify: `deploy/compose/compose.spawner.yml` (comment only)

- [ ] **Step 1: Document the breaking change**

In `compose.spawner.yml`, above the `BUZZ_SPAWNER_AGENT_ENV` default, add a comment (do not change the default itself — other providers may still ride passthrough):

```yaml
      # NOTE: Anthropic credentials (ANTHROPIC_API_KEY / CLAUDE_CODE_OAUTH_TOKEN)
      # named here are DISPLACED per-owner: each owner delivers their own token
      # from the desktop (Settings → Server agents), and agents whose owner has
      # not are held stopped with needs_credential on their status. Host-global
      # values only reach containers for owners... (they do not: owner tokens are
      # required; keep host values only for non-Anthropic passthrough vars.)
```

Actually write it as the truthful two-liner:

```yaml
      # BREAKING: server agents now require a per-owner Claude credential,
      # delivered from the desktop (Settings → Server agents). Host-global
      # ANTHROPIC_API_KEY / CLAUDE_CODE_OAUTH_TOKEN passthrough no longer
      # reaches agent containers; agents without an owner token are held
      # stopped and report needs_credential.
```

Note the plan's earlier statement is the correct one to implement: with no fallback, the hold in reconcile fires for every owner without a token regardless of passthrough. (`build_agent_env` still passes host vars through when `owner_credential` is `None`, but that path is unreachable for starts because reconcile holds first.)

- [ ] **Step 2: Full quality gate**

Run from repo root:

```bash
. ./bin/activate-hermit
cargo test -p buzz-sdk -p buzz-spawner
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cd desktop && pnpm vitest run && pnpm exec biome check src && cd ..
just ci
```

Expected: all PASS. Fix anything that fails before committing.

- [ ] **Step 3: Commit**

```bash
git add deploy/compose/compose.spawner.yml
git commit -m "docs(deploy): note per-owner credential requirement for server agents"
```
