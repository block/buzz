//! Native persona-catalog fetch and trust-boundary projection.
//!
//! The renderer owns presentation/linkage to local personas. Relay paging,
//! signature verification, NIP-33 head selection, and untrusted-content parsing
//! stay here so a catalog refresh crosses IPC once instead of once per page and
//! never performs Schnorr verification on the webview thread.

use std::{collections::HashMap, time::Duration};

use buzz_core_pkg::kind::KIND_PERSONA;
use nostr::Event;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::sync::LazyLock;
use tauri::State;

use crate::{
    app_state::AppState, managed_agents::validate_agent_definition_text,
    native_relay_client::NativeRelayClient,
};

const CATALOG_PAGE_SIZE: usize = 500;
const MAX_CATALOG_PAGES: usize = 40;
const PAGE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_AVATAR_LENGTH: usize = 2_048;
const INLINE_SVG_AVATAR_PREFIX: &str = "data:image/svg+xml,";
const MAX_INLINE_SVG_AVATAR_LENGTH: usize = 8_192;
const MAX_INLINE_RASTER_AVATAR_LENGTH: usize = 256 * 1_024;

static INLINE_RASTER_AVATAR: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^data:image/(?:png|jpeg|gif|webp);base64,([A-Za-z0-9+/]+={0,2})$").ok()
});

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersonaCatalogPublication {
    event_id: String,
    owner_pubkey: String,
    source_persona_id: String,
    created_at: u64,
    agent: CatalogAgentProjection,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogAgentProjection {
    display_name: String,
    avatar_url: Option<String>,
    system_prompt: String,
    runtime: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    name_pool: Vec<String>,
    respond_to: Option<String>,
    parallelism: Option<u64>,
}

/// Fetches the active community's relay-confirmed persona catalog.
///
/// The command accepts no relay or identity input: both are snapshotted from
/// `AppState`, then checked again before return so an in-flight old-community
/// response cannot populate the new community's query cache.
#[tauri::command]
pub(crate) async fn fetch_persona_catalog(
    state: State<'_, AppState>,
    relay_client: State<'_, NativeRelayClient>,
) -> Result<Vec<PersonaCatalogPublication>, String> {
    let keys = state.signing_keys()?;
    let owner = keys.public_key().to_hex();
    let relay_url = crate::relay::relay_ws_url_with_override(&state);
    let session = relay_client.session(relay_url.clone(), keys).await;
    let mut by_id = HashMap::new();
    let mut until = None;

    for _ in 0..MAX_CATALOG_PAGES {
        let mut filter = serde_json::json!({
            "kinds": [KIND_PERSONA],
            "limit": CATALOG_PAGE_SIZE,
        });
        if let Some(until) = until {
            filter["until"] = serde_json::json!(until);
        }
        let page = session.fetch_events(filter, PAGE_TIMEOUT).await?;
        let page_len = page.len();
        // Schnorr verification is CPU-bound. Keep the complete page off the
        // async executor (and therefore off Tauri command scheduling).
        let verified = tauri::async_runtime::spawn_blocking(move || {
            page.into_iter()
                .filter(|event| event.verify().is_ok())
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| format!("catalog signature verification failed: {error}"))?;

        let progress = merge_verified_page(&mut by_id, page_len, verified);
        match progress {
            PageProgress::Done => break,
            PageProgress::Next(next_until) => until = Some(next_until),
        }
    }

    let current_keys = state.signing_keys()?;
    if current_keys.public_key().to_hex() != owner
        || crate::relay::relay_ws_url_with_override(&state) != relay_url
    {
        return Err("persona catalog scope changed while fetching".to_string());
    }

    Ok(publications_from_verified_events(
        by_id.into_values().collect(),
    ))
}

#[derive(Debug, PartialEq)]
enum PageProgress {
    Done,
    Next(u64),
}

fn merge_verified_page(
    by_id: &mut HashMap<String, Event>,
    wire_page_len: usize,
    verified: Vec<Event>,
) -> PageProgress {
    let size_before = by_id.len();
    let oldest = verified
        .iter()
        .map(|event| event.created_at.as_secs())
        .min();
    for event in verified {
        by_id.insert(event.id.to_hex(), event);
    }

    // A short page is the end of the catalog; a page of only repeats means the
    // inclusive `until` cursor cannot advance past tied timestamps.
    if wire_page_len < CATALOG_PAGE_SIZE || by_id.len() == size_before {
        return PageProgress::Done;
    }
    // A full page of invalid signatures cannot supply a trusted cursor.
    oldest.map_or(PageProgress::Done, PageProgress::Next)
}

fn publications_from_verified_events(mut events: Vec<Event>) -> Vec<PersonaCatalogPublication> {
    events.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut claimed = std::collections::HashSet::new();
    let mut publications = Vec::new();

    for event in events {
        if event.kind.as_u16() as u32 != KIND_PERSONA {
            continue;
        }
        let Some(source_persona_id) = coordinate_tag(&event, "d") else {
            continue;
        };
        if source_persona_id.is_empty() {
            continue;
        }
        let owner_pubkey = event.pubkey.to_hex().to_ascii_lowercase();
        let coordinate = (owner_pubkey.clone(), source_persona_id.clone());
        if !claimed.insert(coordinate) {
            continue;
        }

        // Claim happens before visibility or parsing. A valid newest unshared
        // or malformed head is still the NIP-33 head and must not resurrect an
        // older shared definition.
        if exact_tag(&event, "shared").as_deref() != Some("true") {
            continue;
        }
        let Some(agent) = parse_agent(&event.content) else {
            continue;
        };
        publications.push(PersonaCatalogPublication {
            event_id: event.id.to_hex(),
            owner_pubkey,
            source_persona_id,
            created_at: event.created_at.as_secs(),
            agent,
        });
    }
    publications
}

fn coordinate_tag(event: &Event, name: &str) -> Option<String> {
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

fn exact_tag(event: &Event, name: &str) -> Option<String> {
    let matches = event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.len() == 2 && values.first().is_some_and(|value| value == name))
                .then(|| values[1].clone())
        })
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].clone())
}

