//! Fetch the agent's NIP-AE `core` engram at session creation and render it
//! into a prompt section.
//!
//! Scope per Tyler's spec:
//! - Fire one synchronous query for the core head when a *new* session is born.
//! - If a body is found, emit `[Agent Memory — core]\n<profile>`.
//! - If no body is found, emit an onboarding nudge so the agent learns how
//!   to set its own core.
//! - On any *error* (transport, parse), log and emit nothing. We must not
//!   mistake a relay outage for "no core" — that would invite the agent to
//!   overwrite real, just-unreachable memory with a fresh profile.
//! - Either way, session creation is never blocked.

use nostr::{Event, Keys, PublicKey};
use sprout_core::engram::{conversation_key, d_tag, select_head, validate_and_decrypt, Body};
use sprout_core::kind::KIND_AGENT_ENGRAM;

use crate::relay::RestClient;

/// Section header rendered into the prompt.
const SECTION_LABEL: &str = "Agent Memory — core";

/// Onboarding nudge for new agents with no core yet.
///
/// Wording is from Tyler's brief: "No core memory found. Use `sprout mem`
/// to create a core memory. Ask your user about yourself."
pub const ONBOARDING_NUDGE: &str = "No core memory found. \
Use `sprout mem set core \"…\"` to create one (it will hold your identity, \
rules, and goals across sessions). Ask your user about yourself.";

/// Build the rendered prompt section for the agent's core.
///
/// Returns:
/// - `Some(profile_section)` when a valid core exists,
/// - `Some(nudge_section)` when the relay confirmed absence,
/// - `None` when the fetch failed (transport, parse, decrypt) — the caller
///   should inject no section in that case so the agent doesn't conclude
///   memory is empty.
pub async fn build_core_section(
    rest: &RestClient,
    agent_keys: &Keys,
    owner: &PublicKey,
) -> Option<String> {
    match fetch_core_body(rest, agent_keys, owner).await {
        Ok(Some(profile)) => Some(format!("[{SECTION_LABEL}]\n{profile}")),
        Ok(None) => Some(format!("[{SECTION_LABEL}]\n{ONBOARDING_NUDGE}")),
        Err(reason) => {
            tracing::warn!(
                target: "engram::core",
                "core fetch failed: {reason} — emitting no section to avoid \
                 confusing a relay outage with an absent core"
            );
            None
        }
    }
}

/// Query the relay for the core head and decode it. Returns:
/// - `Ok(Some(profile))` if a valid core body was found,
/// - `Ok(None)` if no valid head exists (absent or all events failed validation),
/// - `Err` for transport / parse errors.
async fn fetch_core_body(
    rest: &RestClient,
    agent_keys: &Keys,
    owner: &PublicKey,
) -> Result<Option<String>, String> {
    let k_c = conversation_key(agent_keys.secret_key(), owner);
    let d = d_tag(&k_c, sprout_core::engram::CORE_SLUG);

    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
        .author(agent_keys.public_key())
        .custom_tag(nostr::SingleLetterTag::lowercase(nostr::Alphabet::D), [d])
        .custom_tag(
            nostr::SingleLetterTag::lowercase(nostr::Alphabet::P),
            [owner.to_hex()],
        )
        .limit(16);

    let value = rest
        .query(&[filter])
        .await
        .map_err(|e| format!("relay query failed: {e}"))?;
    let arr = value
        .as_array()
        .ok_or_else(|| "relay query returned non-array".to_string())?;

    let mut valid_with_body: Vec<(Event, Body)> = Vec::with_capacity(arr.len());
    for ev_json in arr {
        let event: Event = match serde_json::from_value(ev_json.clone()) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if event.verify().is_err() {
            continue;
        }
        match validate_and_decrypt(
            &event,
            &agent_keys.public_key(),
            owner,
            agent_keys.secret_key(),
            owner,
        ) {
            Ok(body) => valid_with_body.push((event, body)),
            Err(_) => continue,
        }
    }
    if valid_with_body.is_empty() {
        return Ok(None);
    }
    let events: Vec<Event> = valid_with_body.iter().map(|(e, _)| e.clone()).collect();
    // `select_head` returns `None` only on an empty iterator, which we
    // ruled out above.
    let Some(head) = select_head(events) else {
        return Ok(None);
    };
    let head_id = head.id;
    let body = valid_with_body
        .into_iter()
        .find(|(e, _)| e.id == head_id)
        .map(|(_, b)| b);
    match body {
        Some(Body::Core { profile }) => Ok(Some(profile)),
        // A tombstone or unexpectedly-shaped head means "no usable core."
        _ => Ok(None),
    }
}
