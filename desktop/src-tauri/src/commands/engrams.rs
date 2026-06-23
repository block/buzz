//! Owner-gated engram (NIP-AE memory) reader for the desktop.
//!
//! IXI-7 phase 1: read-only memory surface inside the agent profile panel.
//! One Tauri call per panel open returns the entire decrypted listing —
//! `core` (if present), every non-tombstoned `mem/...` entry, and the
//! outgoing `[[slug]]` refs extracted from each body. The UI computes
//! reachability + orphans from this single payload; no incremental sync.
//!
//! Why this shape:
//! - Owner secret key never leaves Rust. The desktop signs in as the owner
//!   and decrypts via `agent ↔ owner` NIP-44 conversation key.
//! - Owner gating is enforced here: the request is accepted if the agent is
//!   in the local `managed_agents` store OR the agent's live `kind:0`
//!   cryptographically declares the viewer as its NIP-OA owner. Either way
//!   the engrams are NIP-44 encrypted to the viewer's own pubkey, so the
//!   encryption is the real boundary; this gate just decides whether to try.
//!   The UI hides the section for non-owners anyway, but defense in depth.
//! - One call returns everything because the orphans view requires the
//!   full set anyway. Lazy/per-node decrypt is deferred to IXI-60.

use std::collections::HashMap;
use std::time::SystemTime;

use nostr::PublicKey;
use serde::Serialize;
use tauri::{AppHandle, State};

use buzz_core_pkg::engram::{self, extract_refs, select_head, validate_and_decrypt, Body};
use buzz_core_pkg::kind::KIND_AGENT_ENGRAM;

use crate::commands::identity_archive::{extract_oa_owner, fetch_kind0};
use crate::{app_state::AppState, managed_agents::load_managed_agents, relay::query_relay};

/// Hard cap on engrams returned per (agent, owner) pair. Matches the CLI
/// `mem ls` reference. If the relay returns this many we set
/// `truncated = true` so the UI can warn that the list may be incomplete.
const ENGRAM_FETCH_LIMIT: u32 = 5000;

/// One memory entry returned to the UI.
///
/// `slug` is the canonical slug (`core` or `mem/foo/bar`). `body` is the
/// decrypted UTF-8 payload (profile text for core, value for memory).
/// `outgoing_refs` is the list of `[[slug]]` references extracted from the
/// body — used by the UI to BFS reachability from `core`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngramEntry {
    pub slug: String,
    pub body: String,
    pub event_id: String,
    pub created_at: u64,
    pub outgoing_refs: Vec<String>,
}

/// Single-payload response for one panel open. `core` is split out because
/// the UI roots the reachability tree there; `memories` excludes core (and
/// tombstones) so it maps 1:1 to the `mem/...` set.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryListing {
    /// `core` entry, if the agent has one.
    pub core: Option<EngramEntry>,
    /// All non-core, non-tombstoned memories. Sorted by slug.
    pub memories: Vec<EngramEntry>,
    /// True if the relay returned `>= ENGRAM_FETCH_LIMIT` events — list may
    /// be incomplete. UI surfaces a warning. Tracked for follow-up in
    /// IXI-60 (pagination + lazy decrypt).
    pub truncated: bool,
    /// Unix seconds when the response was assembled. UI uses this for
    /// "last loaded" copy on the refetch affordance.
    pub fetched_at: u64,
}

