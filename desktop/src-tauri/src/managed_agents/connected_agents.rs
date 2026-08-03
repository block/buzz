//! Connected self-hosted agents: a distinct record type in a distinct store.
//!
//! A connected agent is one that already runs on a machine the user owns,
//! supervises itself, and holds its own key. Buzz knows its pubkey and where it
//! lives, and nothing else.
//!
//! # Why a separate type and file
//!
//! The first cut of this feature added a `key_custody` field to
//! [`ManagedAgentRecord`] and filtered on it inside `load_managed_agents`. That
//! works, but it makes "Buzz must not act on this agent" a *value* that every
//! reader has to interpret correctly, and it puts a connected agent one missed
//! filter away from the spawn, deploy, tombstone, and key-persisting paths. It
//! also obliged every one of the ~20 `ManagedAgentRecord { .. }` literals in the
//! tree to name a field about custody they have no opinion on.
//!
//! Making it a separate type removes the question instead of answering it:
//!
//! - [`ConnectedAgentRecord`] has no `private_key_nsec`, no `agent_command`, no
//!   `start_on_app_launch`, no `runtime_pid`. A lifecycle path cannot act on one
//!   because there is nothing to act *with* — and it cannot receive one anyway,
//!   because it takes [`ManagedAgentRecord`].
//! - The records live in `connected-agents.json`, so
//!   [`super::load_managed_agents`] cannot return one no matter what it filters,
//!   and an instance-side save cannot erase one no matter what it re-reads.
//! - [`super::storage`] is byte-identical to upstream. Key custody is expressed
//!   by which store a record is in, which is not something a future contributor
//!   can forget to check.
//!
//! The invariants that used to need guards are now properties of the types, and
//! the cross-store tests below assert the ones a type cannot state by itself.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::storage::{atomic_write_json, backup_invalid_store, managed_agents_base_dir};

/// A self-hosted agent Buzz talks to but does not own — the persisted shape.
///
/// Every field is either an identity Buzz only holds the public half of, or a
/// local label. There is deliberately no key, no command, no timeout, no
/// auto-start flag, and no pid: this type cannot describe a process, so no
/// amount of downstream code can use it to start one.
///
/// There is no operational `relay_url`. Every agent relay lookup resolves the
/// active workspace relay at read time (see
/// [`crate::relay::effective_agent_relay_url`]). The optional `community` below
/// is only a display-scope marker; it is never used to route agent traffic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectedAgentRecord {
    /// The agent's own pubkey, 64-char lowercase hex. Normalized at the connect
    /// boundary so every comparison downstream sees one form.
    pub pubkey: String,
    /// Buzz-local label. Not published anywhere — the agent's own kind:10100
    /// profile is the authority on how it presents itself on the relay.
    pub name: String,
    /// `~/.ssh/config` alias of the machine the agent and its key live on.
    ///
    /// A plain `String`, not an `Option`: a connected agent without a host would
    /// be a record whose reachability can never be probed, so the connect
    /// boundary rejects it and the type refuses to represent it.
    pub host: String,
    /// Harness id observed on the host at connect time, e.g. `"claude"`. A
    /// record of what was seen, not a spawn instruction — nothing in Buzz
    /// executes it. `None` when the user connected without a completed probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Normalized relay URL identifying the community where this connection
    /// was created. Records written before this field was introduced remain
    /// readable and visible until they are reconnected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// The connected-agent view handed to the frontend.
