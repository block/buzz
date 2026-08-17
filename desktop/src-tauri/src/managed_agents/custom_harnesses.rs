//! Loader for user-defined ACP harness definitions.
//!
//! Users drop JSON files into `<app-data>/custom_harnesses/` to register
//! arbitrary ACP-speaking agents without modifying the app or opening a PR.
//! Each file describes a single harness; the loader validates, warns on
//! invalid entries, and never propagates errors to the discovery caller.
//!
//! **Security constraint (Will-ratified):** custom definitions carry NO install
//! shell commands. `can_auto_install` is always `false` for custom entries.
//! Only tier-1 compiled-in runtimes retain install-script power.
//!
//! **Avatar URL security:** custom/preset catalog entries MUST NOT carry
//! user-supplied avatar URLs. `HarnessDefinition` intentionally omits
//! `avatar_url` — all icons are bundled assets keyed via `RUNTIME_LOGOS`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::managed_agents::secret_projection::{
    cancel_gc_candidacy, deserialize_env_map, load_secret, serialize_env_map, write_secret,
    ProjectionStore, WriteOutcome,
};
use crate::managed_agents::secret_seam::{migrate_inline_field, FieldMigration};
use crate::managed_agents::types::{AgentDefinition, ManagedAgentRecord};

/// Regex-equivalent predicate for a valid harness ID.
///
/// IDs must match `[a-z0-9_][a-z0-9_-]*` — lowercase alphanumeric plus
/// hyphens and underscores, starting with an alphanumeric or underscore.
/// This mirrors goose's `generate_id` validation and is intentionally
/// more restrictive than the filesystem to prevent path-traversal tricks.
fn is_valid_harness_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Public re-export of `is_valid_harness_id` for callers outside this module
/// (e.g., the `delete_custom_harness` command that must validate caller-supplied ids).
pub(crate) fn is_valid_harness_id_pub(id: &str) -> bool {
    is_valid_harness_id(id)
}

/// User-supplied harness definition deserialized from a JSON file.
///
/// Only the fields a custom harness definition is permitted to carry are
/// included here — install commands and avatar URLs are intentionally absent
/// (security line: no remote icon URLs from user-editable config).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarnessDefinition {
    /// Unique identifier, must match `[a-z0-9_][a-z0-9_-]*`.
    pub id: String,
    /// Human-readable name shown in the UI.
    pub label: String,
    /// Primary executable name or absolute path. May not be empty.
    pub command: String,
    /// Default CLI arguments passed to the command (array, not split-string).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables injected at spawn time. Definition env is applied
    /// first and LOSES on conflict with Buzz-injected vars — `BUZZ_MANAGED_AGENT`
    /// is always authoritative and cannot be overridden here.
    ///
    /// Secret-projection contract (mirrors `ManagedAgentRecord.env_vars`): on a
    /// keyring-backed save this map is stripped to the OS keyring and left empty
    /// on disk, with [`env_ref`](Self::env_ref) pointing at the generation. When
    /// non-empty on disk it is the authoritative inline-fallback state (keyring
    /// unavailable) and wins over any stale `env_ref` on hydration.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Generation reference for the keyring-projected `env` map, set when the
    /// inline `env` was stripped into the keyring under `harness:<id>:env:<gen>`.
    /// Non-secret pointer only. Absent when the definition carries no env, or on
    /// a keyless build where env stays inline in the `0o600` JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_ref: Option<String>,
    /// Runtime-only marker: set when [`env_ref`](Self::env_ref) is present but
    /// its keyring generation could not be hydrated (missing, unreadable, or
    /// malformed bytes). Never serialized — reconstructed on every load by
    /// [`hydrate_harness_env`], mirroring `ManagedAgentRecord::secrets_unavailable`.
    /// Distinguishes a keyring outage from a genuinely-empty `env`: the spawn,
    /// deploy, readiness, and model-discovery gates fail closed on it, and a
    /// metadata save preserves the raw persisted ref instead of erasing it.
    #[serde(skip)]
    pub env_unavailable: bool,
    /// Link to external docs for manual install/setup instructions.
    #[serde(default)]
    pub install_instructions_url: String,
    /// Human-readable install hint shown in Doctor.
    #[serde(default)]
    pub install_hint: String,
}

