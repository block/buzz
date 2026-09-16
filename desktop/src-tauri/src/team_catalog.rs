//! Native team-catalog fetch and trust-boundary projection.
//!
//! The renderer owns presentation/linkage to local teams. Relay paging,
//! signature verification, NIP-33 head selection, and untrusted-content
//! parsing stay here — structurally the persona-catalog equivalent
//! (`persona_catalog.rs`) with kind 30178 and the team content parser swapped
//! in, so a catalog refresh crosses IPC once and never verifies a signature on
//! the webview thread.
//!
//! Content parsing reuses `managed_agents::team_catalog::team_catalog_content_from_event`
//! — the same all-or-nothing parse `add_team_from_catalog` re-runs at add time,
//! so a head this command projects is exactly a head the backend will accept.

use std::{collections::HashMap, time::Duration};

use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
use nostr::Event;
use serde::Serialize;
use tauri::State;

use crate::{
    app_state::AppState,
    managed_agents::team_catalog::{team_catalog_content_from_event, TeamCatalogContent},
    native_relay_client::NativeRelayClient,
};

const CATALOG_PAGE_SIZE: usize = 500;
const MAX_CATALOG_PAGES: usize = 40;
const PAGE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamCatalogPublication {
    event_id: String,
    owner_pubkey: String,
    team_d_tag: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    members: Vec<TeamCatalogMemberProjection>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TeamCatalogMemberProjection {
    member_key: String,
    display_name: String,
    system_prompt: String,
    avatar_url: Option<String>,
    runtime: Option<String>,
    model: Option<String>,
    provider: Option<String>,
}

/// Fetches the active community's relay-confirmed team catalog.
///
/// The command accepts no relay or identity input: both are snapshotted from
/// `AppState`, then checked again before return so an in-flight old-community
/// response cannot populate the new community's query cache.
#[tauri::command]
pub(crate) async fn fetch_team_catalog(
    state: State<'_, AppState>,
    relay_client: State<'_, NativeRelayClient>,
) -> Result<Vec<TeamCatalogPublication>, String> {
    let keys = state.signing_keys()?;
    let owner = keys.public_key().to_hex();
    let relay_url = crate::relay::relay_ws_url_with_override(&state);
    let session = relay_client.session(relay_url.clone(), keys).await;
    let by_id = collect_verified_catalog(|until| {
        let session = &session;
        async move {
            let mut filter = serde_json::json!({
                "kinds": [KIND_TEAM_CATALOG],
                "limit": CATALOG_PAGE_SIZE,
            });
            if let Some(until) = until {
                filter["until"] = serde_json::json!(until);
            }
            let page = session.fetch_events(filter, PAGE_TIMEOUT).await?;
            let page_len = page.len();
            // Schnorr verification is CPU-bound. Keep the complete page off the
            // async executor (and therefore off Tauri command scheduling).
            let verified = tauri::async_runtime::spawn_blocking(move || verify_page(page))
                .await
                .map_err(|error| format!("catalog signature verification failed: {error}"))?;
            Ok((page_len, verified))
        }
    })
    .await?;

    let current_keys = state.signing_keys()?;
    if current_keys.public_key().to_hex() != owner
        || crate::relay::relay_ws_url_with_override(&state) != relay_url
    {
        return Err("team catalog scope changed while fetching".to_string());
    }

    Ok(publications_from_verified_events(
        by_id.into_values().collect(),
    ))
}

/// Page the catalog to exhaustion through `fetch_verified_page`, which returns
/// the wire page length and the signature-verified events for a given inclusive
/// `until` cursor. Kept generic over the fetcher so the paging/termination logic
/// is driven by the exact same code in production and in the cap-boundary
/// regressions, with no relay or Tauri state.
///
/// Exhaustion is proven ONLY by a `Done` (a short page). If the page budget runs
/// out while pages are still full and advancing, the catalog is larger than
/// `MAX_CATALOG_PAGES` can walk: returning the collected heads would silently
/// present a truncated catalog as complete, so fail loudly instead — the same
/// degrade philosophy as `DenseBoundary`/`NoVerifiedEvents`, surfaced by the
/// browse dialog as an error rather than a silently short list.
async fn collect_verified_catalog<F, Fut>(
    mut fetch_verified_page: F,
) -> Result<HashMap<String, Event>, String>
where
    F: FnMut(Option<u64>) -> Fut,
    Fut: std::future::Future<Output = Result<(usize, Vec<Event>), String>>,
{
    let mut by_id = HashMap::new();
    let mut until = None;
    let mut exhausted = false;

    for _ in 0..MAX_CATALOG_PAGES {
        let (page_len, verified) = fetch_verified_page(until).await?;
        match merge_verified_page(&mut by_id, page_len, until, verified) {
            PageProgress::Done => {
                exhausted = true;
                break;
            }
            PageProgress::Next(next_until) => until = Some(next_until),
            // The relay filter exposes no `(created_at, id)` cursor to page
            // within a second, so more than one page at the boundary second
            // cannot be paged past. Fail loudly rather than project a truncated
            // catalog as complete — the browse dialog surfaces this as an error
            // instead of silently dropping every older team.
            PageProgress::DenseBoundary(second) => {
                return Err(format!(
                    "team catalog has more than one page of events at created_at {second}; \
                     the time-only relay cursor cannot page past it"
                ));
            }
            // A full page with no verifiable events cannot advance the cursor on
            // trusted data. Advancing on the wire timestamp would let one forged
            // `created_at` warp the cursor past — and silently drop — every valid
            // team below it, so fail loudly instead.
            PageProgress::NoVerifiedEvents => {
                return Err(
                    "team catalog returned a full page with no verifiable events; \
                     cannot safely advance the cursor"
                        .to_string(),
                );
            }
        }
    }

    if !exhausted {
        return Err(format!(
            "team catalog exceeds the {MAX_CATALOG_PAGES} page fetch budget \
             ({CATALOG_PAGE_SIZE} events per page); cannot list it completely"
        ));
    }

    Ok(by_id)
}

#[derive(Debug, PartialEq)]
enum PageProgress {
    Done,
    Next(u64),
    /// A full page whose oldest verified timestamp cannot drop the inclusive
    /// `until` cursor: more than one page of events shares this second, and the
    /// relay filter has no `(created_at, id)` cursor to escape it.
    DenseBoundary(u64),
    /// A full page with no verifiable events. The cursor can only advance on
    /// trusted timestamps, so there is nothing safe to page with.
    NoVerifiedEvents,
}

/// Retain only events whose Schnorr signature verifies. This is the single
/// trust gate for a relay page: paging, head selection, and content parsing all
/// run on its output, so a forged or tampered event never influences the
/// cursor or the projected catalog. Shared with the paging regression so the
/// test drives the exact seam production does, not a stubbed result.
fn verify_page(page: Vec<Event>) -> Vec<Event> {
    page.into_iter()
        .filter(|event| event.verify().is_ok())
        .collect()
}

fn merge_verified_page(
    by_id: &mut HashMap<String, Event>,
    wire_page_len: usize,
    until: Option<u64>,
    verified: Vec<Event>,
) -> PageProgress {
    // The oldest *verified* timestamp is the only value safe to page with: an
    // unverifiable event must never control the cursor, or one forged
    // `created_at` (e.g. 0) would warp `until` past — and silently drop — every
    // valid team below it. Captured before the page is drained into `by_id`.
    let verified_oldest = verified
        .iter()
        .map(|event| event.created_at.as_secs())
        .min();

    for event in verified {
        by_id.insert(event.id.to_hex(), event);
    }

    // A short page is the end of the catalog.
    if wire_page_len < CATALOG_PAGE_SIZE {
        return PageProgress::Done;
    }

    // A full page must advance on a verified timestamp. With none, the cursor
    // cannot move safely — fail loudly rather than trust the wire or complete.
    let Some(oldest) = verified_oldest else {
        return PageProgress::NoVerifiedEvents;
    };
    // When the oldest verified timestamp cannot drop below the current inclusive
    // `until`, the page is stuck at a dense boundary second: silently stopping
    // would drop every older team and falsely report the catalog exhausted.
    if until.is_some_and(|until| oldest >= until) {
        return PageProgress::DenseBoundary(oldest);
    }
    PageProgress::Next(oldest)
}

fn publications_from_verified_events(mut events: Vec<Event>) -> Vec<TeamCatalogPublication> {
    events.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut claimed = std::collections::HashSet::new();
    let mut publications = Vec::new();

    for event in events {
        if event.kind.as_u16() as u32 != KIND_TEAM_CATALOG {
            continue;
        }
        let Some(team_d_tag) = single_tag(&event, "d") else {
            continue;
        };
        if team_d_tag.is_empty() {
            continue;
        }
        let owner_pubkey = event.pubkey.to_hex().to_ascii_lowercase();
        let coordinate = (owner_pubkey.clone(), team_d_tag.clone());
        if !claimed.insert(coordinate) {
            continue;
        }

        // Claim happens before visibility or parsing. A valid newest unshared
        // or malformed head is still the NIP-33 head and must not resurrect an
        // older shared definition.
        if !event_is_shared(&event) {
            continue;
        }
        // All-or-nothing parse, identical to the add-time re-fetch: a team with
        // any invalid member cannot be adopted, so a partial projection would
        // only offer an un-addable entry.
        let Ok(content) = team_catalog_content_from_event(&event) else {
            continue;
        };
        publications.push(publication(
            event.id.to_hex(),
            owner_pubkey,
            team_d_tag,
            content,
        ));
    }
    publications
}

fn publication(
    event_id: String,
    owner_pubkey: String,
    team_d_tag: String,
    content: TeamCatalogContent,
) -> TeamCatalogPublication {
    TeamCatalogPublication {
        event_id,
        owner_pubkey,
        team_d_tag,
        name: content.name,
        description: content.description,
        instructions: content.instructions,
        members: content
            .members
            .into_iter()
            .map(|member| TeamCatalogMemberProjection {
                member_key: member.member_key,
                display_name: member.display_name,
                system_prompt: member.system_prompt.unwrap_or_default(),
                avatar_url: member.avatar_url,
                runtime: member.runtime,
                model: member.model,
                provider: member.provider,
            })
            .collect(),
    }
}

/// A tag's value, but only when the event carries exactly one of that tag.
///
/// Ambiguity is absence: the relay admits exactly one bounded `d` tag, so a
/// multi-`d` event is malformed and picking the first would resolve a
/// different coordinate than the publisher addressed.
fn single_tag(event: &Event, name: &str) -> Option<String> {
    let matches = event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.len() >= 2 && values.first().is_some_and(|value| value == name))
                .then(|| values[1].clone())
        })
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].clone())
}

#[cfg(test)]
#[path = "team_catalog_tests.rs"]
mod tests;
