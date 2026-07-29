use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    app_state::{AppState, ManagedAgentPersonaMintReservation},
    managed_agents::{
        agent_events::ManagedAgentEventContent,
        ensure_persona_is_active, load_personas,
        retention::{
            active_retention_scope, get_retained_events, open_retention_db,
            retained_tombstone_covers,
        },
        ManagedAgentRecord,
    },
};

/// Public, non-runnable identity link learned from owner-authored kind:30177.
///
/// A secondary install deliberately does not receive the agent's secret key,
/// but it still needs this association to avoid minting a duplicate body for
/// an account-scoped definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedAgentReference {
    pub pubkey: String,
    pub name: String,
    pub persona_id: String,
}

#[tauri::command]
pub async fn list_managed_agent_references(
    app: AppHandle,
) -> Result<Vec<ManagedAgentReference>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let scope = active_retention_scope(&app, &state)?;
        let owner_pubkey = scope.owner_keys.public_key().to_hex();
        let conn = open_retention_db(&scope.db_path)?;
        load_live_managed_agent_references(&conn, &owner_pubkey)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

pub(crate) fn prepare_managed_agent_key_mint<'a>(
    app: &AppHandle,
    state: &'a AppState,
    records: &[ManagedAgentRecord],
    persona_id: Option<&str>,
) -> Result<(nostr::Keys, Option<ManagedAgentPersonaMintReservation<'a>>), String> {
    let Some(persona_id) = persona_id else {
        return Ok((nostr::Keys::generate(), None));
    };
    let reservation = state.reserve_managed_agent_persona_mint(persona_id.to_string())?;

    ensure_persona_is_active(&load_personas(app)?, persona_id)?;
    let scope = active_retention_scope(app, state)?;
    let reference_sync_ready = state.managed_agent_reference_sync_is_ready(&scope.db_path)?;
    let owner_pubkey = scope.owner_keys.public_key().to_hex();
    let conn = open_retention_db(&scope.db_path)?;
    let mut identities = records
        .iter()
        .filter_map(|record| {
            record
                .persona_id
                .as_ref()
                .map(|persona_id| ManagedAgentReference {
                    pubkey: record.pubkey.clone(),
                    name: record.name.clone(),
                    persona_id: persona_id.clone(),
                })
        })
        .collect::<Vec<_>>();
    identities.extend(load_live_managed_agent_references(&conn, &owner_pubkey)?);
    let keys = generate_managed_agent_keys(
        Some(persona_id),
        reference_sync_ready,
        &identities,
        nostr::Keys::generate,
    )?;
    Ok((keys, Some(reservation)))
}

/// Mark an exact relay+owner retention scope safe for persona-backed key
/// generation. The frontend calls this only after its historical 30177/5
/// backfill has been durably reconciled. Both coordinates are checked so a
/// cancelled sync cannot mark a newly selected identity on the same relay.
#[tauri::command]
pub fn mark_managed_agent_reference_sync_ready(
    owner_pubkey: String,
    arrival_relay_url: String,
    app: AppHandle,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(scope) = crate::managed_agents::retention::arrival_retention_scope(
        &app,
        &state,
        &arrival_relay_url,
    )?
    else {
        return Err("managed-agent identity sync scope changed before completion".to_string());
    };
    if !scope
        .owner_keys
        .public_key()
        .to_hex()
        .eq_ignore_ascii_case(owner_pubkey.trim())
    {
        return Err("managed-agent identity sync owner changed before completion".to_string());
    }
    state.mark_managed_agent_reference_sync_ready(scope.db_path)
}

pub(crate) fn load_live_managed_agent_references(
    conn: &rusqlite::Connection,
    owner_pubkey: &str,
) -> Result<Vec<ManagedAgentReference>, String> {
    let retained =
        get_retained_events(conn, buzz_core_pkg::kind::KIND_MANAGED_AGENT, owner_pubkey)?;
    let mut live = Vec::new();
    for event in retained {
        if !retained_tombstone_covers(
            conn,
            buzz_core_pkg::kind::KIND_MANAGED_AGENT,
            owner_pubkey,
            &event.d_tag,
            event.created_at,
        )? {
            live.push((event.d_tag, event.content));
        }
    }
    references_from_retained(live)
}