/// Keyring blob coordinate for a harness definition's projected `env` map.
///
/// The `harness:` namespace is deliberately distinct from the agent-store
/// namespaces (`global:env:` / `agent:…` / `definition:…`) that
/// [`crate::managed_agents::secret_projection::is_projection_key`] matches, so
/// the agent-store GC sweeps never touch harness generations. Harness env edits
/// are rare, so the un-reclaimed generations accrete slowly (there is no harness
/// namespace GC) as an encrypted-at-rest cost documented as a known limitation.
pub(crate) fn harness_env_key(id: &str, gen: &str) -> String {
    format!("harness:{id}:env:{gen}")
}

// ── Env secret projection (mirrors the agent-store `secret_seam`) ────────────
//
// A harness definition carries a single `env` map that can hold provider
// secrets (e.g. `ANTHROPIC_API_KEY`). These three helpers move it between the
// on-disk JSON and the OS keyring under the `harness:<id>:env:<gen>` coordinate.
// Each is generic over [`ProjectionStore`] so tests drive a fake and never the
// live OS keyring (the default `system-keyring` feature makes the live store
// real under `cargo test`).

/// Save-path strip: project a non-empty inline `env` into the keyring under a
/// fresh generation and clear it on disk. Mutates `def` in place.
///
/// # Empty inline `env`: clear vs. preserve
///
/// An empty inline `env` is normally a user-clear (a UI save carries the user's
/// full intent — empty means "no env") and clears the ref. But the TS edit form
/// never round-trips `env_ref`, and it seeds its env from the catalog entry —
/// which is EMPTY for a record whose keyring env could not be hydrated. Saving
/// that partially-hydrated view would erase the still-live ref, turning a
/// keyring outage into permanent pointer loss. So `preserved_ref` carries the
/// raw persisted ref of an on-disk record that is currently `env_unavailable`
/// (computed by [`persisted_unavailable_env_ref`]): on the empty-projection
/// branch it is kept instead of cleared, mirroring the agent seam's
/// `WriteOutcome::Nothing if unavailable` guard. A genuine clear of a healthy
/// record passes `None` and clears normally.
///
/// On a keyring write failure the value stays inline (`0o600` fallback) with the
/// ref cleared (the inline map is now the authoritative fallback state).
fn strip_harness_env<S: ProjectionStore>(
    store: &S,
    def: &mut HarnessDefinition,
    preserved_ref: Option<&str>,
) {
    let id = def.id.clone();
    let inline_env = if !def.env.is_empty() {
        serialize_env_map(&def.env).ok()
    } else {
        None
    };
    match write_secret(
        store,
        |gen| harness_env_key(&id, gen),
        inline_env.as_deref(),
        &format!("harness:{id} env"),
    ) {
        WriteOutcome::Persisted { gen } => {
            cancel_gc_candidacy(store, &harness_env_key(&id, &gen));
            def.env.clear();
            def.env_ref = Some(gen);
        }
        // Empty inline env. Preserve the ref when the persisted record was
        // `env_unavailable` (a save over a partially-hydrated view is not a
        // user-clear); otherwise this is a genuine clear.
        WriteOutcome::Nothing => {
            def.env_ref = preserved_ref.map(str::to_string);
        }
        // Keyring write of a non-empty env failed: value kept inline, ref cleared.
        WriteOutcome::KeptInline { .. } => {
            def.env_ref = None;
        }
    }
}

/// Re-read the on-disk record for `id` and rehydrate it, returning its raw
/// `env_ref` **iff** that record is currently `env_unavailable` (a ref is
/// present but its keyring generation could not be hydrated). Returns `None`
/// when the file is absent, healthy, or carries no ref.
///
/// This is the save-path signal that distinguishes a keyring outage from a
/// genuine user-clear. The TS edit form never round-trips `env_ref` (it always
/// arrives `None`) and seeds its env from the catalog entry — which is empty
/// for an unavailable record — so without this the naive empty-env save would
/// erase the live ref. Taking `dir` as a parameter (rather than a global
/// lookup) keeps the store-injected save tests hermetic.
fn persisted_unavailable_env_ref<S: ProjectionStore>(
    store: &S,
    dir: &Path,
    id: &str,
) -> Option<String> {
    let path = dir.join(format!("{id}.json"));
    let contents = std::fs::read_to_string(&path).ok()?;
    let mut def: HarnessDefinition = serde_json::from_str(&contents).ok()?;
    let raw_ref = def.env_ref.clone();
    hydrate_harness_env(store, &mut def);
    if def.env_unavailable {
        raw_ref
    } else {
        None
    }
}

