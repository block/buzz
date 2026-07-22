//! Tauri commands for exporting and importing `buzz-team-snapshot v1` files.
//!
//! Team snapshots are full-instance bundles: importing mints a fresh keypair
//! and `ManagedAgentRecord` for every member plus one `TeamRecord`. Exporting
//! optionally includes member memory at the requested level.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    commands::{
        agents::{build_deploy_payload, deploy_to_provider},
        export_util::save_bytes_with_dialog,
        personas::resolve_snapshot_import_behavior,
    },
    managed_agents::team_snapshot::{
        build_team_snapshot, decode_team_snapshot_json, decode_team_snapshot_png,
        encode_team_snapshot_json, encode_team_snapshot_png, TeamSnapshot,
    },
    managed_agents::{
        agent_snapshot::{build_snapshot, AgentSnapshot, AgentSnapshotMemoryEntry, MemoryLevel},
        load_managed_agents, load_personas, load_teams, load_teams_readonly, save_managed_agents,
        save_personas, save_teams, AgentDefinition, BackendKind, ManagedAgentRecord, TeamRecord,
    },
    relay::{effective_agent_relay_url, relay_ws_url_with_override, sync_managed_agent_profile},
    util::now_iso,
};

/// Team snapshots have a combined 25 MiB JSON / 50 MiB PNG payload cap.
/// Every member is validated before any persistent write.
pub(crate) const MAX_TEAM_SNAPSHOT_JSON_BYTES: usize = 25 * 1024 * 1024;
pub(crate) const MAX_TEAM_SNAPSHOT_PNG_BYTES: usize = 50 * 1024 * 1024;

const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4e, 0x47];
const ZIP_MAGIC_PREFIX: [u8; 2] = [0x50, 0x4b];
const LEGACY_TEAM_ERROR: &str =
    "Legacy team files are no longer supported. Export a buzz-team-snapshot v1 .team.json or .team.png instead.";

/// Decode a canonical team snapshot, rejecting retired flat team JSON and
/// persona-pack ZIP files with a migration-oriented error.
pub(crate) fn decode_team_snapshot_from_bytes(file_bytes: &[u8]) -> Result<TeamSnapshot, String> {
    if file_bytes.starts_with(&PNG_MAGIC) {
        if file_bytes.len() > MAX_TEAM_SNAPSHOT_PNG_BYTES {
            return Err(format!(
                "Team snapshot file is too large ({} MiB). PNG snapshots must be under 50 MiB.",
                file_bytes.len() / (1024 * 1024)
            ));
        }
        return decode_team_snapshot_png(file_bytes);
    }

    if file_bytes.len() > MAX_TEAM_SNAPSHOT_JSON_BYTES {
        return Err(format!(
            "Team snapshot file is too large ({} MiB). JSON snapshots must be under 25 MiB.",
            file_bytes.len() / (1024 * 1024)
        ));
    }

    // Detect the retired schema before attempting canonical deserialization so
    // old `.team.json` attachments never look like malformed new snapshots.
    let value: serde_json::Value = match serde_json::from_slice(file_bytes) {
        Ok(value) => value,
        Err(_) if file_bytes.starts_with(&ZIP_MAGIC_PREFIX) => {
            return Err(LEGACY_TEAM_ERROR.to_string());
        }
        Err(error) => return Err(format!("Invalid team snapshot JSON: {error}")),
    };
    if value.get("format").and_then(serde_json::Value::as_str)
        != Some(crate::managed_agents::team_snapshot::FORMAT_DISCRIMINATOR)
        && value.get("version").and_then(serde_json::Value::as_u64) == Some(1)
        && value.get("type").and_then(serde_json::Value::as_str) == Some("team")
    {
        return Err(LEGACY_TEAM_ERROR.to_string());
    }

    decode_team_snapshot_json(file_bytes)
}

fn parse_format_is_png(format: &str) -> Result<bool, String> {
    match format {
        "json" | "" => Ok(false),
        "png" => Ok(true),
        other => Err(format!(
            "Invalid format: {other:?} (expected 'json' or 'png')"
        )),
    }
}

