//! `add_team_from_catalog`: copy another owner's published team into the local
//! stores with byte-level rollback on error.
//!
//! Two properties define this command, and both are amendment requirements:
//!
//! **The frontend is not trusted (A2).** The caller supplies only a
//! coordinate — owner pubkey, team d-tag, and the event id it is looking at.
//! The backend re-fetches the CURRENT head at `30178:<owner>:<d>` from the
//! active relay and requires it to be the same event, still carrying the
//! `shared` tag. A head that cannot be established at all is a failure, not a
//! fallback: adding from a coordinate we cannot read is exactly the case where
//! a retracted or superseded team would be copied.
//!
//! **The write uses byte-level rollback.** Both stores are snapshotted (raw
//! bytes) under the store lock before any write. If either save fails, both
//! files are restored from their snapshots. A crash between the two writes
//! leaves the stores inconsistent; retry is the recovery path (the add is
//! idempotent: an orphaned team is found by the replay check, and orphaned
//! member copies are reused by provenance matching).
//!
//! Everything about the projection itself — the schema, the size contract,
//! the member shape — is `managed_agents::team_catalog`'s; this module only
//! verifies provenance and writes records.

use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        team_catalog::{team_catalog_content_from_event, TeamCatalogContent},
        TeamCatalogSource, TeamRecord,
    },
};

mod apply;
#[cfg(test)]
mod tests;

/// The coordinate the frontend asks to add, before any verification.
///
/// `event_id` is what the user is looking at. It is never the source of the
/// content — it is compared against the freshly fetched head, so an add is
/// rejected when the catalog moved underneath the open dialog.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTeamFromCatalogRequest {
    pub owner_pubkey: String,
    pub team_d_tag: String,
    pub event_id: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTeamFromCatalogResult {
    pub team: TeamRecord,
    /// True when the team was already present and nothing was written. The
    /// caller distinguishes "added" from "you already have this".
    pub already_present: bool,
}

/// Add a published team from the community catalog.
#[tauri::command]
pub async fn add_team_from_catalog(
    input: AddTeamFromCatalogRequest,
    app: AppHandle,
) -> Result<AddTeamFromCatalogResult, String> {
    let source = TeamCatalogSource {
        owner_pubkey: input.owner_pubkey,
        team_d_tag: input.team_d_tag,
    }
    .normalized()?;
    let event_id = normalized_event_id(&input.event_id)?;

    // Fetch and verify BEFORE taking the store lock: the network call is the
    // slow part, and holding the lock across it would stall every unrelated
    // agent read for the duration of a relay round-trip.
    let content = {
        let state = app.state::<AppState>();
        verified_catalog_head(&state, &source, &event_id).await?
    };

    let app_for_write = app.clone();
    tokio::task::spawn_blocking(move || apply::add_verified_team(&app_for_write, &source, &content))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

fn normalized_event_id(value: &str) -> Result<String, String> {
    let event_id = value.trim().to_ascii_lowercase();
    if event_id.len() != 64 || !event_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "invalid catalog event id: '{event_id}' (must be 64 hex chars)"
        ));
    }
    Ok(event_id)
}

/// Fetch the current head at the team's catalog coordinate and accept it only
/// if it is the exact event the caller asked for, still shared.
///
/// Each rejection below is a distinct real scenario, not defensive padding:
/// an empty result is a deleted or never-readable coordinate; a differing id
/// is a head the owner republished since the dialog opened; and an id match
/// with the `shared` tag gone is an unshare the reader has not seen yet. All
/// three must fail closed — the alternative is copying a team its owner has
/// already withdrawn from the community.
async fn verified_catalog_head(
    state: &AppState,
    source: &TeamCatalogSource,
    event_id: &str,
) -> Result<TeamCatalogContent, String> {
    use buzz_core_pkg::kind::KIND_TEAM_CATALOG;

    let filter = serde_json::json!({
        "kinds": [KIND_TEAM_CATALOG],
        "authors": [source.owner_pubkey],
        "#d": [source.team_d_tag],
        "limit": 1,
    });
    let events = crate::relay::query_relay(state, &[filter])
        .await
        .map_err(|e| format!("could not verify the team with the relay: {e}"))?;

    let head = events
        .first()
        .ok_or("This team is no longer available in the catalog.")?;

    verified_head_content(head, source, event_id)
}

/// The verification itself, separated from the fetch so every rejection is
/// testable without a relay.
fn verified_head_content(
    head: &nostr::Event,
    source: &TeamCatalogSource,
    event_id: &str,
) -> Result<TeamCatalogContent, String> {
    use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};

    // Verify the signature before trusting ANY field on the event. The relay
    // is not a trusted source of authorship: `pubkey` and `content` are both
    // attacker-controlled if the signature is not checked here.
    head.verify()
        .map_err(|e| format!("the catalog event failed signature verification: {e}"))?;

    if head.kind.as_u16() as u32 != KIND_TEAM_CATALOG {
        return Err("The catalog event is not a team publication.".to_string());
    }
    if head.id.to_hex() != event_id {
        return Err(
            "This team has changed since it was listed. Refresh and try again.".to_string(),
        );
    }
    if !event_is_shared(head) {
        return Err("This team is no longer shared to the community.".to_string());
    }
    // Author and d-tag are re-derived from the verified event rather than
    // taken from the request, so a relay that answers a filter with an
    // unrelated event cannot set provenance.
    if head.pubkey.to_hex() != source.owner_pubkey {
        return Err("The catalog event was published by a different owner.".to_string());
    }
    if head_d_tag(head).as_deref() != Some(source.team_d_tag.as_str()) {
        return Err("The catalog event is for a different team.".to_string());
    }

    team_catalog_content_from_event(head)
}

/// The event's single `d` tag, or `None` when it is absent or not unique.
///
/// Uniqueness matters: the relay's ingest gate (amendment A4) already rejects
/// a multi-`d` 30178, but a reader that took the first of several would
/// resolve a different coordinate than the one it verified against.
fn head_d_tag(event: &nostr::Event) -> Option<String> {
    let mut found: Option<String> = None;
    for tag in event.tags.iter() {
        let values: Vec<&str> = tag.as_slice().iter().map(|s| s.as_str()).collect();
        if values.first() != Some(&"d") {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(values.get(1)?.to_string());
    }
    found
}