/// Load-path hydrate: fill an empty on-disk `env` from the keyring generation
/// named by `env_ref`. A non-empty inline `env` is the authoritative
/// keyring-unavailable fallback and wins over any ref.
///
/// When `env_ref` is present but its generation cannot be hydrated — missing,
/// unreadable, or malformed bytes — this sets [`HarnessDefinition::env_unavailable`]
/// and leaves `env` empty. That marker is distinct from a genuinely-empty `env`
/// (no ref): the spawn/deploy/readiness/model-discovery gates fail closed on it,
/// and the save path preserves the raw ref rather than erasing it. Mirrors the
/// agent tier's `secrets_unavailable`, which `hydrate_all_secrets_for_records`
/// sets on the same failure. The load still succeeds so a single unavailable
/// harness never fails discovery for the rest.
fn hydrate_harness_env<S: ProjectionStore>(store: &S, def: &mut HarnessDefinition) {
    if !def.env.is_empty() {
        return; // inline fallback is authoritative
    }
    let id = def.id.clone();
    match load_secret(
        store,
        def.env_ref.as_deref(),
        |gen| harness_env_key(&id, gen),
        &format!("harness:{id} env"),
    ) {
        Ok(Some(s)) => match deserialize_env_map(&s) {
            Ok(map) => def.env = map,
            Err(e) => {
                tracing::warn!("custom_harnesses: harness {id} env deserialize failed: {e}");
                def.env_unavailable = true;
            }
        },
        Ok(None) => {}
        Err(e) => {
            tracing::warn!("custom_harnesses: {e}");
            def.env_unavailable = true;
        }
    }
}

/// Boot-migration transition: W1-safe field migration of one definition's
/// `env`. An absent inline value here means "already projected on a prior
/// launch," so an existing ref is preserved rather than cleared (the
/// distinction from [`strip_harness_env`], which reads empty as a user-clear).
/// Returns `true` when the ref changed and the file must be rewritten.
///
/// Shares the single W1-safe seam
/// ([`crate::managed_agents::secret_seam::migrate_inline_field`]) with the agent
/// tiers so the two surfaces cannot diverge on the empty-inline semantics.
pub(crate) fn migrate_harness_env<S: ProjectionStore>(
    store: &S,
    def: &mut HarnessDefinition,
) -> bool {
    let id = def.id.clone();
    let inline_env = if !def.env.is_empty() {
        serialize_env_map(&def.env).ok()
    } else {
        None
    };
    match migrate_inline_field(
        store,
        |gen| harness_env_key(&id, gen),
        inline_env.as_deref(),
        def.env_ref.as_deref(),
        &format!("harness:{id} env"),
    ) {
        FieldMigration::Projected { gen } => {
            def.env.clear();
            def.env_ref = Some(gen);
            true
        }
        FieldMigration::Preserved | FieldMigration::Cleared => false,
    }
}

/// Resolve the live secret-projection store, or `None` on a keyless build.
///
/// Consumes the agent-store resolver so harness env shares the exact same
/// `SecretStore` instance (one cache, one mutex) as every other projected
/// secret — never a second handle that could race on the blob.
fn live_projection_store() -> Option<&'static crate::secret_store::SecretStore> {
    crate::managed_agents::storage::agent_secret_store_pub()
}

/// Scan `dir` for `*.json` files and deserialize each into a `HarnessDefinition`,
/// hydrating each definition's `env` from the live keyring.
///
/// Errors per file are logged with `tracing::warn` and skipped — a single
/// malformed file never fails discovery for the rest.  Returns only
/// structurally valid, individually validated definitions.
///
/// Filtering is applied HERE — at the loader boundary — so every consumer
/// (discovery Phase 3, `warm_harness_registry_from_dir`, and through it the
/// spawn/readiness registry) inherits the same guarantees:
///   * `check_id_collision` — a hand-placed `goose.json` can never shadow a
///     built-in or preset id, even on the cold-launch warm path;
///   * duplicate-id dedup within the directory (first file wins).
///
/// **Callers must supply a fresh `dir` path on every `discover_acp_runtimes`
/// call** — this function performs no caching, mirroring goose's
/// `refresh_custom_providers()` pattern.
pub(crate) fn load_custom_harnesses(dir: &Path) -> Vec<HarnessDefinition> {
    load_custom_harnesses_with(live_projection_store(), dir)
}

/// Testable core of [`load_custom_harnesses`], generic over the projection
/// store so tests drive a [`ProjectionStore`] fake instead of the live keyring.
/// Scans + validates the directory, then hydrates each definition's `env`.
pub(crate) fn load_custom_harnesses_with<S: ProjectionStore>(
    store: Option<&S>,
    dir: &Path,
) -> Vec<HarnessDefinition> {
    let mut definitions = load_custom_harnesses_from_disk(dir);
    if let Some(store) = store {
        for def in &mut definitions {
            hydrate_harness_env(store, def);
        }
    }
    definitions
}

