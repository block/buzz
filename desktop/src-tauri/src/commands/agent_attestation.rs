//! Per-agent credential-persistence attestation command.
//!
//! Returns the non-secret `buzz.desktop.exact_agent_credential_persistence.v1`
//! object for one managed agent, so external controllers (or the user, via
//! copy/paste) can verify keyring-backed exact-agent credential persistence
//! without any access to key material. See
//! `managed_agents::persistence_attestation` for the schema and guarantees.

use tauri::{AppHandle, Manager as _};

use crate::app_state::AppState;
use crate::managed_agents::persistence_attestation::{
    build_agent_persistence_attestation, verify_attestation_hash, AgentPersistenceAttestation,
    AttestationInputs,
};
use crate::managed_agents::storage::observe_agent_credential_persistence;

/// Issue the persistence attestation for `pubkey`.
///
/// Read-only: observes the raw persisted store and the keyring via the
/// side-effect-free path; never migrates, writes, or touches key material.
/// Fails closed with `attestation_keyring_unreachable` /
/// `attestation_credential_missing` instead of guessing.
#[tauri::command]
pub async fn get_agent_persistence_attestation(
    app: AppHandle,
    pubkey: String,
) -> Result<AgentPersistenceAttestation, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        // Hold the store lock for a consistent read against concurrent saves.
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let observation = observe_agent_credential_persistence(&app, &pubkey)?;
        let package = app.package_info();
        let stock_release_id = format!("{}@{}", package.name, package.version);
        let issued_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let attestation = build_agent_persistence_attestation(&AttestationInputs {
            agent_pubkey: &pubkey,
            auth_tag: observation.auth_tag.as_deref(),
            inline_key_present: observation.inline_key_present,
            keyring_probe: observation.keyring_probe,
            parallelism: observation.parallelism,
            stock_release_id: &stock_release_id,
            issued_at: &issued_at,
        })?;
        // Self-check the tamper-evidence invariant before handing the object
        // to external verifiers.
        if !verify_attestation_hash(&attestation) {
            return Err("attestation_hash_self_check_failed".to_string());
        }
        Ok(attestation)
    })
    .await
    .map_err(|error| format!("attestation task join failed: {error}"))?
}
