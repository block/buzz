//! Fleet update: keep agent `mem/persona` engrams in sync with persona edits.
//!
//! When a persona's content changes, each agent instantiated from it carries a
//! stale snapshot in its `mem/persona` engram. Fleet update detects the drift
//! (by comparing content hashes, not timestamps — timestamps are fragile across
//! clock skew and export/import) and rewrites the affected engrams.
//!
//! This runs on persona save (immediate propagation) and at app launch (catches
//! edits made while the app was closed). It never restarts running agents — the
//! live resolve path still reads the persona catalog at spawn, so a running
//! agent already sees edits on its next session. The engram is provenance and
//! future-proofing for when it becomes the runtime source; keeping it current
//! is the only job here.

use buzz_core_pkg::engram::{conversation_key, d_tag, select_head};
use buzz_core_pkg::kind::KIND_AGENT_ENGRAM;
use nostr::PublicKey;
use tauri::{AppHandle, Manager};

use super::persona_events::{
    build_persona_engram, persona_content_hash, persona_engram_from_event_as_owner,
    persona_event_content, PERSONA_ENGRAM_SLUG,
};
use super::personas::load_personas;
use super::storage::load_managed_agents;
use super::{ManagedAgentRecord, PersonaRecord};
use crate::app_state::AppState;
use crate::relay;

/// What fleet update should do with one agent's persona engram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetAction {
    /// Engram already matches the current persona — nothing to do.
    NoOp,
    /// Engram is missing or stale — (re)write it.
    Write,
}

/// Decide whether an agent's persona engram needs rewriting.
///
/// Pure and side-effect free so the decision can be unit-tested in isolation:
/// a missing engram (`None`) always writes; otherwise we write iff the stored
/// content version differs from the persona's current content hash.
pub fn fleet_action_for(current_hash: &str, stored_version: Option<&str>) -> FleetAction {
    match stored_version {
        Some(version) if version == current_hash => FleetAction::NoOp,
        _ => FleetAction::Write,
    }
}

/// Run fleet update across all managed agents (launch-time reconciliation).
///
/// Best-effort: relay errors for one agent are logged and skipped, never fatal.
/// Returns the number of engrams written.
pub async fn check_fleet_updates(app: &AppHandle) -> Result<usize, String> {
    let state = app.state::<AppState>();
    let personas = load_personas(app)?;
    let agents = load_managed_agents(app)?;

    let (owner_keys, relay_url) = owner_context(&state)?;
    let mut written = 0;

    for agent in &agents {
        let Some(persona) = persona_for_agent(agent, &personas) else {
            continue;
        };
        match reconcile_agent(&state, &owner_keys, &relay_url, agent, persona).await {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(e) => eprintln!(
                "buzz-desktop: fleet-update: skipped agent {}: {e}",
                short(&agent.pubkey)
            ),
        }
    }

    Ok(written)
}

/// Run fleet update for the agents tied to a single persona (on persona save).
///
/// Targeted variant of [`check_fleet_updates`] — only touches agents whose
/// `persona_id` matches. Best-effort with the same skip-on-error posture.
pub async fn fleet_update_for_persona(app: &AppHandle, persona_id: &str) -> Result<usize, String> {
    let state = app.state::<AppState>();
    let personas = load_personas(app)?;
    let agents = load_managed_agents(app)?;

    let Some(persona) = personas.iter().find(|p| p.id == persona_id) else {
        return Ok(0); // Persona deleted between save and update — nothing to do.
    };

    let (owner_keys, relay_url) = owner_context(&state)?;
    let mut written = 0;

    for agent in &agents {
        if agent.persona_id.as_deref() != Some(persona_id) {
            continue;
        }
        match reconcile_agent(&state, &owner_keys, &relay_url, agent, persona).await {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(e) => eprintln!(
                "buzz-desktop: fleet-update: skipped agent {}: {e}",
                short(&agent.pubkey)
            ),
        }
    }

    Ok(written)
}

/// Reconcile one agent's engram against a persona. Returns `Ok(true)` if a
/// rewrite was published, `Ok(false)` for a no-op.
async fn reconcile_agent(
    state: &AppState,
    owner_keys: &nostr::Keys,
    relay_url: &str,
    agent: &ManagedAgentRecord,
    persona: &PersonaRecord,
) -> Result<bool, String> {
    let agent_keys = nostr::Keys::parse(&agent.private_key_nsec)
        .map_err(|e| format!("invalid agent key: {e}"))?;
    let agent_pubkey = agent_keys.public_key();

    let current_hash = persona_content_hash(&persona_event_content(persona));
    let stored_version = read_engram_source_version(state, owner_keys, &agent_pubkey).await;

    if fleet_action_for(&current_hash, stored_version.as_deref()) == FleetAction::NoOp {
        return Ok(false);
    }

    apply_fleet_update(
        state,
        &agent_keys,
        &owner_keys.public_key(),
        relay_url,
        persona,
    )
    .await?;
    eprintln!(
        "buzz-desktop: fleet-update: rewrote mem/persona engram for agent {} (persona {})",
        short(&agent.pubkey),
        persona.id
    );
    Ok(true)
}

