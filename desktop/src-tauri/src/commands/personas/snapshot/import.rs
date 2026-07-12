//! Import-side helpers for `buzz-agent-snapshot v1`.
//!
//! Extracted from `snapshot.rs` to keep that file under the 1000-line gate.
//! The Tauri commands here (`preview_agent_snapshot_import`,
//! `confirm_agent_snapshot_import`) are re-exported from `snapshot.rs` and
//! registered in `lib.rs` through the same `personas::` path as the export
//! commands.

use nostr::ToBech32;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        agent_snapshot::{decode_snapshot_json, decode_snapshot_png, MemoryLevel},
        load_managed_agents, load_personas, save_managed_agents, save_personas, AgentDefinition,
        ManagedAgentRecord, RespondTo,
    },
    relay::{effective_agent_relay_url, relay_ws_url_with_override, sync_managed_agent_profile},
    util::now_iso,
};

/// Maximum snapshot file size accepted before decode (5 MiB for JSON,
/// 10 MiB for PNG). Mirrors the established persona-import limits.
pub(crate) const MAX_SNAPSHOT_JSON_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_PNG_BYTES: usize = 10 * 1024 * 1024;

// ── Import preview types ──────────────────────────────────────────────────────

/// Materialized preview returned to the UI before any write is committed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotImportPreview {
    /// Agent display name from the snapshot.
    pub display_name: String,
    /// System prompt, if any.
    pub system_prompt: Option<String>,
    /// Effective avatar: data URL if present, otherwise the source URL fallback.
    /// The UI renders this as a single avatar source.
    pub avatar_url: Option<String>,
    /// Memory level declared in the snapshot.
    pub memory_level: String,
    /// Number of memory entries bundled in the snapshot.
    pub memory_entry_count: usize,
    /// True when the snapshot's `respond_to_allowlist` is non-empty. These
    /// pubkeys come from the source environment and are meaningless on the
    /// importer's relay — the UI must offer Keep / Clear.
    pub has_source_allowlist: bool,
    /// Number of source allowlist entries.
    pub source_allowlist_count: usize,
}

/// The confirmation request sent from the UI after the user reviews the preview.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotImportConfirm {
    /// Raw bytes of the snapshot file (.agent.json or .agent.png).
    pub file_bytes: Vec<u8>,
    /// When true, copy source `respond_to_allowlist` to the new agent.
    /// When false (the safe default), the allowlist is cleared.
    pub keep_allowlist: bool,
}

/// Structured result returned after a confirmed import.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotImportResult {
    /// Display name of the newly created agent.
    pub display_name: String,
    /// Pubkey of the new agent (hex).
    pub new_pubkey: String,
    /// Persona id created for the agent.
    pub persona_id: String,
    /// Total memory entries successfully written to the relay.
    pub memory_written: usize,
    /// Total memory entries that were in the snapshot.
    pub memory_total: usize,
    /// Non-empty when one or more memory entries failed to publish.
    /// The agent itself was created successfully — only memory is partial.
    pub memory_errors: Vec<String>,
    /// Non-empty when profile sync encountered a non-fatal relay error.
    pub profile_sync_error: Option<String>,
}

// ── Import helpers ─────────────────────────────────────────────────────────