/// `get_agent_memory` — owner-gated single-payload engram listing.
///
/// Returns the full decrypted set for the (agent, owner) pair where
/// `owner = current viewer`. Refuses if the agent isn't in this desktop's
/// `managed_agents` store (i.e. the viewer is not its owner). This mirrors
/// the relay's hard refusal of cross-owner reads.
///
/// Errors are stringified for the Tauri bridge. The UI distinguishes
/// fetch error vs empty success vs success-with-data; an `Err(_)` return
/// is the "couldn't load" path. An empty `memories` Vec with `core: None`
/// is the legitimate "no memories" empty state.
#[tauri::command]
pub async fn get_agent_memory(
    agent_pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentMemoryListing, String> {
    // ── Owner gating ────────────────────────────────────────────────────
    // The viewer (this desktop's identity) is the prospective owner. The
    // relay query below is `#p`-tagged for the viewer's OWN pubkey and every
    // engram is NIP-44 encrypted to that pubkey, so the viewer decrypts with
    // their own key — the agent's seckey is never needed. Encryption + the
    // relay's server-side `#p` scoping are the real boundary; this gate only
    // decides whether we bother attempting (and avoids a needless roundtrip
    // for agents the viewer plainly doesn't own).
    //
    // We accept the request on either of two owner signals, mirroring the UI's
    // `viewerIsOwner = isCurrentUserOwner || isOwner`:
    //   1. The agent is in this desktop's `managed_agents` store — local
    //      fast-path, no roundtrip. Covers locally-run agents (and older
    //      agents that never advertised an owner).
    //   2. The agent's live `kind:0` cryptographically declares the viewer as
    //      its NIP-OA owner (verified via `extract_oa_owner`). This is the
    //      remote-owner case: the owner runs the agent on another desktop, so
    //      holds no local seckey, but legitimately owns it.
    //
    // (Historical note: this gate used to refuse anything not in
    // `managed_agents`, on the theory that key-custody was the only safe
    // proxy for ownership. That conflated "do I hold the seckey?" with "am I
    // the owner?" — the two diverge exactly for a remote-owned agent, which
    // wrongly locked legitimate owners out of their own memory. The declared-
    // owner path is cleared (PR #917 author signed off); decryption still
    // does the real guarding.)
    let agent = PublicKey::from_hex(&agent_pubkey)
        .map_err(|e| format!("agent pubkey must be 64-hex: {e}"))?;

    let viewer_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    let managed = load_managed_agents(&app)?;
    let is_managed = managed.iter().any(|m| m.pubkey == agent_pubkey);
    let is_declared_owner = if is_managed {
        false // already authorized; skip the relay roundtrip
    } else {
        // Verify the agent's live `kind:0` declares the viewer as owner.
        match fetch_kind0(&state, &agent_pubkey).await? {
            Some(kind0) => extract_oa_owner(&kind0)
                .map(|(owner_hex, _tag)| owner_hex.eq_ignore_ascii_case(&viewer_pubkey))
                .unwrap_or(false),
            None => false,
        }
    };

    if !is_managed && !is_declared_owner {
        return Err(format!(
            "not the owner of agent {agent_pubkey} (no managed-agent record \
             and no verified NIP-OA owner declaration)"
        ));
    }

    // ── Resolve owner key material ──────────────────────────────────────
    // Owner = viewer. Clone the secret key out of the lock immediately so
    // we don't hold the mutex across the relay round trip.
    let (owner_pubkey, owner_seckey) = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        (keys.public_key(), keys.secret_key().clone())
    };

    // ── Relay query ─────────────────────────────────────────────────────
    // Mirrors the CLI `mem ls` filter: kind 30174, authored by the agent,
    // p-tagged for the owner. The relay enforces the same access shape.
    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_ENGRAM],
        "authors": [agent.to_hex()],
        "#p": [owner_pubkey.to_hex()],
        "limit": ENGRAM_FETCH_LIMIT,
    });
    let events = query_relay(&state, &[filter]).await?;
    // `>=` is intentional and accepts a false-positive at exactly
    // ENGRAM_FETCH_LIMIT events: if the relay returned the cap, we can't
    // distinguish "exactly cap" from "cap because clipped". The banner copy
    // says "may be incomplete" which covers the off-by-one. Switch to a
    // delta-cursor sync in IXI-60 if this matters in practice.
    let truncated = events.len() as u32 >= ENGRAM_FETCH_LIMIT;

    // ── Validate, decrypt, group by `d` (NIP-AE Listing) ────────────────
    // Pattern is the CLI's: drop bad apples silently rather than fail the
    // whole listing. A single corrupt event must not deny-of-service the
    // panel.
    let mut groups: HashMap<String, Vec<(nostr::Event, Body)>> = HashMap::new();
    for ev in events {
        if ev.verify().is_err() {
            continue;
        }
        let Some(d_value) = ev
            .tags
            .iter()
            .find(|t| t.kind().to_string() == "d")
            .and_then(|t| t.content())
            .map(|s| s.to_string())
        else {
            continue;
        };
        let body = match validate_and_decrypt(
            &ev,
            &agent,
            &owner_pubkey,
            &owner_seckey,
            &agent, // viewer (owner) decrypts with agent as the conversation peer
        ) {
            Ok(b) => b,
            Err(_) => continue,
        };
        groups.entry(d_value).or_default().push((ev, body));
    }

    // ── Pick head per d-group, drop tombstones, split core vs memories ──
    let mut core: Option<EngramEntry> = None;
    let mut memories: Vec<EngramEntry> = Vec::new();
    for (_d, members) in groups {
        let events: Vec<nostr::Event> = members.iter().map(|(e, _)| e.clone()).collect();
        let Some(head) = select_head(events) else {
            continue;
        };
        let Some((_, body)) = members.into_iter().find(|(e, _)| e.id == head.id) else {
            continue;
        };
        let event_id = head.id.to_hex();
        let created_at = head.created_at.as_secs();
        match body {
            Body::Memory { value: None, .. } => {
                // Tombstone — exclude from the listing.
                continue;
            }
            Body::Core { profile } => {
                let outgoing_refs = extract_refs(&profile);
                core = Some(EngramEntry {
                    slug: engram::CORE_SLUG.to_string(),
                    body: profile,
                    event_id,
                    created_at,
                    outgoing_refs,
                });
            }
            Body::Memory {
                slug,
                value: Some(value),
            } => {
                let outgoing_refs = extract_refs(&value);
                memories.push(EngramEntry {
                    slug,
                    body: value,
                    event_id,
                    created_at,
                    outgoing_refs,
                });
            }
        }
    }

    memories.sort_by(|a, b| a.slug.cmp(&b.slug));

    let fetched_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Ok(AgentMemoryListing {
        core,
        memories,
        truncated,
        fetched_at,
    })
}
