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
//! - Owner gating is enforced here: the requested agent pubkey MUST exist
//!   in the local `managed_agents` store. If not, the command refuses.
//!   The UI hides the section anyway, but defense in depth.
//! - One call returns everything because the orphans view requires the
//!   full set anyway. Lazy/per-node decrypt is deferred to IXI-60.

use std::collections::HashMap;
use std::time::SystemTime;

use nostr::PublicKey;
use serde::Serialize;
use tauri::{AppHandle, State};

use sprout_core::engram::{self, extract_refs, select_head, validate_and_decrypt, Body};
use sprout_core::kind::KIND_AGENT_ENGRAM;

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
    // The viewer (this desktop's identity) is the prospective owner. We
    // accept the request only if the agent appears in our managed_agents
    // store — that's the local source of truth for "agents I own". The UI
    // must already have hidden the section for non-owners; this is just
    // defense in depth.
    //
    // Why `managed_agents` rather than a NIP-OA `kind:0` lookup (the gate
    // the archive command uses): the question this surface needs to answer
    // is "do I have the seckey to decrypt this agent's engrams?", not
    // "does this agent's `kind:0` declare me as its OA owner?". The former
    // is what `managed_agents` records — it can't lie, you either have the
    // key locally or you don't. The latter is forgeable (a malicious agent
    // can put any pubkey in their `auth` tag) and would also require a
    // relay roundtrip on every panel open. Keep this gate; don't swap it
    // back to `resolve_oa_owner` "for consistency" — they're answering
    // different questions.
    let agent = PublicKey::from_hex(&agent_pubkey)
        .map_err(|e| format!("agent pubkey must be 64-hex: {e}"))?;

    let managed = load_managed_agents(&app)?;
    if !managed.iter().any(|m| m.pubkey == agent_pubkey) {
        return Err(format!(
            "not the owner of agent {agent_pubkey} (no managed-agent record)"
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
