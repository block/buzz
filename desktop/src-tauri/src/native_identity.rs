//! Native custody selection and credential-free foreground authentication status.

use nostr::Keys;
use serde::Serialize;

use crate::app_state::{AppState, IdentityStorage};

/// Native build-selected custody; never selected by renderer input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SignerMode {
    Local,
    Remote,
}

impl SignerMode {
    /// build.rs validates the inputs and emits an explicit local/remote value.
    pub(crate) fn compiled() -> Self {
        match env!("BUZZ_DESKTOP_BUILD_SIGNER_MODE") {
            "local" => Self::Local,
            // Any unexpected build output must also fail closed, never load keys.
            _ => Self::Remote,
        }
    }

    pub(crate) fn is_remote(self) -> bool {
        self == Self::Remote
    }
}

/// Evaluate local env/key generation only for a local build. Keeping the lazy
/// boundary here makes accidental reads or placeholder generation falsifiable.
pub(crate) fn bootstrap_local_identity(
    mode: SignerMode,
    local: impl FnOnce() -> (Keys, IdentityStorage),
) -> Option<(Keys, IdentityStorage)> {
    match mode {
        SignerMode::Local => Some(local()),
        SignerMode::Remote => None,
    }
}

/// Credential-free native bootstrap status. Remote absence is not local recovery.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeIdentityStatus {
    mode: SignerMode,
    auth_state: &'static str,
    public_identity: Option<String>,
    generation: u64,
    workspace_active: bool,
}

impl AppState {
    pub(crate) fn native_identity_status(&self) -> Result<NativeIdentityStatus, String> {
        let (auth_state, public_identity, generation) = if self.is_remote_identity() {
            self.native_auth.status()?
        } else {
            ("local", Some(self.identity_public_key()?.to_hex()), 0)
        };
        Ok(NativeIdentityStatus {
            mode: self.signer_mode,
            auth_state,
            public_identity,
            generation,
            workspace_active: !self.is_remote_identity() || self.native_auth.workspace_active()?,
        })
    }
}

/// Read native custody and optional public identity without exposing credentials.
#[tauri::command]
pub(crate) fn get_native_identity_status(
    state: tauri::State<'_, AppState>,
) -> Result<NativeIdentityStatus, String> {
    state.native_identity_status()
}

#[cfg(test)]
#[path = "native_identity_tests.rs"]
mod tests;

/// Capture renderer authority using the generation supplied by its mounted realm.
pub(crate) fn renderer_signer(
    state: &AppState,
    generation: Option<u64>,
) -> Result<crate::active_user_signer::ActiveUserSigner, String> {
    if state.is_remote_identity() {
        state
            .native_auth
            .workspace_signer(generation, &crate::relay::relay_ws_url_with_override(state))
    } else {
        state.active_signer()
    }
}

