use nostr::ToBech32;
use serde::Serialize;
use serde_json::Value;
use tauri::State;
use zeroize::Zeroize;

use crate::{
    app_state::{keyring_service, AppState},
    commands::identity_archive::{extract_oa_owner, fetch_kind0},
    events,
    managed_agents::persona_events::monotonic_created_at,
    models::ProfileInfo,
    nostr_convert,
    relay::{query_relay, submit_event_with_keys},
    secret_store::SecretStore,
};

const EXTERNAL_AGENT_KEY_PREFIX: &str = "external-agent-nsec:";

#[derive(Debug, Serialize)]
pub struct ExternalAgentIdentityStatus {
    pub linked: bool,
}

fn normalize_pubkey(pubkey: &str) -> Result<String, String> {
    let normalized = pubkey.trim().to_ascii_lowercase();
    nostr::PublicKey::from_hex(&normalized)
        .map_err(|_| "invalid external agent public key".to_string())?;
    Ok(normalized)
}

fn external_agent_key(pubkey: &str) -> String {
    format!("{EXTERNAL_AGENT_KEY_PREFIX}{pubkey}")
}

fn verify_agent_owner(
    event: &nostr::Event,
    expected_owner: &nostr::PublicKey,
) -> Result<(), String> {
    let Some((owner, _)) = extract_oa_owner(event) else {
        return Err("the agent profile has no verified owner attestation".to_string());
    };
    if !owner.eq_ignore_ascii_case(&expected_owner.to_hex()) {
        return Err("this Buzz identity does not own the external agent".to_string());
    }
    Ok(())
}

fn build_owner_auth_tag(
    owner_keys: &nostr::Keys,
    agent_pubkey: &nostr::PublicKey,
) -> Result<(String, nostr::Tag), String> {
    let auth_json = buzz_sdk_pkg::nip_oa::compute_auth_tag(owner_keys, agent_pubkey, "")
        .map_err(|e| format!("failed to build owner attestation: {e}"))?;
    let parts: Vec<String> = serde_json::from_str(&auth_json)
        .map_err(|e| format!("failed to parse owner attestation: {e}"))?;
    let tag =
        nostr::Tag::parse(parts).map_err(|e| format!("failed to encode owner attestation: {e}"))?;
    Ok((auth_json, tag))
}

#[tauri::command]
pub fn get_external_agent_identity_status(
    pubkey: String,
) -> Result<ExternalAgentIdentityStatus, String> {
    let pubkey = normalize_pubkey(&pubkey)?;
    let linked = SecretStore::shared(keyring_service())
        .load(&external_agent_key(&pubkey))?
        .is_some();
    Ok(ExternalAgentIdentityStatus { linked })
}

#[tauri::command]
pub async fn link_external_agent_identity(
    pubkey: String,
    mut nsec: String,
    state: State<'_, AppState>,
) -> Result<ExternalAgentIdentityStatus, String> {
    let pubkey = normalize_pubkey(&pubkey)?;
    let result = async {
        let agent_keys =
            nostr::Keys::parse(nsec.trim()).map_err(|_| "invalid private key".to_string())?;
        if agent_keys.public_key().to_hex() != pubkey {
            return Err("the private key does not belong to this agent".to_string());
        }

        let owner_keys = state.signing_keys()?;
        let event = fetch_kind0(&state, &pubkey)
            .await?
            .ok_or_else(|| "the agent has no published Buzz profile".to_string())?;
        verify_agent_owner(&event, &owner_keys.public_key())?;

        SecretStore::shared(keyring_service()).store(
            &external_agent_key(&pubkey),
            &agent_keys
                .secret_key()
                .to_bech32()
                .map_err(|e| e.to_string())?,
        )?;

        Ok(ExternalAgentIdentityStatus { linked: true })
    }
    .await;
    nsec.zeroize();
    result
}

#[tauri::command]
pub async fn update_external_agent_profile(
    pubkey: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    about: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProfileInfo, String> {
    let pubkey = normalize_pubkey(&pubkey)?;
    let stored_nsec = SecretStore::shared(keyring_service())
        .load(&external_agent_key(&pubkey))?
        .ok_or_else(|| "link this external agent before editing its profile".to_string())?;
    let agent_keys = nostr::Keys::parse(stored_nsec.trim())
        .map_err(|_| "the linked private key is invalid".to_string())?;
    if agent_keys.public_key().to_hex() != pubkey {
        return Err("the linked private key no longer matches this agent".to_string());
    }

    let owner_keys = state.signing_keys()?;
    let prior_event = fetch_kind0(&state, &pubkey)
        .await?
        .ok_or_else(|| "the agent has no published Buzz profile".to_string())?;
    verify_agent_owner(&prior_event, &owner_keys.public_key())?;

    let current: Value = serde_json::from_str(&prior_event.content).unwrap_or(Value::Null);
    let dn = display_name
        .as_deref()
        .or_else(|| current.get("display_name").and_then(Value::as_str));
    let name = current.get("name").and_then(Value::as_str);
    let picture = avatar_url
        .as_deref()
        .or_else(|| current.get("picture").and_then(Value::as_str));
    let about = about
        .as_deref()
        .or_else(|| current.get("about").and_then(Value::as_str));
    let nip05 = current.get("nip05").and_then(Value::as_str);
    let (auth_json, auth_tag) = build_owner_auth_tag(&owner_keys, &agent_keys.public_key())?;

    let builder = events::build_profile(dn, name, picture, about, nip05)?
        .tags([auth_tag])
        .custom_created_at(monotonic_created_at(Some(
            prior_event.created_at.as_secs() as i64
        )));
    submit_event_with_keys(builder, &state, &agent_keys, Some(&auth_json)).await?;

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [pubkey.clone()],
            "limit": 1
        })],
    )
    .await?;
    Ok(events
        .first()
        .map(nostr_convert::profile_info_from_event)
        .transpose()?
        .unwrap_or_else(|| ProfileInfo {
            pubkey,
            display_name: dn.map(str::to_string),
            avatar_url: picture.map(str::to_string),
            about: about.map(str::to_string),
            nip05_handle: nip05.map(str::to_string),
            owner_pubkey: Some(owner_keys.public_key().to_hex()),
            has_profile_event: true,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_agent_secret_key_is_namespaced_by_normalized_pubkey() {
        let pubkey = "01".repeat(32);
        assert_eq!(
            external_agent_key(&pubkey),
            format!("external-agent-nsec:{pubkey}")
        );
    }

    #[test]
    fn rejects_non_pubkeys_without_echoing_input() {
        let error = normalize_pubkey("not-a-key").unwrap_err();
        assert_eq!(error, "invalid external agent public key");
    }
}