/// Raw directory scan + per-file validation, WITHOUT env hydration. The
/// store-independent half of [`load_custom_harnesses_with`].
fn load_custom_harnesses_from_disk(dir: &Path) -> Vec<HarnessDefinition> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return vec![],
        Err(err) => {
            tracing::warn!(
                "custom_harnesses: cannot read directory {}: {err}",
                dir.display()
            );
            return vec![];
        }
    };

    let mut definitions = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("custom_harnesses: failed to read {}: {err}", path.display());
                continue;
            }
        };

        let def: HarnessDefinition = match serde_json::from_str(&contents) {
            Ok(d) => d,
            Err(err) => {
                tracing::warn!(
                    "custom_harnesses: invalid JSON in {}: {err}",
                    path.display()
                );
                continue;
            }
        };

        if let Err(reason) = validate_harness_definition(&def) {
            tracing::warn!("custom_harnesses: skipping {} — {reason}", path.display());
            continue;
        }

        // A custom file must never shadow a built-in or preset id — enforced at
        // the loader so the warm path can't admit what discovery would reject.
        if let Err(reason) = check_id_collision(&def.id) {
            tracing::warn!("custom_harnesses: skipping {} — {reason}", path.display());
            continue;
        }

        // Dedup within the directory itself (a file's id is taken from its JSON
        // content, not its filename, so two files can carry the same id).
        if !seen_ids.insert(def.id.clone()) {
            tracing::warn!(
                "custom_harnesses: skipping {} — duplicate id {:?}",
                path.display(),
                def.id
            );
            continue;
        }

        definitions.push(def);
    }

    definitions
}

/// Validate a deserialized `HarnessDefinition` against the invariants that
/// the rest of the discovery code depends on.
fn validate_harness_definition(def: &HarnessDefinition) -> Result<(), String> {
    if def.id.is_empty() {
        return Err("id must not be empty".into());
    }
    if !is_valid_harness_id(&def.id) {
        return Err(format!(
            "id {:?} does not match [a-z0-9_][a-z0-9_-]* — use lowercase letters, digits, hyphens, and underscores only",
            def.id
        ));
    }
    if def.command.trim().is_empty() {
        return Err("command must not be empty".into());
    }
    if def.label.trim().is_empty() {
        return Err("label must not be empty".into());
    }
    // Args travel to the harness through the comma-delimited
    // `BUZZ_ACP_AGENT_ARGS` env transport (clap `value_delimiter = ','` on the
    // buzz-acp side), so a literal comma inside one argument would silently
    // split into two arguments at runtime. Reject at the validation boundary —
    // shared by save (UI/Tauri) and load (hand-authored files) — so the
    // invariant holds regardless of how the definition arrives.
    if let Some(arg) = def.args.iter().find(|a| a.contains(',')) {
        return Err(format!(
            "args: argument {arg:?} contains a comma — arguments are passed via a \
             comma-delimited transport and would be split at spawn time; \
             use separate argument entries instead"
        ));
    }
    // Validate env keys through the shared boundary validator.  This closes the
    // reserved-key bypass exploit (BUZZ_AUTH_TAG=x forgery shape), rejects
    // NUL bytes that would panic Command::env, and enforces size limits.
    crate::managed_agents::env_vars::validate_user_env_keys(&def.env)
        .map_err(|e| format!("env: {e}"))?;
    // Validate install instructions URL scheme when non-empty.
    if !def.install_instructions_url.is_empty() {
        let url = def.install_instructions_url.trim();
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err(format!(
                "installInstructionsUrl must start with https:// or http://, got: {:?}",
                url
            ));
        }
    }
    Ok(())
}

/// Public wrapper so the `save_custom_harness` Tauri command can validate
/// without duplicating the rules.
pub(crate) fn validate_harness_definition_pub(def: &HarnessDefinition) -> Result<(), String> {
    validate_harness_definition(def)
}

// ── Built-in ID set ──────────────────────────────────────────────────────────

/// IDs reserved for the compiled-in catalog. A custom definition whose `id`
/// collides with a built-in or preset is rejected to prevent shadowing (e.g. a
/// file called `cursor.json` hiding the pre-existing tier-2 preset).
///
/// Derived at compile time from `PRESET_HARNESSES` (tier-2) plus the four
/// tier-1 runtimes — no hand-maintained copy.  Adding a preset to
/// `PRESET_HARNESSES` automatically reserves its ID without a separate edit.
fn builtin_ids() -> impl Iterator<Item = &'static str> {
    const TIER1: &[&str] = &["goose", "claude", "codex", "buzz-agent"];
    let tier2 = crate::managed_agents::discovery::preset_harness_ids();
    TIER1.iter().copied().chain(tier2.iter().copied())
}

