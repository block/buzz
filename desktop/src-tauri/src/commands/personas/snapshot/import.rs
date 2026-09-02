//! Import-side helpers for `buzz-agent-snapshot v1`.
//!
//! Extracted from `snapshot.rs` to keep that file under the 1500-line gate.
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
        agent_snapshot::{extract_chunk_payload_png, AgentSnapshot, MemoryLevel},
        agent_snapshot_envelope::{
            decrypt_envelope, parse_chunk_payload, resolve_unlock_secret, ChunkPayload,
            LOCKED_CARD_REFUSAL,
        },
        load_managed_agents, load_personas, save_managed_agents, save_personas, AgentDefinition,
        ManagedAgentRecord, RespondTo,
    },
    relay::{effective_agent_relay_url, relay_ws_url_with_override},
    util::now_iso,
};

/// Maximum snapshot file size accepted before decode (5 MiB for JSON,
/// 10 MiB for PNG). Mirrors the established persona-import limits.
pub(crate) const MAX_SNAPSHOT_JSON_BYTES: usize = 5 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_PNG_BYTES: usize = 10 * 1024 * 1024;

const LEGACY_PERSONA_FILE_SUFFIXES: [&str; 4] =
    [".persona.md", ".persona.json", ".persona.png", ".zip"];

pub(super) fn reject_legacy_persona_filename(file_name: &str) -> Result<(), String> {
    if LEGACY_PERSONA_FILE_SUFFIXES
        .iter()
        .any(|suffix| file_name.to_ascii_lowercase().ends_with(suffix))
    {
        return Err(
            "Legacy persona files are no longer supported. Export an .agent.json or .agent.png snapshot instead."
                .to_string(),
        );
    }
    Ok(())
}

// ── Import preview types ──────────────────────────────────────────────────────

/// Materialized preview returned to the UI before any write is committed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotImportPreview {
    /// Agent display name from the snapshot.
    pub display_name: String,
    /// Whether the exported source definition was built in. This is display
    /// metadata only; confirmed imports are always independent custom agents.
    pub is_builtin: bool,
    /// Preferred model from the exported definition.
    pub model: Option<String>,
    /// Preferred runtime from the exported definition.
    pub runtime: Option<String>,
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
    /// Full source allowlist entries, surfaced before import so hidden access
    /// configuration is never reduced to a count.
    pub source_allowlist: Vec<String>,
    /// Pretty-printed, validated manifest exactly as decoded from the file.
    /// The UI makes this available before confirmation for full payload review.
    pub manifest_json: String,
    /// True when the snapshot came from a locked (encrypted) card that this
    /// machine successfully unlocked. Cards that cannot be unlocked never
    /// reach a preview — they fail closed with the locked-card refusal.
    pub locked: bool,
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
/// **Memory consistency:** any manifest whose `memory.entries` is non-empty
/// despite `memory.level == None` is rejected before any write, regardless of
/// the enclosing format.
///
/// **Size cap:** PNG inputs over 10 MiB and JSON inputs over 5 MiB are rejected
/// before allocation to avoid avoidable large-input work.
///
/// **Locked cards:** a structurally valid locked envelope parses successfully
/// as `ChunkPayload::Locked` — no decryption happens here. Callers that can
/// unlock go through [`decode_snapshot_for_import`]; callers that only need
/// transit validation (e.g. `fetch_snapshot_bytes`) accept `Locked` as-is.
pub(crate) fn parse_snapshot_payload_from_bytes(file_bytes: &[u8]) -> Result<ChunkPayload, String> {
    let payload: ChunkPayload = if file_bytes.len() >= 4 && file_bytes[..4] == PNG_MAGIC {
        if file_bytes.len() > MAX_SNAPSHOT_PNG_BYTES {
            return Err(format!(
                "Snapshot file is too large ({} MiB). PNG snapshots must be under 10 MiB.",
                file_bytes.len() / (1024 * 1024)
            ));
        }
        let chunk_json = extract_chunk_payload_png(file_bytes)?;
        let mut payload = parse_chunk_payload(&chunk_json)?;
        // The PNG image body is the portable avatar. It deliberately wins over
        // a manifest avatar *URL*, which may only be reachable by the sender.
        // A 1×1 export placeholder leaves the manifest fallback intact.
        // Inline manifest avatar *bytes* are authoritative and never
        // overridden: trading cards supply the generated card artwork as the
        // PNG body and carry the agent's real avatar inline — adopting the
        // body there would import the card as the agent's face.
        // Locked envelopes stay opaque here — there is no manifest to override
        // until the unlock path decrypts one.
        if let ChunkPayload::Plain(snapshot) = &mut payload {
            if snapshot.profile.avatar_data_url.is_none() {
                if let Some(avatar_data_url) =
                    crate::managed_agents::snapshot_avatar::snapshot_png_avatar_data_url(
                        file_bytes,
                    )?
                {
                    snapshot.profile.avatar_data_url = Some(avatar_data_url);
                }
            }
        }
        payload
    } else {
        // JSON path — apply size cap before serde allocation.
        if file_bytes.len() > MAX_SNAPSHOT_JSON_BYTES {
            return Err(format!(
                "Snapshot file is too large ({} MiB). JSON snapshots must be under 5 MiB.",
                file_bytes.len() / (1024 * 1024)
            ));
        }
        parse_chunk_payload(file_bytes)?
    };
    // Consistency check: none + non-empty entries is always malformed,
    // regardless of enclosing format. Enforced at decode time for plain
    // payloads here, and after decryption for locked ones (see
    // `enforce_memory_consistency` callers).
    if let ChunkPayload::Plain(snapshot) = &payload {
        enforce_memory_consistency(snapshot)?;
    }
    Ok(payload)
}