/// Resolve the behavioral defaults for an incoming agent snapshot.
///
/// This is the single authoritative selection path for all import-time
/// allowlist and behavioral decisions. It is extracted as a pure, testable
/// function so that unit tests exercise the exact production logic rather
/// than a reconstruction of it.
///
/// # UI contract
///
/// The Keep/Clear toggle is shown whenever `has_source_allowlist` is true
/// (i.e. the raw allowlist is non-empty), regardless of the source mode.
/// The mode (`respond_to` wire string) and the list are independent axes.
///
/// # Decision table
///
/// | Source mode  | Non-empty list | keep=true            | keep=false              |
/// |--------------|----------------|----------------------|-------------------------|
/// | allowlist    | yes            | preserve mode + list | owner-only + empty      |
/// | allowlist    | no             | **Err** (reject)     | **Err** (reject)        |
/// | non-allowlist| yes            | preserve mode + list | preserve mode + empty   |
/// | non-allowlist| no             | preserve mode        | preserve mode           |
///
/// Allowlist-mode + empty list is always rejected: the UI showed no choice
/// and there is no coherent value to write.
///
/// Non-allowlist + non-empty + Clear: preserve the source mode but empty the
/// list.  Only allowlist-mode requires a mode downgrade on Clear, because
/// `allowlist` without entries is an invalid state.  Non-allowlist modes
/// remain valid with an empty list.
pub(crate) fn resolve_snapshot_import_behavior(
    raw_respond_to: Option<&str>,
    raw_allowlist: &[String],
    parallelism: Option<u32>,
    keep_allowlist: bool,
) -> Result<crate::managed_agents::MintBehavioralDefaults, String> {
    use crate::managed_agents::{
        resolve_mint_behavioral_defaults, validate_respond_to_allowlist, RespondTo,
    };

    // Step 1: normalize allowlist; reject malformed pubkeys immediately.
    let normalized_allowlist = validate_respond_to_allowlist(raw_allowlist)?;

    // Step 2: detect source mode and whether a list was present.
    let source_mode: Option<RespondTo> = match raw_respond_to {
        Some(wire) => Some(RespondTo::parse_wire(wire)?),
        None => None,
    };
    let is_source_allowlist_mode = source_mode == Some(RespondTo::Allowlist);
    let has_source_allowlist = !normalized_allowlist.is_empty();

    // Step 3: hard-reject allowlist-mode + empty list before any key
    // generation — no coherent value can be written either way.
    if is_source_allowlist_mode && !has_source_allowlist {
        return Err(
            "snapshot respond-to mode is 'allowlist' but the allowlist is empty — \
             cannot import: no pubkeys to grant access to"
                .to_string(),
        );
    }

    // Step 4: apply Keep/Clear when the toggle was visible (list non-empty),
    // or preserve the source mode when it was not.
    let (resolved_mode, resolved_allowlist) = if has_source_allowlist {
        if keep_allowlist {
            // Keep: preserve source mode and validated list.
            (source_mode, normalized_allowlist)
        } else if is_source_allowlist_mode {
            // Clear on allowlist-mode: must downgrade mode to owner-only because
            // allowlist mode without entries is an invalid state.
            (Some(RespondTo::OwnerOnly), Vec::new())
        } else {
            // Clear on non-allowlist mode: preserve source mode, empty the list.
            // Non-allowlist modes are valid without entries.
            (source_mode, Vec::new())
        }
    } else {
        // No list present → toggle was never shown; preserve source mode as-is.
        (source_mode, normalized_allowlist)
    };

    resolve_mint_behavioral_defaults(
        resolved_mode,
        resolved_allowlist,
        parallelism,
        None, // no definition record; all inputs are explicit from the snapshot
    )
}

const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4e, 0x47];