/// Return an error string if `id` conflicts with a built-in harness ID.
pub(crate) fn check_id_collision(id: &str) -> Result<(), String> {
    if builtin_ids().any(|reserved| reserved.eq_ignore_ascii_case(id)) {
        return Err(format!(
            "id {:?} is reserved for a built-in harness and cannot be overridden",
            id
        ));
    }
    Ok(())
}

// ── Loaded harness registry (F2 — spawn resolution for custom/preset) ────────
//
// `known_acp_runtime` / `known_acp_runtime_exact` only search the static
// `KNOWN_ACP_RUNTIMES` table, so custom and preset harnesses were invisible at
// spawn time, causing silent fallback to buzz-agent.
//
// The fix: `discover_acp_runtimes_from` populates this registry with every
// non-builtin definition after each discovery run. Spawn, readiness, and
// summary paths query `lookup_loaded_harness` to get the live definition for a
// given id or command. If a harness id that an agent references is gone from the
// registry, the caller gets a typed error — never a silent buzz-agent fallback.

use std::sync::{Arc, RwLock};

/// Mutex used by tests to serialize writes to the loaded-harness registry.
///
/// The registry is a process-global singleton.  Parallel test execution can
/// interleave warm → lookup pairs from different tests, causing false failures.
/// Every test that calls `warm_harness_registry_from_dir` or
/// `update_loaded_harness_registry` must hold this guard for the lifetime of
/// its assertion block.
#[cfg(test)]
pub(crate) fn registry_test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Thread-safe registry of non-builtin (preset + custom) harness definitions,
/// populated on every `discover_acp_runtimes_from` call and queried at spawn time.
pub(super) fn loaded_harness_registry() -> &'static RwLock<Vec<Arc<HarnessDefinition>>> {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<RwLock<Vec<Arc<HarnessDefinition>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(Vec::new()))
}

/// Replace the registry contents with `definitions`. Called once per
/// `discover_acp_runtimes_from` run AND on `save_custom_harness` /
/// `delete_custom_harness` so spawn can always resolve the harness without
/// waiting for the next full discovery.
pub(crate) fn update_loaded_harness_registry(definitions: Vec<HarnessDefinition>) {
    let arcs: Vec<Arc<HarnessDefinition>> = definitions.into_iter().map(Arc::new).collect();
    // Use `into_inner` to recover from a poisoned lock — the registry is a
    // plain replaceable Vec with no torn invariant, so poison recovery is safe.
    let mut guard = match loaded_harness_registry().write() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::warn!("custom_harnesses: loaded-harness registry was poisoned; recovering");
            poisoned.into_inner()
        }
    };
    *guard = arcs;
}

/// Look up a loaded (non-builtin) harness by **id**. Returns `None` when the id
/// is unknown. Uses `into_inner` to recover from a poisoned lock so a panic in
/// one thread never permanently blocks all spawn attempts.
pub(crate) fn lookup_loaded_harness_by_id(id: &str) -> Option<Arc<HarnessDefinition>> {
    let guard = match loaded_harness_registry().read() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::warn!(
                "custom_harnesses: loaded-harness registry read lock was poisoned; recovering"
            );
            poisoned.into_inner()
        }
    };
    guard.iter().find(|d| d.id == id).cloned()
}

/// The effective harness runtime id for `record`: an explicit per-record
/// `runtime` pin wins, else the linked persona's `runtime`, else `""`.
///
/// This is the single source of the resolution order that
/// [`crate::managed_agents::resolve_effective_harness_descriptor`] and
/// [`crate::managed_agents::resolve_effective_agent_env`] use to find the
/// harness definition, so the fail-closed [`unavailable_harness_id`] gate
/// consults exactly the harness a spawn would launch — never a different one.
pub(crate) fn effective_runtime_id<'a>(
    record: &'a ManagedAgentRecord,
    personas: &'a [AgentDefinition],
) -> &'a str {
    record
        .runtime
        .as_deref()
        .or_else(|| {
            record.persona_id.as_deref().and_then(|pid| {
                personas
                    .iter()
                    .find(|p| p.id == pid)
                    .and_then(|p| p.runtime.as_deref())
            })
        })
        .unwrap_or("")
}

