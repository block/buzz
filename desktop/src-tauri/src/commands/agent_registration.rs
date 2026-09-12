//! Register an independently operated agent under the current owner.
//!
//! This path publishes only the owner-authored kind:30177 directory/policy
//! record. It never generates, imports, or persists the agent's private key,
//! and it never creates a local managed-agent runtime record.

use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
use nostr::{JsonUtil, PublicKey};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        agent_events::{build_agent_event_from_content, ManagedAgentEventContent},
        persona_events::monotonic_created_at,
        retention::{
            get_retained_event, mark_synced, open_retention_db, retain_event, RetainedEvent,
            RetentionScope,
        },
        validate_respond_to_allowlist, RespondTo,
    },
    relay::{query_relay_at_with_keys, relay_http_base_url, submit_signed_event_at_with_keys},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterExistingAgentRequest {
    pub agent_pubkey: String,
    #[serde(default)]
    pub respond_to: RespondTo,
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExistingAgentPublicationStatus {
    Published,
    Queued,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterExistingAgentResult {
    pub agent_pubkey: String,
    pub display_name: String,
    pub publication_status: ExistingAgentPublicationStatus,
    pub already_registered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_message: Option<String>,
}

struct PreparedExistingAgentRegistration {
    scope: RetentionScope,
    event: nostr::Event,
    retained: RetainedEvent,
    display_name: String,
}

fn has_valid_existing_registration(
    events: &[nostr::Event],
    profile: &nostr::Event,
    agent_pubkey: &str,
    owner_pubkey: &str,
    display_name: &str,
    respond_to: RespondTo,
    respond_to_allowlist: &[String],
) -> bool {
    crate::nostr_convert::relay_agents_from_managed_agent_events(
        events,
        std::slice::from_ref(profile),
    )
    .iter()
    .any(|agent| {
        agent.pubkey == agent_pubkey
            && agent.owner_pubkey.as_deref() == Some(owner_pubkey)
            && agent.name == display_name
            && agent.respond_to == Some(respond_to)
            && agent.respond_to_allowlist == respond_to_allowlist
    })
}

fn normalize_registration_policy(
    respond_to: RespondTo,
    respond_to_allowlist: &[String],
) -> Result<(RespondTo, Vec<String>), String> {
    let normalized = validate_respond_to_allowlist(respond_to_allowlist)?;
    if respond_to == RespondTo::Allowlist && normalized.is_empty() {
        return Err("Selected people requires at least one 64-character public key.".to_string());
    }

    Ok((
        respond_to,
        if respond_to == RespondTo::Allowlist {
            normalized
        } else {
            Vec::new()
        },
    ))
}

fn ensure_not_locally_managed<'a>(
    managed_pubkeys: impl IntoIterator<Item = &'a str>,
    agent_pubkey: &str,
) -> Result<(), String> {
    if managed_pubkeys
        .into_iter()
        .any(|pubkey| pubkey.eq_ignore_ascii_case(agent_pubkey))
    {
        return Err(
            "This agent is already managed by this Desktop. Edit its access policy instead."
                .to_string(),
        );
    }
    Ok(())
}

fn latest_registration_created_at(
    events: &[nostr::Event],
    agent_pubkey: &str,
    owner_pubkey: &str,
) -> Option<i64> {
    events
        .iter()
        .filter(|event| {
            event.kind.as_u16() as u32 == KIND_MANAGED_AGENT
                && event.pubkey.to_hex() == owner_pubkey
                && event.tags.iter().any(|tag| {
                    let tag = tag.as_slice();
                    tag.first().map(String::as_str) == Some("d")
                        && tag.get(1).map(String::as_str) == Some(agent_pubkey)
                })
        })
        .map(|event| event.created_at.as_secs() as i64)
        .max()
}

fn normalize_agent_pubkey(value: &str) -> Result<String, String> {
    PublicKey::from_hex(value.trim())
        .map(|pubkey| pubkey.to_hex())
        .map_err(|_| "Enter a valid 64-character agent public key.".to_string())
}

fn verified_profile_name(
    profile: &nostr::Event,
    agent_pubkey: &str,
    owner_pubkey: &str,
) -> Result<String, String> {
    if profile.kind.as_u16() != 0 || profile.pubkey.to_hex() != agent_pubkey {
        return Err("The relay returned the wrong agent profile.".to_string());
    }
    let verified_owner = crate::nostr_convert::profile_valid_oa_owner_pubkey(profile)
        .ok_or_else(|| {
            "This profile does not contain a valid owner attestation. Ask the agent to publish its NIP-OA profile first."
                .to_string()
        })?;
    if verified_owner != owner_pubkey {
        return Err("This agent is attested to a different owner.".to_string());
    }

    let profile_info = crate::nostr_convert::profile_info_from_event(profile)?;
    let display_name = profile_info
        .display_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "The agent profile needs a display name before registration.".to_string())?;
    crate::managed_agents::validate_managed_agent_definition_text(&display_name, None, None)
        .map_err(|error| format!("The agent profile name is unsafe: {error}"))?;
    Ok(display_name)
}

