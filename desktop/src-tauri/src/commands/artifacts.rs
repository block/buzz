//! Native NIP-AR reads for the Synaxis HTML Review Canvas.
//!
//! Artifact revisions are state changes, not conversation messages, so they
//! never travel through the ordinary timeline or WebSocket filters (the relay
//! rejects NIP-AR predicates there). This module reads them over the relay's
//! HTTP `/query` bridge and refuses to hand the webview anything that has not
//! passed, in Rust:
//!
//! 1. Nostr event ID and Schnorr signature verification,
//! 2. the complete NIP-AR envelope validation shared with the relay
//!    (`buzz_core::artifact::validate`),
//! 3. exact channel, artifact identity, and artifact type binding.
//!
//! Review document bytes are fetched through the same relay media boundary as
//! other hash-bound attachments and are returned only when the declared size,
//! SHA-256, and UTF-8 encoding all agree.

use buzz_core_pkg::artifact::{self, ArtifactOp};
use nostr::Event;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::{
    app_state::AppState,
    commands::media_download::{fetch_blob_bytes_with_cap, validate_download_url},
    relay::{
        assert_expected_relay_scope, assert_expected_signer, query_relay, query_relay_at_with_keys,
        relay_api_base_url_with_override,
    },
};

/// NIP-AR `type` of a factory-generated, hash-bound HTML review revision.
pub const REVIEW_ARTIFACT_TYPE: &str = "synaxis.html-review";
/// NIP-AR `type` of a human feedback revision bound to a reviewed revision.
pub const FEEDBACK_ARTIFACT_TYPE: &str = "synaxis.artifact-feedback";
/// Largest review document Buzz will fetch and render.
pub const MAX_REVIEW_DOCUMENT_BYTES: u64 = 4 * 1024 * 1024;
/// Review artifact rows requested per relay page.
const REVIEW_LIST_PAGE_LIMIT: usize = 200;
/// Most current reviews returned to the Canvas inbox.
const MAX_REVIEW_LIST_ROWS: usize = 1000;
/// Most reviewed revisions one feedback listing may ask about.
const MAX_TARGET_REVISIONS: usize = 8;
/// Most explicit feedback revisions one listing may ask about (the
/// disposition list of a single review revision).
const MAX_FEEDBACK_IDS: usize = 100;
/// Feedback revisions requested per relay page (the relay allows up to 1000).
const FEEDBACK_PAGE_LIMIT: usize = 200;
/// Most feedback revisions one listing reads across all pages. The relay
/// orders newest first, so a listing past this keeps the newest and reports
/// `truncated`; it never drops older comments silently.
const MAX_FEEDBACK_ROWS: usize = 1000;

/// A verified review revision and the artifact's current head.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewArtifactRevision {
    /// The requested revision, already ID/signature/envelope verified.
    pub event: serde_json::Value,
    /// Event ID of the artifact's current revision (equals `event.id` when
    /// the requested revision is current).
    pub current_event_id: String,
}

/// Verified current HTML reviews for the persistent Canvas inbox.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewArtifactListing {
    /// Current, verified `synaxis.html-review` revisions, newest first.
    pub events: Vec<serde_json::Value>,
    /// Events the relay returned that failed verification and were withheld.
    pub rejected: usize,
    /// The relay holds more current reviews than one listing reads.
    pub truncated: bool,
}

/// Verified feedback revisions plus how many returned events were rejected.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFeedbackListing {
    /// Verified `synaxis.artifact-feedback` revisions, newest first.
    pub events: Vec<serde_json::Value>,
    /// Events the relay returned that failed verification and were withheld.
    pub rejected: usize,
    /// The relay holds more feedback for the requested revisions than one
    /// listing reads (`MAX_FEEDBACK_ROWS`), so the oldest comments are absent.
    pub truncated: bool,
}

fn canonical_uuid(value: &str, label: &str) -> Result<String, String> {
    artifact::canonical_uuid(value)
        .map(|id| id.to_string())
        .map_err(|_| format!("invalid {label}: expected a canonical lowercase UUID"))
}

fn canonical_event_id(value: &str, label: &str) -> Result<String, String> {
    artifact::event_id(value)
        .map(|_| value.to_string())
        .map_err(|_| format!("invalid {label}: expected 64 lowercase hex characters"))
}