/// Decode a `buzz-agent-snapshot v1` manifest from raw bytes.
///
/// Sniffs by magic bytes (PNG signature) first, then falls back to JSON.
/// Fails closed on malformed content, wrong format, or unsupported version.
/// Never trusts the file extension — only the bytes.
///
/// **PNG memory policy:** any PNG manifest whose `memory.level` is not `None`,
/// or whose `memory.entries` is non-empty despite `level == None`, is rejected
/// before any write.  Plaintext memory is accepted only from `.agent.json`.
///
/// **Size cap:** PNG inputs over 10 MiB and JSON inputs over 5 MiB are rejected
/// before allocation to avoid avoidable large-input work.
pub(crate) fn decode_snapshot_from_bytes(
    file_bytes: &[u8],
) -> Result<crate::managed_agents::agent_snapshot::AgentSnapshot, String> {
    if file_bytes.len() >= 4 && file_bytes[..4] == PNG_MAGIC {
        if file_bytes.len() > MAX_SNAPSHOT_PNG_BYTES {
            return Err(format!(
                "Snapshot file is too large ({} MiB). PNG snapshots must be under 10 MiB.",
                file_bytes.len() / (1024 * 1024)
            ));
        }
        let snapshot = decode_snapshot_png(file_bytes)?;
        // Hard reject: PNG must never carry memory.
        if snapshot.memory.level != crate::managed_agents::agent_snapshot::MemoryLevel::None {
            return Err(
                "Cannot import a memory-bearing .agent.png — use .agent.json for \
                 snapshots that include memory."
                    .to_string(),
            );
        }
        if !snapshot.memory.entries.is_empty() {
            return Err(
                "Snapshot is malformed: .agent.png carries memory entries despite \
                 memory.level being 'none'."
                    .to_string(),
            );
        }
        return Ok(snapshot);
    }
    // JSON path — apply size cap before serde allocation.
    if file_bytes.len() > MAX_SNAPSHOT_JSON_BYTES {
        return Err(format!(
            "Snapshot file is too large ({} MiB). JSON snapshots must be under 5 MiB.",
            file_bytes.len() / (1024 * 1024)
        ));
    }
    let snapshot = decode_snapshot_json(file_bytes)?;
    // Consistency check: none + non-empty entries is always malformed,
    // regardless of format.  Mirrors the PNG path above so the rule is
    // enforced at decode time for both formats.
    if !snapshot.memory.entries.is_empty() && snapshot.memory.level == MemoryLevel::None {
        return Err(
            "Snapshot is malformed: memory.level is 'none' but entries are present.".to_string(),
        );
    }
    Ok(snapshot)
}

// ── `preview_agent_snapshot_import` ──────────────────────────────────────────