fn prepare_registration_at(
    scope: RetentionScope,
    profile: &nostr::Event,
    agent_pubkey: &str,
    relay_created_at: Option<i64>,
    respond_to: RespondTo,
    respond_to_allowlist: Vec<String>,
) -> Result<PreparedExistingAgentRegistration, String> {
    let owner_pubkey = scope.owner_keys.public_key().to_hex();
    let display_name = verified_profile_name(profile, agent_pubkey, &owner_pubkey)?;
    let content = ManagedAgentEventContent {
        name: display_name.clone(),
        persona_id: None,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        parallelism: 1,
        respond_to,
        respond_to_allowlist,
    };

    let conn = open_retention_db(&scope.db_path)?;
    let existing = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner_pubkey, agent_pubkey)?;
    let previous_created_at = existing
        .as_ref()
        .map(|row| row.created_at)
        .into_iter()
        .chain(relay_created_at)
        .max();
    let event = build_agent_event_from_content(agent_pubkey, &content)?
        .custom_created_at(monotonic_created_at(previous_created_at))
        .sign_with_keys(&scope.owner_keys)
        .map_err(|error| format!("failed to sign existing-agent registration: {error}"))?;
    let retained = RetainedEvent {
        kind: KIND_MANAGED_AGENT,
        pubkey: owner_pubkey,
        d_tag: agent_pubkey.to_string(),
        content: event.content.clone(),
        created_at: event.created_at.as_secs() as i64,
        raw_event: event.as_json(),
        pending_sync: true,
    };
    retain_event(&conn, &retained)
        .map_err(|error| format!("failed to queue existing-agent registration: {error}"))?;

    Ok(PreparedExistingAgentRegistration {
        scope,
        event,
        retained,
        display_name,
    })
}