/// Write the initial `mem/persona` engram for a freshly created agent.
///
/// Called from `create_managed_agent` once the persona is known. Best-effort:
/// the caller logs and continues on error so engram failure never blocks agent
/// creation. The owner key comes from app state; the persona is resolved by id.
pub async fn write_persona_engram_at_creation(
    state: &AppState,
    agent_keys: &nostr::Keys,
    relay_url: &str,
    persona_id: &str,
    app: &AppHandle,
) -> Result<(), String> {
    let personas = load_personas(app)?;
    let persona = personas
        .iter()
        .find(|p| p.id == persona_id)
        .ok_or_else(|| format!("persona {persona_id} not found"))?;

    let owner_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key()
    };
    apply_fleet_update(state, agent_keys, &owner_pubkey, relay_url, persona).await
}

/// Build and publish a fresh `mem/persona` engram for one agent.
async fn apply_fleet_update(
    state: &AppState,
    agent_keys: &nostr::Keys,
    owner_pubkey: &PublicKey,
    relay_url: &str,
    persona: &PersonaRecord,
) -> Result<(), String> {
    let now = now_secs();
    let event = build_persona_engram(agent_keys, owner_pubkey, persona, now)?;
    publish_agent_event(state, agent_keys, relay_url, &event).await
}

/// Fetch the agent's current `mem/persona` engram from the relay and return the
/// recorded `provenance.source_version`. `None` if no engram exists or it can't
/// be read/decrypted — both cases trigger a (re)write, which is safe under
/// NIP-33 latest-wins replacement.
///
/// The owner holds the only key needed: the agent↔owner conversation key is
/// symmetric, so the owner decrypts with its own secret key and the agent's
/// pubkey. No agent secret key is required at read time.
async fn read_engram_source_version(
    state: &AppState,
    owner_keys: &nostr::Keys,
    agent_pubkey: &PublicKey,
) -> Option<String> {
    let owner_pubkey = owner_keys.public_key();
    let k_c = conversation_key(owner_keys.secret_key(), agent_pubkey);
    let expected_d = d_tag(&k_c, PERSONA_ENGRAM_SLUG);

    // Read-gated kind:30174 query: owner-authored auth, #p = owner (the owner
    // reading engrams addressed to them). #d narrows to the persona slot.
    let filter = serde_json::json!({
        "kinds": [KIND_AGENT_ENGRAM],
        "authors": [agent_pubkey.to_hex()],
        "#p": [owner_pubkey.to_hex()],
        "#d": [expected_d],
    });
    let events = relay::query_relay(state, &[filter]).await.ok()?;

    let head = select_head(events)?;
    let body = persona_engram_from_event_as_owner(&head, agent_pubkey, owner_keys).ok()?;
    Some(body.provenance.source_version)
}

/// Publish an engram event signed by the agent, authenticating the write with
/// the agent's keys (NIP-98) and NIP-OA auth tag.
async fn publish_agent_event(
    state: &AppState,
    agent_keys: &nostr::Keys,
    relay_url: &str,
    event: &nostr::Event,
) -> Result<(), String> {
    let url = format!("{}/events", relay::relay_http_base_url(relay_url));
    let body_bytes = nostr::JsonUtil::as_json(event).into_bytes();
    let auth = relay::build_nip98_auth_header_for_keys(
        agent_keys,
        &reqwest::Method::POST,
        &url,
        &body_bytes,
    )?;

    let response = state
        .http_client
        .post(&url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .map_err(|e| format!("publish request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(relay::relay_error_message(response).await);
    }

    let result: relay::SubmitEventResponse = response
        .json()
        .await
        .map_err(|e| format!("failed to parse publish response: {e}"))?;

    if !result.accepted {
        return Err(format!("relay rejected engram: {}", result.message));
    }

    Ok(())
}

/// Resolve the persona an agent was instantiated from, if any.
fn persona_for_agent<'a>(
    agent: &ManagedAgentRecord,
    personas: &'a [PersonaRecord],
) -> Option<&'a PersonaRecord> {
    let persona_id = agent.persona_id.as_deref()?;
    personas.iter().find(|p| p.id == persona_id)
}

/// Snapshot the owner's keys and relay URL for a fleet-update pass.
fn owner_context(state: &AppState) -> Result<(nostr::Keys, String), String> {
    let owner_keys = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.clone()
    };
    let relay_url = relay::relay_ws_url_with_override(state);
    Ok((owner_keys, relay_url))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn short(pubkey: &str) -> &str {
    &pubkey[..pubkey.len().min(8)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_engram_writes() {
        assert_eq!(fleet_action_for("abc123", None), FleetAction::Write);
    }

    #[test]
    fn matching_hash_is_noop() {
        assert_eq!(
            fleet_action_for("abc123", Some("abc123")),
            FleetAction::NoOp
        );
    }

    #[test]
    fn differing_hash_writes() {
        assert_eq!(
            fleet_action_for("abc123", Some("def456")),
            FleetAction::Write
        );
    }
}