/// The key-generation boundary for persona-backed agents.
///
/// `reference_sync_ready` distinguishes a genuinely empty retained identity
/// index from one that has not caught up yet. `identities` contains both local
/// runnable records and account-scoped non-runnable references. Any existing
/// link blocks the factory entirely: the invariant is one persona id, one
/// keypair, independent of relay event ordering or which duplicate happened to
/// remain live in a previous failure.
pub(crate) fn generate_managed_agent_keys<F>(
    persona_id: Option<&str>,
    reference_sync_ready: bool,
    identities: &[ManagedAgentReference],
    generate: F,
) -> Result<nostr::Keys, String>
where
    F: FnOnce() -> nostr::Keys,
{
    let Some(persona_id) = persona_id else {
        return Ok(generate());
    };
    if !reference_sync_ready {
        return Err(
            "agent identities are still syncing; wait for sync to finish before starting this persona"
                .to_string(),
        );
    }

    let mut existing_pubkeys = identities
        .iter()
        .filter(|reference| reference.persona_id == persona_id)
        .map(|reference| reference.pubkey.as_str())
        .collect::<Vec<_>>();
    existing_pubkeys.sort_unstable();
    existing_pubkeys.dedup();
    if !existing_pubkeys.is_empty() {
        return Err(format!(
            "persona {persona_id} already has a managed-agent keypair ({}) and cannot mint another",
            existing_pubkeys.join(", ")
        ));
    }

    Ok(generate())
}

fn references_from_retained(
    events: impl IntoIterator<Item = (String, String)>,
) -> Result<Vec<ManagedAgentReference>, String> {
    let mut references = events
        .into_iter()
        .filter_map(|(pubkey, content)| {
            let content: ManagedAgentEventContent = match serde_json::from_str(&content) {
                Ok(content) => content,
                Err(error) => {
                    return Some(Err(format!("failed to parse managed-agent event: {error}")))
                }
            };
            content.persona_id.map(|persona_id| {
                Ok(ManagedAgentReference {
                    pubkey,
                    name: content.name,
                    persona_id,
                })
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    references.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    Ok(references)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn retained_links_form_non_runnable_reference_index() {
        let linked = serde_json::json!({
            "name": "Rimac-Buzz",
            "persona_id": "rimac-definition",
            "parallelism": 1,
            "respond_to": "owner-only"
        })
        .to_string();
        let standalone = serde_json::json!({
            "name": "Standalone",
            "parallelism": 1,
            "respond_to": "owner-only"
        })
        .to_string();

        let references = references_from_retained([
            ("497d45dd".to_string(), linked),
            ("standalone-pubkey".to_string(), standalone),
        ])
        .unwrap();

        assert_eq!(
            references,
            vec![ManagedAgentReference {
                pubkey: "497d45dd".to_string(),
                name: "Rimac-Buzz".to_string(),
                persona_id: "rimac-definition".to_string(),
            }]
        );
    }

    #[test]
    fn unsynced_empty_index_fails_closed_before_key_generation() {
        let calls = AtomicUsize::new(0);
        let error = generate_managed_agent_keys(Some("rimac-definition"), false, &[], || {
            calls.fetch_add(1, Ordering::Relaxed);
            nostr::Keys::generate()
        })
        .unwrap_err();

        assert!(error.contains("still syncing"));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn an_existing_persona_key_blocks_mint_without_selecting_a_winner() {
        let identities = vec![
            ManagedAgentReference {
                pubkey: "original-key".to_string(),
                name: "Rimac-Buzz".to_string(),
                persona_id: "rimac-definition".to_string(),
            },
            ManagedAgentReference {
                pubkey: "previous-duplicate".to_string(),
                name: "Rimac-Buzz".to_string(),
                persona_id: "rimac-definition".to_string(),
            },
        ];
        let calls = AtomicUsize::new(0);
        let error =
            generate_managed_agent_keys(Some("rimac-definition"), true, &identities, || {
                calls.fetch_add(1, Ordering::Relaxed);
                nostr::Keys::generate()
            })
            .unwrap_err();

        assert!(error.contains("original-key"));
        assert!(error.contains("previous-duplicate"));
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "no second keypair may be generated"
        );
    }

    #[test]
    fn a_ready_unclaimed_persona_generates_exactly_one_keypair() {
        let calls = AtomicUsize::new(0);
        let keys = generate_managed_agent_keys(Some("new-definition"), true, &[], || {
            calls.fetch_add(1, Ordering::Relaxed);
            nostr::Keys::generate()
        })
        .unwrap();

        assert!(!keys.public_key().to_hex().is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
