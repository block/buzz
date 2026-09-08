//! Deliberate local credential provisioning, separate from mint/import snapshots.
use crate::{app_state::AppState, managed_agents as agents, nostr_convert, relay};
use nostr::{Event, Keys, ToBech32};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};
use zeroize::Zeroize;

/// Explicit user-supplied secret and selected owner/community/agent coordinate.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddExistingAgentRequest {
    owner: String,
    community: String,
    pubkey: String,
    private_key: String,
}

impl Drop for AddExistingAgentRequest {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

fn check_scope(state: &AppState, input: &AddExistingAgentRequest) -> Result<(), String> {
    if state.signing_keys()?.public_key().to_hex() != input.owner
        || relay::relay_ws_url_with_override(state).trim_end_matches('/') != input.community
    {
        return Err("Identity or community changed; reopen Add existing agent".into());
    }
    Ok(())
}

fn verified_record(
    input: &AddExistingAgentRequest,
    profiles: &[Event],
    policies: &[Event],
) -> Result<agents::ManagedAgentRecord, String> {
    // Parse only a secret key (not Keys::parse's public-key interpretations).
    let secret = nostr::SecretKey::parse(input.private_key.trim())
        .map_err(|_| "Invalid agent private key")?;
    let keys = Keys::new(secret);
    if keys.public_key().to_hex() != input.pubkey {
        return Err("Private key does not match the selected agent".into());
    }
    if nostr_convert::verified_agent_owners_from_profiles(profiles).get(&input.pubkey)
        != Some(&input.owner)
    {
        return Err("Selected profile is not verified as owned by your identity".into());
    }
    let policy = policies
        .iter()
        .filter(|event| {
            event.kind == nostr::Kind::Custom(30177)
                && event.pubkey.to_hex() == input.owner
                && event
                    .tags
                    .iter()
                    .filter(|tag| tag.as_slice().first().is_some_and(|v| v == "d"))
                    .map(|tag| tag.as_slice())
                    .collect::<Vec<_>>()
                    == vec![&["d".to_string(), input.pubkey.clone()][..]]
        })
        .max_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| b.id.cmp(&a.id))
        })
        .ok_or("Owner-signed agent profile is unavailable")?;
    policy
        .verify()
        .map_err(|_| "Invalid owner-signed agent profile")?;
    let content = agents::agent_events::managed_agent_content_from_event(policy)
        .map_err(|_| "Invalid owner-signed agent profile")?;
    let persona = content
        .persona_id
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .ok_or("The existing agent must have a linked persona")?;
    agents::validate_managed_agent_definition_text(&content.name, Some(persona), None)?;
    if !(1..=32).contains(&content.parallelism) {
        return Err("Invalid agent parallelism".into());
    }
    let allowlist = agents::validate_respond_to_allowlist(&content.respond_to_allowlist)?;
    if content.respond_to == agents::RespondTo::Allowlist && allowlist.is_empty() {
        return Err("Invalid agent access policy".into());
    }
    let avatar_url = profiles
        .iter()
        .filter(|p| p.pubkey.to_hex() == input.pubkey)
        .max_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| b.id.cmp(&a.id))
        })
        .and_then(|p| serde_json::from_str::<serde_json::Value>(&p.content).ok())
        .and_then(|p| p.get("picture").and_then(|v| v.as_str()).map(str::to_owned));
    let now = chrono::Utc::now().to_rfc3339();
    // Local execution configuration is deliberately unset. The SAME persona
    // supplies defaults at normal launch; no source runtime or credentials move.
    serde_json::from_value(serde_json::json!({
        "pubkey": input.pubkey, "name": content.name, "persona_id": persona,
        "avatar_url": avatar_url,
        "private_key_nsec": keys.secret_key().to_bech32().map_err(|_| "Cannot encode agent key")?,
        "relay_url": input.community, "acp_command": agents::DEFAULT_ACP_COMMAND, "agent_command": "",
        "agent_args": [], "mcp_command": "", "turn_timeout_seconds": 0,
        "system_prompt": null, "parallelism": content.parallelism,
        "respond_to": content.respond_to, "respond_to_allowlist": allowlist,
        "start_on_app_launch": false, "auto_restart_on_config_change": false,
        "created_at": now, "updated_at": now, "last_started_at": null,
        "last_stopped_at": null, "last_exit_code": null, "last_error": null
    }))
    .map_err(|_| "Cannot prepare local agent identity".into())
}