fn current_revision_filter(channel_id: &str, artifact_id: &str) -> serde_json::Value {
    serde_json::json!({
        "artifact": "current",
        "#d": [artifact_id],
        "#h": [channel_id],
        "limit": 1,
    })
}

fn exact_revision_filter(revision_event_id: &str) -> serde_json::Value {
    serde_json::json!({ "ids": [revision_event_id], "limit": 1 })
}

fn current_reviews_filter(offset: usize, limit: usize) -> serde_json::Value {
    serde_json::json!({
        "artifact": "current",
        "#type": [REVIEW_ARTIFACT_TYPE],
        "limit": limit,
        "offset": offset,
    })
}

fn feedback_by_target_filter(
    channel_id: &str,
    target_revision_ids: &[String],
    offset: usize,
    limit: usize,
) -> serde_json::Value {
    serde_json::json!({
        "artifact": "current",
        "#h": [channel_id],
        "#type": [FEEDBACK_ARTIFACT_TYPE],
        "#target_revision": target_revision_ids,
        "limit": limit,
        "offset": offset,
    })
}

/// Reads one feedback query page by page until the relay returns a short page
/// or the aggregate cap is exceeded. The relay's `received_at DESC, id ASC`
/// order makes offsets stable. One row past the cap is requested so that
/// truncation is detected exactly instead of guessed from a full last page.
#[derive(Default)]
struct FeedbackPager {
    events: Vec<Event>,
    /// Limit of the page last requested, to tell a short page from a full one.
    requested: usize,
    finished: bool,
}

impl FeedbackPager {
    /// The next `(offset, limit)` to ask the relay for, or `None` when done.
    fn next_page(&mut self) -> Option<(usize, usize)> {
        if self.finished {
            return None;
        }
        let offset = self.events.len();
        self.requested = FEEDBACK_PAGE_LIMIT.min(MAX_FEEDBACK_ROWS + 1 - offset);
        Some((offset, self.requested))
    }

    fn accept(&mut self, page: Vec<Event>) {
        let short = page.len() < self.requested;
        self.events.extend(page.into_iter().take(self.requested));
        self.finished = short || self.events.len() > MAX_FEEDBACK_ROWS;
    }

    /// The rows read (at most `MAX_FEEDBACK_ROWS`) and whether more exist.
    fn finish(mut self) -> (Vec<Event>, bool) {
        let truncated = self.events.len() > MAX_FEEDBACK_ROWS;
        self.events.truncate(MAX_FEEDBACK_ROWS);
        (self.events, truncated)
    }
}

fn feedback_by_id_filter(feedback_event_ids: &[String]) -> serde_json::Value {
    serde_json::json!({ "ids": feedback_event_ids, "limit": feedback_event_ids.len() })
}

/// Fully verify one artifact event against the identity the caller asked for.
///
/// Returns the envelope only when the event ID, signature, NIP-AR envelope,
/// home channel, and type all hold and the revision is not a tombstone.
fn verify_artifact_event(
    event: &Event,
    channel_id: &str,
    artifact_type: &str,
) -> Result<artifact::ArtifactEnvelope, String> {
    event
        .verify()
        .map_err(|_| "artifact revision failed event ID or signature verification".to_string())?;
    let envelope = artifact::validate(event)
        .map_err(|reason| format!("invalid artifact envelope: {reason}"))?;
    if envelope.home.to_string() != channel_id {
        return Err("artifact revision belongs to a different channel".to_string());
    }
    if envelope.artifact_type != artifact_type {
        return Err(format!(
            "artifact revision is not a {artifact_type} (found {})",
            envelope.artifact_type
        ));
    }
    if envelope.op == ArtifactOp::Delete {
        return Err("artifact revision is a deletion".to_string());
    }
    Ok(envelope)
}

/// Select the single verified review revision with the expected identity.
///
/// `expected_event_id` pins an exact revision; otherwise the relay's `current`
/// answer must contain exactly one event. Anything else is an error rather
/// than a best guess.
fn select_review_revision(
    events: Vec<Event>,
    channel_id: &str,
    artifact_id: &str,
    expected_event_id: Option<&str>,
) -> Result<Event, String> {
    let mut candidates: Vec<Event> = events
        .into_iter()
        .filter(|event| expected_event_id.is_none_or(|id| event.id.to_hex() == id))
        .collect();
    if candidates.len() != 1 {
        return Err("review artifact revision not found".to_string());
    }
    let event = candidates.remove(0);
    let envelope = verify_artifact_event(&event, channel_id, REVIEW_ARTIFACT_TYPE)?;
    if envelope.id.to_string() != artifact_id {
        return Err("review artifact revision belongs to a different artifact".to_string());
    }
    Ok(event)
}

