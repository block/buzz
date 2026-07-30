//! Credential export for [`BackendKind::External`] agents.
//!
//! Buzz mints an external agent's identity but never runs it. This command hands
//! the user the env their own `buzz-acp` needs, so the harness they start comes
//! up with the same effective configuration a local spawn would have produced.
//!
//! # Security
//!
//! The response contains the agent's nsec. Revealing an agent nsec to the
//! frontend is the established posture — `SecretRevealDialog` already renders
//! `CreateManagedAgentResponse::private_key_nsec` verbatim on every create, and
//! `commands::identity::get_nsec` exports the *owner's* nsec. What this command
//! adds is repeatability, which is required: the user rebuilds the container.
//!
//! Three rules follow from that, all load-bearing:
//!
//! 1. **The backend gate below is the security boundary.** Without it this is a
//!    generic nsec-export endpoint for every local agent, reachable by pubkey.
//! 2. [`ExternalAgentEnvResponse`] deliberately does **not** derive `Debug` — a
//!    `{:?}` on a struct holding an nsec is how secrets reach logs.
//! 3. This type must never reach `observer.emit`, `retain_*`, or
//!    `agent_event_content`. There is no invoke middleware logging results, so
//!    the rule is prohibitive rather than enforced by a redaction layer.
//!
//! [`BackendKind::External`]: crate::managed_agents::BackendKind::External

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{load_managed_agents, load_personas, BackendKind},
};

/// The env block for an external agent's container.
///
/// No `Debug` derive — see the module-level security note.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentEnvResponse {
    /// Sorted `KEY -> value` pairs, for rendering as a table.
    pub env: std::collections::BTreeMap<String, String>,
    /// The same pairs pre-rendered as `KEY=value` lines, ready to paste into a
    /// `docker run --env-file` file. Provided so the copy button and the
    /// displayed text cannot drift apart.
    pub env_file: String,
}

/// Build the env block an external agent's `buzz-acp` needs.
///
/// Refuses for any backend other than `External`: local agents are spawned by
/// Buzz with this env already applied, and provider agents have it pushed to the
/// provider binary — neither has a reason to export a reusable secret.
#[tauri::command]
pub async fn get_external_agent_env(
    pubkey: String,
    app: AppHandle,
) -> Result<ExternalAgentEnvResponse, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();

        // Resolved before taking the store lock: both touch other mutexes.
        let owner_hex = super::agents::workspace_owner_hex(&state)?;
        let workspace_relay = crate::relay::relay_ws_url_with_override(&state);

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;

        // The security boundary. See the module-level note.
        if record.backend != BackendKind::External {
            return Err(
                "env export is only available for agents that run outside Buzz".to_string(),
            );
        }

        // Fails closed on a keyring outage rather than handing back a block whose
        // BUZZ_PRIVATE_KEY is empty — the container would start with no identity
        // and fail relay auth in a way that looks like a relay problem.
        if let Some(error) = crate::managed_agents::spawn_key_refusal(record) {
            return Err(error);
        }

        let personas = load_personas(&app).unwrap_or_default();
        let teams = crate::managed_agents::load_teams(&app).unwrap_or_default();
        let global = crate::managed_agents::load_global_agent_config(&app).unwrap_or_default();

        // Same authoritative resolvers spawn uses, so the exported block and a
        // local spawn of the same record cannot disagree.
        let descriptor = crate::managed_agents::readiness::resolve_effective_harness_descriptor(
            record, &personas, &global,
        )?;
        let cfg = crate::managed_agents::effective_config::resolve_effective_config(
            record, &personas, &global,
        )
        .require_resolved()?;
        let (gate, _remove) =
            crate::managed_agents::build_respond_to_env(record, Some(owner_hex.as_str()))?;
        let team_instructions =
            crate::managed_agents::spawn_hash::effective_team_instructions(record, &teams);
        let relay_url =
            crate::relay::effective_agent_relay_url(&record.relay_url, &workspace_relay);

        let env = crate::managed_agents::external_env::external_agent_env(
            record,
            &descriptor,
            &cfg,
            &relay_url,
            team_instructions.as_deref(),
            &gate,
        );
        let env_file = crate::managed_agents::external_env::render_env_file(&env);

        Ok(ExternalAgentEnvResponse { env, env_file })
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}
