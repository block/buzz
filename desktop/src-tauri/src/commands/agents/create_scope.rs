use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{load_managed_agents, CreateManagedAgentRequest, ManagedAgentSummary},
};

use super::{
    build_deploy_payload, deploy_to_provider, start_local_agent_with_preflight,
    workspace_owner_hex,
};

pub(super) struct CapturedCreateScope {
    relay_url: Option<String>,
    signer_pubkey: Option<String>,
}

impl CapturedCreateScope {
    pub(super) fn bind(
        input: &CreateManagedAgentRequest,
        state: &AppState,
    ) -> Result<Self, String> {
        crate::relay::bind_expected_relay_scope(
            input.expected_relay_url.as_deref(),
            crate::relay::relay_ws_url_with_override(state),
        )?;
        crate::relay::bind_expected_signer(
            input.expected_signer_pubkey.as_deref(),
            workspace_owner_hex(state)?,
        )?;
        Ok(Self {
            relay_url: input.expected_relay_url.clone(),
            signer_pubkey: input.expected_signer_pubkey.clone(),
        })
    }

    pub(super) async fn start_local(
        &self,
        app: &AppHandle,
        state: &AppState,
        pubkey: &str,
    ) -> Result<ManagedAgentSummary, String> {
        start_local_agent_with_preflight(
            app,
            state,
            pubkey,
            true,
            self.relay_url.as_deref(),
            self.signer_pubkey.as_deref(),
        )
        .await
    }

    pub(super) async fn deploy_provider(
        &self,
        app: &AppHandle,
        state: &AppState,
        pubkey: &str,
        provider_id: &str,
        config: &serde_json::Value,
    ) -> Result<(), String> {
        let agent_json = {
            let _guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let records = load_managed_agents(app)?;
            let record = records
                .iter()
                .find(|record| record.pubkey == pubkey)
                .ok_or_else(|| "agent disappeared".to_string())?;
            build_deploy_payload(app, state, record)?
        };
        deploy_to_provider(
            app,
            state,
            pubkey,
            provider_id,
            config,
            agent_json,
            None,
            self.relay_url.as_deref(),
            self.signer_pubkey.as_deref(),
        )
        .await
    }
}