#[tauri::command]
pub async fn register_existing_agent(
    input: RegisterExistingAgentRequest,
    app: AppHandle,
) -> Result<RegisterExistingAgentResult, String> {
    let agent_pubkey = normalize_agent_pubkey(&input.agent_pubkey)?;
    let (respond_to, respond_to_allowlist) =
        normalize_registration_policy(input.respond_to, &input.respond_to_allowlist)?;
    let state = app.state::<AppState>();
    let scope = crate::managed_agents::retention::active_retention_scope(&app, &state)?;
    let owner_pubkey = scope.owner_keys.public_key().to_hex();
    let api_base_url = relay_http_base_url(&scope.relay_url);
    let events = query_relay_at_with_keys(
        &state,
        &api_base_url,
        &[
            serde_json::json!({
                "kinds": [0],
                "authors": [&agent_pubkey],
                "limit": 1,
            }),
            serde_json::json!({
                "kinds": [KIND_MANAGED_AGENT],
                "authors": [&owner_pubkey],
                "#d": [&agent_pubkey],
                "limit": 1,
            }),
        ],
        &scope.owner_keys,
        None,
    )
    .await?;

    let profile = events
        .iter()
        .find(|event| event.kind.as_u16() == 0 && event.pubkey.to_hex() == agent_pubkey)
        .cloned()
        .ok_or_else(|| {
            "No profile was found for this agent. Ask the agent to publish its profile first."
                .to_string()
        })?;
    let display_name = verified_profile_name(&profile, &agent_pubkey, &owner_pubkey)?;

    if has_valid_existing_registration(
        &events,
        &profile,
        &agent_pubkey,
        &owner_pubkey,
        &display_name,
        respond_to,
        &respond_to_allowlist,
    ) {
        return Ok(RegisterExistingAgentResult {
            agent_pubkey,
            display_name,
            publication_status: ExistingAgentPublicationStatus::Published,
            already_registered: true,
            relay_message: None,
        });
    }

    let relay_created_at = latest_registration_created_at(&events, &agent_pubkey, &owner_pubkey);

    let prepared = tokio::task::spawn_blocking({
        let app = app.clone();
        let agent_pubkey = agent_pubkey.clone();
        let respond_to_allowlist = respond_to_allowlist.clone();
        move || {
            let state = app.state::<AppState>();
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let managed_agents = crate::managed_agents::load_managed_agents(&app)?;
            ensure_not_locally_managed(
                managed_agents.iter().map(|record| record.pubkey.as_str()),
                &agent_pubkey,
            )?;
            prepare_registration_at(
                scope,
                &profile,
                &agent_pubkey,
                relay_created_at,
                respond_to,
                respond_to_allowlist,
            )
        }
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))??;

    let publish_result = submit_signed_event_at_with_keys(
        &prepared.event,
        &state,
        &api_base_url,
        &prepared.scope.owner_keys,
    )
    .await;

    match publish_result {
        Ok(_) => {
            let conn = open_retention_db(&prepared.scope.db_path)?;
            mark_synced(
                &conn,
                prepared.retained.kind,
                &prepared.retained.pubkey,
                &prepared.retained.d_tag,
                prepared.retained.created_at,
                &prepared.retained.content,
            )?;
            let _ = app.emit("agents-data-changed", ());
            Ok(RegisterExistingAgentResult {
                agent_pubkey,
                display_name: prepared.display_name,
                publication_status: ExistingAgentPublicationStatus::Published,
                already_registered: false,
                relay_message: None,
            })
        }
        Err(error) => Ok(RegisterExistingAgentResult {
            agent_pubkey,
            display_name: prepared.display_name,
            publication_status: ExistingAgentPublicationStatus::Queued,
            already_registered: false,
            relay_message: Some(error),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};

    fn profile_for(owner: &nostr::Keys, agent: &nostr::Keys, name: &str) -> nostr::Event {
        let compat_owner = nostr::Keys::parse(&owner.secret_key().to_secret_hex()).unwrap();
        let compat_agent = nostr::PublicKey::from_hex(&agent.public_key().to_hex()).unwrap();
        let auth_json =
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&compat_owner, &compat_agent, "").unwrap();
        let auth_parts: Vec<String> = serde_json::from_str(&auth_json).unwrap();
        let auth_tag = Tag::parse(auth_parts).unwrap();
        EventBuilder::new(
            Kind::Metadata,
            serde_json::json!({"display_name": name}).to_string(),
        )
        .tags([auth_tag])
        .sign_with_keys(agent)
        .unwrap()
    }

    fn scope(dir: &std::path::Path, owner: nostr::Keys) -> RetentionScope {
        RetentionScope {
            db_path: dir.join("retention.db"),
            relay_url: "ws://relay.invalid".to_string(),
            owner_keys: owner,
        }
    }

    #[test]
    fn registration_uses_existing_pubkey_and_owner_signature_without_local_key_material() {
        let dir = tempfile::tempdir().unwrap();
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let viewer = nostr::Keys::generate().public_key().to_hex();
        let agent_pubkey = agent.public_key().to_hex();
        let profile = profile_for(&owner, &agent, "Tess");

        let prepared = prepare_registration_at(
            scope(dir.path(), owner.clone()),
            &profile,
            &agent_pubkey,
            None,
            RespondTo::Allowlist,
            vec![owner.public_key().to_hex(), viewer.clone()],
        )
        .unwrap();

        assert_eq!(prepared.event.pubkey, owner.public_key());
        assert_eq!(prepared.retained.d_tag, agent_pubkey);
        assert!(prepared.event.tags.iter().any(|tag| {
            let tag = tag.as_slice();
            tag.first().map(String::as_str) == Some("d")
                && tag.get(1).map(String::as_str) == Some(prepared.retained.d_tag.as_str())
        }));
        let content: serde_json::Value = serde_json::from_str(&prepared.event.content).unwrap();
        assert_eq!(content["name"], "Tess");
        assert_eq!(content["respond_to"], "allowlist");
        assert_eq!(
            content["respond_to_allowlist"],
            serde_json::json!([owner.public_key().to_hex(), viewer])
        );
        assert_eq!(content["parallelism"], 1);
        assert!(content.get("private_key_nsec").is_none());
        assert!(content.get("auth_tag").is_none());
        assert!(content.get("env_vars").is_none());
    }

    #[test]
    fn registration_rejects_an_agent_attested_to_another_owner() {
        let dir = tempfile::tempdir().unwrap();
        let owner = nostr::Keys::generate();
        let other_owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let profile = profile_for(&other_owner, &agent, "Wrong owner");

        let error = prepare_registration_at(
            scope(dir.path(), owner),
            &profile,
            &agent.public_key().to_hex(),
            None,
            RespondTo::OwnerOnly,
            Vec::new(),
        )
        .err()
        .unwrap();

        assert_eq!(error, "This agent is attested to a different owner.");
        assert!(!dir.path().join("retention.db").exists());
    }

    #[test]
    fn registration_rejects_an_agent_without_an_owner_attestation() {
        let dir = tempfile::tempdir().unwrap();
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let profile = EventBuilder::new(
            Kind::Metadata,
            serde_json::json!({"display_name": "Unattested"}).to_string(),
        )
        .sign_with_keys(&agent)
        .unwrap();

        let error = prepare_registration_at(
            scope(dir.path(), owner),
            &profile,
            &agent.public_key().to_hex(),
            None,
            RespondTo::OwnerOnly,
            Vec::new(),
        )
        .err()
        .unwrap();

        assert_eq!(
            error,
            "This profile does not contain a valid owner attestation. Ask the agent to publish its NIP-OA profile first."
        );
        assert!(!dir.path().join("retention.db").exists());
    }

    #[test]
    fn registration_rejects_malformed_pubkeys_before_network_or_storage() {
        assert_eq!(
            normalize_agent_pubkey("not-a-pubkey").unwrap_err(),
            "Enter a valid 64-character agent public key."
        );
    }

    #[test]
    fn registration_policy_normalizes_allowlist_and_requires_a_selected_person() {
        let upper = "A".repeat(64);
        let lower = "a".repeat(64);
        assert_eq!(
            normalize_registration_policy(
                RespondTo::Allowlist,
                &[format!(" {upper} "), lower.clone()],
            )
            .unwrap(),
            (RespondTo::Allowlist, vec![lower])
        );
        assert_eq!(
            normalize_registration_policy(RespondTo::Allowlist, &[]).unwrap_err(),
            "Selected people requires at least one 64-character public key."
        );
    }

    #[test]
    fn registration_policy_clears_irrelevant_allowlist_entries() {
        let pubkey = "a".repeat(64);
        assert_eq!(
            normalize_registration_policy(RespondTo::OwnerOnly, std::slice::from_ref(&pubkey))
                .unwrap(),
            (RespondTo::OwnerOnly, Vec::new())
        );
        assert_eq!(
            normalize_registration_policy(RespondTo::Anyone, &[pubkey]).unwrap(),
            (RespondTo::Anyone, Vec::new())
        );
    }

    #[test]
    fn registration_rejects_a_pubkey_already_managed_by_this_desktop() {
        let managed_pubkey = "a".repeat(64);
        assert_eq!(
            ensure_not_locally_managed([managed_pubkey.as_str()], &managed_pubkey).unwrap_err(),
            "This agent is already managed by this Desktop. Edit its access policy instead."
        );
        assert!(ensure_not_locally_managed([managed_pubkey.as_str()], &"b".repeat(64),).is_ok());
    }

    #[test]
    fn existing_registration_requires_the_exact_agent_coordinate() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let other_agent = nostr::Keys::generate();
        let profile = profile_for(&owner, &agent, "Tess");
        let content = ManagedAgentEventContent {
            name: "Other agent".to_string(),
            persona_id: None,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            parallelism: 1,
            respond_to: RespondTo::OwnerOnly,
            respond_to_allowlist: Vec::new(),
        };
        let wrong_coordinate =
            build_agent_event_from_content(&other_agent.public_key().to_hex(), &content)
                .unwrap()
                .sign_with_keys(&owner)
                .unwrap();

        assert!(!has_valid_existing_registration(
            &[wrong_coordinate],
            &profile,
            &agent.public_key().to_hex(),
            &owner.public_key().to_hex(),
            "Tess",
            RespondTo::OwnerOnly,
            &[],
        ));
    }

    #[test]
    fn existing_registration_requires_requested_policy_allowlist_and_current_name() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let agent_pubkey = agent.public_key().to_hex();
        let owner_pubkey = owner.public_key().to_hex();
        let profile = profile_for(&owner, &agent, "Tess");

        let content = ManagedAgentEventContent {
            name: "Tess".to_string(),
            persona_id: None,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            parallelism: 1,
            respond_to: RespondTo::OwnerOnly,
            respond_to_allowlist: Vec::new(),
        };
        let owner_only = build_agent_event_from_content(&agent_pubkey, &content)
            .unwrap()
            .sign_with_keys(&owner)
            .unwrap();

        assert!(has_valid_existing_registration(
            std::slice::from_ref(&owner_only),
            &profile,
            &agent_pubkey,
            &owner_pubkey,
            "Tess",
            RespondTo::OwnerOnly,
            &[],
        ));
        assert!(!has_valid_existing_registration(
            std::slice::from_ref(&owner_only),
            &profile,
            &agent_pubkey,
            &owner_pubkey,
            "Renamed Tess",
            RespondTo::OwnerOnly,
            &[],
        ));

        let anyone = ManagedAgentEventContent {
            respond_to: RespondTo::Anyone,
            ..content.clone()
        };
        let anyone = build_agent_event_from_content(&agent_pubkey, &anyone)
            .unwrap()
            .sign_with_keys(&owner)
            .unwrap();
        assert!(!has_valid_existing_registration(
            &[anyone],
            &profile,
            &agent_pubkey,
            &owner_pubkey,
            "Tess",
            RespondTo::OwnerOnly,
            &[],
        ));

        let allowlisted_pubkey = nostr::Keys::generate().public_key().to_hex();
        let allowlist = ManagedAgentEventContent {
            respond_to: RespondTo::Allowlist,
            respond_to_allowlist: vec![allowlisted_pubkey.clone()],
            ..content
        };
        let allowlist = build_agent_event_from_content(&agent_pubkey, &allowlist)
            .unwrap()
            .sign_with_keys(&owner)
            .unwrap();
        assert!(has_valid_existing_registration(
            &[allowlist],
            &profile,
            &agent_pubkey,
            &owner_pubkey,
            "Tess",
            RespondTo::Allowlist,
            &[allowlisted_pubkey],
        ));
    }

    #[test]
    fn registration_supersedes_a_future_relay_head() {
        let dir = tempfile::tempdir().unwrap();
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let agent_pubkey = agent.public_key().to_hex();
        let profile = profile_for(&owner, &agent, "Tess");
        let future_created_at = nostr::Timestamp::now().as_secs() as i64 + 3_600;

        let prepared = prepare_registration_at(
            scope(dir.path(), owner),
            &profile,
            &agent_pubkey,
            Some(future_created_at),
            RespondTo::OwnerOnly,
            Vec::new(),
        )
        .unwrap();

        assert!(prepared.event.created_at.as_secs() as i64 > future_created_at);
    }
}
