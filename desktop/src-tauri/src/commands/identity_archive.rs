//! NIP-IA identity archival commands.
//!
//! These commands let the desktop:
//!
//! - resolve a viewee's NIP-OA owner via their live `kind:0` (gates the
//!   "Archive" button when the current user is the owner-of-agent),
//! - submit `kind:9035` archive and `kind:9036` unarchive requests (consent
//!   path is selected by the relay; we just build the wire form),
//! - read the relay's `kind:13535` archive snapshot to drive UI flair.
//!
//! Spec: `docs/nips/NIP-IA.md`. The relay performs full authorization —
//! see §Owner-of-Agent Requests and §Relay Processing Algorithm.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    events,
    managed_agents::try_regenerate_nest,
    relay::{
        classify_request_error, query_relay, query_relay_at, relay_api_base_url,
        relay_http_base_url, relay_ws_url, relay_ws_url_with_override, submit_event,
        workspace_relay_override, SubmitEventResponse,
    },
};

/// A relay target resolved from a single workspace-override read, so a caller
/// that performs several relay requests cannot mix two relays if the workspace
/// override changes mid-flight.
///
/// `relay_ws_url_with_override` and `relay_api_base_url_with_override` each read
/// the override independently; a workspace switch between two such reads can
/// pair one relay's NIP-11 signer with another relay's snapshot query.
/// Capturing both fields from one read — matching those two functions' exact
/// precedence, including the standalone `BUZZ_RELAY_HTTP` path when no override
/// is set — guarantees the pair is internally consistent.
pub(crate) struct RelayTarget {
    /// Relay WebSocket URL (drives the NIP-11 fetch and the rendered footer).
    pub ws_url: String,
    /// Relay HTTP API base URL (drives `/query`).
    pub api_base_url: String,
}

/// Capture the effective relay target once, before any network work.
pub(crate) fn capture_relay_target(state: &AppState) -> RelayTarget {
    match workspace_relay_override(state) {
        Some(url) => RelayTarget {
            api_base_url: relay_http_base_url(&url),
            ws_url: url,
        },
        None => RelayTarget {
            ws_url: relay_ws_url(),
            api_base_url: relay_api_base_url(),
        },
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Read `target`'s live `kind:0` event and extract the first valid NIP-OA
/// `auth` tag plus the verified owner pubkey.
///
/// Mirrors the verification the relay will do (per spec gotcha #3: the
/// preimage subject is the *target* pubkey, not the request signer). The
/// `buzz-sdk` lives on nostr 0.36; the desktop is on 0.37, so we bridge
/// via hex round-trip exactly like `relay::build_profile_event` does.
pub(crate) fn extract_oa_owner(target_kind0: &nostr::Event) -> Option<(String, [String; 4])> {
    let target_hex = target_kind0.pubkey.to_hex();
    let target_compat = nostr::PublicKey::from_hex(&target_hex).ok()?;

    for tag in target_kind0.tags.iter() {
        let slice = tag.as_slice();
        if slice.first().map(String::as_str) != Some("auth") || slice.len() != 4 {
            continue;
        }
        let json = serde_json::to_string(slice).ok()?;
        match buzz_sdk_pkg::nip_oa::verify_auth_tag(&json, &target_compat) {
            Ok(owner) => {
                let raw: [String; 4] = [
                    slice[0].clone(),
                    slice[1].clone(),
                    slice[2].clone(),
                    slice[3].clone(),
                ];
                return Some((owner.to_hex(), raw));
            }
            Err(_) => continue,
        }
    }
    None
}

pub(crate) async fn fetch_kind0(
    state: &AppState,
    pubkey: &str,
) -> Result<Option<nostr::Event>, String> {
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [pubkey.to_ascii_lowercase()],
            "limit": 1,
        })],
    )
    .await?;
    Ok(events.into_iter().next())
}

// ── Owner-of-agent resolution ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct OwnerOfAgent {
    /// Owner pubkey (hex) recovered from the viewee's verified NIP-OA `auth` tag.
    pub owner: String,
    /// True iff `owner` equals the current user's pubkey. Lets the frontend
    /// gate the "Archive" button without a second round-trip.
    pub is_me: bool,
}