/// Decode and validate a snapshot file, returning a preview for the
/// confirmation UI. No writes of any kind are performed.
///
/// `file_bytes` is the raw binary content of the `.agent.json` or
/// `.agent.png` file. The format is sniffed from the content, not the
/// extension, so an incorrectly-named file is handled correctly.
///
/// Returns an `AgentSnapshotImportPreview` or a descriptive error. Errors
/// represent irrecoverable failures (corrupt / unsupported file) and are
/// shown directly to the user.
#[tauri::command]
pub async fn preview_agent_snapshot_import(
    file_bytes: Vec<u8>,
    file_name: String,
) -> Result<AgentSnapshotImportPreview, String> {
    tokio::task::spawn_blocking(move || {
        let _ = file_name; // used for context in error messages only
        let snapshot = decode_snapshot_from_bytes(&file_bytes)?;

        let memory_level = match snapshot.memory.level {
            MemoryLevel::None => "none",
            MemoryLevel::Core => "core",
            MemoryLevel::Everything => "everything",
        }
        .to_string();

        Ok(AgentSnapshotImportPreview {
            display_name: snapshot.profile.display_name.clone(),
            system_prompt: snapshot.definition.system_prompt.clone(),
            // Effective avatar: data URL wins; URL fallback if no data URL.
            avatar_url: snapshot
                .profile
                .avatar_data_url
                .clone()
                .or_else(|| snapshot.profile.avatar_url.clone()),
            memory_level,
            memory_entry_count: snapshot.memory.entries.len(),
            source_allowlist_count: snapshot.definition.respond_to_allowlist.len(),
            has_source_allowlist: !snapshot.definition.respond_to_allowlist.is_empty(),
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

// ── `confirm_agent_snapshot_import` ──────────────────────────────────────────

/// Import a `buzz-agent-snapshot v1` file as a brand-new agent.
///
/// Phase sequence:
///   1. Validate — decode the manifest and reject early on any error.
///   2. Mint — generate a new keypair + NIP-OA auth tag; create a
///      `AgentDefinition` + `ManagedAgentRecord` through the same primitives
///      used by the normal create flow.
///   3. Publish — kind:30175 definition via retention path; kind:0 profile
///      via `sync_managed_agent_profile`.
///   4. Memory — for each opted-in entry, build a fresh `kind:30174` event
///      with `engram::build_event` under the new agent↔owner conversation
///      key and POST it to the relay. Failures are collected and returned as
///      `memory_errors`; the agent itself is already created.
///
/// Importing the same file twice yields two distinct agents with different
/// keypairs. No source identity material (pubkey, nsec, auth_tag, relay_url,
/// env_vars, backend, lineage) is consumed.
#[tauri::command]
pub async fn confirm_agent_snapshot_import(
    input: AgentSnapshotImportConfirm,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentSnapshotImportResult, String> {
    // ── Phase 1: validate (no I/O) ───────────────────────────────────────────
    let snapshot = decode_snapshot_from_bytes(&input.file_bytes)?;

    let display_name = snapshot.profile.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err("Snapshot display name is empty.".to_string());
    }

    // ── Resolve behavioral defaults ──────────────────────────────────────────
    let minted = resolve_snapshot_import_behavior(
        snapshot.definition.respond_to.as_deref(),
        &snapshot.definition.respond_to_allowlist,
        snapshot.definition.parallelism,
        input.keep_allowlist,
    )?;
    let minted_parallelism = minted.parallelism;

    // Effective avatar: data URL wins; URL fallback when data URL is absent.
    let effective_avatar: Option<String> = snapshot
        .profile
        .avatar_data_url
        .clone()
        .or_else(|| snapshot.profile.avatar_url.clone());

    // Wire-format string for the persona definition's respond_to field.
    // Omit when it is the default (owner-only) to keep definitions clean.
    let respond_to_wire: Option<String> = if minted.respond_to != RespondTo::default() {
        Some(minted.respond_to.as_str().to_string())
    } else {
        None
    };

    // ── Phase 2: mint keys + auth tag (sync, outside lock) ───────────────────
    let (agent_keys, private_key_nsec, pubkey, auth_tag, owner_pubkey_hex) = {
        let owner_keys = state.signing_keys()?;
        let agent_keys = nostr::Keys::generate();
        let pubkey = agent_keys.public_key().to_hex();
        let private_key_nsec = agent_keys
            .secret_key()
            .to_bech32()
            .map_err(|e| format!("failed to encode agent private key: {e}"))?;

        // NIP-OA auth tag: bridge nostr 0.37 → 0.36 (buzz-sdk) via hex round-trip.
        let compat_owner = nostr::Keys::parse(&owner_keys.secret_key().to_secret_hex())
            .map_err(|e| format!("failed to bridge owner keys: {e}"))?;
        let compat_agent = nostr::PublicKey::from_hex(&pubkey)
            .map_err(|e| format!("failed to bridge agent pubkey: {e}"))?;
        let auth_tag = Some(
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&compat_owner, &compat_agent, "")
                .map_err(|e| format!("failed to compute NIP-OA auth tag: {e}"))?,
        );
        let owner_pubkey_hex = owner_keys.public_key().to_hex();
        (
            agent_keys,
            private_key_nsec,
            pubkey,
            auth_tag,
            owner_pubkey_hex,
        )
    };

    // ── Phase 3a: create AgentDefinition + ManagedAgentRecord (sync lock) ──────
    let (persona, record) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;

        let mut personas = load_personas(&app)?;
        let mut records = load_managed_agents(&app)?;

        // Guard against duplicate pubkey (astronomically unlikely but safe).
        if records.iter().any(|r| r.pubkey == pubkey) {
            return Err(format!("generated pubkey {pubkey} already exists — retry"));
        }

        let now = now_iso();
        let persona_id = uuid::Uuid::new_v4().to_string();

        // Build persona from snapshot definition.
        let persona = AgentDefinition {
            id: persona_id.clone(),
            display_name: display_name.clone(),
            avatar_url: effective_avatar.clone(),
            system_prompt: snapshot
                .definition
                .system_prompt
                .clone()
                .unwrap_or_default(),
            runtime: snapshot.definition.runtime.clone(),
            model: snapshot.definition.model.clone(),
            provider: snapshot.definition.provider.clone(),
            name_pool: snapshot.definition.name_pool.clone(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: std::collections::BTreeMap::new(),
            respond_to: respond_to_wire.clone(),
            respond_to_allowlist: minted.respond_to_allowlist.clone(),
            parallelism: minted_parallelism,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        personas.push(persona.clone());
        save_personas(&app, &personas)?;

        // Enqueue the kind:30175 persona event via the retention path.
        super::super::pending::retain_persona_pending(&app, &state, &persona);

        // Build the managed agent record — no machine-local commands, no
        // secrets, no lineage from the snapshot.
        let record = ManagedAgentRecord {
            pubkey: pubkey.clone(),
            name: display_name.clone(),
            display_name: None,
            slug: None,
            persona_id: Some(persona_id.clone()),
            private_key_nsec: private_key_nsec.clone(),
            auth_tag: auth_tag.clone(),
            relay_url: String::new(), // resolves to workspace relay at runtime
            avatar_url: effective_avatar.clone(),
            // Machine-local commands: derive from the runtime catalog at
            // spawn time — never manufacture from snapshot data.
            acp_command: crate::managed_agents::DEFAULT_ACP_COMMAND.to_string(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: snapshot.definition.idle_timeout_seconds,
            max_turn_duration_seconds: snapshot.definition.max_turn_duration_seconds,
            parallelism: minted_parallelism
                .unwrap_or(crate::managed_agents::DEFAULT_AGENT_PARALLELISM),
            system_prompt: snapshot.definition.system_prompt.clone(),
            model: snapshot.definition.model.clone(),
            provider: snapshot.definition.provider.clone(),
            persona_source_version: None,
            env_vars: std::collections::BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: crate::managed_agents::BackendKind::Local,
            backend_agent_id: None,
            provider_binary_path: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            // Instance-level behavioral defaults agree with the resolved
            // definition: both come from the single minted struct so they
            // are always consistent at mint time.
            respond_to: minted.respond_to,
            respond_to_allowlist: minted.respond_to_allowlist.clone(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            definition_respond_to: respond_to_wire.clone(),
            definition_respond_to_allowlist: minted.respond_to_allowlist.clone(),
            definition_parallelism: minted_parallelism,
            relay_mesh: None,
            runtime: snapshot.definition.runtime.clone(),
            name_pool: snapshot.definition.name_pool.clone(),
        };

        records.push(record.clone());
        save_managed_agents(&app, &records)?;

        // Enqueue the kind:30177 managed-agent event via retention.
        // (Uses the same pattern as agents.rs::retain_managed_agent_pending
        // inlined here to avoid cross-module private-fn access.)
        retain_agent_pending(&app, &state, &record);

        crate::managed_agents::try_regenerate_nest(&app);

        // Notify other mounted clients of local persona+managed-agent writes,
        // matching the contract used by other local managed-agent mutations.
        let _ = app.emit("agents-data-changed", ());

        (persona, record)
    };

    // ── Phase 3b: publish kind:0 profile (async, outside lock) ───────────────
    let relay_url =
        effective_agent_relay_url(&record.relay_url, &relay_ws_url_with_override(&state));
    let profile_sync_error = sync_managed_agent_profile(
        &state,
        &relay_url,
        &agent_keys,
        &display_name,
        effective_avatar.as_deref(),
        auth_tag.as_deref(),
    )
    .await
    .err();

    // ── Phase 4: restore memory (async, outside lock) ─────────────────────────
    let memory_total = snapshot.memory.entries.len();
    let mut memory_written = 0usize;
    let mut memory_errors: Vec<String> = Vec::new();

    if memory_total > 0 {
        let owner_pubkey = nostr::PublicKey::from_hex(&owner_pubkey_hex)
            .map_err(|e| format!("failed to parse owner pubkey: {e}"))?;

        // Monotonic timestamp seed: use current time, bumped by 1 per entry
        // so no two events land at the same second.
        let base_ts = nostr::Timestamp::now().as_secs();

        for (idx, entry) in snapshot.memory.entries.iter().enumerate() {
            let body = if entry.slug == buzz_core_pkg::engram::CORE_SLUG {
                buzz_core_pkg::engram::Body::Core {
                    profile: entry.body.clone(),
                }
            } else {
                buzz_core_pkg::engram::Body::Memory {
                    slug: entry.slug.clone(),
                    value: Some(entry.body.clone()),
                }
            };

            let created_at = base_ts + idx as u64;
            match buzz_core_pkg::engram::build_event(&agent_keys, &owner_pubkey, &body, created_at)
            {
                Ok(event) => {
                    let event_json = nostr::JsonUtil::as_json(&event).into_bytes();
                    let url = format!("{}/events", crate::relay::relay_http_base_url(&relay_url));
                    match submit_engram_event(
                        &state,
                        &agent_keys,
                        &event_json,
                        &url,
                        auth_tag.as_deref(),
                    )
                    .await
                    {
                        Ok(()) => memory_written += 1,
                        Err(e) => memory_errors.push(format!("slug {:?}: {e}", entry.slug)),
                    }
                }
                Err(e) => {
                    memory_errors.push(format!("slug {:?}: build failed: {e}", entry.slug));
                }
            }
        }
    }

    Ok(AgentSnapshotImportResult {
        display_name,
        new_pubkey: pubkey,
        persona_id: persona.id,
        memory_written,
        memory_total,
        memory_errors,
        profile_sync_error,
    })
}

/// Inline retention for the managed-agent kind:30177 event — mirrors
/// `agents::retain_managed_agent_pending` without requiring cross-module
/// private function access.
fn retain_agent_pending(app: &AppHandle, state: &AppState, record: &ManagedAgentRecord) {
    use crate::managed_agents::{
        agent_events::{agent_event_content, build_agent_event},
        managed_agents_base_dir,
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        let content = serde_json::to_string(&agent_event_content(record))
            .map_err(|e| format!("failed to serialize agent content: {e}"))?;
        let (owner_pubkey, event) = {
            let keys = state.signing_keys()?;
            let owner_pubkey = keys.public_key().to_hex();
            let existing =
                get_retained_event(&conn, KIND_MANAGED_AGENT, &owner_pubkey, &record.pubkey)?;
            if existing.as_ref().is_some_and(|row| row.content == content) {
                return Ok(());
            }
            let event = build_agent_event(record)?
                .custom_created_at(monotonic_created_at(existing.map(|row| row.created_at)))
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign agent event: {e}"))?;
            (owner_pubkey, event)
        };
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_MANAGED_AGENT,
                pubkey: owner_pubkey,
                d_tag: record.pubkey.clone(),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: snapshot-import retain-agent: {e}");
    }
}

/// POST a pre-built signed engram event to the relay, authenticating as the
/// new agent.
async fn submit_engram_event(
    state: &AppState,
    agent_keys: &nostr::Keys,
    event_json: &[u8],
    url: &str,
    auth_tag: Option<&str>,
) -> Result<(), String> {
    use crate::relay::build_nip98_auth_header_for_keys;
    use reqwest::Method;

    let auth = build_nip98_auth_header_for_keys(agent_keys, &Method::POST, url, event_json)?;
    let mut request = state
        .http_client
        .post(url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json");
    if let Some(tag) = auth_tag {
        request = request.header("x-auth-tag", tag);
    }
    let response = request
        .body(event_json.to_vec())
        .send()
        .await
        .map_err(|e| crate::relay::classify_request_error(&e))?;

    if !response.status().is_success() {
        let msg = crate::relay::relay_error_message(response).await;
        return Err(format!("relay rejected engram: {msg}"));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("failed to read relay response: {e}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("relay response not JSON: {e}"))?;
    let accepted = parsed
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !accepted {
        let message = parsed
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(format!("relay rejected engram: {message}"));
    }
    Ok(())
}