/// The effective harness id when that harness's projected `env` is unavailable —
/// its `env_ref` is present but could not be hydrated from the keyring
/// (missing, unreadable, or malformed generation).
///
/// This is the harness tier of the fail-closed spawn gate, alongside
/// [`crate::managed_agents::spawn_key_refusal`]'s instance-tier check,
/// [`crate::managed_agents::unavailable_definition_id`]'s definition tier, and
/// the global-tier check in `spawn_agent_child`. Returns the offending id
/// (rather than a bool) so the spawn path can name it in the refusal without a
/// second lookup; status callers use `.is_some()`. A record whose effective
/// harness is a healthy definition, a builtin (no registry entry), or a
/// dangling id yields `None`.
pub(crate) fn unavailable_harness_id(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
) -> Option<String> {
    let runtime_id = effective_runtime_id(record, personas);
    lookup_loaded_harness_by_id(runtime_id)
        .filter(|def| def.env_unavailable)
        .map(|def| def.id.clone())
}

/// Whether ANY secret tier for `record` is currently unavailable — its ref is
/// present but could not be hydrated from the keyring (missing, unreadable, or
/// malformed generation). ORs the three registry-derivable tiers that every
/// fail-closed spawn seam already consults individually:
///
/// - the instance tier (`record.secrets_unavailable`),
/// - the definition tier ([`crate::managed_agents::unavailable_definition_id`]),
/// - the harness tier ([`unavailable_harness_id`]).
///
/// This is the single predicate the restart-eligibility boundaries call before
/// any `stop_managed_agent_process`: an unavailable record's hydrated env is
/// empty and therefore indistinguishable from an intentionally-empty one at the
/// `agent_readiness`/`resolve_effective_agent_env` layer, so those raw signals
/// cannot be trusted to gate a destructive stop-then-respawn. Gating here keeps
/// a running/setup process alive rather than stopping it and only discovering
/// the refusal at respawn (`FailedAfterStop`). The global tier is not
/// registry-derivable from a record and is covered by the spawn-time gate.
pub(crate) fn effective_secrets_unavailable(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
) -> bool {
    record.secrets_unavailable
        || crate::managed_agents::unavailable_definition_id(record, personas).is_some()
        || unavailable_harness_id(record, personas).is_some()
}

/// Warm the loaded-harness registry synchronously from `custom_dir`.
///
/// Must be called **before** `restore_managed_agents_on_launch` so that cold
/// relaunches can resolve custom/preset harness ids without a full discover
/// round-trip (which is driven by the frontend and arrives later).
///
/// This is intentionally lightweight: it only loads the custom JSON files and
/// the static preset list — no PATH probing, no availability checks.
pub(crate) fn warm_harness_registry_from_dir(custom_dir: Option<&std::path::Path>) {
    // Load only the preset list from the discovery module (static, free).
    let preset_defs = crate::managed_agents::discovery::preset_harness_definitions();
    let custom_defs = custom_dir.map(load_custom_harnesses).unwrap_or_default();
    let mut all: Vec<HarnessDefinition> = preset_defs;
    all.extend(custom_defs);
    update_loaded_harness_registry(all);
}

/// Publish the loaded-harness registry from a **fresh** directory read while
/// holding the persist mutex — the discovery-side counterpart of
/// `save_and_warm` / `delete_and_warm`.
///
/// `discover_acp_runtimes_from` runs slow auth probes between its directory
/// scan and its registry publish. Publishing the pre-probe snapshot could
/// clobber a `save_and_warm` that landed in that window, leaving a freshly
/// saved harness unresolvable at spawn until the next discovery. Re-reading
/// the directory at publish time, under the same mutex as save/delete, makes
/// the documented guarantee ("the registry always reflects disk at warm time")
/// actually hold. Only the publish is locked — never the probes.
pub(crate) fn warm_harness_registry_locked(custom_dir: Option<&std::path::Path>) {
    let _guard = persist_mutex().lock().unwrap_or_else(|e| e.into_inner());
    warm_harness_registry_from_dir(custom_dir);
}

/// Global mutex that serializes save/delete filesystem mutations and the
/// subsequent registry warm as a single atomic unit.
///
/// Without this lock, two concurrent `save_custom_harness` calls could
/// interleave: A writes file-A, B writes file-B, B re-warms (sees only B),
/// A re-warms (sees A + B, wins) — but if timing goes the other way B's warm
/// wins and misses A. Holding the lock for the write+warm pair guarantees that
/// the registry always reflects the complete set of files on disk at the time
/// of the warm.
fn persist_mutex() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};
    static PERSIST: OnceLock<Mutex<()>> = OnceLock::new();
    PERSIST.get_or_init(|| Mutex::new(()))
}