/// Fetch the review artifact's current head and, when asked, one exact earlier
/// revision. Both reads are verified before anything reaches the webview.
#[tauri::command]
pub async fn get_review_artifact_revision(
    channel_id: String,
    artifact_id: String,
    revision_event_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ReviewArtifactRevision, String> {
    let channel_id = canonical_uuid(&channel_id, "channel ID")?;
    let artifact_id = canonical_uuid(&artifact_id, "artifact ID")?;
    let revision_event_id = revision_event_id
        .as_deref()
        .map(|id| canonical_event_id(id, "revision event ID"))
        .transpose()?;

    let current = select_review_revision(
        query_relay(
            &state,
            &[current_revision_filter(&channel_id, &artifact_id)],
        )
        .await?,
        &channel_id,
        &artifact_id,
        None,
    )?;
    let current_event_id = current.id.to_hex();

    let event = match revision_event_id {
        Some(id) if id != current_event_id => select_review_revision(
            query_relay(&state, &[exact_revision_filter(&id)]).await?,
            &channel_id,
            &artifact_id,
            Some(&id),
        )?,
        _ => current,
    };
    Ok(ReviewArtifactRevision {
        event: serde_json::to_value(&event)
            .map_err(|e| format!("failed to encode artifact revision: {e}"))?,
        current_event_id,
    })
}

/// Keep only verified, undeleted current HTML reviews and sort them newest first.
fn partition_review_artifacts(mut events: Vec<Event>, truncated: bool) -> ReviewArtifactListing {
    events.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    let mut seen = std::collections::HashSet::new();
    let mut verified = Vec::new();
    let mut rejected = 0;
    for event in events {
        if event.verify().is_err() {
            rejected += 1;
            continue;
        }
        let Ok(envelope) = artifact::validate(&event) else {
            rejected += 1;
            continue;
        };
        if envelope.artifact_type != REVIEW_ARTIFACT_TYPE || envelope.op == ArtifactOp::Delete {
            rejected += 1;
            continue;
        }
        if seen.insert(envelope.id) {
            verified.push(event);
        }
    }
    ReviewArtifactListing {
        events: verified
            .iter()
            .filter_map(|event| serde_json::to_value(event).ok())
            .collect(),
        rejected,
        truncated,
    }
}

/// List current HTML review artifacts available on the active relay.
///
/// The HTTP artifact query applies the authenticated relay scope before
/// pagination. The native boundary then verifies every event's ID, signature,
/// envelope, type, and non-deletion state before exposing it to the webview.
#[tauri::command]
pub async fn list_review_artifacts(
    state: State<'_, AppState>,
) -> Result<ReviewArtifactListing, String> {
    let mut events = Vec::new();
    let mut finished = false;
    while !finished && events.len() <= MAX_REVIEW_LIST_ROWS {
        let offset = events.len();
        let limit = REVIEW_LIST_PAGE_LIMIT.min(MAX_REVIEW_LIST_ROWS + 1 - offset);
        let page = query_relay(&state, &[current_reviews_filter(offset, limit)]).await?;
        finished = page.len() < limit;
        events.extend(page);
    }
    let truncated = events.len() > MAX_REVIEW_LIST_ROWS;
    events.truncate(MAX_REVIEW_LIST_ROWS);
    Ok(partition_review_artifacts(events, truncated))
}

/// Keep only verified feedback revisions; count the rest as rejected.
fn partition_feedback(
    events: Vec<Event>,
    channel_id: &str,
    truncated: bool,
) -> ReviewFeedbackListing {
    let mut seen = std::collections::HashSet::new();
    let mut verified: Vec<Event> = Vec::new();
    let mut rejected = 0;
    for event in events {
        let acceptable = verify_artifact_event(&event, channel_id, FEEDBACK_ARTIFACT_TYPE)
            .is_ok_and(|envelope| envelope.op == ArtifactOp::Create);
        if !acceptable {
            // A forged twin (same ID, altered body) is rejected on its own and
            // can never shadow the genuine event that follows it.
            rejected += 1;
        } else if seen.insert(event.id) {
            verified.push(event);
        }
    }
    verified.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
    ReviewFeedbackListing {
        events: verified
            .iter()
            .filter_map(|event| serde_json::to_value(event).ok())
            .collect(),
        rejected,
        truncated,
    }
}

/// List verified human feedback revisions attached to reviewed revisions
/// (`#target_revision`) and/or named by a later revision's dispositions.
#[tauri::command]
pub async fn list_review_feedback(
    channel_id: String,
    target_revision_ids: Vec<String>,
    feedback_event_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ReviewFeedbackListing, String> {
    let channel_id = canonical_uuid(&channel_id, "channel ID")?;
    if target_revision_ids.len() > MAX_TARGET_REVISIONS {
        return Err(format!(
            "too many reviewed revisions (max {MAX_TARGET_REVISIONS})"
        ));
    }
    if feedback_event_ids.len() > MAX_FEEDBACK_IDS {
        return Err(format!(
            "too many feedback revisions (max {MAX_FEEDBACK_IDS})"
        ));
    }
    let targets = target_revision_ids
        .iter()
        .map(|id| canonical_event_id(id, "reviewed revision event ID"))
        .collect::<Result<Vec<_>, _>>()?;
    let ids = feedback_event_ids
        .iter()
        .map(|id| canonical_event_id(id, "feedback revision event ID"))
        .collect::<Result<Vec<_>, _>>()?;

    let mut events = Vec::new();
    let mut truncated = false;
    if !targets.is_empty() {
        let mut pager = FeedbackPager::default();
        while let Some((offset, limit)) = pager.next_page() {
            let filter = feedback_by_target_filter(&channel_id, &targets, offset, limit);
            pager.accept(query_relay(&state, &[filter]).await?);
        }
        let (rows, more) = pager.finish();
        events.extend(rows);
        truncated = more;
    }
    if !ids.is_empty() {
        events.extend(query_relay(&state, &[feedback_by_id_filter(&ids)]).await?);
    }
    Ok(partition_feedback(events, &channel_id, truncated))
}

/// The relay's answer to "do you hold exactly this feedback revision?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReconcileStatus {
    /// The relay holds this exact event: its feedback is accepted.
    Held,
    /// The authoritative read returned no such event. Not, by itself, proof
    /// that it was never accepted or never will be.
    Absent,
}

/// Result of [`reconcile_review_feedback_event`].
#[derive(Debug, Serialize)]
pub struct ReviewFeedbackReconcile {
    pub status: ReconcileStatus,
}

/// The current head under one feedback artifact identity, in its channel.
/// Feedback is create-only, so that head is the one revision that can exist.
fn reconcile_filter(channel_id: &str, artifact_id: &str) -> serde_json::Value {
    serde_json::json!({
        "artifact": "current",
        "#d": [artifact_id],
        "#h": [channel_id],
        "limit": 1,
    })
}

/// Whether `held` is the very event the caller signed: same ID, author, kind,
/// timestamp, tags, and content, with a valid signature. The signature bytes
/// are not compared; a re-signature of the same ID is the same event.
fn is_exact_feedback_event(held: &Event, expected: &Event) -> bool {
    held.verify().is_ok()
        && held.id == expected.id
        && held.pubkey == expected.pubkey
        && held.kind == expected.kind
        && held.created_at == expected.created_at
        && held.tags == expected.tags
        && held.content == expected.content
}

fn reconcile_status(held: &[Event], expected: &Event) -> ReconcileStatus {
    if held
        .iter()
        .any(|candidate| is_exact_feedback_event(candidate, expected))
    {
        ReconcileStatus::Held
    } else {
        ReconcileStatus::Absent
    }
}

/// The caller's event must itself be a valid feedback *create* for exactly
/// this channel and artifact before the relay is asked about it.
fn verify_feedback_for_reconcile(
    event: &Event,
    channel_id: &str,
    artifact_id: &str,
) -> Result<(), String> {
    let envelope = verify_artifact_event(event, channel_id, FEEDBACK_ARTIFACT_TYPE)?;
    if envelope.op != ArtifactOp::Create {
        return Err("feedback is create-only".to_string());
    }
    if envelope.id.to_string() != artifact_id {
        return Err("feedback event belongs to a different artifact".to_string());
    }
    Ok(())
}

/// Ask the relay, over its writer-authoritative artifact query, whether it
/// already holds exactly this signed feedback revision.
///
/// A publish whose outcome is unknown (timeout, dropped connection, a process
/// that died mid-send) may have committed on the relay with only its
/// acknowledgement lost. This read settles that before the event is resent or
/// its draft is discarded: `held` means accepted. It reads only; it never
/// submits. It fails closed when the captured relay or signer is no longer the
/// active one, and rejects (rather than answering `absent`) when the relay
/// cannot be read.
#[tauri::command]
pub async fn reconcile_review_feedback_event(
    channel_id: String,
    feedback_artifact_id: String,
    event: Event,
    expected_relay_url: String,
    expected_signer_pubkey: String,
    state: State<'_, AppState>,
) -> Result<ReviewFeedbackReconcile, String> {
    let channel_id = canonical_uuid(&channel_id, "channel ID")?;
    let artifact_id = canonical_uuid(&feedback_artifact_id, "feedback artifact ID")?;
    if expected_relay_url.trim().is_empty() || expected_signer_pubkey.trim().is_empty() {
        return Err("feedback reconciliation needs its captured relay and signer".to_string());
    }
    verify_feedback_for_reconcile(&event, &channel_id, &artifact_id)?;

    let relay_base = relay_api_base_url_with_override(&state);
    assert_expected_relay_scope(Some(&expected_relay_url), &relay_base)?;
    let keys = state.signing_keys()?;
    assert_expected_signer(Some(&expected_signer_pubkey), &keys.public_key().to_hex())?;
    if event.pubkey != keys.public_key() {
        return Err("feedback was not signed by the active identity".to_string());
    }

    let held = query_relay_at_with_keys(
        &state,
        &relay_base,
        &[reconcile_filter(&channel_id, &artifact_id)],
        &keys,
        None,
    )
    .await?;
    Ok(ReviewFeedbackReconcile {
        status: reconcile_status(&held, &event),
    })
}

/// Check fetched review bytes against the declared size and SHA-256 and the
/// UTF-8 requirement. Pure so every rejection is testable without HTTP.
fn verify_review_document(
    bytes: &[u8],
    expected_sha256: &str,
    expected_size: u64,
) -> Result<(), String> {
    if bytes.len() as u64 != expected_size {
        return Err(format!(
            "size mismatch: fetched {} bytes but {} were declared",
            bytes.len(),
            expected_size
        ));
    }
    if hex::encode(Sha256::digest(bytes)) != expected_sha256 {
        return Err("hash mismatch: fetched bytes do not match the declared SHA-256".to_string());
    }
    if std::str::from_utf8(bytes).is_err() {
        return Err("review document is not valid UTF-8".to_string());
    }
    Ok(())
}

/// Fetch one hash-bound, size-bounded review document from the relay's media
/// store. The bytes cross IPC only after size, SHA-256, and UTF-8 agree.
#[tauri::command]
pub async fn fetch_review_document(
    url: String,
    expected_sha256: String,
    expected_size: u64,
    state: State<'_, AppState>,
) -> Result<tauri::ipc::Response, String> {
    validate_download_url(&url, &relay_api_base_url_with_override(&state))?;
    let expected_sha256 = canonical_event_id(&expected_sha256, "document SHA-256")?;
    if expected_size == 0 || expected_size > MAX_REVIEW_DOCUMENT_BYTES {
        return Err(format!(
            "review document size must be between 1 and {MAX_REVIEW_DOCUMENT_BYTES} bytes"
        ));
    }
    // The declared size is also the streaming cap: a longer body aborts early.
    let bytes = fetch_blob_bytes_with_cap(&url, &state, expected_size, None).await?;
    verify_review_document(&bytes, &expected_sha256, expected_size)?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    const CHANNEL: &str = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
    const ARTIFACT: &str = "04737c81-e5e8-4412-bb47-f446813cfeba";
    const OTHER_ARTIFACT: &str = "14737c81-e5e8-4412-bb47-f446813cfeba";
    const PREV: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn artifact_event(
        keys: &Keys,
        artifact_type: &str,
        channel: &str,
        artifact: &str,
        op: &str,
    ) -> Event {
        let mut tags: Vec<Vec<String>> = vec![
            vec!["ar".into(), "1".into()],
            vec!["d".into(), artifact.into()],
            vec!["h".into(), channel.into()],
            vec!["type".into(), artifact_type.into()],
            vec!["op".into(), op.into()],
        ];
        if op != "delete" {
            tags.push(vec!["title".into(), "Checkout review".into()]);
        }
        if op != "create" {
            tags.push(vec!["prev".into(), PREV.into()]);
        }
        EventBuilder::new(Kind::Custom(45010), if op == "delete" { "" } else { "{}" })
            .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
            .sign_with_keys(keys)
            .unwrap()
    }

    fn review(keys: &Keys) -> Event {
        artifact_event(keys, REVIEW_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "create")
    }

    fn mutate(event: &Event, edit: impl FnOnce(&mut serde_json::Value)) -> Event {
        let mut value = serde_json::to_value(event).unwrap();
        edit(&mut value);
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn filters_use_explicit_artifact_and_exact_id_semantics() {
        assert_eq!(
            current_revision_filter(CHANNEL, ARTIFACT),
            serde_json::json!({
                "artifact": "current", "#d": [ARTIFACT], "#h": [CHANNEL], "limit": 1
            })
        );
        assert_eq!(
            current_reviews_filter(200, 50),
            serde_json::json!({
                "artifact": "current",
                "#type": [REVIEW_ARTIFACT_TYPE],
                "limit": 50,
                "offset": 200
            })
        );
        assert_eq!(
            exact_revision_filter(PREV),
            serde_json::json!({ "ids": [PREV], "limit": 1 })
        );
        let by_target = feedback_by_target_filter(CHANNEL, &[PREV.to_string()], 400, 200);
        assert_eq!(by_target["artifact"], "current");
        assert_eq!(
            by_target["#type"],
            serde_json::json!([FEEDBACK_ARTIFACT_TYPE])
        );
        assert_eq!(by_target["#target_revision"], serde_json::json!([PREV]));
        assert_eq!(by_target["limit"], 200);
        assert_eq!(by_target["offset"], 400);
    }

    #[test]
    fn accepts_a_valid_review_revision_and_returns_it_unchanged() {
        let event = review(&Keys::generate());
        let selected = select_review_revision(
            vec![event.clone()],
            CHANNEL,
            ARTIFACT,
            Some(&event.id.to_hex()),
        )
        .unwrap();
        assert_eq!(selected.id, event.id);
    }

    #[test]
    fn rejects_tampered_content_and_forged_signatures() {
        let keys = Keys::generate();
        let event = review(&keys);
        let tampered = mutate(&event, |v| v["content"] = serde_json::json!("{\"evil\":1}"));
        assert!(select_review_revision(vec![tampered], CHANNEL, ARTIFACT, None).is_err());

        // A valid signature over a *different* event cannot vouch for this ID.
        let other = artifact_event(
            &keys,
            REVIEW_ARTIFACT_TYPE,
            CHANNEL,
            OTHER_ARTIFACT,
            "create",
        );
        let forged = mutate(&event, |v| {
            v["sig"] = serde_json::json!(other.sig.to_string())
        });
        let error = select_review_revision(vec![forged], CHANNEL, ARTIFACT, None).unwrap_err();
        assert!(error.contains("verification"), "{error}");
    }

    #[test]
    fn binds_channel_artifact_type_and_exact_revision() {
        let keys = Keys::generate();
        let event = review(&keys);
        let other_channel = "8a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
        assert!(
            select_review_revision(vec![event.clone()], other_channel, ARTIFACT, None)
                .unwrap_err()
                .contains("different channel")
        );
        assert!(
            select_review_revision(vec![event.clone()], CHANNEL, OTHER_ARTIFACT, None)
                .unwrap_err()
                .contains("different artifact")
        );
        assert!(
            select_review_revision(vec![event.clone()], CHANNEL, ARTIFACT, Some(PREV))
                .unwrap_err()
                .contains("not found")
        );

        let feedback = artifact_event(&keys, FEEDBACK_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "create");
        assert!(
            select_review_revision(vec![feedback], CHANNEL, ARTIFACT, None)
                .unwrap_err()
                .contains("not a synaxis.html-review")
        );

        let deleted = artifact_event(&keys, REVIEW_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "delete");
        assert!(
            select_review_revision(vec![deleted], CHANNEL, ARTIFACT, None)
                .unwrap_err()
                .contains("deletion")
        );
    }

    #[test]
    fn current_answer_must_be_exactly_one_event() {
        let keys = Keys::generate();
        let first = review(&keys);
        let second = artifact_event(&keys, REVIEW_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "update");
        assert!(select_review_revision(vec![], CHANNEL, ARTIFACT, None).is_err());
        assert!(select_review_revision(vec![first, second], CHANNEL, ARTIFACT, None).is_err());
    }

    #[test]
    fn rejects_non_artifact_events_even_when_correctly_signed() {
        let chat = EventBuilder::new(Kind::Custom(9), "hello")
            .tags([Tag::parse(["h", CHANNEL]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(select_review_revision(vec![chat], CHANNEL, ARTIFACT, None).is_err());
    }

    #[test]
    fn review_listing_withholds_invalid_events_and_dedupes_artifact_heads() {
        let keys = Keys::generate();
        let first = review(&keys);
        let newer_same_artifact =
            artifact_event(&keys, REVIEW_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "update");
        let other = artifact_event(
            &keys,
            REVIEW_ARTIFACT_TYPE,
            CHANNEL,
            OTHER_ARTIFACT,
            "create",
        );
        let tampered = mutate(&other, |v| v["content"] = serde_json::json!("changed"));
        let wrong_type = artifact_event(&keys, FEEDBACK_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "create");
        let deleted = artifact_event(
            &keys,
            REVIEW_ARTIFACT_TYPE,
            CHANNEL,
            OTHER_ARTIFACT,
            "delete",
        );

        let listing = partition_review_artifacts(
            vec![
                first,
                newer_same_artifact,
                other.clone(),
                other,
                tampered,
                wrong_type,
                deleted,
            ],
            true,
        );
        assert_eq!(listing.events.len(), 2);
        assert_eq!(listing.rejected, 3);
        assert!(listing.truncated);
    }

    #[test]
    fn feedback_listing_withholds_unverified_events_and_dedupes() {
        let keys = Keys::generate();
        let good = artifact_event(&keys, FEEDBACK_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "create");
        let tampered = mutate(&good, |v| v["content"] = serde_json::json!("changed"));
        let wrong_type = review(&keys);
        let update = artifact_event(
            &keys,
            FEEDBACK_ARTIFACT_TYPE,
            CHANNEL,
            OTHER_ARTIFACT,
            "update",
        );
        let listing = partition_feedback(
            vec![good.clone(), good.clone(), tampered, wrong_type, update],
            CHANNEL,
            false,
        );
        assert_eq!(listing.events.len(), 1);
        assert_eq!(listing.events[0]["id"], good.id.to_hex());
        assert_eq!(listing.rejected, 3);
    }

    /// Drive the pager against a simulated relay holding `rows` feedback rows,
    /// returning the `(offset, limit)` requests made, the rows kept, and the
    /// truncation flag.
    fn read_pages(rows: usize) -> (Vec<(usize, usize)>, usize, bool) {
        let template = artifact_event(
            &Keys::generate(),
            FEEDBACK_ARTIFACT_TYPE,
            CHANNEL,
            ARTIFACT,
            "create",
        );
        let mut pager = FeedbackPager::default();
        let mut requests = Vec::new();
        while let Some((offset, limit)) = pager.next_page() {
            requests.push((offset, limit));
            let count = rows.saturating_sub(offset).min(limit);
            pager.accept(vec![template.clone(); count]);
        }
        let (events, truncated) = pager.finish();
        (requests, events.len(), truncated)
    }

    #[test]
    fn feedback_paging_stops_at_the_first_short_page() {
        // An empty relay and a page one row short of full both end at once.
        assert_eq!(read_pages(0), (vec![(0, 200)], 0, false));
        assert_eq!(read_pages(199), (vec![(0, 200)], 199, false));
        // A page that is exactly full is only known to be last once the next,
        // empty page comes back: the 201st row must not be lost either.
        assert_eq!(read_pages(200), (vec![(0, 200), (200, 200)], 200, false));
        assert_eq!(read_pages(201), (vec![(0, 200), (200, 200)], 201, false));
        assert_eq!(
            read_pages(401),
            (vec![(0, 200), (200, 200), (400, 200)], 401, false)
        );
    }

    #[test]
    fn feedback_paging_reports_truncation_exactly_at_the_cap() {
        let capped = vec![
            (0, 200),
            (200, 200),
            (400, 200),
            (600, 200),
            (800, 200),
            // One probe row past the cap proves whether more exist.
            (1000, 1),
        ];
        // Exactly at the cap nothing is missing, so nothing is reported.
        assert_eq!(read_pages(MAX_FEEDBACK_ROWS), (capped.clone(), 1000, false));
        // One row more is dropped and reported; the request count stays bounded
        // however many rows the relay holds.
        assert_eq!(
            read_pages(MAX_FEEDBACK_ROWS + 1),
            (capped.clone(), MAX_FEEDBACK_ROWS, true)
        );
        assert_eq!(read_pages(50_000), (capped, MAX_FEEDBACK_ROWS, true));
    }

    #[test]
    fn review_document_requires_matching_size_hash_and_utf8() {
        let html = b"<!doctype html><section data-synaxis-review-id=\"a\">x</section>";
        let digest = hex::encode(Sha256::digest(html));
        assert!(verify_review_document(html, &digest, html.len() as u64).is_ok());
        assert!(verify_review_document(html, &digest, html.len() as u64 + 1)
            .unwrap_err()
            .contains("size mismatch"));
        assert!(
            verify_review_document(html, &"0".repeat(64), html.len() as u64)
                .unwrap_err()
                .contains("hash mismatch")
        );
        let binary = [0xff, 0xfe, 0x00];
        let binary_digest = hex::encode(Sha256::digest(binary));
        assert!(verify_review_document(&binary, &binary_digest, 3)
            .unwrap_err()
            .contains("UTF-8"));
    }

    #[test]
    fn identifiers_must_be_canonical() {
        assert!(canonical_uuid(ARTIFACT, "artifact ID").is_ok());
        assert!(canonical_uuid(&ARTIFACT.to_uppercase(), "artifact ID").is_err());
        assert!(canonical_uuid("00000000-0000-0000-0000-000000000000", "artifact ID").is_err());
        assert!(canonical_event_id(PREV, "id").is_ok());
        assert!(canonical_event_id(&PREV.to_uppercase(), "id").is_err());
        assert!(canonical_event_id("abc", "id").is_err());
    }

    #[test]
    fn reconcile_filter_names_the_artifact_identity_in_its_channel() {
        assert_eq!(
            reconcile_filter(CHANNEL, ARTIFACT),
            serde_json::json!({
                "artifact": "current", "#d": [ARTIFACT], "#h": [CHANNEL], "limit": 1
            })
        );
    }

    #[test]
    fn only_the_exact_signed_event_counts_as_held() {
        let keys = Keys::generate();
        let mine = artifact_event(&keys, FEEDBACK_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "create");
        // The relay returning the very event that was signed is a hit.
        assert_eq!(
            reconcile_status(&[mine.clone()], &mine),
            ReconcileStatus::Held
        );
        assert_eq!(reconcile_status(&[], &mine), ReconcileStatus::Absent);

        // Another event under the same artifact identity is not this one.
        let other = artifact_event(
            &Keys::generate(),
            FEEDBACK_ARTIFACT_TYPE,
            CHANNEL,
            ARTIFACT,
            "create",
        );
        assert_eq!(reconcile_status(&[other], &mine), ReconcileStatus::Absent);

        // A body altered after signing fails verification, so it never counts.
        let tampered = mutate(&mine, |v| v["content"] = serde_json::json!("changed"));
        assert_eq!(
            reconcile_status(&[tampered], &mine),
            ReconcileStatus::Absent
        );
    }

    #[test]
    fn reconcile_refuses_events_that_are_not_this_feedback_create() {
        let keys = Keys::generate();
        let good = artifact_event(&keys, FEEDBACK_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "create");
        assert!(verify_feedback_for_reconcile(&good, CHANNEL, ARTIFACT).is_ok());
        assert!(verify_feedback_for_reconcile(&good, CHANNEL, OTHER_ARTIFACT).is_err());
        assert!(verify_feedback_for_reconcile(
            &good,
            "8a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
            ARTIFACT
        )
        .is_err());
        let review_event = review(&keys);
        assert!(verify_feedback_for_reconcile(&review_event, CHANNEL, ARTIFACT).is_err());
        let update = artifact_event(&keys, FEEDBACK_ARTIFACT_TYPE, CHANNEL, ARTIFACT, "update");
        assert!(verify_feedback_for_reconcile(&update, CHANNEL, ARTIFACT).is_err());
        let tampered = mutate(&good, |v| v["content"] = serde_json::json!("changed"));
        assert!(verify_feedback_for_reconcile(&tampered, CHANNEL, ARTIFACT).is_err());
    }
}
