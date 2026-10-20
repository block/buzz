//! Shared thread catch-up recaps over existing NIP-AR artifacts (kind 45010).
//!
//! A shared recap is a `buzz.thread_catch_up` artifact homed in the thread's
//! channel (`h`) and anchored to its root (`root`). Its content is
//! `{version:1, recap, correction?:{pointId,text}}`. The recap is immutable:
//! every later revision copies it unchanged, and a revision's `correction` is
//! the one correction that revision introduced. Corrections, their authors,
//! and their times are therefore read from signed revision history, never from
//! caller-supplied or payload-claimed attribution. See `docs/thread-catch-up.md`.
//!
//! The relay's channel read/write permissions govern every operation; this
//! module adds no kind, endpoint, or relay behavior.
use std::collections::{HashMap, HashSet};

use buzz_core_pkg::artifact::{self, ArtifactOp};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::State;

use super::thread_catch_up::{
    assert_catch_up_scope, validate_catch_up_recap_at, CatchUpRecap, CatchUpScope,
};
use crate::{
    app_state::AppState,
    relay::{
        assert_expected_relay_scope, assert_expected_signer, query_relay_at_with_keys,
        relay_api_base_url_with_override, submit_signed_event_at_with_keys,
    },
};

/// NIP-AR content type for shared thread catch-up recaps.
pub const THREAD_CATCH_UP_TYPE: &str = "buzz.thread_catch_up";
const ARTIFACT_KIND: u16 = 45010;
const TITLE: &str = "Thread catch-up";
/// Most recently changed shared recaps listed per thread. Reads fetch one
/// more to detect that the list was capped.
const LIST_LIMIT: usize = 50;
/// History page size and page cap: at most 2,000 revisions per read.
const HISTORY_PAGE: usize = 500;
const HISTORY_MAX_PAGES: usize = 4;
/// Read-side bounds for untrusted payloads, mirroring the generation limits
/// (12 points, 8 KiB of point text) plus room for cited context IDs.
const MAX_POINTS: usize = 12;
const MAX_POINT_TEXT_BYTES: usize = 8 * 1024;
/// Per point and per correction; bytes also bound the frontend's UTF-16
/// length limit of 2,000.
const MAX_TEXT_BYTES: usize = 2_000;
const MAX_SOURCE_IDS: usize = 400;
const MAX_ID_BYTES: usize = 64;
const MAX_WARNINGS: usize = 20;

/// Error returned when a correction lost a `prev` race. The `conflict:` prefix
/// is stable for the frontend, which keeps the draft and reloads the recap.
pub const CATCH_UP_CONFLICT: &str =
    "conflict: this recap changed since it was loaded; your correction was not saved";
const SCOPE_CHANGED: &str =
    "active community or identity changed during the catch-up operation; nothing was saved";

/// One attributed correction, derived from a signed revision.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCatchUpCorrection {
    pub event_id: String,
    pub point_id: String,
    pub text: String,
    pub author_pubkey: String,
    pub created_at: u64,
}

/// A shared recap projected from its current revision and signed history.
///
/// `event_id` is the current revision (the next correction's `prev`).
/// `author_pubkey`/`created_at` identify the sharer only when
/// `history_complete`; otherwise they are the earliest retrieved revision's
/// signer and time, and the sharer is unknown.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedThreadCatchUp {
    pub artifact_id: String,
    pub event_id: String,
    pub author_pubkey: String,
    pub created_at: u64,
    pub recap: CatchUpRecap,
    pub corrections: Vec<ThreadCatchUpCorrection>,
    pub history_complete: bool,
}