///
/// Distinct from [`ConnectedAgentRecord`] only in casing: the record is the
/// on-disk shape (snake_case, matching `managed-agents.json`) and this is the
/// wire shape (camelCase, matching every other Tauri command). Keeping them
/// separate means a future storage field is not automatically exposed to the UI.
///
/// Deliberately not a `ManagedAgentSummary`. That type carries `status`, `pid`,
/// `log_path`, `needs_restart`, `start_on_app_launch`, and
/// `auto_restart_on_config_change` — every one a claim about a process Buzz
/// supervises. Projecting a connected agent onto it would force this surface to
/// invent a lifecycle it has no access to (a self-supervised agent with no local
/// pid is not "stopped"), and the UI would then render controls that cannot
/// work. The narrow shape is what makes "no start/stop button" a property of the
/// type rather than a rule someone has to remember.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedAgentSummary {
    pub pubkey: String,
    pub name: String,
    pub host: String,
    pub harness: Option<String>,
    /// Community where this connection was created, or `None` for a legacy
    /// record that predates community scoping.
    pub community: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&ConnectedAgentRecord> for ConnectedAgentSummary {
    /// Total and infallible. The custody-field version of this projection had to
    /// fall back to an empty host for a record that reached it under local
    /// custody; with a dedicated type that case does not exist.
    fn from(record: &ConnectedAgentRecord) -> Self {
        Self {
            pubkey: record.pubkey.clone(),
            name: record.name.clone(),
            host: record.host.clone(),
            harness: record.harness.clone(),
            community: record.community.clone(),
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
        }
    }
}

/// Path of the connected-agent store, beside `managed-agents.json`.
pub(crate) fn connected_agents_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("connected-agents.json"))
}

/// Load the connected self-hosted agents.
///
/// No key hydration, because there is no key to hydrate: a keyring lookup here
/// would query for a secret that by definition does not exist locally, and a
/// miss would be indistinguishable from an outage.
///
/// Parse failure is fail-loud with the evidence preserved, matching
/// [`super::storage::load_managed_agents`]: a later in-app save rewrites this
/// file wholesale, which would otherwise silently destroy a malformed hand edit.
pub(crate) fn load_connected_agents(app: &AppHandle) -> Result<Vec<ConnectedAgentRecord>, String> {
    load_connected_agents_at(&connected_agents_store_path(app)?)
}

/// Path-based seam, so the store's behavior is testable over a tempdir without
/// a Tauri app handle. Mirrors the `hydrate_keys` / `hydrate_keys_with` split in
/// [`super::storage`].
pub(crate) fn load_connected_agents_at(path: &Path) -> Result<Vec<ConnectedAgentRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read connected agent store: {error}"))?;
    serde_json::from_str(&content).map_err(|error| {
        backup_invalid_store(path);
        format!("failed to parse connected agent store (preserved as .invalid): {error}")
    })
}

/// Save the connected self-hosted agents.
///
/// A wholesale rewrite of this file only. It cannot disturb `managed-agents.json`
/// — which is the point of the separate store — so unlike the custody-field
/// design there is no other half to re-read and no way for an unrelated save to
/// erase these rows.
///
/// Uses the ordinary [`atomic_write_json`], not the `0o600` restricted variant:
/// that exists for files carrying plaintext agent nsecs, and this type cannot
/// hold one.
pub(crate) fn save_connected_agents(
    app: &AppHandle,
    connected: &[ConnectedAgentRecord],
) -> Result<(), String> {
    save_connected_agents_at(&connected_agents_store_path(app)?, connected)
}

/// Path-based seam. See [`load_connected_agents_at`].
pub(crate) fn save_connected_agents_at(
    path: &Path,
    connected: &[ConnectedAgentRecord],
) -> Result<(), String> {
    let mut sorted = connected.to_vec();
    sort_for_stable_diffs(&mut sorted);
    let payload = serde_json::to_vec_pretty(&sorted)
        .map_err(|error| format!("failed to serialize connected agents: {error}"))?;
    // `atomic_write_json` canonicalizes to preserve a symlink at `path`, which
    // requires the target to exist. A first save has nothing to canonicalize.
    if !path.exists() {
        fs::write(path, b"[]")
            .map_err(|error| format!("failed to create connected agent store: {error}"))?;
    }
    atomic_write_json(path, &payload)
}

/// Order by name then pubkey, matching how instances are sorted, so the file
/// produces stable diffs.
fn sort_for_stable_diffs(records: &mut [ConnectedAgentRecord]) {
    records.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.pubkey.cmp(&right.pubkey))
    });
}

/// Normalize a relay URL so equivalent community spellings compare equally.
pub fn normalize_community_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

#[cfg(test)]
#[path = "connected_agents_tests.rs"]
mod tests;
