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

use buzz_core::engram::{conversation_key, d_tag, select_head, validate_and_decrypt, Body};
use buzz_core::kind::KIND_AGENT_ENGRAM;
use nostr::{Event, Keys, PublicKey};

use crate::relay::RestClient;

/// Section header rendered into the prompt.
const SECTION_LABEL: &str = "Agent Memory — core";

/// Fallback system prompt for an agent with neither an operator-configured
/// system prompt nor a core memory. Injected into the system role at session
/// birth (see system-prompt composition in `pool.rs`).
///
/// Deliberately tight — orient the agent, show it the one command it needs to
/// speak, and point it at the durable fix (owner-set core). Written as one
/// voice with the rest of the system prompt, not a bolted-on snippet.
pub const FALLBACK_SYSTEM_PROMPT: &str = "You are an agent in Buzz, a \
Nostr-based chat platform where humans and agents collaborate. Humans only see \
what you post — your tool calls and reasoning are invisible, so surface what \
matters in a message.\n\nTo post to a channel: \
`buzz messages send --channel <id> --content '...'`. Mention someone with \
`@Name` in the content.\n\nYou have no identity configured yet. Ask your owner \
to set your `core` memory — it holds who you are, your rules, and your goals \
across every session.";

/// Outcome of a core-engram fetch, kept as three distinct states so the
/// caller can compose the system prompt correctly.
///
/// The distinction matters: "confirmed empty" invites a fallback identity,
/// but "unavailable" (relay/decrypt failure) must NOT — otherwise a transient
/// outage would hand an established agent a brand-new identity and tempt it to
/// overwrite real-but-unreachable memory.
pub enum CoreFetch {
    /// A valid core exists. Pre-rendered as `[Agent Memory — core]\n<profile>`.
    Present(String),
    /// The relay confirmed the agent has no core (empty result set).
    ConfirmedEmpty,
    /// Fetch/decrypt/parse failed, or timed out. We learned nothing — treat as
    /// neither present nor empty.
    Unavailable,
}

/// Fetch the agent's core engram and classify the result into [`CoreFetch`].
///
/// The `[Agent Memory — core]` framing lives here so the section header is
/// defined in exactly one place; the *empty* and *unavailable* policies are
/// decided by the caller (system-prompt composition), not baked in.
pub async fn fetch_core(rest: &RestClient, agent_keys: &Keys, owner: &PublicKey) -> CoreFetch {
    match fetch_core_body(rest, agent_keys, owner).await {
        Ok(Some(profile)) => CoreFetch::Present(format!("[{SECTION_LABEL}]\n{profile}")),
        Ok(None) => CoreFetch::ConfirmedEmpty,
        Err(reason) => {
            tracing::warn!(
                target: "engram::core",
                "core fetch failed: {reason} — treating as Unavailable to avoid \
                 confusing a relay outage with an absent core"
            );
            CoreFetch::Unavailable
        }
    }
}

/// Query the relay for the core head and decode it. Returns:
/// - `Ok(Some(profile))` if a valid core body was found,
/// - `Ok(None)` only if the relay confirmed absence (empty result set),
/// - `Err(reason)` if the relay returned candidates we could not parse,
///   verify, or decrypt — those are NOT treated as absence (would let an
///   unreadable but real core be silently overwritten by the onboarding nudge),
/// - `Err` for transport / parse errors.
async fn fetch_core_body(
    rest: &RestClient,
    agent_keys: &Keys,
    owner: &PublicKey,
) -> Result<Option<String>, String> {
    let k_c = conversation_key(agent_keys.secret_key(), owner);
    let d = d_tag(&k_c, buzz_core::engram::CORE_SLUG);

    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(KIND_AGENT_ENGRAM as u16))
        .author(agent_keys.public_key())
        .custom_tags(nostr::SingleLetterTag::lowercase(nostr::Alphabet::D), [d])
        .custom_tags(
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
    decode_core_body(arr, agent_keys, owner)
}