/// Constrain the staging preview before dispatch. Optional agent, key, community,
/// project, media and profile mutations must not inherit local-custody behavior.
/// The admitted publishing commands capture their own generation-bound signer.
pub(crate) fn preview_command_allowed(command: &str) -> bool {
    matches!(
        command,
        "get_native_identity_status"
            | "start_builderlab_login"
            | "cancel_builderlab_login"
            | "clear_builderlab_auth"
            | "get_builderlab_auth"
            | "activate_remote_workspace"
            | "get_identity"
            | "get_profile"
            | "update_profile"
            | "update_profile_at_relay"
            | "decrypt_observer_event"
            | "get_user_profile"
            | "get_users_batch"
            | "get_user_notes"
            | "search_users"
            | "get_presence"
            | "get_os_idle_seconds"
            | "get_default_relay_url"
            | "auto_connect_default_relay_enabled"
            | "is_shared_identity"
            | "get_relay_ws_url"
            | "get_relay_http_url"
            | "get_media_proxy_port"
            | "get_active_workspace"
            | "fetch_workspace_icon"
            | "fetch_join_policy"
            | "get_channels"
            | "get_open_channel_directory"
            | "get_channel_details"
            | "get_channel_members"
            | "get_canvas"
            | "get_feed"
            | "search_messages"
            | "get_forum_posts"
            | "get_forum_thread"
            | "get_thread_replies"
            | "get_channel_reconnect_repair"
            | "get_channel_window"
            | "get_channel_messages_before"
            | "get_event"
            | "get_events"
            | "get_contact_list"
            | "get_notes_timeline"
            | "get_global_notes"
            | "get_note"
            | "get_note_reactions"
            | "get_liked_notes"
            | "relay_requires_membership"
            | "list_relay_members"
            | "get_my_relay_membership"
            | "get_relay_self"
            | "resolve_oa_owner"
            | "list_relay_agents"
            | "revalidate_relay_agents"
            | "list_managed_agents"
            | "list_managed_agent_runtimes"
            | "list_personas"
            | "list_teams"
            | "get_channel_workflows"
            | "get_channels_workflows"
            | "get_workflow"
            | "get_workflow_runs"
            | "get_run_approvals"
            | "get_bestie_assignment"
            | "get_huddle_state"
            | "get_huddle_agent_pubkeys"
            | "get_voice_input_mode"
            | "get_agent_models"
            | "agent_access_owner_only"
            | "get_agent_config_surface"
            | "get_baked_build_env_keys"
            | "get_baked_build_env"
            | "get_global_agent_config"
            | "get_tts_settings"
            | "get_audio_output_device"
            | "list_audio_output_devices"
            | "get_model_status"
            | "list_channel_templates"
            | "get_observer_retention_days"
            | "archive_size_stats"
            | "get_agent_usage_series"
            | "list_save_subscriptions"
            | "read_archived_events"
            | "read_archived_observer_events_for_channel"
            | "read_unindexed_observer_rows"
            | "observer_archive_default_enabled"
            | "agent_metric_archive_default_enabled"
            | "merge_save_subscription_kinds"
            | "remove_save_subscription_kind"
            | "create_save_subscription"
            | "delete_save_subscription"
            | "index_observer_channel_id"
            | "announce_archive_sync_epoch"
            | "start_archive_sync"
            | "stop_archive_sync"
            | "channel_head_cache_load"
            | "channel_head_cache_store"
            | "channel_head_cache_clear"
            | "observed_unread_open_scope"
            | "observed_unread_ingest"
            | "unread_catch_up"
            | "sign_event"
            | "create_auth_event"
            | "send_channel_message"
            | "join_channel"
            | "build_observer_control_event"
            | "nip44_encrypt_to_self"
            | "nip44_decrypt_from_self"
            | "take_pending_community_deep_link"
            | "acknowledge_pending_community_deep_link"
            | "take_pending_navigation_deep_link"
            | "acknowledge_pending_navigation_deep_link"
            | "clear_pending_navigation_deep_links"
            | "take_pending_entity_deep_link"
            | "acknowledge_pending_entity_deep_link"
            | "title_bar_double_click"
            | "show_native_notification"
            | "take_pending_activations"
            | "notification_permission_state"
            | "request_notification_access"
            | "set_prevent_sleep_active"
            | "is_auto_update_supported"
            | "set_window_vibrancy"
            | "perform_sidebar_default_haptic"
            | "clear_tray_agent_activity"
            | "requeue_tray_actions"
            | "take_tray_actions"
            | "update_tray_agent_activity"
            | "copy_text_to_clipboard"
            | "read_clipboard_text"
            | "relay_reconnect_hook_configured"
    )
}

/// Wrap the actual registered dispatcher, not a renderer-only visibility gate.
pub(crate) fn preview_dispatch<R: tauri::Runtime>(
    handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let error = invoke
            .message
            .state_ref()
            .try_get::<AppState>()
            .and_then(|state| {
                if !state.is_remote_identity() {
                    return None;
                }
                let command = invoke.message.command();
                if !preview_command_allowed(command) {
                    return Some(
                        "This operation is disabled in the remote messaging preview".to_owned(),
                    );
                }
                if matches!(
                    command,
                    "get_native_identity_status"
                        | "start_builderlab_login"
                        | "cancel_builderlab_login"
                        | "clear_builderlab_auth"
                        | "get_builderlab_auth"
                        | "activate_remote_workspace"
                        | "title_bar_double_click"
                        | "is_auto_update_supported"
                        | "set_window_vibrancy"
                ) {
                    return None;
                }
                let generation =
                    invocation_generation(invoke.message.payload(), invoke.message.headers());
                renderer_signer(&state, generation).err()
            });
        if let Some(error) = error {
            invoke.resolver.reject(error);
            true
        } else {
            handler(invoke)
        }
    }
}

/// Raw binary IPC carries metadata in headers; never expand file bytes into JSON.
pub(crate) use crate::invocation_authority::invocation_generation;

#[cfg(test)]
mod raw_generation_tests {
    use super::invocation_generation;
    use tauri::ipc::InvokeBody;

    #[test]
    fn raw_generation_requires_valid_header_and_json_keeps_its_own_field() {
        let raw = InvokeBody::Raw(vec![0, 255, 1]);
        let mut headers = tauri::http::HeaderMap::new();
        assert_eq!(invocation_generation(&raw, &headers), None);
        for bad in ["", "-1", "1.5", "18446744073709551616", " 4"] {
            headers.insert("x-buzz-identity-generation", bad.parse().unwrap());
            assert_eq!(invocation_generation(&raw, &headers), None);
        }
        headers.insert("x-buzz-identity-generation", "4".parse().unwrap());
        assert_eq!(invocation_generation(&raw, &headers), Some(4));
        assert_eq!(
            invocation_generation(
                &InvokeBody::Json(serde_json::json!({"expectedGeneration": 8})),
                &headers
            ),
            Some(8)
        );
        assert_eq!(
            invocation_generation(&InvokeBody::Json(serde_json::json!({})), &headers),
            None
        );
    }
}
