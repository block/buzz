//! Name checks for explicit local creation. Presence never makes a name reusable.
use crate::{
    app_state::AppState,
    managed_agents::{load_managed_agents, load_personas},
};
use tauri::AppHandle;

/// Refuse a second identity with the same name, permitting updates of the exact key.
pub(crate) fn reject_collision<'a>(
    name: &str,
    except: Option<&str>,
    entries: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), String> {
    if entries.into_iter().any(|(existing, id)| {
        existing.trim().eq_ignore_ascii_case(name.trim()) && except != Some(id)
    }) {
        return Err(format!("An agent named {name} already exists. Use its existing identity or choose a unique name."));
    }
    Ok(())
}

/// Check the current owner/community before minting or renaming. Callers hold the
/// device's async name-transition lock across this check and their final persist.
/// A full or unavailable directory cannot prove uniqueness and fails closed.
pub(crate) async fn preflight(
    app: &AppHandle,
    state: &AppState,
    name: &str,
    persona_id: Option<&str>,
    existing_pubkey: Option<&str>,
) -> Result<(), String> {
    let policy = super::active(app)?;
    policy.require_local_agent(name, existing_pubkey, persona_id)?;
    if !policy.unique_names {
        return Ok(());
    }
    let records = load_managed_agents(app)?;
    reject_collision(
        name,
        existing_pubkey,
        records.iter().map(|r| (r.name.as_str(), r.pubkey.as_str())),
    )?;
    let personas = load_personas(app)?;
    reject_collision(
        name,
        persona_id,
        personas
            .iter()
            .map(|p| (p.display_name.as_str(), p.id.as_str())),
    )?;
    let keys = state.signing_keys()?;
    let owner = keys.public_key().to_hex();
    let relay = crate::relay::relay_api_base_url_with_override(state);
    // Owner-authored instance records cover exact names that full-text search
    // cannot tokenize (including punctuation and emoji). Never infer vacancy
    // from an incomplete owner directory.
    let owned = crate::relay::query_relay_at_with_keys(
        state,
        &relay,
        &[serde_json::json!({
            "kinds":[30177], "authors":[owner], "limit":500
        })],
        &keys,
        None,
    )
    .await?;
    if owned.len() >= 500 {
        return Err("Cannot verify uniqueness: the owned-agent directory is incomplete.".into());
    }
    for event in owned {
        if event.pubkey != keys.public_key() || event.kind.as_u16() != 30177 {
            continue;
        }
        event
            .verify()
            .map_err(|e| format!("Invalid owned-agent directory entry: {e}"))?;
        let content: serde_json::Value =
            serde_json::from_str(&event.content).map_err(|e| e.to_string())?;
        let existing_name = content
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let existing_key = event
            .tags
            .iter()
            .find_map(|tag| {
                let values = tag.as_slice();
                (values.first().map(String::as_str) == Some("d"))
                    .then(|| values.get(1).map(String::as_str))
                    .flatten()
            })
            .unwrap_or("");
        reject_collision(name, existing_pubkey, [(existing_name, existing_key)])?;
    }
    let events = crate::relay::query_relay_at_with_keys(
        state,
        &relay,
        &[serde_json::json!({
            "kinds":[0], "search":name.trim(), "search_mode":"prefix", "limit":500
        })],
        &keys,
        None,
    )
    .await?;
    if events.len() >= 500 {
        return Err("Cannot verify a unique agent name: too many matching profiles. Choose a more distinctive name.".into());
    }
    for event in events {
        if crate::nostr_convert::profile_valid_oa_owner_pubkey(&event).as_deref() != Some(&owner) {
            continue;
        }
        let profile = crate::nostr_convert::user_search_result_from_event(&event);
        reject_collision(
            name,
            existing_pubkey,
            [(
                profile.display_name.as_deref().unwrap_or(""),
                profile.pubkey.as_str(),
            )],
        )?;
    }
    crate::relay::assert_expected_relay_scope(
        Some(&relay),
        &crate::relay::relay_api_base_url_with_override(state),
    )?;
    crate::relay::assert_expected_signer(
        Some(&owner),
        &state.signing_keys()?.public_key().to_hex(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_existing_identity_can_keep_a_name() {
        assert!(reject_collision(" Scout ", None, [("sCoUt", "old-key")]).is_err());
        assert!(reject_collision("Scout", Some("new-key"), [("Scout", "old-key")]).is_err());
        assert!(reject_collision("Scout", Some("old-key"), [("Scout", "old-key")]).is_ok());
        assert!(reject_collision("Notebook", None, [("Scout", "old-key")]).is_ok());
    }
}