/// The shared recaps of one thread and how much of the thread they cover.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCatchUpList {
    /// Readable recaps, most recently changed first.
    pub recaps: Vec<SharedThreadCatchUp>,
    /// Recaps within the listed window that were omitted because they are
    /// malformed, unverifiable, or rewritten.
    pub unreadable_count: usize,
    /// False when this thread has more than 50 current recaps.
    pub complete: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListThreadCatchUpInput {
    scope: CatchUpScope,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareThreadCatchUpInput {
    scope: CatchUpScope,
    recap: CatchUpRecap,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectThreadCatchUpInput {
    scope: CatchUpScope,
    artifact_id: String,
    prev_event_id: String,
    point_id: String,
    text: String,
}

/// List the recaps shared with this thread, with their correction history.
#[tauri::command]
pub async fn list_thread_catch_up_artifacts(
    input: ListThreadCatchUpInput,
    state: State<'_, AppState>,
) -> Result<ThreadCatchUpList, String> {
    let bound = bind_scope(&state, &input.scope)?;
    let shared = list_shared(&state, &bound).await?;
    ensure_still_bound(&state, &bound)?;
    Ok(shared)
}

/// Share a reviewed recap with its own thread as a new artifact.
#[tauri::command]
pub async fn share_thread_catch_up(
    input: ShareThreadCatchUpInput,
    state: State<'_, AppState>,
) -> Result<SharedThreadCatchUp, String> {
    let bound = bind_scope(&state, &input.scope)?;
    let recap = serde_json::to_value(&input.recap).map_err(|e| e.to_string())?;
    check_recap(&recap, &bound)?;
    // Older snapshots remain readable, but new shares need a frozen revision
    // for every citation so same-second edits can be detected.
    if input.recap.points.iter().any(|point| {
        point
            .source_ids
            .iter()
            .any(|id| !input.recap.source_revisions.contains_key(id))
    }) {
        return Err("source_changed: this recap has no saved source revisions; regenerate it before sharing".into());
    }
    // Revalidates the root and every cited source against the relay.
    validate_catch_up_recap_at(
        &state,
        &bound.api_base,
        &bound.keys,
        &input.scope,
        &input.recap,
    )
    .await?;
    publish_share(&state, &bound, recap).await
}

/// Add one attributed correction to a shared recap without changing it.
#[tauri::command]
pub async fn correct_thread_catch_up(
    input: CorrectThreadCatchUpInput,
    state: State<'_, AppState>,
) -> Result<SharedThreadCatchUp, String> {
    let bound = bind_scope(&state, &input.scope)?;
    publish_correction(
        &state,
        &bound,
        &input.artifact_id,
        &input.prev_event_id,
        &input.point_id,
        &input.text,
    )
    .await
}

// ── Scope ───────────────────────────────────────────────────────────────────

/// The relay base and signer captured once and used for every read and write
/// of one operation.
struct BoundScope {
    api_base: String,
    keys: Keys,
    channel_id: String,
    root_event_id: String,
}

fn bind_scope(state: &AppState, scope: &CatchUpScope) -> Result<BoundScope, String> {
    // The shared asserts treat a blank expectation as unscoped; this
    // operation always belongs to one captured community and identity.
    if scope.relay_url.trim().is_empty() || scope.signer_pubkey.trim().is_empty() {
        return Err("catch-up scope is missing its community or identity".into());
    }
    artifact::canonical_uuid(&scope.channel_id).map_err(|e| format!("invalid channel: {e}"))?;
    artifact::event_id(&scope.root_event_id).map_err(|e| format!("invalid thread root: {e}"))?;
    assert_catch_up_scope(state, scope)?;
    let api_base = relay_api_base_url_with_override(state)
        .trim_end_matches('/')
        .to_string();
    assert_expected_relay_scope(Some(&scope.relay_url), &api_base)?;
    let keys = state.signing_keys()?;
    assert_expected_signer(Some(&scope.signer_pubkey), &keys.public_key().to_hex())?;
    Ok(BoundScope {
        api_base,
        keys,
        channel_id: scope.channel_id.clone(),
        root_event_id: scope.root_event_id.clone(),
    })
}

/// Fail closed when the active relay or identity moved since [`bind_scope`].
fn ensure_still_bound(state: &AppState, bound: &BoundScope) -> Result<(), String> {
    let api_base = relay_api_base_url_with_override(state);
    let keys = state.signing_keys()?;
    if api_base.trim_end_matches('/') != bound.api_base
        || keys.public_key() != bound.keys.public_key()
    {
        return Err(SCOPE_CHANGED.into());
    }
    Ok(())
}

// ── Relay reads ─────────────────────────────────────────────────────────────

/// Current-state filter for this thread's recaps, optionally narrowed by `d`.
fn current_filter(bound: &BoundScope, ids: Option<&[String]>) -> Value {
    let mut filter = scoped_filter(bound, "current");
    filter["limit"] = json!(LIST_LIMIT + 1);
    if let Some(ids) = ids {
        filter["#d"] = json!(ids);
    }
    filter
}

/// One page of revision history for the named recaps in this thread.
fn history_filter(bound: &BoundScope, ids: &[String], page: usize) -> Value {
    let mut filter = scoped_filter(bound, "history");
    filter["#d"] = json!(ids);
    filter["limit"] = json!(HISTORY_PAGE);
    filter["offset"] = json!(page * HISTORY_PAGE);
    filter
}

fn scoped_filter(bound: &BoundScope, view: &str) -> Value {
    json!({
        "artifact": view,
        "kinds": [ARTIFACT_KIND],
        "#h": [bound.channel_id],
        "#type": [THREAD_CATCH_UP_TYPE],
        "#root": [bound.root_event_id],
    })
}

async fn query(state: &AppState, bound: &BoundScope, filter: Value) -> Result<Vec<Event>, String> {
    query_relay_at_with_keys(state, &bound.api_base, &[filter], &bound.keys, None).await
}

/// A current revision paired with its projection (or why it is unsupported).
type Loaded = (Head, Result<SharedThreadCatchUp, String>);

/// Current recaps (all, or the named ones) projected with history.
struct LoadedPage {
    loaded: Vec<Loaded>,
    /// Current revisions in the window that are not readable recaps.
    unreadable: usize,
    /// False when the relay held more than `LIST_LIMIT` current recaps.
    complete: bool,
}

/// Load and project current recaps (all, or the named ones) with history.
async fn load_shared(
    state: &AppState,
    bound: &BoundScope,
    ids: Option<&[String]>,
) -> Result<LoadedPage, String> {
    let mut current = query(state, bound, current_filter(bound, ids)).await?;
    // The relay returns the most recently changed first; keep that window.
    let complete = current.len() <= LIST_LIMIT;
    current.truncate(LIST_LIMIT);
    let mut unreadable = 0;
    let heads: Vec<Head> = current
        .into_iter()
        .filter_map(|event| match Head::parse(event, bound) {
            Ok(head) => Some(head),
            Err(error) => {
                tracing::warn!(%error, "omitting unreadable shared catch-up recap");
                unreadable += 1;
                None
            }
        })
        .collect();
    if heads.is_empty() {
        return Ok(LoadedPage {
            loaded: vec![],
            unreadable,
            complete,
        });
    }
    let ids: Vec<String> = heads.iter().map(|head| head.artifact_id.clone()).collect();
    let mut revisions = HashMap::new();
    for page in 0..HISTORY_MAX_PAGES {
        let events = query(state, bound, history_filter(bound, &ids, page)).await?;
        let last = events.len() < HISTORY_PAGE;
        for event in events {
            revisions.insert(event.id.to_hex(), event);
        }
        if last
            || heads
                .iter()
                .all(|head| chain_reaches_create(head, &revisions))
        {
            break;
        }
    }
    let loaded = heads
        .into_iter()
        .map(|head| {
            let shared = project(&head, &revisions, bound);
            (head, shared)
        })
        .collect();
    Ok(LoadedPage {
        loaded,
        unreadable,
        complete,
    })
}

async fn list_shared(state: &AppState, bound: &BoundScope) -> Result<ThreadCatchUpList, String> {
    let page = load_shared(state, bound, None).await?;
    let mut list = ThreadCatchUpList {
        recaps: Vec::with_capacity(page.loaded.len()),
        unreadable_count: page.unreadable,
        complete: page.complete,
    };
    for (_, shared) in page.loaded {
        match shared {
            Ok(shared) => list.recaps.push(shared),
            Err(error) => {
                tracing::warn!(%error, "omitting unsupported shared catch-up recap");
                list.unreadable_count += 1;
            }
        }
    }
    Ok(list)
}

/// Load exactly one shared recap in this thread.
async fn load_one(
    state: &AppState,
    bound: &BoundScope,
    artifact_id: &str,
) -> Result<(Head, SharedThreadCatchUp), String> {
    let page = load_shared(state, bound, Some(&[artifact_id.to_string()])).await?;
    if page.unreadable > 0 {
        return Err("this shared recap cannot be read by this client".into());
    }
    let [(head, shared)] = <[Loaded; 1]>::try_from(page.loaded)
        .map_err(|_| "this shared recap is no longer available in this thread".to_string())?;
    Ok((head, shared?))
}

// ── Projection ──────────────────────────────────────────────────────────────

/// A verified current revision whose envelope and content are a recap in
/// this thread.
struct Head {
    artifact_id: String,
    event: Event,
    /// Complete content, including fields this client does not interpret.
    content: Map<String, Value>,
    recap: Value,
}

impl Head {
    fn parse(event: Event, bound: &BoundScope) -> Result<Self, String> {
        let envelope = check_revision(&event, bound, None)?;
        if envelope.op == ArtifactOp::Delete {
            return Err("recap is deleted".into());
        }
        let (content, recap) = parse_content(&event.content)?;
        check_recap(&recap, bound)?;
        Ok(Self {
            artifact_id: envelope.id.to_string(),
            event,
            content,
            recap,
        })
    }
}

/// Verify a revision's signature and that its envelope belongs to this
/// thread's recaps (and to `artifact_id`, when given). Relay filters are not
/// trusted to have done this.
fn check_revision(
    event: &Event,
    bound: &BoundScope,
    artifact_id: Option<&str>,
) -> Result<artifact::ArtifactEnvelope, String> {
    event
        .verify()
        .map_err(|_| "revision signature is invalid")?;
    let envelope = artifact::validate(event).map_err(|e| format!("invalid artifact: {e}"))?;
    if envelope.artifact_type != THREAD_CATCH_UP_TYPE
        || envelope.home.to_string() != bound.channel_id
        || envelope.root.as_deref().map(hex::encode).as_deref()
            != Some(bound.root_event_id.as_str())
        || artifact_id.is_some_and(|id| envelope.id.to_string() != id)
    {
        return Err("revision is not a recap in this thread".into());
    }
    Ok(envelope)
}

/// Parse revision content, returning it whole alongside its recap.
fn parse_content(content: &str) -> Result<(Map<String, Value>, Value), String> {
    let Ok(Value::Object(content)) = serde_json::from_str::<Value>(content) else {
        return Err("recap content is not a JSON object".into());
    };
    if content.get("version") != Some(&json!(1)) {
        return Err("unsupported recap content version".into());
    }
    let recap = content
        .get("recap")
        .cloned()
        .ok_or("recap content has no recap")?;
    Ok((content, recap))
}

/// Follow `prev` from the head to the creating revision; any gap means the
/// history is incomplete.
fn chain_reaches_create(head: &Head, revisions: &HashMap<String, Event>) -> bool {
    let mut cursor = &head.event;
    for _ in 0..=revisions.len() {
        match prev_of(cursor) {
            None => return true,
            Some(prev) => match revisions.get(&prev) {
                Some(event) => cursor = event,
                None => return false,
            },
        }
    }
    false
}

fn prev_of(event: &Event) -> Option<String> {
    event
        .tags
        .iter()
        .map(Tag::as_slice)
        .find(|tag| tag.len() == 2 && tag[0] == "prev")
        .map(|tag| tag[1].clone())
}

/// Project a head and its signed history. Fails when any reachable revision
/// is unverifiable, outside this thread, or rewrote the recap: such an
/// artifact is not an immutable recap this client can attribute or correct.
fn project(
    head: &Head,
    revisions: &HashMap<String, Event>,
    bound: &BoundScope,
) -> Result<SharedThreadCatchUp, String> {
    let recap_value = &head.recap;
    let point_ids: HashSet<&str> = recap_value["points"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|point| point["id"].as_str())
        .collect();
    let mut chain = vec![&head.event];
    let mut seen = HashSet::from([head.event.id]);
    let mut complete = false;
    loop {
        let cursor = chain[chain.len() - 1];
        let Some(prev) = prev_of(cursor) else {
            complete = true;
            break;
        };
        let Some(event) = revisions.get(&prev) else {
            break;
        };
        if !seen.insert(event.id) {
            return Err("recap history is cyclic".into());
        }
        chain.push(event);
    }
    chain.reverse();
    let mut corrections = Vec::new();
    let mut previous_correction = None;
    for (index, event) in chain.iter().enumerate() {
        let envelope = check_revision(event, bound, Some(&head.artifact_id))?;
        if envelope.op == ArtifactOp::Delete {
            continue;
        }
        let (content, recap) = parse_content(&event.content)?;
        if &recap != recap_value {
            return Err("a revision rewrote the shared recap".into());
        }
        let Some(correction) = content.get("correction") else {
            previous_correction = None;
            continue;
        };
        if envelope.op == ArtifactOp::Create {
            return Err("a recap cannot be created with a correction".into());
        }
        let point_id = correction["pointId"].as_str().unwrap_or_default();
        let text = correction["text"].as_str().unwrap_or_default().trim();
        if !point_ids.contains(point_id) || check_correction_text(text).is_err() {
            return Err("a revision carries a malformed correction".into());
        }
        let introduced = envelope.op == ArtifactOp::Update
            && (index > 0 || complete)
            && previous_correction.as_ref() != Some(correction);
        previous_correction = Some(correction.clone());
        if !introduced {
            continue;
        }
        corrections.push(ThreadCatchUpCorrection {
            event_id: event.id.to_hex(),
            point_id: point_id.into(),
            text: text.into(),
            author_pubkey: event.pubkey.to_hex(),
            created_at: event.created_at.as_secs(),
        });
    }
    let origin = chain[0];
    Ok(SharedThreadCatchUp {
        artifact_id: head.artifact_id.clone(),
        event_id: head.event.id.to_hex(),
        author_pubkey: origin.pubkey.to_hex(),
        created_at: origin.created_at.as_secs(),
        recap: serde_json::from_value(recap_value.clone())
            .map_err(|e| format!("recap does not match the catch-up contract: {e}"))?,
        corrections,
        history_complete: complete,
    })
}

// ── Validation ──────────────────────────────────────────────────────────────

/// Shape-check an untrusted recap for this thread before reading or
/// publishing it. Source authority is checked separately on share.
fn check_recap(recap: &Value, bound: &BoundScope) -> Result<(), String> {
    let invalid = |reason: &str| Err(format!("invalid recap: {reason}"));
    if recap["version"] != json!(1) {
        return invalid("unsupported version");
    }
    if recap["channelId"].as_str() != Some(bound.channel_id.as_str())
        || recap["rootEventId"].as_str() != Some(bound.root_event_id.as_str())
    {
        return invalid("belongs to another thread");
    }
    match (recap["since"].as_u64(), recap["until"].as_u64()) {
        (Some(since), Some(until)) if since <= until => {}
        _ => return invalid("bad time range"),
    }
    let Some(sources) = recap["sourceIds"].as_array() else {
        return invalid("missing sources");
    };
    if sources.len() > MAX_SOURCE_IDS {
        return invalid("too many sources");
    }
    let mut known = HashSet::new();
    for source in sources {
        let source = source.as_str().unwrap_or_default();
        if artifact::event_id(source).is_err() || !known.insert(source) {
            return invalid("bad source ID");
        }
    }
    let Some(points) = recap["points"].as_array() else {
        return invalid("missing points");
    };
    if points.len() > MAX_POINTS {
        return invalid("too many points");
    }
    let mut ids = HashSet::new();
    let mut text_bytes = 0;
    for point in points {
        let id = point["id"].as_str().unwrap_or_default();
        let text = point["text"].as_str().unwrap_or_default();
        let cited = point["sourceIds"].as_array();
        if id.is_empty() || id.len() > MAX_ID_BYTES || !ids.insert(id) {
            return invalid("bad point ID");
        }
        if !matches!(point["category"].as_str(), Some("decision" | "question")) {
            return invalid("bad point category");
        }
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return invalid("empty or oversized point");
        }
        text_bytes += text.len();
        if !cited.is_some_and(|cited| {
            !cited.is_empty()
                && cited
                    .iter()
                    .all(|id| id.as_str().is_some_and(|id| known.contains(id)))
        }) {
            return invalid("point cites an unknown source");
        }
    }
    if text_bytes > MAX_POINT_TEXT_BYTES {
        return invalid("point text too large");
    }
    let coverage = &recap["coverage"];
    if coverage["messageCount"].as_u64().is_none()
        || coverage["inputMessageCount"].as_u64().is_none()
        || !coverage["truncated"].is_boolean()
        || !coverage["warnings"].as_array().is_some_and(|warnings| {
            warnings.len() <= MAX_WARNINGS && warnings.iter().all(Value::is_string)
        })
    {
        return invalid("bad coverage");
    }
    Ok(())
}

fn check_correction_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() || text.trim() != text {
        return Err("correction must be nonblank, trimmed text".into());
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(format!("correction is limited to {MAX_TEXT_BYTES} bytes"));
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("correction contains control characters".into());
    }
    Ok(())
}