fn parse_memory_level(s: &str) -> Result<MemoryLevel, String> {
    match s {
        "none" | "" => Ok(MemoryLevel::None),
        "core" => Ok(MemoryLevel::Core),
        "everything" => Ok(MemoryLevel::Everything),
        other => Err(format!(
            "Invalid memory_level: {other:?} (expected 'none', 'core', or 'everything')"
        )),
    }
}

fn effective_avatar(member: &AgentSnapshot) -> Option<String> {
    member
        .profile
        .avatar_data_url
        .clone()
        .or_else(|| member.profile.avatar_url.clone())
}

/// Build a definition from a team member snapshot without consuming its memory.
fn definition_from_snapshot(
    member: &AgentSnapshot,
    keep_allowlist: bool,
    now: &str,
) -> Result<AgentDefinition, String> {
    let behavior = resolve_snapshot_import_behavior(
        member.definition.respond_to.as_deref(),
        &member.definition.respond_to_allowlist,
        member.definition.parallelism,
        keep_allowlist,
    )?;
    let respond_to = (behavior.respond_to != crate::managed_agents::RespondTo::default())
        .then(|| behavior.respond_to.as_str().to_string());

    Ok(AgentDefinition {
        id: Uuid::new_v4().to_string(),
        display_name: member.profile.display_name.trim().to_string(),
        avatar_url: effective_avatar(member),
        system_prompt: member.definition.system_prompt.clone().unwrap_or_default(),
        runtime: member.definition.runtime.clone(),
        model: member.definition.model.clone(),
        provider: member.definition.provider.clone(),
        name_pool: member.definition.name_pool.clone(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: Default::default(),
        respond_to,
        respond_to_allowlist: behavior.respond_to_allowlist,
        parallelism: behavior.parallelism,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}

pub(crate) fn build_import_definitions(
    snapshot: &TeamSnapshot,
    keep_allowlist: bool,
    now: &str,
) -> Result<Vec<AgentDefinition>, String> {
    snapshot
        .members
        .iter()
        .map(|member| definition_from_snapshot(member, keep_allowlist, now))
        .collect()
}

/// Assemble the one new team record that references freshly built definitions.
pub(crate) fn build_import_team(
    snapshot: &TeamSnapshot,
    persona_ids: Vec<String>,
    now: &str,
) -> Result<TeamRecord, String> {
    let name = snapshot.team.name.trim();
    if name.is_empty() {
        return Err("Team snapshot name is empty.".to_string());
    }

    Ok(TeamRecord {
        id: Uuid::new_v4().to_string(),
        name: name.to_string(),
        description: snapshot.team.description.clone(),
        persona_ids,
        instructions: snapshot.team.instructions.clone(),
        is_builtin: false,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}

fn member_preview(member: &AgentSnapshot) -> TeamSnapshotMemberPreview {
    TeamSnapshotMemberPreview {
        display_name: member.profile.display_name.clone(),
        system_prompt: member.definition.system_prompt.clone(),
        avatar_url: effective_avatar(member),
        has_source_allowlist: !member.definition.respond_to_allowlist.is_empty(),
        source_allowlist_count: member.definition.respond_to_allowlist.len(),
    }
}

/// Preview metadata for one definition that will be imported with a team.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotMemberPreview {
    pub display_name: String,
    pub system_prompt: Option<String>,
    pub avatar_url: Option<String>,
    pub has_source_allowlist: bool,
    pub source_allowlist_count: usize,
}

/// Materialized team snapshot preview. No write happens before confirmation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotImportPreview {
    pub name: String,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub members: Vec<TeamSnapshotMemberPreview>,
    pub has_source_allowlist: bool,
}

/// Confirmation input for a team snapshot import.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotImportConfirm {
    pub file_bytes: Vec<u8>,
    /// Applied uniformly to every member in v1.
    pub keep_allowlist: bool,
    /// Applied uniformly to every imported member. Absent preserves local.
    #[serde(default)]
    pub backend: Option<BackendKind>,
}

/// Per-member outcome reported after a confirmed team snapshot import.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotImportMemberResult {
    pub display_name: String,
    pub pubkey: String,
    pub persona_id: String,
    pub memory_written: usize,
    pub memory_total: usize,
    pub memory_errors: Vec<String>,
    pub profile_sync_error: Option<String>,
}

/// Result of a team snapshot import.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotImportResult {
    pub team: TeamRecord,
    pub persona_ids: Vec<String>,
    pub members: Vec<TeamSnapshotImportMemberResult>,
}