/// The shared malformed-memory guard: `memory.level == none` with non-empty
/// entries is always rejected before any write.
fn enforce_memory_consistency(
    snapshot: &crate::managed_agents::agent_snapshot::AgentSnapshot,
) -> Result<(), String> {
    if snapshot.memory.level == MemoryLevel::None && !snapshot.memory.entries.is_empty() {
        return Err(
            "Snapshot is malformed: memory.level is 'none' but entries are present.".to_string(),
        );
    }
    Ok(())
}

/// Decode a plain snapshot from raw bytes, refusing locked cards.
///
/// Test-only convenience: production call sites either unlock through
/// [`decode_snapshot_for_import`] or validate structurally through
/// [`parse_snapshot_payload_from_bytes`].
#[cfg(test)]
pub(crate) fn decode_snapshot_from_bytes(
    file_bytes: &[u8],
) -> Result<crate::managed_agents::agent_snapshot::AgentSnapshot, String> {
    match parse_snapshot_payload_from_bytes(file_bytes)? {
        ChunkPayload::Plain(snapshot) => Ok(*snapshot),
        ChunkPayload::Locked(_) => Err(LOCKED_CARD_REFUSAL.to_string()),
    }
}

/// Decode a snapshot for import, unlocking locked cards when — and only
/// when — this machine holds one of the envelope's two exact key endpoints
/// (the owner identity or the named local agent record).
///
/// Returns the decoded manifest and whether it came from a locked envelope.
/// When neither endpoint exists, fails closed with the locked-card refusal —
/// never partial plaintext, never crypto details.
pub(crate) fn decode_snapshot_for_import(
    file_bytes: &[u8],
    owner_keys: Option<&nostr::Keys>,
    records: &[ManagedAgentRecord],
) -> Result<(crate::managed_agents::agent_snapshot::AgentSnapshot, bool), String> {
    match parse_snapshot_payload_from_bytes(file_bytes)? {
        ChunkPayload::Plain(snapshot) => Ok((*snapshot, false)),
        ChunkPayload::Locked(envelope) => {
            let secret = resolve_unlock_secret(&envelope, owner_keys, records)
                .ok_or_else(|| LOCKED_CARD_REFUSAL.to_string())?;
            let snapshot = decrypt_envelope(&envelope, &secret)?;
            enforce_memory_consistency(&snapshot)?;
            Ok((snapshot, true))
        }
    }
}