fn parse_agent(content: &str) -> Option<CatalogAgentProjection> {
    let value: Value = serde_json::from_str(content).ok()?;
    let object = value.as_object()?;
    let display_name = object.get("display_name")?.as_str()?.to_string();
    let system_prompt = object
        .get("system_prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    validate_agent_definition_text(&display_name, &system_prompt).ok()?;

    let respond_to = match object.get("respond_to").and_then(Value::as_str) {
        Some("allowlist") => Some("owner-only".to_string()),
        Some(value @ ("owner-only" | "anyone")) => Some(value.to_string()),
        _ => None,
    };
    let parallelism = object
        .get("parallelism")
        .and_then(Value::as_u64)
        .filter(|value| (1..=32).contains(value));
    let name_pool = object
        .get("name_pool")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();

    Some(CatalogAgentProjection {
        display_name,
        avatar_url: object
            .get("avatar_url")
            .and_then(Value::as_str)
            .filter(|value| safe_avatar(value))
            .map(ToOwned::to_owned),
        system_prompt,
        runtime: optional_string(object.get("runtime")),
        model: optional_string(object.get("model")),
        provider: optional_string(object.get("provider")),
        name_pool,
        respond_to,
        parallelism,
    })
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn safe_avatar(value: &str) -> bool {
    if value.starts_with(INLINE_SVG_AVATAR_PREFIX) {
        return value.len() <= MAX_INLINE_SVG_AVATAR_LENGTH;
    }
    if value.len() <= MAX_INLINE_RASTER_AVATAR_LENGTH {
        if let Some(captures) = INLINE_RASTER_AVATAR
            .as_ref()
            .and_then(|pattern| pattern.captures(value))
        {
            return captures
                .get(1)
                .is_some_and(|payload| payload.as_str().len() % 4 == 0);
        }
    }
    value.len() <= MAX_HTTP_AVATAR_LENGTH
        && !value.chars().any(char::is_whitespace)
        && !value.contains(['(', ')'])
        && url::Url::parse(value)
            .ok()
            .is_some_and(|url| matches!(url.scheme(), "http" | "https"))
}

#[cfg(test)]
#[path = "persona_catalog_tests.rs"]
mod tests;
