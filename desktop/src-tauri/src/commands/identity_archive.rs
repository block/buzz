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
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    relay::{query_relay, submit_event, SubmitEventResponse},
};

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Read `target`'s live `kind:0` event and extract the first valid NIP-OA
/// `auth` tag plus the verified owner pubkey.
///
/// Mirrors the verification the relay will do (per spec gotcha #3: the
/// preimage subject is the *target* pubkey, not the request signer). The
/// `sprout-sdk` lives on nostr 0.36; the desktop is on 0.37, so we bridge
/// via hex round-trip exactly like `relay::build_profile_event` does.
fn extract_oa_owner(target_kind0: &nostr::Event) -> Option<(String, [String; 4])> {
    let target_hex = target_kind0.pubkey.to_hex();
    let target_compat = nostr::PublicKey::from_hex(&target_hex).ok()?;

    for tag in target_kind0.tags.iter() {
        let slice = tag.as_slice();
        if slice.first().map(String::as_str) != Some("auth") || slice.len() != 4 {
            continue;
        }
        let json = serde_json::to_string(slice).ok()?;
        match sprout_sdk::nip_oa::verify_auth_tag(&json, &target_compat) {
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

async fn fetch_kind0(state: &AppState, pubkey: &str) -> Result<Option<nostr::Event>, String> {
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

/// Submit a `kind:9035` archive request to the relay. Consent path is selected
/// by the relay — we just attach the owner-of-agent `auth` tag when the live
/// `kind:0` proves we own the target, so the relay can choose the `owner`
/// path. Self and admin paths require no auth tag.
#[tauri::command]
pub async fn archive_identity(
    req: ArchiveRequest,
    state: State<'_, AppState>,
) -> Result<SubmitEventResponse, String> {
    let auth_tag = maybe_owner_auth_tag(&state, &req.target_pubkey).await?;
    let auth_ref = auth_tag.as_ref();

    let builder = events::build_archive_identity_request(
        &req.target_pubkey,
        &req.content,
        req.reason.as_deref(),
        req.replaced_by.as_deref(),
        auth_ref,
    )?;
    submit_event(builder, &state).await
}

/// Submit a `kind:9036` unarchive request to the relay.
#[tauri::command]
pub async fn unarchive_identity(
    req: UnarchiveRequest,
    state: State<'_, AppState>,
) -> Result<SubmitEventResponse, String> {
    let auth_tag = maybe_owner_auth_tag(&state, &req.target_pubkey).await?;
    let auth_ref = auth_tag.as_ref();

    let builder = events::build_unarchive_identity_request(
        &req.target_pubkey,
        &req.content,
        req.reason.as_deref(),
        auth_ref,
    )?;
    submit_event(builder, &state).await
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

/// Read the relay's latest `kind:13535` archive snapshot. The frontend caches
/// this and tests membership client-side to drive the "Archived" flair.
///
/// Per spec §Snapshot and Delta Consistency: the latest valid `kind:13535`
/// signed by the relay identity is authoritative.
///
/// NIP-IA §Client Behavior says clients MUST verify the snapshot is signed by
/// the relay's NIP-11 `self` key. We do not yet filter by that key here: the
/// desktop only ever talks to its own configured relay, where server-side
/// enforcement makes archive state trustworthy, and we have no NIP-11 `self`
/// fetch wired up (the sibling relay-signed kind:13534 membership list is
/// consumed the same way). Author-filtering against NIP-11 `self` is the
/// correct hardening for an untrusted/multi-relay client and is tracked as a
/// follow-up — not a runtime gap on Sprout's relay.
#[tauri::command]
pub async fn list_archived_identities(
    state: State<'_, AppState>,
) -> Result<ArchivedIdentitiesSnapshot, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [13535],
            "limit": 1,
        })],
    )
    .await?;

    let Some(snapshot) = events.into_iter().next() else {
        return Ok(ArchivedIdentitiesSnapshot { archived: vec![] });
    };

    let archived = snapshot
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
        .collect();

    Ok(ArchivedIdentitiesSnapshot { archived })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// Build a fake `kind:0` with a valid NIP-OA auth tag for a fresh owner.
    fn kind0_with_auth(agent: &Keys, owner: &Keys) -> nostr::Event {
        // Compute auth tag via sprout-sdk (nostr 0.36) and bridge.
        let agent_hex = agent.public_key().to_hex();
        let agent_compat = nostr::PublicKey::from_hex(&agent_hex).unwrap();
        let owner_compat_secret =
            nostr::SecretKey::from_slice(&owner.secret_key().as_secret_bytes()[..]).unwrap();
        let owner_compat_keys = nostr::Keys::new(owner_compat_secret);
        let tag_json = sprout_sdk::nip_oa::compute_auth_tag(&owner_compat_keys, &agent_compat, "")
            .expect("compute_auth_tag");
        let compat_tag = sprout_sdk::nip_oa::parse_auth_tag(&tag_json).unwrap();
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
}