/// Write `definition` to `dir`, then atomically warm the loaded-harness
/// registry.  Callers MUST use this function (not `save_custom_harness_to_dir`
/// or `warm_harness_registry_from_dir` separately) so that concurrent saves
/// cannot produce a stale registry snapshot.
pub(crate) fn save_and_warm(
    dir: &Path,
    definition: &HarnessDefinition,
    rename_old_id: Option<&str>,
) -> Result<SaveOutcome, String> {
    let _guard = persist_mutex().lock().unwrap_or_else(|e| e.into_inner());
    let outcome = save_custom_harness_to_dir(dir, definition, rename_old_id)?;
    warm_harness_registry_from_dir(Some(dir));
    Ok(outcome)
}

/// Delete `id.json` from `dir`, then atomically warm the loaded-harness
/// registry.  Callers MUST use this function (not `fs::remove_file` +
/// `warm_harness_registry_from_dir` separately) for the same reason as
/// `save_and_warm`.
pub(crate) fn delete_and_warm(dir: &Path, id: &str) -> Result<(), String> {
    let _guard = persist_mutex().lock().unwrap_or_else(|e| e.into_inner());
    let target = dir.join(format!("{id}.json"));
    match std::fs::remove_file(&target) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("failed to delete harness {id:?}: {e}")),
    }
    warm_harness_registry_from_dir(Some(dir));
    Ok(())
}

/// The outcome of a successful [`save_custom_harness_to_dir`] call.
#[allow(dead_code)] // Fields consumed by tests; production callers use the Result shape.
pub(crate) struct SaveOutcome {
    /// The path of the newly written harness file.
    pub target_path: std::path::PathBuf,
    /// The path of the old file that was removed on a rename, if any.
    /// `None` when the id was unchanged or this was a new harness.
    pub removed_old_path: Option<std::path::PathBuf>,
}

/// Write a harness definition to `dir/<id>.json`, projecting its `env` secrets
/// into the OS keyring first. The resolved live [`ProjectionStore`] is used;
/// see [`save_custom_harness_to_dir_with`] for the store-injected core.
///
/// The caller is responsible for full validation (id, env, etc.) BEFORE
/// calling this function — no validation is performed here.
pub(crate) fn save_custom_harness_to_dir(
    dir: &Path,
    definition: &HarnessDefinition,
    rename_old_id: Option<&str>,
) -> Result<SaveOutcome, String> {
    save_custom_harness_to_dir_with(live_projection_store(), dir, definition, rename_old_id)
}