/// Pure decoder: given the relay's JSON array, decide whether we have a
/// readable core, confirmed absence, or an ambiguous unreadable-state.
///
/// - Empty array → `Ok(None)` (confirmed absence; caller renders the nudge).
/// - At least one event decrypts → use the winning head's body.
///   * Body::Core → `Ok(Some(profile))`
///   * Body::Tombstone or unexpected shape → `Ok(None)` (treat as absent).
/// - Non-empty array but nothing decrypts → `Err` (fail closed; caller
///   emits no section, so the agent does not assume memory is empty and
///   try to overwrite a real-but-unreadable core).
fn decode_core_body(
    arr: &[serde_json::Value],
    agent_keys: &Keys,
    owner: &PublicKey,
) -> Result<Option<String>, String> {
    if arr.is_empty() {
        return Ok(None);
    }
    let mut valid_with_body: Vec<(Event, Body)> = Vec::with_capacity(arr.len());
    let mut candidates_seen = 0usize;
    let mut last_decrypt_err: Option<String> = None;
    for ev_json in arr {
        let event: Event = match serde_json::from_value(ev_json.clone()) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if event.verify().is_err() {
            continue;
        }
        candidates_seen += 1;
        match validate_and_decrypt(
            &event,
            &agent_keys.public_key(),
            owner,
            agent_keys.secret_key(),
            owner,
        ) {
            Ok(body) => valid_with_body.push((event, body)),
            Err(e) => {
                last_decrypt_err = Some(e.to_string());
                continue;
            }
        }
    }
    if valid_with_body.is_empty() {
        if candidates_seen > 0 {
            return Err(format!(
                "{candidates_seen} core candidate(s) returned but none decryptable                  (last error: {})",
                last_decrypt_err.as_deref().unwrap_or("unknown")
            ));
        }
        return Err(
            "relay returned core candidate(s) that could not be parsed or verified".to_string(),
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::engram::{build_event, Body};
    use serde_json::json;

    /// Empty array → confirmed absence → Ok(None), so the caller emits the
    /// onboarding nudge. This is the only path that maps to "no core."
    #[test]
    fn decode_empty_array_is_confirmed_absence() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let out = decode_core_body(&[], &agent, &owner.public_key()).unwrap();
        assert_eq!(out, None);
    }

    /// Happy path: a real, decryptable core event yields the profile.
    #[test]
    fn decode_valid_core_returns_profile() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let body = Body::Core {
            profile: "I am Sami.".to_string(),
        };
        let ev = build_event(&agent, &owner.public_key(), &body, 1_700_000_000).unwrap();
        let arr = vec![serde_json::to_value(&ev).unwrap()];
        let out = decode_core_body(&arr, &agent, &owner.public_key()).unwrap();
        assert_eq!(out.as_deref(), Some("I am Sami."));
    }

    /// Regression: when the relay returns a kind:30174 event addressed to
    /// this agent that we cannot decrypt (here: encrypted to a *different*
    /// owner's key, so the MAC fails for this agent↔owner pair), we MUST
    /// return Err and NOT Ok(None). Returning Ok(None) would cause the
    /// harness to emit the onboarding nudge, inviting the agent to overwrite
    /// a real-but-unreadable core.
    #[test]
    fn decode_undecryptable_candidate_is_err_not_absent() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let wrong_owner = Keys::generate();
        // Build an engram encrypted to wrong_owner (not owner). It will pass
        // sig verification but fail MAC/decrypt for the agent↔owner pair.
        let body = Body::Core {
            profile: "secret".to_string(),
        };
        let ev = build_event(&agent, &wrong_owner.public_key(), &body, 1_700_000_000).unwrap();
        let arr = vec![serde_json::to_value(&ev).unwrap()];
        let result = decode_core_body(&arr, &agent, &owner.public_key());
        assert!(result.is_err(), "expected Err, got: {result:?}");
        let msg = result.unwrap_err();
        assert!(msg.contains("decryptable"), "got: {msg}");
    }

    /// An unexpectedly-shaped head (here: a Memory body in what was supposed
    /// to be the core slot) is a legitimate, decryptable "no usable core" —
    /// Ok(None). Real `rm core` is refused at the CLI, so this is a defensive
    /// branch for malformed data on the wire.
    #[test]
    fn decode_non_core_body_is_absent() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let body = Body::Memory {
            slug: "mem/x".to_string(),
            value: None,
        };
        let ev = build_event(&agent, &owner.public_key(), &body, 1_700_000_000).unwrap();
        let arr = vec![serde_json::to_value(&ev).unwrap()];
        let out = decode_core_body(&arr, &agent, &owner.public_key()).unwrap();
        assert_eq!(out, None);
    }

    /// Non-empty array with only garbage entries (not even parseable as
    /// events) is also treated as a fetch error, not absence.
    #[test]
    fn decode_unparseable_candidates_is_err() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let arr = vec![json!({"not": "an event"}), json!("garbage")];
        let result = decode_core_body(&arr, &agent, &owner.public_key());
        assert!(result.is_err(), "expected Err, got: {result:?}");
    }
}
