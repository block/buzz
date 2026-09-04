//! `add_team_from_catalog`: copy another owner's published team into the local
//! stores with byte-level rollback on error.
//!
//! **Frontend is not trusted (A2).** The caller supplies only a coordinate
//! (owner pubkey, team d-tag, viewed event id); the backend re-fetches the
//! CURRENT head at `30178:<owner>:<d>` and requires it to be the same event,
//! still `shared`. A head that cannot be read is a failure, not a fallback —
//! that is exactly the case where a retracted or superseded team would be
//! copied.
//!
//! **Byte-level rollback.** Both stores are snapshotted (raw bytes) under the
//! store lock before any write; if either save fails, both are restored. A
//! crash between the two writes leaves the stores inconsistent — retry is the
//! recovery path, since the add is idempotent (an orphaned team is found by
//! the replay check, orphaned member copies reused by provenance matching).
//!
//! The projection itself — schema, size contract, member shape — belongs to
//! `managed_agents::team_catalog`; this module only verifies provenance and
//! writes records.

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
/// `event_id` is never the source of content — it is compared against the
/// freshly fetched head, so an add is rejected when the catalog moved
/// underneath the open dialog.
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
    /// True when the team was already present and nothing was written.
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

    // Snapshot the community boundary — relay, owner, and retention db — BEFORE
    // the relay round-trip. Everything downstream is pinned to this scope: the
    // query authenticates against it, the write fences against it, and the
    // adopted heads enqueue into it. A workspace switch during the await can
    // then no longer publish community A's team into community B's retention db.
    let scope = {
        let state = app.state::<AppState>();
        crate::managed_agents::retention::active_retention_scope(&app, &state)?
    };

    // Fetch and verify BEFORE taking the store lock: holding it across the
    // relay round-trip would stall every unrelated agent read. The query hits
    // the captured relay with the captured owner's auth, not the live workspace.
    let content = {
        let state = app.state::<AppState>();
        verified_catalog_head(&state, &scope, &source, &event_id).await?
    };

    let app_for_write = app.clone();
    tokio::task::spawn_blocking(move || {
        apply::add_verified_team(&app_for_write, scope, &source, &content)
    })
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
/// Each rejection below is a distinct scenario: an empty result is a deleted
/// or never-readable coordinate; a differing id is a head republished since
/// the dialog opened; an id match with the `shared` tag gone is an unshare the
/// reader has not seen. All three fail closed — else a withdrawn team is
/// copied.
async fn verified_catalog_head(
    state: &AppState,
    scope: &crate::managed_agents::retention::RetentionScope,
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
    // Query the CAPTURED relay with the CAPTURED owner's NIP-98 auth, not the
    // live workspace: a switch mid-command must not retarget the verification
    // fetch to a different tenant than the one the adoption commits into.
    let api_base_url = crate::relay::relay_http_base_url(&scope.relay_url);
    let events = crate::relay::query_relay_at_with_keys(
        state,
        &api_base_url,
        &[filter],
        &scope.owner_keys,
        None,
    )
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

    // Verify the signature before trusting ANY field: `pubkey` and `content`
    // are attacker-controlled if it is not checked here.
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
    // Author and d-tag are re-derived from the verified event, not the
    // request, so a relay answering with an unrelated event cannot set
    // provenance.
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
/// Uniqueness matters: the relay's ingest gate (A4) already rejects a
/// multi-`d` 30178, but a reader taking the first of several would resolve a
/// different coordinate than the one it verified against.
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
