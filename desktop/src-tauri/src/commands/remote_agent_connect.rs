//! Connecting a self-hosted agent — one that already runs on a machine the
//! user owns, supervises itself, and holds its own key.
//!
//! This is *connect*, not create. Every other agent path in Buzz mints an
//! identity, writes a key, and takes responsibility for a process. Here Buzz
//! learns about an identity that already exists and records where it lives.
//! The result is a [`ConnectedAgentRecord`] in its own store — a type with no
//! key, no command, and no pid, which is what keeps it out of every spawn,
//! deploy, auto-start, profile-republish, and tombstone path. Those paths take
//! `ManagedAgentRecord`, so they cannot receive one of these by construction
//! rather than by filtering.
//!
//! Three things this deliberately does not do:
//!
//! - **No key transport.** The agent's nsec never crosses the network, is
//!   never requested, and is never stored. Buzz holds the public half only.
//! - **No published claim.** Connecting does not emit an owner-signed
//!   kind:30177 "I manage this agent" event. That event is what
//!   `delete_managed_agent` tombstones, so publishing it would let Buzz
//!   assert — and later revoke — the directory entry for an agent it cannot
//!   restart. A self-hosted agent's directory presence is its own replaceable
//!   kind:10100, signed with the key Buzz has never held.
//! - **No lifecycle.** There is no start, stop, restart, or deploy here, and
//!   `disconnect` removes Buzz's local pointer without touching the remote
//!   process. Disconnecting an agent that is happily running is expected to
//!   leave it running.

use tauri::{AppHandle, Manager};

use nostr::nips::nip19::FromBech32;

use crate::app_state::AppState;
use crate::managed_agents::ssh_config::parse_ssh_config;
use crate::managed_agents::storage::{load_agent_definitions, load_managed_agents};
use crate::managed_agents::{
    load_connected_agents, save_connected_agents, ConnectedAgentRecord, ConnectedAgentSummary,
};
use crate::util::now_iso;

/// Longest accepted local label. Matches nothing on the wire — this name is
/// Buzz-local, so the limit only needs to keep the list readable.
const MAX_CONNECTED_NAME_LEN: usize = 64;

/// Normalize a user-supplied agent pubkey to 64-char lowercase hex.
///
/// Both `npub1…` and bare hex are accepted because both are things a user
/// legitimately has on hand: `npub` is what an agent's own tooling prints,
/// hex is what appears in event tags and relay queries. Normalizing at this
/// one boundary means the stored record and every comparison downstream sees
/// a single form — a mixed-case hex duplicate of an already-connected agent
/// would otherwise slip past the collision check below.
pub(crate) fn normalize_agent_pubkey(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("agent pubkey is required".to_string());
    }
    if let Some(stripped) = trimmed.strip_prefix("nsec") {
        // Refuse loudly and specifically. A user who pastes a secret key here
        // has made a serious mistake, and "invalid pubkey" would not tell them
        // what it was. The value itself is never echoed back.
        let _ = stripped;
        return Err(
            "that is a secret key (nsec), not a pubkey — a self-hosted agent's secret must \
             never leave its own machine. Paste the agent's npub instead."
                .to_string(),
        );
    }
    let parsed = if trimmed.starts_with("npub") {
        nostr::PublicKey::from_bech32(trimmed)
            .map_err(|_| "invalid npub — check for a truncated or mistyped value".to_string())?
    } else {
        nostr::PublicKey::from_hex(trimmed).map_err(|_| {
            "invalid agent pubkey — expected an npub or 64 hex characters".to_string()
        })?
    };
    Ok(parsed.to_hex())
}

/// Validate the Buzz-local label for a connected agent.
pub(crate) fn validate_connected_name(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("agent name is required".to_string());
    }
    if trimmed.chars().count() > MAX_CONNECTED_NAME_LEN {
        return Err(format!(
            "agent name must be at most {MAX_CONNECTED_NAME_LEN} characters"
        ));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err("agent name must not contain control characters".to_string());
    }
    Ok(trimmed.to_string())
}

/// Resolve a host alias against the user's own `~/.ssh/config`.
///
/// Requiring a real alias is not gratuitous strictness. The host is a probe
/// target: `probe_agent_host` re-resolves it through this same parsed config
/// and refuses anything it cannot find, so a free-form host string would
/// produce a connected agent whose reachability could never be reported — a
/// row that silently never works. Failing at connect time, with the fix named,
/// is the honest alternative.
fn resolve_connect_host(host: &str) -> Result<String, String> {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return Err("host is required".to_string());
    }
    let known = parse_ssh_config();
    known
        .iter()
        .find(|candidate| candidate.host == trimmed)
        .map(|candidate| candidate.host.clone())
        .ok_or_else(|| {
            format!(
                "'{trimmed}' is not a Host in ~/.ssh/config. Add a stanza for it (Buzz reaches \
                 self-hosted agents through your own ssh config) and try again."
            )
        })
}