/// In-memory bytes for the native team sharing flow.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodedTeamSnapshotPayload {
    pub file_bytes: Vec<u8>,
    pub file_name: String,
}

fn import_backend_or_local(backend: Option<BackendKind>) -> BackendKind {
    backend.unwrap_or_default()
}

/// Per-member minted key material, assembled before entering the store lock.
struct MintedMember {
    definition: AgentDefinition,
    record: ManagedAgentRecord,
    agent_keys: nostr::Keys,
    pubkey: String,
    auth_tag: Option<String>,
    display_name: String,
    effective_avatar: Option<String>,
}

fn build_team_export_snapshot(
    team: &TeamRecord,
    personas: &[AgentDefinition],
    records: &[ManagedAgentRecord],
    memory_level: MemoryLevel,
    memory_entries_by_persona: &std::collections::HashMap<String, Vec<AgentSnapshotMemoryEntry>>,
) -> Result<TeamSnapshot, String> {
    let members = team
        .persona_ids
        .iter()
        .map(|id| {
            let persona = personas
                .iter()
                .find(|persona| persona.id == *id)
                .ok_or_else(|| {
                    format!("team {} references missing agent definition {id}", team.id)
                })?;

            // Find the team instance for this persona (if any).
            let instance = records.iter().find(|r| {
                r.team_id.as_deref() == Some(&team.id)
                    && r.persona_id.as_deref() == Some(&persona.id)
            });

            // Use memory only when we have a live instance and a non-None level.
            let (effective_level, entries) = match (instance, memory_level) {
                (Some(_), level) if level != MemoryLevel::None => {
                    let entries = memory_entries_by_persona
                        .get(&persona.id)
                        .cloned()
                        .unwrap_or_default();
                    (level, entries)
                }
                _ => (MemoryLevel::None, Vec::new()),
            };

            Ok(build_snapshot(
                &persona.clone().into_agent_record(),
                effective_level,
                entries,
                None,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(build_team_snapshot(team, members))
}

async fn materialize_team_snapshot_bytes(
    id: String,
    is_png: bool,
    memory_level: MemoryLevel,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EncodedTeamSnapshotPayload, String> {
    let effective_memory_level = memory_level;

    let (team, personas, records) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let team = load_teams(&app)?
            .into_iter()
            .find(|team| team.id == id)
            .ok_or_else(|| format!("team {id} not found"))?;
        let personas = load_personas(&app)?;
        let records = load_managed_agents(&app)?;
        (team, personas, records)
    };

    // Fetch memory for each member that has a live instance, outside the lock.
    let mut memory_entries_by_persona: std::collections::HashMap<
        String,
        Vec<AgentSnapshotMemoryEntry>,
    > = std::collections::HashMap::new();

    if effective_memory_level != MemoryLevel::None {
        for persona_id in &team.persona_ids {
            let instance = records.iter().find(|r| {
                r.team_id.as_deref() == Some(&team.id)
                    && r.persona_id.as_deref() == Some(persona_id.as_str())
            });
            if let Some(instance) = instance {
                let listing = crate::commands::engrams::get_agent_memory(
                    instance.pubkey.clone(),
                    app.clone(),
                    state.clone(),
                )
                .await?;
                let mut entries = Vec::new();
                if let Some(core) = listing.core {
                    entries.push(AgentSnapshotMemoryEntry {
                        slug: core.slug,
                        body: core.body,
                    });
                }
                if effective_memory_level == MemoryLevel::Everything {
                    for mem in listing.memories {
                        entries.push(AgentSnapshotMemoryEntry {
                            slug: mem.slug,
                            body: mem.body,
                        });
                    }
                }
                memory_entries_by_persona.insert(persona_id.clone(), entries);
            }
        }
    }

    let snapshot = build_team_export_snapshot(
        &team,
        &personas,
        &records,
        effective_memory_level,
        &memory_entries_by_persona,
    )?;
    let slug = crate::util::slugify(&team.name, "team", 50);
    let (file_bytes, file_name) = if is_png {
        let bytes = encode_team_snapshot_png(&snapshot)
            .map_err(|e| format!("Failed to encode .team.png: {e}"))?;
        if bytes.len() > MAX_TEAM_SNAPSHOT_PNG_BYTES {
            return Err(
                "Team snapshot exceeds the 50 MiB size limit for .team.png files.".to_string(),
            );
        }
        (bytes, format!("{slug}.team.png"))
    } else {
        let bytes = encode_team_snapshot_json(&snapshot)
            .map_err(|e| format!("Failed to encode .team.json: {e}"))?;
        if bytes.len() > MAX_TEAM_SNAPSHOT_JSON_BYTES {
            return Err(
                "Team snapshot exceeds the 25 MiB size limit for .team.json files.".to_string(),
            );
        }
        (bytes, format!("{slug}.team.json"))
    };
    Ok(EncodedTeamSnapshotPayload {
        file_bytes,
        file_name,
    })
}

/// Export a team snapshot with optional member memory.
#[tauri::command]
pub async fn export_team_snapshot(
    id: String,
    format: String,
    memory_level: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let is_png = parse_format_is_png(&format)?;
    let memory_level = parse_memory_level(&memory_level)?;
    let payload =
        materialize_team_snapshot_bytes(id, is_png, memory_level, app.clone(), state).await?;
    if is_png {
        save_bytes_with_dialog(
            &app,
            &payload.file_name,
            "PNG image",
            &["png"],
            &payload.file_bytes,
        )
        .await
    } else {
        save_bytes_with_dialog(
            &app,
            &payload.file_name,
            "Team snapshot",
            &["json"],
            &payload.file_bytes,
        )
        .await
    }
}

/// Encode a team snapshot for the native send flow without opening a dialog.
#[tauri::command]
pub async fn encode_team_snapshot_for_send(
    id: String,
    format: String,
    memory_level: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EncodedTeamSnapshotPayload, String> {
    let memory_level = parse_memory_level(&memory_level)?;
    materialize_team_snapshot_bytes(id, parse_format_is_png(&format)?, memory_level, app, state)
        .await
}

/// Decode a team snapshot into a confirmation preview without writing anything.
#[tauri::command]
pub async fn preview_team_snapshot_import(
    file_bytes: Vec<u8>,
    _file_name: String,
) -> Result<TeamSnapshotImportPreview, String> {
    tokio::task::spawn_blocking(move || {
        let snapshot = decode_team_snapshot_from_bytes(&file_bytes)?;
        let members: Vec<_> = snapshot.members.iter().map(member_preview).collect();
        Ok(TeamSnapshotImportPreview {
            name: snapshot.team.name,
            description: snapshot.team.description,
            instructions: snapshot.team.instructions,
            has_source_allowlist: members.iter().any(|member| member.has_source_allowlist),
            members,
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Import a team snapshot, minting full agent instances for every member.
///
/// Phase sequence:
///   1. Validate — decode the manifest and resolve all behavioral defaults.
///      Fail immediately if any member is invalid — zero writes.
///   2. Mint — generate a fresh keypair + NIP-OA auth tag for EVERY member.
///      If ANY generation fails, return immediately — zero writes.
///   3. Store — inside `managed_agents_store_lock`: write all `AgentDefinition`s
///      + all `ManagedAgentRecord`s (with `team_id` set) + `TeamRecord`.
///      Both store files are snapshotted (or noted absent) before the first
///      write. On any write error the pre-import state is restored — including
///      deleting a file that was absent, cleaning minted keyring entries, and
///      surfacing rollback failures alongside the original error. This makes
///      the store phase all-or-none for ordinary application errors; a process
///      crash between atomic file commits is NOT covered.
///   4. Profile sync — for each member, call `sync_managed_agent_profile`.
///      Best-effort; errors are collected per member.
///   5. Memory restore — for each member with non-empty snapshot memory,
///      publish each entry as a `kind:30174` engram event. Best-effort.
///
/// Importing the same file twice yields two distinct teams with different
/// agent keypairs (same as individual agent import).
#[tauri::command]
pub async fn confirm_team_snapshot_import(
    input: TeamSnapshotImportConfirm,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TeamSnapshotImportResult, String> {
    // ── Phase 1: validate (no I/O) ───────────────────────────────────────────
    let snapshot = decode_team_snapshot_from_bytes(&input.file_bytes)?;
    let import_backend = import_backend_or_local(input.backend);
    if let BackendKind::Provider { config, id } = &import_backend {
        crate::managed_agents::validate_provider_config(config)?;
        crate::managed_agents::resolve_provider_binary(id)?;
    }
    let now = now_iso();

    // Resolve behavioral defaults for every member before any key generation.
    let definitions = build_import_definitions(&snapshot, input.keep_allowlist, &now)?;
    let persona_ids: Vec<String> = definitions.iter().map(|d| d.id.clone()).collect();
    let imported_team = build_import_team(&snapshot, persona_ids.clone(), &now)?;

    // ── Phase 2: mint keys + auth tags (sync, outside lock) ─────────────────
    // All mints must succeed before we enter the store. If any fails, zero writes.
    let owner_pubkey_hex = {
        let keys = state.signing_keys()?;
        keys.public_key().to_hex()
    };

    let mut minted: Vec<MintedMember> = Vec::with_capacity(snapshot.members.len());
    for (member, definition) in snapshot.members.iter().zip(definitions) {
        let display_name = definition.display_name.clone();
        let effective_avatar_url = effective_avatar(member);
        let respond_to_wire = definition.respond_to.clone();
        let minted_parallelism = definition.parallelism;

        let (agent_keys, private_key_nsec, pubkey, auth_tag) = {
            let owner_keys = state.signing_keys()?;
            let agent_keys = nostr::Keys::generate();
            let pubkey = agent_keys.public_key().to_hex();
            let private_key_nsec = {
                use nostr::ToBech32;
                agent_keys
                    .secret_key()
                    .to_bech32()
                    .map_err(|e| format!("failed to encode agent private key: {e}"))?
            };
            // NIP-OA auth tag: bridge nostr 0.37 → 0.36 (buzz-sdk) via hex round-trip.
            let compat_owner = nostr::Keys::parse(&owner_keys.secret_key().to_secret_hex())
                .map_err(|e| format!("failed to bridge owner keys: {e}"))?;
            let compat_agent = nostr::PublicKey::from_hex(&pubkey)
                .map_err(|e| format!("failed to bridge agent pubkey: {e}"))?;
            let auth_tag = Some(
                buzz_sdk_pkg::nip_oa::compute_auth_tag(&compat_owner, &compat_agent, "")
                    .map_err(|e| format!("failed to compute NIP-OA auth tag: {e}"))?,
            );
            (agent_keys, private_key_nsec, pubkey, auth_tag)
        };

        // Build the ManagedAgentRecord for this member.
        let record = ManagedAgentRecord {
            pubkey: pubkey.clone(),
            name: display_name.clone(),
            display_name: None,
            slug: None,
            persona_id: Some(definition.id.clone()),
            private_key_nsec: private_key_nsec.clone(),
            auth_tag: auth_tag.clone(),
            relay_url: String::new(),
            avatar_url: effective_avatar_url.clone(),
            acp_command: crate::managed_agents::DEFAULT_ACP_COMMAND.to_string(),
            agent_command: String::new(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 0,
            idle_timeout_seconds: member.definition.idle_timeout_seconds,
            max_turn_duration_seconds: member.definition.max_turn_duration_seconds,
            parallelism: minted_parallelism
                .unwrap_or(crate::managed_agents::DEFAULT_AGENT_PARALLELISM),
            system_prompt: member.definition.system_prompt.clone(),
            model: member.definition.model.clone(),
            provider: member.definition.provider.clone(),
            persona_source_version: None,
            env_vars: std::collections::BTreeMap::new(),
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: import_backend.clone(),
            backend_agent_id: None,
            provider_binary_path: if let BackendKind::Provider { ref id, .. } = import_backend {
                crate::managed_agents::resolve_provider_binary(id)
                    .ok()
                    .map(|path| path.display().to_string())
            } else {
                None
            },
            team_id: Some(imported_team.id.clone()),
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: {
                use crate::managed_agents::RespondTo;
                respond_to_wire
                    .as_deref()
                    .map(RespondTo::parse_wire)
                    .transpose()?
                    .unwrap_or_default()
            },
            respond_to_allowlist: definition.respond_to_allowlist.clone(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            definition_respond_to: respond_to_wire.clone(),
            definition_respond_to_allowlist: definition.respond_to_allowlist.clone(),
            definition_parallelism: minted_parallelism,
            relay_mesh: None,
            runtime: member.definition.runtime.clone(),
            name_pool: member.definition.name_pool.clone(),
        };

        minted.push(MintedMember {
            definition,
            record,
            agent_keys,
            pubkey,
            auth_tag,
            display_name,
            effective_avatar: effective_avatar_url,
        });
    }

    // ── Phase 3: store (sync, inside lock) ──────────────────────────────────
    let team = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;

        // Guard against duplicate pubkeys (astronomically unlikely).
        let existing_records = load_managed_agents(&app)?;
        for m in &minted {
            if existing_records.iter().any(|r| r.pubkey == m.pubkey) {
                return Err(format!(
                    "generated pubkey {} already exists — retry",
                    m.pubkey
                ));
            }
        }

        // Snapshot both store files for rollback on partial write failure.
        // Distinguish "file exists with content" from "file absent" so rollback
        // can delete a file created by the import rather than leaving orphaned
        // records.
        let agents_store_path = crate::managed_agents::storage::managed_agents_store_path(&app)?;
        let agents_store_snapshot = match std::fs::read(&agents_store_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("failed to snapshot agent store: {e}")),
        };
        let teams_store_path = crate::managed_agents::teams_store_path(&app)?;
        let teams_store_snapshot = match std::fs::read(&teams_store_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("failed to snapshot teams store: {e}")),
        };

        // Pre-read teams via the read-only loader BEFORE any agent commits.
        // This avoids load_teams()'s write-on-load side effect (teams.rs:165-166
        // saves whenever the file is absent or built-ins changed). A failure here
        // aborts cleanly — zero writes have occurred.
        let mut teams = load_teams_readonly(&teams_store_path)?;

        // Collect minted pubkeys for keyring cleanup on rollback.
        let minted_pubkeys: Vec<&str> = minted.iter().map(|m| m.pubkey.as_str()).collect();

        // Restore the agent store to pre-import state and clean minted keyring
        // entries. Returns the original error, extended with rollback details.
        let rollback_agents = |original_err: String| -> String {
            let mut errors = vec![original_err];
            // Clean minted keyring entries.
            for pubkey in &minted_pubkeys {
                if let Err(e) = crate::managed_agents::storage::try_delete_agent_key(pubkey) {
                    errors.push(format!("keyring cleanup {pubkey}: {e}"));
                }
            }
            // Restore agent store file.
            let restore = match &agents_store_snapshot {
                Some(bytes) => crate::managed_agents::storage::atomic_write_json_restricted(
                    &agents_store_path,
                    bytes,
                ),
                None => match std::fs::remove_file(&agents_store_path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    other => other.map_err(|e| e.to_string()),
                },
            };
            if let Err(e) = restore {
                errors.push(format!("agent store restore: {e}"));
            }
            if errors.len() == 1 {
                errors.into_iter().next().unwrap()
            } else {
                errors.join("; ")
            }
        };

        // Write all definitions.
        let mut personas = load_personas(&app)?;
        for m in &minted {
            personas.push(m.definition.clone());
        }
        if let Err(e) = save_personas(&app, &personas) {
            return Err(rollback_agents(e));
        }

        // Write all managed-agent records.
        let mut records = existing_records;
        for m in &minted {
            records.push(m.record.clone());
        }
        if let Err(e) = save_managed_agents(&app, &records) {
            return Err(rollback_agents(e));
        }

        // Write the team record. `teams` was pre-loaded via the read-only
        // loader before any agent commits, so a read/parse failure already
        // aborted before any phase-3 write. save_teams sorts and persists.
        teams.push(imported_team.clone());
        if let Err(e) = save_teams(&app, &teams) {
            let err = rollback_agents(e);
            // Also restore teams store.
            let teams_restore = match &teams_store_snapshot {
                Some(bytes) => {
                    crate::managed_agents::storage::atomic_write_json(&teams_store_path, bytes)
                }
                None => match std::fs::remove_file(&teams_store_path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    other => other.map_err(|e| e.to_string()),
                },
            };
            return Err(match teams_restore {
                Ok(()) => err,
                Err(teams_err) => format!("{err}; teams store restore: {teams_err}"),
            });
        }

        // All writes committed — safe to update in-memory state.
        for m in &minted {
            crate::commands::personas::retain_persona_pending(&app, &state, &m.definition);
        }
        for m in &minted {
            retain_agent_pending(&app, &state, &m.record);
        }
        crate::commands::teams::retain_team_pending(&app, &state, &imported_team);

        crate::managed_agents::try_regenerate_nest(&app);
        let _ = app.emit("agents-data-changed", ());

        imported_team
    };

    // ── Phase 4-6: profile sync, provider deploy, memory restore ──────────
    let relay_ws = relay_ws_url_with_override(&state);
    let mut member_results: Vec<TeamSnapshotImportMemberResult> = Vec::with_capacity(minted.len());

    for (m, snap_member) in minted.iter().zip(snapshot.members.iter()) {
        let relay_url = effective_agent_relay_url(&m.record.relay_url, &relay_ws);

        // Phase 4: profile sync (best-effort).
        let profile_sync_error = sync_managed_agent_profile(
            &state,
            &relay_url,
            &m.agent_keys,
            &m.display_name,
            m.effective_avatar.as_deref(),
            m.auth_tag.as_deref(),
        )
        .await
        .err();

        // Phase 5: provider deploy for remote imports (best-effort).
        if let BackendKind::Provider { id, config } = &m.record.backend {
            let agent_json = {
                let _store_guard = state
                    .managed_agents_store_lock
                    .lock()
                    .map_err(|e| e.to_string())?;
                let records = load_managed_agents(&app)?;
                let record = records
                    .iter()
                    .find(|record| record.pubkey == m.pubkey)
                    .ok_or_else(|| format!("agent {} not found", m.pubkey))?;
                build_deploy_payload(&app, &state, record)?
            };
            let _ = deploy_to_provider(&app, &state, &m.pubkey, id, config, agent_json, None).await;
        }

        // Phase 6: memory restore (best-effort).
        let memory_total = snap_member.memory.entries.len();
        let mut memory_written = 0usize;
        let mut memory_errors: Vec<String> = Vec::new();

        if memory_total > 0 {
            let owner_pubkey = nostr::PublicKey::from_hex(&owner_pubkey_hex)
                .map_err(|e| format!("failed to parse owner pubkey: {e}"))?;
            let base_ts = nostr::Timestamp::now().as_secs();

            for (idx, entry) in snap_member.memory.entries.iter().enumerate() {
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
                match buzz_core_pkg::engram::build_event(
                    &m.agent_keys,
                    &owner_pubkey,
                    &body,
                    created_at,
                ) {
                    Ok(event) => {
                        use nostr::JsonUtil;
                        let event_json = event.as_json().into_bytes();
                        let url =
                            format!("{}/events", crate::relay::relay_http_base_url(&relay_url));
                        match submit_engram_event(
                            &state,
                            &m.agent_keys,
                            &event_json,
                            &url,
                            m.auth_tag.as_deref(),
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

        member_results.push(TeamSnapshotImportMemberResult {
            display_name: m.display_name.clone(),
            pubkey: m.pubkey.clone(),
            persona_id: m.definition.id.clone(),
            memory_written,
            memory_total,
            memory_errors,
            profile_sync_error,
        });
    }

    Ok(TeamSnapshotImportResult {
        team,
        persona_ids,
        members: member_results,
    })
}

/// Inline retention for the managed-agent kind:30177 event — mirrors
/// `commands::personas::snapshot::import::retain_agent_pending`.
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
        eprintln!("buzz-desktop: team-snapshot-import retain-agent: {e}");
    }
}

/// POST a pre-built signed engram event to the relay, authenticating as the
/// new agent. Mirrors the same helper in `snapshot::import`.
async fn submit_engram_event(
    state: &AppState,
    agent_keys: &nostr::Keys,
    event_json: &[u8],
    url: &str,
    auth_tag: Option<&str>,
) -> Result<(), String> {
    use crate::relay::build_nip98_auth_header_for_keys;
    use reqwest::Method;

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

#[cfg(test)]
mod tests;