/// Resolve `target`'s NIP-OA owner by reading its live `kind:0` and verifying
/// the embedded `auth` tag. Returns `None` if the target has no kind:0, no
/// `auth` tag, or the tag fails verification.
///
/// This is what gates the owner-path archive button: the frontend calls this,
/// and if `is_me == true`, shows the button.
#[tauri::command]
pub async fn resolve_oa_owner(
    target_pubkey: String,
    state: State<'_, AppState>,
) -> Result<Option<OwnerOfAgent>, String> {
    let Some(kind0) = fetch_kind0(&state, &target_pubkey).await? else {
        return Ok(None);
    };

    let Some((owner_hex, _tag)) = extract_oa_owner(&kind0) else {
        return Ok(None);
    };

    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    Ok(Some(OwnerOfAgent {
        is_me: my_pubkey.eq_ignore_ascii_case(&owner_hex),
        owner: owner_hex,
    }))
}

// ── Archive / unarchive requests ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveRequest {
    pub target_pubkey: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub replaced_by: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnarchiveRequest {
    pub target_pubkey: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Roster refresh a successful archive/unarchive triggers. Binding the action
/// to a *type* rather than a closure selected at each call site is what closes
/// the regression Thufir found: the command wrapper passes a value (`&app`)
/// with no callback to construct, so the "regenerate on success" selection
/// lives entirely inside the cores below — where the tests traverse it. The
/// production binding is the single, irreducible `AppHandle` adapter.
pub(crate) trait NestRegenTrigger {
    fn trigger(&self);
}

impl NestRegenTrigger for AppHandle {
    fn trigger(&self) {
        try_regenerate_nest(self);
    }
}

/// Submit `builder` to the active workspace relay, then trigger `on_success`
/// exactly once iff the relay accepted the event.
///
/// This pins the shared half of the archive/unarchive → AGENTS.md-regeneration
/// contract: regeneration is best-effort roster maintenance, so it must fire on
/// a successful submission and must NOT fire when the submit is rejected (a
/// rejected request changed nothing to re-render).
async fn submit_then_regenerate(
    builder: nostr::EventBuilder,
    state: &AppState,
    on_success: impl FnOnce(),
) -> Result<SubmitEventResponse, String> {
    let response = submit_event(builder, state).await?;
    on_success();
    Ok(response)
}

/// `AppHandle`-free core of [`archive_identity`]: resolve the owner-of-agent
/// `auth` tag, build the real `kind:9035` request, submit it, and trigger
/// `regen` so a successful archive refreshes the roster.
///
/// The command wrapper is untestable (it needs a live Tauri runtime for its
/// `AppHandle`), so this core owns the whole orchestration — including *binding*
/// the regeneration trigger onto the successful-submit path. The wrapper only
/// hands it the `AppHandle` as the trigger; a test drives the exact archive
/// wiring with a counting trigger over a loopback relay. RED-on-revert: change
/// `|| regen.trigger()` to `|| {}` here and
/// `archive_core_fires_regen_only_on_accepted_submit` fails while the unarchive
/// core test stays green.
async fn archive_identity_core(
    req: &ArchiveRequest,
    state: &AppState,
    regen: &impl NestRegenTrigger,
) -> Result<SubmitEventResponse, String> {
    let auth_tag = maybe_owner_auth_tag(state, &req.target_pubkey).await?;
    let builder = events::build_archive_identity_request(
        &req.target_pubkey,
        &req.content,
        req.reason.as_deref(),
        req.replaced_by.as_deref(),
        auth_tag.as_ref(),
    )?;
    submit_then_regenerate(builder, state, || regen.trigger()).await
}

/// `AppHandle`-free core of [`unarchive_identity`]: builds the real `kind:9036`
/// request and triggers `regen` on acceptance. See [`archive_identity_core`]
/// for why this seam is extracted. RED-on-revert: change `|| regen.trigger()`
/// to `|| {}` here and `unarchive_core_fires_regen_only_on_accepted_submit`
/// fails while the archive core test stays green.
async fn unarchive_identity_core(
    req: &UnarchiveRequest,
    state: &AppState,
    regen: &impl NestRegenTrigger,
) -> Result<SubmitEventResponse, String> {
    let auth_tag = maybe_owner_auth_tag(state, &req.target_pubkey).await?;
    let builder = events::build_unarchive_identity_request(
        &req.target_pubkey,
        &req.content,
        req.reason.as_deref(),
        auth_tag.as_ref(),
    )?;
    submit_then_regenerate(builder, state, || regen.trigger()).await
}

/// Submit a `kind:9035` archive request to the relay. Consent path is selected
/// by the relay — we just attach the owner-of-agent `auth` tag when the live
/// `kind:0` proves we own the target, so the relay can choose the `owner`
/// path. Self and admin paths require no auth tag.
///
/// On acceptance, refresh AGENTS.md so a just-archived agent drops from the
/// roster without waiting for the next unrelated edit or app restart. The
/// regen is fire-and-forget and fail-open like every other mutation site; it
/// races the relay's kind:13535 snapshot update, so a stale render self-heals
/// on the next regen.
#[tauri::command]
pub async fn archive_identity(
    req: ArchiveRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SubmitEventResponse, String> {
    archive_identity_core(&req, &state, &app).await
}

/// Submit a `kind:9036` unarchive request to the relay. See
/// [`archive_identity`]: refresh the roster so an unarchived agent reappears
/// promptly, fail-open against the same snapshot race.
#[tauri::command]
pub async fn unarchive_identity(
    req: UnarchiveRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SubmitEventResponse, String> {
    unarchive_identity_core(&req, &state, &app).await
}

/// If the current user is the verified NIP-OA owner of `target`, return the
/// `auth` tag elements (label, owner, conditions, sig) for attachment to a
/// 9035/9036 request. Otherwise return `None` (self / admin / no-path).
///
/// The relay independently re-fetches the target's live `kind:0` and verifies
/// against it; this tag is intent + freshness evidence, not the authority.
async fn maybe_owner_auth_tag(
    state: &AppState,
    target_pubkey: &str,
) -> Result<Option<[String; 4]>, String> {
    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    // Self path: never attach auth (spec §Self Requests: if actor==target and
    // an `auth` tag is also present, relay MUST treat it as self).
    if my_pubkey.eq_ignore_ascii_case(target_pubkey) {
        return Ok(None);
    }

    let Some(kind0) = fetch_kind0(state, target_pubkey).await? else {
        return Ok(None);
    };
    let Some((owner_hex, raw_tag)) = extract_oa_owner(&kind0) else {
        return Ok(None);
    };

    if !owner_hex.eq_ignore_ascii_case(&my_pubkey) {
        return Ok(None);
    }
    Ok(Some(raw_tag))
}

// ── Archive snapshot ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ArchivedIdentitiesSnapshot {
    /// Lowercase hex pubkeys present in the latest relay-signed `kind:13535`.
    pub archived: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RelayInformationDocument {
    #[serde(default, rename = "self")]
    self_: Option<String>,
}

pub(crate) async fn fetch_relay_self(state: &AppState) -> Result<Option<String>, String> {
    fetch_relay_self_at(state, &relay_ws_url_with_override(state)).await
}

/// How long a fetched NIP-11 `self` pubkey stays valid in
/// [`AppState::relay_self_cache`]. The relay's signing identity changes only
/// on an operator-driven key rotation, so minutes of staleness are safe; the
/// TTL exists so even that rare rotation converges without an app restart.
pub(crate) const RELAY_SELF_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Read a still-fresh cached `self` pubkey for `relay_url`, if any. Fails open
/// (cache miss) on a poisoned lock — the fetch path never depends on the cache.
fn cached_relay_self(state: &AppState, relay_url: &str) -> Option<String> {
    let cache = state.relay_self_cache.lock().ok()?;
    let (fetched_at, relay_self) = cache.get(relay_url)?;
    (fetched_at.elapsed() < RELAY_SELF_CACHE_TTL).then(|| relay_self.clone())
}

/// Like [`fetch_relay_self`] but reads NIP-11 from an explicit relay WS URL
/// instead of re-resolving the workspace override. Used by
/// [`fetch_archived_pubkeys_at`] so the advertised signer and the snapshot
/// query belong to the same captured relay target.
///
/// Successful lookups are cached per relay URL for [`RELAY_SELF_CACHE_TTL`]:
/// send-time agent revalidation calls this on every agent-mention send, and
/// the uncached GET was a measurable slice of that latency. Only a verified
/// `Some` is cached — `Ok(None)` covers transient states (non-2xx status, a
/// document momentarily missing `self`) that must be re-tried, not pinned.
pub(crate) async fn fetch_relay_self_at(
    state: &AppState,
    relay_url: &str,
) -> Result<Option<String>, String> {
    if let Some(cached) = cached_relay_self(state, relay_url) {
        return Ok(Some(cached));
    }

    let http_url = relay_http_base_url(relay_url);
    let response = state
        .http_client
        .get(&http_url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .map_err(|e| classify_request_error(&e))?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let doc = response
        .json::<RelayInformationDocument>()
        .await
        .map_err(|_| "relay returned malformed NIP-11 document".to_string())?;

    let Some(relay_self) = doc.self_.map(|value| value.to_ascii_lowercase()) else {
        return Ok(None);
    };

    if relay_self.len() == 64 && relay_self.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Ok(mut cache) = state.relay_self_cache.lock() {
            cache.insert(
                relay_url.to_string(),
                (std::time::Instant::now(), relay_self.clone()),
            );
        }
        Ok(Some(relay_self))
    } else {
        Ok(None)
    }
}

fn archived_pubkeys_from_snapshot(snapshot: &nostr::Event) -> Vec<String> {
    snapshot
        .tags
        .iter()
        .filter_map(|t| {
            let slice = t.as_slice();
            if slice.first().map(String::as_str) == Some("p") && slice.len() >= 2 {
                let pk = slice[1].to_ascii_lowercase();
                if pk.len() == 64 && pk.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(pk);
                }
            }
            None
        })
        .collect()
}

/// Read the relay's latest valid `kind:13535` archive snapshot as lowercase
/// hex pubkeys. Shared by the `list_archived_identities` command (frontend
/// flair) and the backend nest regen (excluding archived agents from
/// `AGENTS.md`).
///
/// Per NIP-IA §Client Behavior and §Snapshot and Delta Consistency, only a
/// snapshot signed by the relay identity advertised in NIP-11 `self` can affect
/// archive state. Every failure path — no stable `self`, no snapshot, a bad
/// signature or wrong author, or a query error — **fails open** with an empty
/// set rather than trusting unauthenticated relay-authoritative state.
pub(crate) async fn fetch_archived_pubkeys(state: &AppState) -> Vec<String> {
    fetch_archived_pubkeys_at(state, &capture_relay_target(state)).await
}

/// Like [`fetch_archived_pubkeys`] but resolves both the NIP-11 signer and the
/// snapshot query against one captured [`RelayTarget`] instead of re-reading
/// the workspace override for each. This keeps a regeneration's advertised
/// signer and its snapshot query on the same relay even if the workspace
/// override changes between the two awaits.
pub(crate) async fn fetch_archived_pubkeys_at(
    state: &AppState,
    target: &RelayTarget,
) -> Vec<String> {
    let Ok(Some(relay_self)) = fetch_relay_self_at(state, &target.ws_url).await else {
        return vec![];
    };

    let query = query_relay_at(
        state,
        &target.api_base_url,
        &[serde_json::json!({
            "authors": [relay_self.clone()],
            "kinds": [13535],
            "limit": 1,
        })],
    )
    .await;
    let Ok(events) = query else {
        return vec![];
    };

    let Some(snapshot) = events.into_iter().next() else {
        return vec![];
    };

    // Defense-in-depth: the filter should already restrict author, but the
    // client must still reject malformed or wrongly signed relay state.
    if !snapshot.verify_id() || !snapshot.verify_signature() {
        return vec![];
    }
    if !snapshot.pubkey.to_hex().eq_ignore_ascii_case(&relay_self) {
        return vec![];
    }

    archived_pubkeys_from_snapshot(&snapshot)
}

/// Read the relay's latest valid `kind:13535` archive snapshot. The frontend
/// caches this and tests membership client-side to drive the "Archived" flair.
#[tauri::command]
pub async fn list_archived_identities(
    state: State<'_, AppState>,
) -> Result<ArchivedIdentitiesSnapshot, String> {
    Ok(ArchivedIdentitiesSnapshot {
        archived: fetch_archived_pubkeys(&state).await,
    })
}

/// Read the active relay's NIP-11 `self` pubkey (its own signing key, hex).
///
/// A public, unauthenticated document read reused by the moderation UI to tell
/// whether a DM peer is the relay identity (a moderation DM). Fails open: an
/// unreachable relay, a document without `self`, or a malformed value all
/// return `None`, and callers must treat that as "not the relay" — the disable
/// is an affordance, not enforcement, so a false negative is the safe failure.
#[tauri::command]
pub async fn get_relay_self(state: State<'_, AppState>) -> Result<Option<String>, String> {
    fetch_relay_self(&state).await
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counting [`NestRegenTrigger`] double: records how many times the core
    /// fires regeneration on the successful-submit path, standing in for the
    /// production `AppHandle` binding without a live Tauri runtime.
    #[cfg(not(target_os = "windows"))]
    #[derive(Default)]
    struct CountingRegen(AtomicUsize);

    #[cfg(not(target_os = "windows"))]
    impl CountingRegen {
        fn count(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[cfg(not(target_os = "windows"))]
    impl NestRegenTrigger for CountingRegen {
        fn trigger(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Build a fake `kind:0` with a valid NIP-OA auth tag for a fresh owner.
    fn kind0_with_auth(agent: &Keys, owner: &Keys) -> nostr::Event {
        // Compute auth tag via buzz-sdk (nostr 0.36) and bridge.
        let agent_hex = agent.public_key().to_hex();
        let agent_compat = nostr::PublicKey::from_hex(&agent_hex).unwrap();
        let owner_compat_secret =
            nostr::SecretKey::from_slice(owner.secret_key().as_secret_bytes()).unwrap();
        let owner_compat_keys = nostr::Keys::new(owner_compat_secret);
        let tag_json =
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_compat_keys, &agent_compat, "")
                .expect("compute_auth_tag");
        let compat_tag = buzz_sdk_pkg::nip_oa::parse_auth_tag(&tag_json).unwrap();
        let tag = Tag::parse(compat_tag.as_slice()).unwrap();
        EventBuilder::new(Kind::Metadata, "{}")
            .tags([tag])
            .sign_with_keys(agent)
            .unwrap()
    }

    #[test]
    fn extract_oa_owner_returns_owner_for_valid_tag() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let kind0 = kind0_with_auth(&agent, &owner);

        let (recovered, raw) = extract_oa_owner(&kind0).expect("auth tag should verify");
        assert_eq!(recovered, owner.public_key().to_hex());
        assert_eq!(raw[0], "auth");
        assert_eq!(raw[1], owner.public_key().to_hex());
        // conditions empty by construction
        assert_eq!(raw[2], "");
        assert_eq!(raw[3].len(), 128);
    }

    #[test]
    fn extract_oa_owner_ignores_kind0_without_auth_tag() {
        let agent = Keys::generate();
        let kind0 = EventBuilder::new(Kind::Metadata, "{}")
            .sign_with_keys(&agent)
            .unwrap();
        assert!(extract_oa_owner(&kind0).is_none());
    }

    #[test]
    fn archived_pubkeys_from_snapshot_accepts_only_valid_p_tags() {
        let relay = Keys::generate();
        let valid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let uppercase = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
        let snapshot = EventBuilder::new(Kind::Custom(13535), "")
            .tags([
                Tag::parse(["-"]).unwrap(),
                Tag::parse(["p", valid]).unwrap(),
                Tag::parse(["p", uppercase]).unwrap(),
                Tag::parse(["p", "not-hex"]).unwrap(),
            ])
            .sign_with_keys(&relay)
            .unwrap();

        let expected = vec![valid.to_string(), uppercase.to_ascii_lowercase()];
        assert_eq!(archived_pubkeys_from_snapshot(&snapshot), expected);
    }

    #[test]
    fn relay_information_document_reads_nip11_self_field() {
        let doc: RelayInformationDocument = serde_json::from_str(
            r#"{"name":"test relay","self":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        )
        .expect("NIP-11 document");

        assert_eq!(
            doc.self_.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );
    }

    /// Spawn a loopback NIP-11 endpoint that counts hits and serves `self_hex`
    /// (or a bare 503 when `self_hex` is `None`). Returns the `ws://` base and
    /// the shared hit counter.
    async fn spawn_nip11_relay(self_hex: Option<String>) -> (String, std::sync::Arc<AtomicUsize>) {
        use axum::{http::StatusCode, routing::get, Json, Router};

        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let route_hits = hits.clone();
        let router = Router::new().route(
            "/",
            get(move || {
                let self_hex = self_hex.clone();
                let route_hits = route_hits.clone();
                async move {
                    route_hits.fetch_add(1, Ordering::SeqCst);
                    match self_hex {
                        Some(self_hex) => Ok(Json(serde_json::json!({ "self": self_hex }))),
                        None => Err(StatusCode::SERVICE_UNAVAILABLE),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });
        (format!("ws://{addr}"), hits)
    }

    /// The send path revalidates agent mentions on every agent-mention send,
    /// and each pass used to re-GET the NIP-11 document. A second lookup
    /// within the TTL must be served from [`AppState::relay_self_cache`]
    /// without touching the relay. RED-on-revert: drop the `cached_relay_self`
    /// check and the hit counter reads 2.
    #[tokio::test]
    async fn relay_self_second_fetch_within_ttl_is_served_from_cache() {
        let self_hex = Keys::generate().public_key().to_hex();
        let (relay_url, hits) = spawn_nip11_relay(Some(self_hex.clone())).await;
        let state = crate::app_state::build_app_state();

        let first = fetch_relay_self_at(&state, &relay_url).await.unwrap();
        let second = fetch_relay_self_at(&state, &relay_url).await.unwrap();

        assert_eq!(first.as_deref(), Some(self_hex.as_str()));
        assert_eq!(second.as_deref(), Some(self_hex.as_str()));
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "the second in-TTL lookup must not re-GET the NIP-11 document"
        );
    }

    /// A non-success NIP-11 response yields `Ok(None)` and MUST stay
    /// retryable: caching the outage would blank the agent directory (and
    /// every send-time revalidation) for the full TTL after one relay blip.
    #[tokio::test]
    async fn relay_self_non_success_response_is_not_cached() {
        let (relay_url, hits) = spawn_nip11_relay(None).await;
        let state = crate::app_state::build_app_state();

        assert_eq!(fetch_relay_self_at(&state, &relay_url).await.unwrap(), None);
        assert_eq!(fetch_relay_self_at(&state, &relay_url).await.unwrap(), None);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "a failed lookup must retry the relay, never pin the outage"
        );
    }

    /// An entry older than [`RELAY_SELF_CACHE_TTL`] must be refetched so a
    /// relay-side key rotation converges without an app restart.
    #[tokio::test]
    async fn relay_self_expired_cache_entry_is_refetched() {
        let self_hex = Keys::generate().public_key().to_hex();
        let (relay_url, hits) = spawn_nip11_relay(Some(self_hex.clone())).await;
        let state = crate::app_state::build_app_state();
        // `Instant` is opaque, so expiry is staged by planting an already-stale
        // entry rather than sleeping through the TTL. Skip (vacuous pass) if
        // the platform clock cannot represent an instant that far back.
        let Some(stale_instant) = std::time::Instant::now()
            .checked_sub(RELAY_SELF_CACHE_TTL + std::time::Duration::from_secs(1))
        else {
            return;
        };
        state
            .relay_self_cache
            .lock()
            .unwrap()
            .insert(relay_url.clone(), (stale_instant, "b".repeat(64)));

        let refreshed = fetch_relay_self_at(&state, &relay_url).await.unwrap();

        assert_eq!(refreshed.as_deref(), Some(self_hex.as_str()));
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "an expired entry must be refetched from the relay"
        );
    }

    /// Spec test-vector regression for gotcha #3: the NIP-OA preimage subject
    /// is the *target/agent* pubkey, not the request signer. The vectors in
    /// `docs/nips/NIP-IA.md` §Test Vectors fix concrete values; verifying the
    /// vector's `auth` tag under the vector's agent pubkey MUST yield the
    /// vector's owner pubkey. If our `extract_oa_owner` ever stops using the
    /// agent pubkey as the preimage subject, this test fails loudly.
    #[test]
    fn extract_oa_owner_matches_nip_ia_test_vector() {
        // From docs/nips/NIP-IA.md §Test Vectors → "NIP-OA auth tag".
        const AGENT_HEX: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        const OWNER_HEX: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        const CONDITIONS: &str = "kind=1&created_at<1713957000";
        const SIG: &str = "8b7df2575caf0a108374f8471722b233c53f9ff827a8b0f91861966c3b9dd5cb2e189eae9f49d72187674c2f5bd244145e10ff86c9f257ffe65a1ee5f108b369";

        // We don't have the agent's secret key (it's `0x...02` in the spec, but
        // we don't need to re-sign a kind:0 — we just need a kind:0 whose
        // `pubkey` is AGENT_HEX and whose tags carry this auth tag). Sign with
        // a *different* agent and then construct an unsigned-event-shaped
        // struct ourselves. nostr 0.37 doesn't easily allow forging `pubkey`
        // mismatched with the signing key, so we build via the public
        // constructor that requires a key — and for THIS test, the kind:0
        // signature is not checked (we only call extract_oa_owner which reads
        // the event's pubkey field and the auth tag bytes).
        let agent_secret = nostr::SecretKey::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000002",
        )
        .unwrap();
        let agent_keys = nostr::Keys::new(agent_secret);
        assert_eq!(agent_keys.public_key().to_hex(), AGENT_HEX);

        let auth_tag = nostr::Tag::parse(["auth", OWNER_HEX, CONDITIONS, SIG]).unwrap();
        let kind0 = EventBuilder::new(Kind::Metadata, "{}")
            .tags([auth_tag])
            .sign_with_keys(&agent_keys)
            .unwrap();

        let (owner, raw) = extract_oa_owner(&kind0).expect("spec vector should verify");
        assert_eq!(owner, OWNER_HEX);
        assert_eq!(raw[1], OWNER_HEX);
        assert_eq!(raw[2], CONDITIONS);
        assert_eq!(raw[3], SIG);
    }

    /// Regression: the frontend sends the request payload in camelCase
    /// (`targetPubkey`, `replacedBy`); these structs MUST deserialize it.
    /// Without `#[serde(rename_all = "camelCase")]` the archive/unarchive
    /// commands fail to deserialize at runtime — a failure the e2e mock hides
    /// because it returns before parsing the payload. Red-if-broken guard.
    #[test]
    fn archive_request_deserializes_camel_case_payload() {
        let req: ArchiveRequest = serde_json::from_str(
            r#"{"targetPubkey":"abc","content":"bye","reason":"bot-rebuilt","replacedBy":"def"}"#,
        )
        .expect("camelCase archive payload must deserialize");
        assert_eq!(req.target_pubkey, "abc");
        assert_eq!(req.content, "bye");
        assert_eq!(req.reason.as_deref(), Some("bot-rebuilt"));
        assert_eq!(req.replaced_by.as_deref(), Some("def"));

        // Minimal payload (only the required field) still deserializes.
        let minimal: UnarchiveRequest =
            serde_json::from_str(r#"{"targetPubkey":"abc"}"#).expect("minimal payload");
        assert_eq!(minimal.target_pubkey, "abc");
        assert_eq!(minimal.content, "");
        assert!(minimal.reason.is_none());
    }

    /// Regression for the cross-relay capture defect: `fetch_archived_pubkeys_at`
    /// must resolve BOTH the NIP-11 signer and the `/query` snapshot against the
    /// single captured [`RelayTarget`], never re-reading the live workspace
    /// override. Two loopback relays advertise distinct signers and archive
    /// distinct pubkeys; we capture relay A, then mutate the override to relay B
    /// before the fetch. Because capture happens once up front, the override's
    /// value at any later instant — including between the two archive awaits —
    /// is irrelevant by construction, so setting it to B is the strongest form
    /// of that perturbation. A must supply both the signer and the snapshot.
    ///
    /// RED-on-revert: restore `fetch_archived_pubkeys` to read the override for
    /// each leg (`fetch_relay_self` + `query_relay`) and this returns B's pubkey.
    #[tokio::test]
    async fn archived_fetch_never_crosses_relays_mid_flight() {
        use crate::app_state::build_app_state;
        use crate::relay_admission::{reset_rate_limit_gate, TEST_SERIAL};
        use axum::{routing::get, routing::post, Json, Router};

        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();

        // Build a loopback relay that advertises `relay_keys` as its NIP-11
        // `self` and serves a relay-signed 13535 snapshot archiving `archived`.
        async fn spawn_relay(relay_keys: Keys, archived: String) -> String {
            let self_hex = relay_keys.public_key().to_hex();
            let snapshot = EventBuilder::new(Kind::Custom(13535), "")
                .tags([
                    Tag::parse(["-"]).unwrap(),
                    Tag::parse(["p", &archived]).unwrap(),
                ])
                .sign_with_keys(&relay_keys)
                .unwrap();
            let snapshot_json = serde_json::to_value(&snapshot).unwrap();

            let router = Router::new()
                .route(
                    "/",
                    get(move || {
                        let self_hex = self_hex.clone();
                        async move { Json(serde_json::json!({ "self": self_hex })) }
                    }),
                )
                .route(
                    "/query",
                    post(move || {
                        let snapshot_json = snapshot_json.clone();
                        async move { Json(serde_json::json!([snapshot_json])) }
                    }),
                );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            tokio::spawn(async move {
                axum::serve(listener, router).await.ok();
            });
            format!("ws://{addr}")
        }

        let relay_a_keys = Keys::generate();
        let relay_b_keys = Keys::generate();
        // Distinct archived pubkeys, unrelated to either relay's signing key —
        // nostr 0.37's EventBuilder silently drops a `p` tag that references the
        // event's own signer, so the archived key must not equal the relay key.
        let archived_on_a = Keys::generate().public_key().to_hex();
        let archived_on_b = Keys::generate().public_key().to_hex();
        let relay_a = spawn_relay(relay_a_keys, archived_on_a.clone()).await;
        let relay_b = spawn_relay(relay_b_keys, archived_on_b.clone()).await;

        let state = build_app_state();

        // Capture relay A, then swap the override to relay B before the fetch.
        *state.relay_url_override.lock().unwrap() = Some(relay_a.clone());
        let target = capture_relay_target(&state);
        *state.relay_url_override.lock().unwrap() = Some(relay_b.clone());

        let archived = fetch_archived_pubkeys_at(&state, &target).await;

        assert_eq!(
            archived,
            vec![archived_on_a],
            "signer and snapshot must both come from the captured relay A, \
             never the mutated override (relay B)"
        );
        reset_rate_limit_gate();
    }

    /// Spawn a loopback `/events` relay that answers every submit with the
    /// given `accepted` verdict, so the archive/unarchive cores see a real
    /// success or rejection over the wire. Returns the `ws://` base.
    ///
    /// The literal `/events` route below is why this file carries an
    /// `EVENTS_INVENTORY` row (one occurrence, zero guard calls): a test
    /// loopback, never a production egress site.
    #[cfg(not(target_os = "windows"))]
    async fn spawn_submit_relay(accepted: bool) -> String {
        use axum::{routing::post, Json, Router};

        let router = Router::new()
            .route(
                "/events",
                post(move || async move {
                    Json(serde_json::json!({
                        "event_id": "e".repeat(64),
                        "accepted": accepted,
                        "message": if accepted { "" } else { "rejected by relay" },
                    }))
                }),
            )
            // The cores resolve the owner-of-agent auth tag first, which reads
            // the target's live kind:0; answer with an empty result set so that
            // read resolves to "no owner tag" without a live upstream relay.
            .route("/query", post(|| async { Json(serde_json::json!([])) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });
        format!("ws://{addr}")
    }

    /// Regression for the outsider-reported item 1, archive site: a successful
    /// `kind:9035` archive MUST trigger nest regeneration, and a rejected
    /// submit MUST NOT. This drives the production [`archive_identity_core`]
    /// (the exact seam the command wrapper delegates to), forwarding a counting
    /// hook against a loopback relay. RED-on-revert: replace the core's
    /// forwarded `on_success` with `|| {}` and the "fires once" assertion fails;
    /// this pins the archive command's callback independently of unarchive.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn archive_core_fires_regen_only_on_accepted_submit() {
        use crate::app_state::build_app_state;
        use crate::relay_admission::{reset_rate_limit_gate, TEST_SERIAL};

        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();

        let state = build_app_state();
        let req = ArchiveRequest {
            target_pubkey: Keys::generate().public_key().to_hex(),
            content: String::new(),
            reason: None,
            replaced_by: None,
        };

        // Accepted archive → hook fires exactly once.
        *state.relay_url_override.lock().unwrap() = Some(spawn_submit_relay(true).await);
        let regen = CountingRegen::default();
        let response = archive_identity_core(&req, &state, &regen)
            .await
            .expect("accepted archive returns Ok");
        assert!(response.accepted);
        assert_eq!(
            regen.count(),
            1,
            "an accepted archive must trigger regeneration exactly once"
        );

        // Rejected submit → error propagates, hook never fires.
        *state.relay_url_override.lock().unwrap() = Some(spawn_submit_relay(false).await);
        let regen = CountingRegen::default();
        let result = archive_identity_core(&req, &state, &regen).await;
        assert!(result.is_err(), "a rejected archive must return an error");
        assert_eq!(
            regen.count(),
            0,
            "a rejected archive changed nothing, so regeneration must not fire"
        );

        reset_rate_limit_gate();
    }

    /// Regression for item 1, unarchive site: mirrors
    /// [`archive_core_fires_regen_only_on_accepted_submit`] against the
    /// `kind:9036` [`unarchive_identity_core`]. RED-on-revert: replace that
    /// core's forwarded `on_success` with `|| {}` and this fails while the
    /// archive test stays green — proving each command's callback is pinned
    /// independently, not just the shared `submit_then_regenerate`.
    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn unarchive_core_fires_regen_only_on_accepted_submit() {
        use crate::app_state::build_app_state;
        use crate::relay_admission::{reset_rate_limit_gate, TEST_SERIAL};

        let _serial = TEST_SERIAL.lock().await;
        reset_rate_limit_gate();

        let state = build_app_state();
        let req = UnarchiveRequest {
            target_pubkey: Keys::generate().public_key().to_hex(),
            content: String::new(),
            reason: None,
        };

        // Accepted unarchive → hook fires exactly once.
        *state.relay_url_override.lock().unwrap() = Some(spawn_submit_relay(true).await);
        let regen = CountingRegen::default();
        let response = unarchive_identity_core(&req, &state, &regen)
            .await
            .expect("accepted unarchive returns Ok");
        assert!(response.accepted);
        assert_eq!(
            regen.count(),
            1,
            "an accepted unarchive must trigger regeneration exactly once"
        );

        // Rejected submit → error propagates, hook never fires.
        *state.relay_url_override.lock().unwrap() = Some(spawn_submit_relay(false).await);
        let regen = CountingRegen::default();
        let result = unarchive_identity_core(&req, &state, &regen).await;
        assert!(result.is_err(), "a rejected unarchive must return an error");
        assert_eq!(
            regen.count(),
            0,
            "a rejected unarchive changed nothing, so regeneration must not fire"
        );

        reset_rate_limit_gate();
    }
}