/// Add the exact owned relay identity without minting, publishing, or starting it.
#[tauri::command]
pub async fn add_existing_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    input: AddExistingAgentRequest,
) -> Result<(), String> {
    // Workspace apply holds this same async authority. Identity can change
    // during queries; it is revalidated under identity_mutation before writing.
    if input.pubkey.len() != 64
        || input.owner.len() != 64
        || input.private_key.len() > 128
        || input.community.len() > 2048
    {
        return Err("Invalid existing-agent input".into());
    }
    let _workspace = state.workspace_apply_lock.lock().await;
    check_scope(&state, &input)?;
    let profiles = relay::query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0], "authors": [&input.pubkey], "limit": 1
        })],
    )
    .await
    .map_err(|_| "Cannot fetch the existing agent profile")?;
    let policies = relay::query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [30177], "authors": [&input.owner], "#d": [&input.pubkey], "limit": 1
        })],
    )
    .await
    .map_err(|_| "Cannot fetch the owner-signed agent profile")?;
    let record = verified_record(&input, &profiles, &policies)?;
    commit_verified(&app, &state, &input, record)
}

fn commit_verified<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    input: &AddExistingAgentRequest,
    mut record: agents::ManagedAgentRecord,
) -> Result<(), String> {
    let _identity = state
        .identity_mutation
        .lock()
        .map_err(|_| "Identity lock unavailable")?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|_| "Agent store unavailable")?;
    check_scope(state, input)?;
    let personas: Vec<_> = agents::storage::load_agent_definitions(app)?
        .iter()
        .filter_map(|record| record.to_definition_view())
        .collect();
    if !personas
        .iter()
        .any(|p| Some(&p.id) == record.persona_id.as_ref() && p.is_active)
    {
        return Err("The linked persona must already be available on this Desktop".into());
    }
    if let Some(saved) = agents::load_managed_agents(app)?
        .iter()
        .find(|r| r.pubkey == record.pubkey)
    {
        if saved.persona_id != record.persona_id
            || (!saved.relay_url.is_empty() && saved.relay_url != record.relay_url)
        {
            return Err("Existing agent profile or community does not match".into());
        }
        // The saved owner link must be usable by the exact current owner
        // BEFORE any duplicate or repair effect. Launch preparation enforces
        // this same verifier, so an absent/foreign/invalid attestation is
        // refused here instead of reporting a healthy duplicate or a repaired
        // key for a record that later refuses to run as unowned. No write, no
        // silent re-attestation or owner migration.
        agents::runtime_configurations::verify_owner(saved, &input.owner).map_err(|_| {
            "Saved agent ownership is missing, invalid, or belongs to a different identity"
                .to_string()
        })?;
        if nostr::SecretKey::parse(&saved.private_key_nsec)
            .ok()
            .is_some_and(|key| Keys::new(key).public_key().to_hex() == input.pubkey)
        {
            return Ok(()); // Healthy duplicate: zero writes, no lifecycle changes.
        }
    }
    let owner = state.signing_keys()?;
    let compat_owner = nostr::Keys::parse(&owner.secret_key().to_secret_hex())
        .map_err(|_| "Cannot attest agent ownership")?;
    let agent =
        nostr::PublicKey::from_hex(&input.pubkey).map_err(|_| "Invalid agent public key")?;
    record.auth_tag = Some(
        buzz_sdk_pkg::nip_oa::compute_auth_tag(&compat_owner, &agent, "")
            .map_err(|_| "Cannot attest agent ownership")?,
    );
    agents::storage::import_existing_agent_key(app, record)
        .map_err(|_| "Could not save local agent identity; retry Add existing agent")?;
    let _ = app.emit("agents-data-changed", ());
    Ok(())
}

#[cfg(test)]
mod tests;