// ── Writes ──────────────────────────────────────────────────────────────────

/// Stable artifact ID for one signer sharing one recap, so a retry after a
/// lost acknowledgement finds the existing share instead of duplicating it.
fn share_artifact_id(signer: &str, recap: &Value) -> String {
    let name = format!("{THREAD_CATCH_UP_TYPE}\n{signer}\n{recap}");
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, name.as_bytes()).to_string()
}

fn envelope_tags(
    artifact_id: &str,
    bound: &BoundScope,
    title: &str,
    op: &str,
    prev: Option<&str>,
) -> Vec<Vec<String>> {
    let mut tags = vec![
        vec!["ar".into(), "1".into()],
        vec!["d".into(), artifact_id.into()],
        vec!["h".into(), bound.channel_id.clone()],
        vec!["type".into(), THREAD_CATCH_UP_TYPE.into()],
        vec!["title".into(), title.into()],
        vec!["op".into(), op.into()],
        vec!["root".into(), bound.root_event_id.clone()],
    ];
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.into()]);
    }
    tags
}

fn sign_revision(
    bound: &BoundScope,
    content: &Value,
    tags: Vec<Vec<String>>,
) -> Result<Event, String> {
    let tags = tags
        .into_iter()
        .map(|tag| Tag::parse(tag).map_err(|e| format!("invalid artifact tag: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let event = EventBuilder::new(Kind::from_u16(ARTIFACT_KIND), content.to_string())
        .allow_self_tagging()
        .tags(tags)
        .sign_with_keys(&bound.keys)
        .map_err(|e| format!("failed to sign recap: {e}"))?;
    artifact::validate(&event).map_err(|e| format!("invalid artifact: {e}"))?;
    Ok(event)
}

fn build_share_event(bound: &BoundScope, recap: Value) -> Result<Event, String> {
    let artifact_id = share_artifact_id(&bound.keys.public_key().to_hex(), &recap);
    let tags = envelope_tags(&artifact_id, bound, TITLE, "create", None);
    sign_revision(bound, &json!({"version": 1, "recap": recap}), tags)
}

/// Build the correction revision from the current one: the recap and every
/// unfamiliar content field and annotation tag are copied unchanged; only
/// `correction` and the envelope's `op`/`prev` are replaced. Signer-bound
/// NIP-OA `auth` tags are not copied: they attest the previous signer.
fn build_correction_event(
    bound: &BoundScope,
    head: &Head,
    point_id: &str,
    text: &str,
) -> Result<Event, String> {
    let mut content = head.content.clone();
    content.insert(
        "correction".into(),
        json!({"pointId": point_id, "text": text}),
    );
    let title = head
        .event
        .tags
        .iter()
        .map(Tag::as_slice)
        .find(|tag| tag.len() == 2 && tag[0] == "title")
        .map(|tag| tag[1].as_str())
        .unwrap_or(TITLE);
    let mut tags = envelope_tags(
        &head.artifact_id,
        bound,
        title,
        "update",
        Some(&head.event.id.to_hex()),
    );
    const REPLACED: [&str; 10] = [
        "ar", "d", "h", "type", "title", "op", "root", "prev", "auth", "p",
    ];
    tags.extend(
        head.event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .filter(|tag| !REPLACED.contains(&tag[0].as_str())),
    );
    sign_revision(bound, &Value::Object(content), tags)
}

/// The relay answers a lost `prev` race (or a taken identity) with HTTP 409
/// and a `conflict:` reason.
fn is_conflict(error: &str) -> bool {
    error.contains("409 Conflict") || error.contains("conflict:")
}

async fn publish_share(
    state: &AppState,
    bound: &BoundScope,
    recap: Value,
) -> Result<SharedThreadCatchUp, String> {
    let event = build_share_event(bound, recap)?;
    let head = Head::parse(event.clone(), bound)?;
    ensure_still_bound(state, bound)?;
    match submit_signed_event_at_with_keys(&event, state, &bound.api_base, &bound.keys).await {
        Ok(_) => project(&head, &HashMap::new(), bound),
        Err(error) if is_conflict(&error) => {
            // Same signer and recap derive the same ID: return the earlier
            // share when it is ours and unchanged; otherwise surface the error.
            match load_one(state, bound, &head.artifact_id).await {
                Ok((existing, shared))
                    if shared.history_complete
                        && shared.author_pubkey == bound.keys.public_key().to_hex()
                        && existing.recap == head.recap =>
                {
                    Ok(shared)
                }
                _ => Err(format!(
                    "this recap was already shared but could not be loaded: {error}"
                )),
            }
        }
        Err(error) => Err(error),
    }
}

async fn publish_correction(
    state: &AppState,
    bound: &BoundScope,
    artifact_id: &str,
    prev_event_id: &str,
    point_id: &str,
    text: &str,
) -> Result<SharedThreadCatchUp, String> {
    artifact::canonical_uuid(artifact_id).map_err(|e| format!("invalid recap ID: {e}"))?;
    artifact::event_id(prev_event_id).map_err(|e| format!("invalid revision: {e}"))?;
    let text = text.trim();
    check_correction_text(text)?;
    let (head, mut shared) = load_one(state, bound, artifact_id).await?;
    // Never re-target the edit at a newer head: that would overwrite a
    // correction the user has not seen. The relay's `prev` check closes the
    // remaining window between this read and the write.
    if shared.event_id != prev_event_id {
        return Err(CATCH_UP_CONFLICT.into());
    }
    if !head.recap["points"]
        .as_array()
        .is_some_and(|points| points.iter().any(|p| p["id"].as_str() == Some(point_id)))
    {
        return Err("the corrected point is not part of this recap".into());
    }
    let event = build_correction_event(bound, &head, point_id, text)?;
    ensure_still_bound(state, bound)?;
    match submit_signed_event_at_with_keys(&event, state, &bound.api_base, &bound.keys).await {
        Ok(_) => {}
        Err(error) if is_conflict(&error) => return Err(CATCH_UP_CONFLICT.into()),
        Err(error) => return Err(error),
    }
    shared.event_id = event.id.to_hex();
    shared.corrections.push(ThreadCatchUpCorrection {
        event_id: event.id.to_hex(),
        point_id: point_id.into(),
        text: text.into(),
        author_pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs(),
    });
    Ok(shared)
}

#[cfg(test)]
#[path = "thread_catch_up_artifacts_tests.rs"]
mod tests;