/// Store-injected core of [`save_custom_harness_to_dir`]: strip `env` secrets
/// into `store`, then write the stripped definition to `dir/<id>.json` using a
/// backup-swap strategy that is safe on all platforms (including Windows where
/// `fs::rename` over an existing file fails with "access denied"):
///
/// 1. Serialize the STRIPPED definition and write it to a unique temp file via
///    `atomic_write_file`, created `0o600` before any bytes hit disk (the JSON
///    carries inline env in the keyless / keyring-unavailable fallback).
/// 2. If the target already exists, rename it to `<target>.bak` (the backup).
/// 3. `commit()` the temp file (renames temp → target).
///    * On success: delete `.bak` (best-effort). A leaked `.bak` is harmless
///      when the file it backs up had its env projected into the keyring, but
///      carries inline secrets in the keyring-unavailable fallback — which is
///      why the boot migration scrubs `*.json.bak` (see
///      [`crate::migration::migration_harness_secrets`]).
///    * On failure: restore `.bak` → target so the original is never lost.
/// 4. If `rename_old_id` is `Some`, remove `<dir>/<old_id>.json` after the
///    new file is committed (non-fatal if NotFound).
///
/// `definition` is NOT mutated — a clone is stripped so the caller keeps the
/// full `env` for the returned catalog entry (edit round-trip). On a keyless
/// build (`store` is `None`) the `env` stays inline in the `0o600` JSON.
///
/// Renaming a record whose env is currently `env_unavailable` (a live keyring
/// ref that could not be hydrated) is refused with an error: the ref is keyed by
/// id, so a rename can neither re-read it under the new id nor safely carry it
/// forward without stranding it at an unwritten coordinate.
///
/// No cross-process secret-transaction lock is taken here: the `harness:`
/// keyring namespace is deliberately excluded from the agent-store GC
/// ([`crate::managed_agents::secret_projection::is_projection_key`]), so no
/// concurrent sweep can ever delete a harness generation between its write and
/// its JSON commit — the window that lock exists to close does not exist here.
pub(crate) fn save_custom_harness_to_dir_with<S: ProjectionStore>(
    store: Option<&S>,
    dir: &Path,
    definition: &HarnessDefinition,
    rename_old_id: Option<&str>,
) -> Result<SaveOutcome, String> {
    use atomic_write_file::AtomicWriteFile;
    use std::io::Write;

    // Strip env → keyring on a clone. The keyring write happens BEFORE the
    // atomic JSON commit (the commit point), mirroring the agent-store seam.
    let mut to_write = definition.clone();
    if let Some(store) = store {
        // Preserve a live-but-unhydrated ref across a same-id metadata save; the
        // TS form round-trips neither the ref nor the marker and seeds `env`
        // from the (empty) catalog entry, so without this the empty-env save
        // would erase the ref — turning a keyring outage into pointer loss.
        //
        // A rename of an `env_unavailable` record has no safe outcome: the ref
        // is keyed by id (`harness:<id>:env:<gen>`), so re-reading under the new
        // id sees "new harness, empty env" (ref erased), and carrying the ref
        // forward would resolve to `harness:<new_id>:env:<gen>` — a coordinate
        // that was never written, leaving the record permanently unavailable.
        // Refuse it until the keyring hydrates or the env is re-entered.
        let preserved_ref = match rename_old_id {
            Some(old_id) => {
                if persisted_unavailable_env_ref(store, dir, old_id).is_some() {
                    return Err(format!(
                        "cannot rename harness {old_id:?}: its keyring env is currently \
                         unavailable, so the ref cannot be carried to the new id \
                         (retry once the keyring hydrates or re-enter the env)"
                    ));
                }
                None
            }
            None => persisted_unavailable_env_ref(store, dir, &to_write.id),
        };
        strip_harness_env(store, &mut to_write, preserved_ref.as_deref());
    }

    let json = serde_json::to_string_pretty(&to_write)
        .map_err(|e| format!("failed to serialize harness definition: {e}"))?;

    let target_path = dir.join(format!("{}.json", to_write.id));
    let bak_path = dir.join(format!("{}.json.bak", to_write.id));

    // Stage write to temp file, owner-only before any bytes hit disk.
    let mut file = AtomicWriteFile::open(&target_path)
        .map_err(|e| format!("failed to open {}: {e}", target_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("failed to set {} permissions: {e}", target_path.display()))?;
    }
    file.write_all(json.as_bytes())
        .map_err(|e| format!("failed to write harness definition: {e}"))?;

    // Back up the existing target before committing so we can restore on failure.
    let had_backup = if target_path.exists() {
        std::fs::rename(&target_path, &bak_path).map_err(|e| {
            format!(
                "failed to back up {} before replace: {e}",
                target_path.display()
            )
        })?;
        true
    } else {
        false
    };

    // Commit temp → target.  If commit fails and we made a backup, restore it.
    if let Err(e) = file.commit() {
        if had_backup {
            if let Err(restore_err) = std::fs::rename(&bak_path, &target_path) {
                // Both commit and restore failed — surface both so the user
                // can recover manually.
                return Err(format!(
                    "failed to finalize harness definition: {e}; \
                     also failed to restore backup: {restore_err} \
                     (original may be at {})",
                    bak_path.display()
                ));
            }
        }
        return Err(format!("failed to finalize harness definition: {e}"));
    }

    // Commit succeeded — remove the backup (best-effort). A leaked `.bak` is
    // harmless once env was projected to the keyring, but secret-bearing in the
    // inline fallback; the boot migration scrubs `*.json.bak` for that case.
    if had_backup {
        let _ = std::fs::remove_file(&bak_path);
    }

    // Remove old file on id rename, after the new file is committed.
    let mut removed_old_path = None;
    if let Some(old_id) = rename_old_id {
        let old_path = dir.join(format!("{old_id}.json"));
        match std::fs::remove_file(&old_path) {
            Ok(()) => removed_old_path = Some(old_path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                // New file is already committed — log and continue so the
                // registry re-warm picks up the new id.
                tracing::warn!(
                    "save_custom_harness_to_dir: failed to remove old harness {old_id:?}: {e}"
                );
            }
        }
    }

    Ok(SaveOutcome {
        target_path,
        removed_old_path,
    })
}

#[cfg(test)]
#[path = "custom_harnesses_tests.rs"]
mod tests;