/// List the self-hosted agents this machine is connected to.
#[tauri::command]
pub async fn list_connected_agents(app: AppHandle) -> Result<Vec<ConnectedAgentSummary>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_connected_agents(&app)?;
        Ok(records.iter().map(ConnectedAgentSummary::from).collect())
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Record a self-hosted agent that already runs on `host`.
///
/// `harness` is the id observed by the host probe (e.g. `"claude"`). It is
/// stored as an observation for display; nothing in Buzz executes it.
#[tauri::command]
pub async fn connect_remote_agent(
    host: String,
    pubkey: String,
    name: String,
    harness: Option<String>,
    app: AppHandle,
) -> Result<ConnectedAgentSummary, String> {
    tokio::task::spawn_blocking(move || {
        // Validate everything before taking the store lock: none of these
        // checks need the store, and a bad input should not serialize behind
        // an unrelated agent save.
        let host = resolve_connect_host(&host)?;
        let pubkey = normalize_agent_pubkey(&pubkey)?;
        let name = validate_connected_name(&name)?;
        let harness = harness.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });

        let state = app.state::<AppState>();

        // Connecting your own identity would make you an agent that replies to
        // your own messages. The relay- and desktop-side loop guards key off
        // author identity, so this is the one collision they cannot help with.
        // An unavailable identity is not a reason to block the connect.
        if let Ok(keys) = state.signing_keys() {
            if keys.public_key().to_hex() == pubkey {
                return Err(
                    "that is your own pubkey. Connect the agent's identity, not yours — \
                     an agent sharing your key would answer your own messages."
                        .to_string(),
                );
            }
        }

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;

        // Collision checks span BOTH stores. Separating the stores is what makes
        // the lifecycle exclusion structural, but uniqueness is the one property
        // that does not partition: one identity with a record in each store
        // would be two answers to "who is this pubkey", and two agents sharing a
        // name would be ambiguous at every mention site.
        let connected = load_connected_agents(&app)?;
        if let Some(clash) = connected.iter().find(|record| record.pubkey == pubkey) {
            return Err(format!(
                "that agent is already connected as '{}' on {}",
                clash.name, clash.host
            ));
        }

        // Both halves of `managed-agents.json`: keyed instances and the key-less
        // definitions folded into the same file. A definition's name is just as
        // mentionable, so checking only instances would let a connect shadow one.
        let managed = load_managed_agents(&app)?;
        let definitions = load_agent_definitions(&app)?;
        if let Some(clash) = managed
            .iter()
            .chain(definitions.iter())
            .find(|record| record.pubkey == pubkey)
        {
            return Err(format!(
                "'{}' is an agent Buzz already manages on this machine — it holds that agent's \
                 key, so it cannot also be connected as self-hosted",
                clash.name
            ));
        }

        let name_taken = connected
            .iter()
            .any(|record| record.name.eq_ignore_ascii_case(&name))
            || managed
                .iter()
                .chain(definitions.iter())
                .any(|record| record.name.eq_ignore_ascii_case(&name));
        if name_taken {
            return Err(format!(
                "an agent named '{name}' already exists — names are how agents are mentioned, \
                 so pick a different one"
            ));
        }

        let now = now_iso();
        let record = connected_record(&host, &pubkey, &name, harness, &now);
        let summary = ConnectedAgentSummary::from(&record);

        let mut connected = connected;
        connected.push(record);
        save_connected_agents(&app, &connected)?;

        Ok(summary)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Build the stored record for a connected agent.
///
/// A pure function so the invariants that matter are directly testable without
/// a Tauri app handle. With a dedicated record type most of them are no longer
/// assertions at all: there is no key field to leave empty, no `agent_command`
/// to leave blank, and no `start_on_app_launch` to set false. The type states
/// them, so this function only has to be correct about the six facts Buzz knows.
pub(crate) fn connected_record(
    host: &str,
    pubkey: &str,
    name: &str,
    harness: Option<String>,
    now: &str,
) -> ConnectedAgentRecord {
    ConnectedAgentRecord {
        pubkey: pubkey.to_string(),
        name: name.to_string(),
        host: host.to_string(),
        harness,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

/// Forget a connected agent.
///
/// Local-only by construction: this removes Buzz's pointer and nothing else.
/// It deliberately does not take the paths `delete_managed_agent` takes —
/// no process stop (Buzz owns no process), no keyring delete (Buzz holds no
/// key), and above all no kind:30177 tombstone or NIP-IA archive. Those
/// publish the owner's assertion that an agent is gone; running them for an
/// agent that is still alive on its own machine would remove a working agent
/// from every member picker and autocomplete on the relay.
#[tauri::command]
pub async fn disconnect_remote_agent(pubkey: String, app: AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let pubkey = normalize_agent_pubkey(&pubkey)?;
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;

        let mut connected = load_connected_agents(&app)?;
        let before = connected.len();
        connected.retain(|record| record.pubkey != pubkey);
        if connected.len() == before {
            return Err(format!("connected agent {pubkey} not found"));
        }
        save_connected_agents(&app, &connected)
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

#[cfg(test)]
#[path = "remote_agent_connect_tests.rs"]
mod tests;
