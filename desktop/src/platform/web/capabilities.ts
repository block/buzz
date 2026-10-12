// Web capability map: every Tauri command the client invokes is classified into a group, and each group has a
// policy that can be flipped later (feature-by-feature, per JM):
//   "placeholder" → UI shows <NativeOnly/> ("Available in the desktop app"), command rejects quietly with NativeOnlyError
//   "hide"        → UI should not render the surface at all (use useWebCapability(group).hidden)
//   "proxy"       → will be served by a paired desktop / BFF (not built yet; behaves like placeholder until then)
// Groups are matched in order; first match wins. Commands implemented in the web registry are "web" and never reach here.
export type CapabilityPolicy = "placeholder" | "hide" | "proxy";
export type CapabilityGroup = {
  id: string;
  label: string;
  policy: CapabilityPolicy;
  match: RegExp;
  note: string;
};

export const CAPABILITY_GROUPS: CapabilityGroup[] = [
  {
    id: "agents",
    label: "Managed agents, personas & teams",
    policy: "placeholder",
    note: "Spawns local ACP/agent sidecars; needs a paired desktop host.",
    match:
      /^((create|update)_managed_agent|get_managed_agent_log|set_managed_agent_.*|send_managed_agent_channel_message|install_acp_runtime|discover_(acp_auth_methods|acp_providers|git_bash_prerequisite|managed_agent_prereqs)|save_custom_harness|get_runtime_file_config|get_model_status|.*openclaw.*|add_team_from_catalog|(preview|confirm)_(agent|team)_snapshot_import|encode_team_snapshot_for_send|set_team_shared|update_persona_and_publish|get_bestie_assignment|read_(archived|unindexed)_observer.*|apply_openclaw.*|.*_pocket_voice_unused|list_managed_agents|start_managed_agent|stop_managed_agent|delete_managed_agent|restart_managed_agent_runtime|start_managed_agent_runtime|stop_managed_agent_runtime|put_managed_agent_runtime_lifecycle|reconcile_managed_agent_runtimes|connect_acp_runtime|discover_(acp_commands|agent_models|backend_providers)|probe_backend_provider|get_agent_(config_surface|memory|models|turn_metric_near|usage_series)|put_agent_session_config|set_agent_(avatar_communities|managed_profiles)|set_global_agent_config|get_global_agent_config|list_(personas|teams|agent_cards|managed_agent_runtimes|relay_agents)|create_(persona|team)|update_(persona|team)|delete_(persona|team|custom_harness)|set_persona_(active|shared)|reconcile_inbound_persona_event|revalidate_relay_agents|save_agent_card|load_agent_card|mint_agent_card|card_mint_.*|export_(agent|team)_snapshot|encode_agent_snapshot_for_send|fetch_(snapshot_bytes|persona_catalog|team_catalog)|assign_bestie|clear_bestie_assignment|resolve_bestie_conversation|community_bots_.*|call_openclaw_workspace_mcp_tool|refresh_openclaw_workspace_mcp|mesh_.*|build_observer_control_event|decrypt_observer_event|index_observer_channel_id|observed_unread_.*|observer_archive_default_enabled|agent_(access_owner_only|metric_archive_default_enabled)|has_managed_agent_channel_message_marker|get_git_identity|resolve_oa_owner)$/,
  },
  {
    id: "webviews",
    label:
      "Native webviews (pinned sites, Stage, Drive, playground, HTML preview, browser agent)",
    policy: "placeholder",
    note: "WKWebView surfaces; iframes blocked by X-Frame-Options on most sites. Pinned sites specifically are REMOVED from web Settings, the left nav and /pins/* (decision #1 in docs/pwa-web-differences.md).",
    match:
      /^(pin_webview_.*|playground_.*|browser_(agent_.*|drive|observe_poll)|register_html_preview|revoke_html_preview|reload_webview|focus_popout_window)$/,
  },
  {
    id: "huddle_audio",
    label: "Huddles, audio devices, TTS, voice",
    policy: "placeholder",
    note: "LiveKit + cpal today; candidate for WebRTC + Web Audio.",
    match:
      /^(ensure_huddle_agent_voice_settings|(delete|import)_pocket_voice|sync_agents_to_active_huddle|add_agent_to_huddle|(get|set)_(huddle_.*|audio_output_device|tts_settings|voice_input_mode|pocket_voice|tts_enabled)|(start|end|join|leave|confirm|reconnect|open)_huddle.*|huddle_.*|list_(audio_output_devices|voice_registry)|speak_agent_message|preview_pocket_voice|interrupt_huddle_speech|remove_agent_from_huddle|reconnect_huddle_audio|get_huddle_.*|set_huddle_.*)$/,
  },
  {
    id: "git_projects",
    label: "Local git / gh / project checkouts",
    policy: "placeholder",
    note: "Needs a host with the repos, or GitHub API instead.",
    match:
      /^(.*project_.*|get_project_.*|(create|delete)_project_remote_branch|merge_project_pull_request|gh_pr_list|clone_project_repository|open_project_repository_folder|get_project_(local_repo_file_content|repo_diff|repo_file_content)|list_project_local_repositories|publish_project_pull_request_merged_status|sign_project_.*|validate_repos_dir)$/,
  },
  {
    id: "terminals",
    label: "Terminals / PTY",
    policy: "placeholder",
    note: "Needs a server-side PTY over WebSocket + xterm.js.",
    match:
      /^(open_project_terminal|open_project_merge_recovery_terminal|prepare_term_session_launch|terminal_.*|term_.*)$/,
  },
  {
    id: "keys_at_rest",
    label: "OS keyring, backups, pairing, recovery, BuilderLab",
    policy: "hide",
    note: "Replaced by the signer (NIP-07 now, NIP-46 next); keys never live in the page.",
    match:
      /^(.*builderlab.*|check_builderlab_community_name|get_nsec|import_identity|persist_current_identity|generate_backup_passphrase|create_ncryptsec_backup|save_ncryptsec_copy|verify_ncryptsec_backup|start_identity_recovery_pairing|start_pairing|confirm_pairing_sas|cancel_pairing|archive_identity|unarchive_identity|list_archived_identities|(bind|cancel|clear|get|list|delete|start)_builderlab_.*|sign_nostr_identity_binding|announce_archive_sync_epoch|(start|stop)_archive_sync|archive_events|read_archived_events)$/,
  },
  {
    id: "os_integration",
    label:
      "Tray, haptics, idle/sleep, vibrancy, windows, deep links, updater, notifications, clipboard images",
    policy: "placeholder",
    note: "Some have Web equivalents (Notification API, Clipboard API, display-mode).",
    match:
      /^(list_popout_windows|open_popout_window|clear_tray_agent_activity|update_tray_agent_activity|take_tray_actions|requeue_tray_actions|perform_sidebar_default_haptic|get_os_idle_seconds|set_prevent_sleep_active|set_window_vibrancy|title_bar_double_click|show_native_notification|copy_image_to_clipboard|is_auto_update_supported|(acknowledge|take|clear)_pending_.*|relay_reconnect_hook(_configured)?|get_baked_build_env(_keys)?|get_media_proxy_port|get_admin_origin|set_admin_origin)$/,
  },
  {
    id: "media_and_previews",
    label: "Media upload/fetch, downloads, link previews",
    policy: "proxy",
    note: "Needs relay /media/upload (NIP-98) + a BFF for link-preview metadata (S3).",
    match:
      /^(upload_media(_bytes)?|pick_and_upload_(image|media)|fetch_(media_bytes|workspace_icon)|cancel_(media_fetch|media_upload|link_preview_metadata)|release_(media_fetch|media_upload|link_preview_metadata)|download_(file|image)|link_preview.*|get_link_preview.*|fetch_link_preview.*)$/,
  },
  {
    id: "relay_admin",
    label: "Relay admin, moderation, membership admin",
    policy: "proxy",
    note: "Relay admin API; port with NIP-98 in S6.",
    match:
      /^(admin_.*|(add|remove|change)_relay_member|get_relay_self|fetch_join_policy)$/,
  },
  {
    id: "chat_data_s2",
    label:
      "Chat data beyond S1 (threads, DMs, forum, notes, canvas, search, feed, workflows, profile edit, reactions…)",
    policy: "placeholder",
    note: "Pure-nostr; scheduled for S2.",
    match: /.*/,
  },
];

export function capabilityGroupFor(cmd: string): CapabilityGroup {
  return CAPABILITY_GROUPS.find((g) => g.match.test(cmd)) as CapabilityGroup;
}

// Runtime policy overrides (flip a group without a rebuild): localStorage "buzz-web-capability-overrides" = {"agents":"hide"}
export function groupPolicy(id: string): CapabilityPolicy {
  try {
    const o = JSON.parse(
      localStorage.getItem("buzz-web-capability-overrides") || "{}",
    );
    if (o[id]) return o[id];
  } catch {
    /* ignore */
  }
  return (CAPABILITY_GROUPS.find((g) => g.id === id) as CapabilityGroup).policy;
}

/** For UI: `const { hidden, available } = webCapability("agents")` */
export function webCapability(groupId: string) {
  const p = groupPolicy(groupId);
  return {
    available: false,
    policy: p,
    hidden: p === "hide",
    label: (CAPABILITY_GROUPS.find((g) => g.id === groupId) as CapabilityGroup)
      .label,
  };
}