async fn materialize_import_avatar<F, Fut>(
    avatar_data_url: Option<&str>,
    avatar_url: Option<&str>,
    upload: F,
) -> Result<Option<String>, String>
where
    F: FnOnce(Vec<u8>) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let Some(avatar_data_url) = avatar_data_url else {
        return Ok(avatar_url.map(str::to_string));
    };
    let avatar_bytes =
        crate::managed_agents::agent_snapshot::decode_avatar_data_url(avatar_data_url)
            .ok_or_else(|| "Snapshot avatar data is malformed.".to_string())?;
    upload(avatar_bytes).await.map(Some)
}

// ── `preview_agent_snapshot_import` ──────────────────────────────────────────

/// Decode and validate a snapshot file, returning a preview for the
/// confirmation UI. No writes of any kind are performed.
///
/// `file_bytes` is the raw binary content of the `.agent.json` or
/// `.agent.png` file. The format is sniffed from the content, not the
/// extension, so an incorrectly-named file is handled correctly.
///
/// Locked cards are unlocked here when this machine holds one of the
/// envelope's two exact key endpoints; a card that cannot be unlocked fails
/// with the locked-card refusal (shown directly to the user), never a
/// partial preview. Identity-recovery mode is tolerated: owner keys are
/// simply unavailable, so only the agent-record endpoint can unlock.
///
/// Returns an `AgentSnapshotImportPreview` or a descriptive error. Errors
/// represent irrecoverable failures (corrupt / unsupported / locked-to-
/// someone-else file) and are shown directly to the user.
#[tauri::command]
pub async fn preview_agent_snapshot_import(
    file_bytes: Vec<u8>,
    file_name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentSnapshotImportPreview, String> {
    // Key material + records are gathered up front (cheap, lock-scoped) so
    // the blocking decode below owns plain data.
    let owner_keys = state.signing_keys().ok();
    let records = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        load_managed_agents(&app)?
    };
    tokio::task::spawn_blocking(move || {
        reject_legacy_persona_filename(&file_name)?;
        let (snapshot, locked) =
            decode_snapshot_for_import(&file_bytes, owner_keys.as_ref(), &records)?;

        build_agent_snapshot_import_preview(&snapshot, locked)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

pub(crate) fn build_agent_snapshot_import_preview(
    snapshot: &AgentSnapshot,
    locked: bool,
) -> Result<AgentSnapshotImportPreview, String> {
    let memory_level = match snapshot.memory.level {
        MemoryLevel::None => "none",
        MemoryLevel::Core => "core",
        MemoryLevel::Everything => "everything",
    }
    .to_string();

    let manifest_json = serde_json::to_string_pretty(snapshot)
        .map_err(|e| format!("failed to render snapshot manifest: {e}"))?;
    let source_allowlist = snapshot.definition.respond_to_allowlist.clone();

    Ok(AgentSnapshotImportPreview {
        display_name: snapshot.profile.display_name.clone(),
        is_builtin: snapshot.definition.source_is_builtin,
        model: snapshot.definition.model.clone(),
        runtime: snapshot.definition.runtime.clone(),
        system_prompt: snapshot.definition.system_prompt.clone(),
        // Effective avatar: data URL wins; URL fallback if no data URL.
        avatar_url: snapshot
            .profile
            .avatar_data_url
            .clone()
            .or_else(|| snapshot.profile.avatar_url.clone()),
        memory_level,
        memory_entry_count: snapshot.memory.entries.len(),
        source_allowlist_count: source_allowlist.len(),
        has_source_allowlist: !source_allowlist.is_empty(),
        source_allowlist,
        manifest_json,
        locked,
    })
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
    // ── Phase 1: validate (no writes) ────────────────────────────────────────
    // Locked cards unlock only via this machine's exact key endpoints;
    // anything else fails closed here, before key generation.
    let snapshot = {
        let owner_keys = state.signing_keys().ok();
        let records = {
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|e| e.to_string())?;
            load_managed_agents(&app)?
        };
        decode_snapshot_for_import(&input.file_bytes, owner_keys.as_ref(), &records)?.0
    };

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

    // Profile metadata must contain a hosted URL. Inline avatar data can be far
    // larger than the relay's kind:0 content limit, so upload imported pixels
    // before minting or persisting the new agent. Failing here keeps import
    // atomic instead of creating an agent whose profile can never publish.
    let effective_avatar = materialize_import_avatar(
        snapshot.profile.avatar_data_url.as_deref(),
        snapshot.profile.avatar_url.as_deref(),
        |avatar_bytes| async {
            crate::commands::media::upload_image_bytes(avatar_bytes, &state)
                .await
                .map(|descriptor| descriptor.url)
                .map_err(|error| format!("Could not upload the imported avatar: {error}"))
        },
    )
    .await?;

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
            description: crate::managed_agents::effective_agent_description(
                snapshot.profile.about.as_deref(),
            ),
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
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
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
            // Linked definitions remain the sole description authority. Do
            // not persist a second instance copy that can go stale after an
            // edit or survive a later definition deletion.
            description: None,
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
            provider_policy_pending: false,
            provider_binary_path: None,
            team_id: None,
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
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            definition_respond_to: respond_to_wire.clone(),
            definition_respond_to_allowlist: minted.respond_to_allowlist.clone(),
            definition_parallelism: minted_parallelism,
            relay_mesh: None,
            effort_level: None,
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
    let profile_sync_error = crate::commands::agents::publish_persona_profile(
        &state,
        &record.relay_url,
        &agent_keys,
        &display_name,
        effective_avatar.as_deref(),
        &persona,
        auth_tag.as_deref(),
    )
    .await;

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
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let scope = crate::managed_agents::retention::active_retention_scope(app, state)?;
        let conn = open_retention_db(&scope.db_path)?;
        let content = serde_json::to_string(&agent_event_content(record))
            .map_err(|e| format!("failed to serialize agent content: {e}"))?;
        let (owner_pubkey, event) = {
            let keys = &scope.owner_keys;
            let owner_pubkey = keys.public_key().to_hex();
            let existing =
                get_retained_event(&conn, KIND_MANAGED_AGENT, &owner_pubkey, &record.pubkey)?;
            if existing.as_ref().is_some_and(|row| row.content == content) {
                return Ok(());
            }
            let event = build_agent_event(record)?
                .custom_created_at(monotonic_created_at(existing.map(|row| row.created_at)))
                .sign_with_keys(keys)
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
pub(crate) async fn submit_engram_event(
    state: &AppState,
    agent_keys: &nostr::Keys,
    event_json: &[u8],
    url: &str,
    auth_tag: Option<&str>,
) -> Result<(), String> {
    use crate::relay::build_nip98_auth_header_for_keys;
    use reqwest::Method;

    crate::egress_guard::assert_no_key_backup_bytes(event_json, "persona snapshot engram submit")?;

    // Wait before signing: the relay enforces NIP-98 freshness (±60s) and the
    // gate may hold for up to MAX_HINT_SECONDS (300s). Building auth before the
    // wait produces a stale `created_at` that the relay will reject.
    crate::relay_admission::wait_for_rate_limit().await;
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

// ── NIP-49 egress guard: boundary 7 (persona snapshot engram submit) ─────────

#[cfg(test)]
mod egress_guard_tests {
    use super::submit_engram_event;

    const NCRYPTSEC: &str = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";

    /// An engram body carrying an ncryptsec must be rejected by the guard
    /// before any network I/O (the target port is a discard address; a guard
    /// error — not a connection error — proves the abort ordering).
    #[tokio::test]
    async fn blocks_ncryptsec_before_network() {
        let state = crate::app_state::build_app_state();
        let keys = nostr::Keys::generate();
        let body = format!("{{\"content\":\"{NCRYPTSEC}\"}}");
        let err = submit_engram_event(
            &state,
            &keys,
            body.as_bytes(),
            "http://127.0.0.1:9/events",
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("key-backup material"), "{err}");
    }
}

#[cfg(test)]
#[path = "import_avatar_tests.rs"]
mod import_avatar_tests;
