mod app_state;
mod commands;
mod events;
mod huddle;
mod managed_agents;
mod media_proxy;
mod migration;
mod models;
pub mod nostr_convert;
mod prevent_sleep;
mod relay;
mod templates;
mod util;

use app_state::{build_app_state, resolve_persisted_identity, AppState};
use commands::*;
use huddle::audio_output::{
    get_audio_output_device, list_audio_output_devices, set_audio_output_device,
};
use huddle::{
    add_agent_to_huddle, check_pipeline_hotstart, confirm_huddle_active, download_voice_models,
    end_huddle, get_huddle_agent_pubkeys, get_huddle_state, get_model_status, get_voice_input_mode,
    join_huddle, leave_huddle, push_audio_pcm, set_tts_enabled, set_voice_input_mode,
    speak_agent_message, start_huddle, start_stt_pipeline,
};
use managed_agents::{
    ensure_nest, kill_stale_tracked_processes, load_managed_agents,
    restore_managed_agents_on_launch, save_managed_agents, sync_managed_agent_processes,
    try_regenerate_nest, BackendKind, ManagedAgentProcess,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_window_state::StateFlags;
use url::Url;

fn shutdown_managed_agents(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let mut changed = sync_managed_agent_processes(&mut records, &mut runtimes);
    changed |= kill_stale_tracked_processes(&mut records, &runtimes);

    // Stop all tracked agents. Send SIGTERM to all process
    // groups first, then wait for exits in parallel to avoid serial 1s waits.
    struct AgentToStop {
        idx: usize,
        pid: u32,
        runtime: Option<ManagedAgentProcess>,
    }

    let mut to_stop: Vec<AgentToStop> = Vec::new();
    for (idx, record) in records.iter_mut().enumerate() {
        if record.backend != BackendKind::Local {
            continue;
        }
        if record.runtime_pid.is_none() && !runtimes.contains_key(&record.pubkey) {
            continue;
        }
        let runtime = runtimes.remove(&record.pubkey);
        let Some(pid) = runtime
            .as_ref()
            .map(|rt| rt.child.id())
            .or(record.runtime_pid)
        else {
            continue;
        };
        to_stop.push(AgentToStop { idx, pid, runtime });
    }

    if !to_stop.is_empty() {
        changed = true;

        // Fan-out: send SIGTERM to all process groups at once.
        #[cfg(unix)]
        for agent in &to_stop {
            let pgid = -(agent.pid as i32);
            unsafe {
                libc::kill(pgid, libc::SIGTERM);
            }
        }

        // Wait up to 2s for all to exit, checking in a polling loop.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if to_stop
                .iter()
                .all(|a| !managed_agents::process_is_running(a.pid))
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // Fan-out: SIGKILL any survivors.
        #[cfg(unix)]
        for agent in &to_stop {
            if managed_agents::process_is_running(agent.pid) {
                let pgid = -(agent.pid as i32);
                unsafe {
                    libc::kill(pgid, libc::SIGKILL);
                }
            }
        }

        // Reap children and update records.
        for mut agent in to_stop {
            if let Some(ref mut rt) = agent.runtime {
                // Best-effort reap — don’t block shutdown if the child is stuck
                // in uninterruptible sleep. The zombie will be cleaned up when
                // our process exits and launchd reaps it.
                let _ = rt.child.try_wait();
                // Write log marker (best-effort).
                let record = &records[agent.idx];
                let _ = managed_agents::append_log_marker(
                    &rt.log_path,
                    &format!(
                        "=== stopped {} ({}) at {} ===",
                        record.name,
                        record.pubkey,
                        util::now_iso()
                    ),
                );
            }
            let record = &mut records[agent.idx];
            record.runtime_pid = None;
            record.last_stopped_at = Some(util::now_iso());
            record.updated_at = util::now_iso();
            record.last_exit_code = None;
            record.last_error = None;
        }
    }

    // Final sweep: kill any orphaned agent processes we have PID file receipts
    // for that escaped process-group kills or weren't tracked in records.
    // All tracked PIDs have already been killed above, so pass an empty skip list.
    managed_agents::sweep_orphaned_agent_processes(app, &[]);

    if changed {
        save_managed_agents(app, &records)?;
    }

    Ok(())
}

/// Parse the query string of a `sprout://message?…` URL into the JSON
/// payload emitted on `deep-link-message`. Returns `None` when a required
/// param (`channel`, `id`) is missing or empty — mirroring the validation
/// policy of the `connect` arm so the frontend never sees a half-formed
/// payload (e.g. `channelId: ""` from `channel=&id=foo`).
///
/// Pulled out of `handle_deep_link_url` so it can be unit-tested without
/// a live `tauri::AppHandle`.
fn parse_message_deep_link(url: &Url) -> Option<serde_json::Value> {
    let mut channel: Option<String> = None;
    let mut message_id: Option<String> = None;
    let mut thread: Option<String> = None;
    for (k, v) in url.query_pairs() {
        let v = v.into_owned();
        if v.is_empty() {
            continue;
        }
        match k.as_ref() {
            "channel" => channel = Some(v),
            "id" => message_id = Some(v),
            "thread" => thread = Some(v),
            _ => {}
        }
    }
    let (channel_id, message_id) = (channel?, message_id?);
    Some(serde_json::json!({
        "channelId": channel_id,
        "messageId": message_id,
        "threadRootId": thread,
    }))
}

/// Handle an incoming `sprout://` deep link URL.
///
/// Currently supports:
/// - `sprout://connect?relay=<ws(s)://...>` — emits `deep-link-connect` to the frontend
fn handle_deep_link_url(app: &tauri::AppHandle, url_str: &str) {
    let url = match Url::parse(url_str) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("sprout-desktop: invalid deep link URL {url_str:?}: {e}");
            return;
        }
    };

    if url.scheme() != "sprout" {
        eprintln!("sprout-desktop: ignoring non-sprout deep link: {url_str}");
        return;
    }

    match url.host_str() {
        Some("connect") => {
            let relay = url
                .query_pairs()
                .find(|(k, _)| k == "relay")
                .map(|(_, v)| v.into_owned());
            let Some(relay_url) = relay else {
                eprintln!("sprout-desktop: connect deep link missing relay param: {url_str}");
                return;
            };
            // Validate the relay URL is ws:// or wss://
            match Url::parse(&relay_url) {
                Ok(parsed) if parsed.scheme() == "ws" || parsed.scheme() == "wss" => {}
                Ok(parsed) => {
                    eprintln!(
                        "sprout-desktop: rejecting non-websocket relay URL scheme {:?}: {relay_url}",
                        parsed.scheme()
                    );
                    return;
                }
                Err(e) => {
                    eprintln!("sprout-desktop: invalid relay URL {relay_url:?}: {e}");
                    return;
                }
            }
            let _ = app.emit("deep-link-connect", relay_url);
        }
        Some("message") => {
            // `sprout://message?channel=<uuid>&id=<eventId>[&thread=<rootId>]`
            //
            // Validation policy mirrors the `connect` arm: parse what we
            // need, refuse to emit anything if a required param is missing
            // so the frontend never sees a half-formed payload. The
            // frontend listener mirrors `parseMessageLink` in TS — we keep
            // structure on this side (serde JSON) and let the TS code own
            // any further normalisation.
            let Some(payload) = parse_message_deep_link(&url) else {
                eprintln!("sprout-desktop: message deep link missing channel or id: {url_str}");
                return;
            };
            let _ = app.emit("deep-link-message", payload);
        }
        Some(action) => {
            eprintln!("sprout-desktop: unknown deep link action: {action}");
        }
        None => {
            eprintln!("sprout-desktop: deep link missing action: {url_str}");
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // Focus the existing window when a duplicate instance launches.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
            // Forward any deep link URLs from the duplicate launch.
            for arg in &argv {
                if arg.starts_with("sprout://") {
                    handle_deep_link_url(app, arg);
                }
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .plugin(tauri_plugin_websocket::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin({
            use tauri_plugin_global_shortcut::ShortcutState;

            // Generation counter for the release delay task. Incremented on
            // every press — a delayed release only fires if the generation
            // hasn't changed (i.e. no new press happened during the delay).
            // This prevents press→release→press within 200 ms from having
            // the first release clobber the second press.
            let ptt_press_gen = Arc::new(std::sync::atomic::AtomicU64::new(0));

            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _shortcut, event| {
                    let state = match app.try_state::<AppState>() {
                        Some(s) => s,
                        None => return,
                    };

                    // Only act if a huddle is active and mode is PTT.
                    let (is_ptt_mode, is_active) = match state.huddle_state.lock() {
                        Ok(hs) => (
                            hs.voice_input_mode == huddle::VoiceInputMode::PushToTalk,
                            matches!(
                                hs.phase,
                                huddle::HuddlePhase::Connected | huddle::HuddlePhase::Active
                            ),
                        ),
                        Err(_) => return,
                    };

                    if !is_ptt_mode || !is_active {
                        return;
                    }

                    match event.state {
                        ShortcutState::Pressed => {
                            // Bump generation — invalidates any pending release delay.
                            ptt_press_gen.fetch_add(1, std::sync::atomic::Ordering::Release);

                            if let Ok(hs) = state.huddle_state.lock() {
                                hs.ptt_active
                                    .store(true, std::sync::atomic::Ordering::Release);
                                // Only cancel TTS if it's actually playing — avoids
                                // a stale cancel flag that drops the next queued message.
                                if hs.tts_active.load(std::sync::atomic::Ordering::Acquire) {
                                    hs.tts_cancel
                                        .store(true, std::sync::atomic::Ordering::Release);
                                }
                            }
                            // Emit ptt-state=true to the frontend.
                            // The React side plays the press audio cue on this event
                            // (Web Audio API via HuddleContext). Rust-side rodio audio
                            // was considered but rejected: the rodio OutputStream must
                            // outlive the handler and sharing it across the shortcut
                            // closure adds lifecycle complexity for marginal gain.
                            // The React implementation is sufficient and simpler.
                            let _ = app.emit("ptt-state", true);
                        }
                        ShortcutState::Released => {
                            // Capture generation at release time.
                            let gen_at_release =
                                ptt_press_gen.load(std::sync::atomic::Ordering::Acquire);
                            let gen_arc = Arc::clone(&ptt_press_gen);
                            let app_handle = app.clone();
                            // 200 ms release delay — captures the tail of the utterance.
                            // Only applies if no new press happened during the delay.
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                                // Check generation — if it changed, a new press arrived.
                                if gen_arc.load(std::sync::atomic::Ordering::Acquire)
                                    != gen_at_release
                                {
                                    return; // Superseded by a new press.
                                }
                                if let Some(state) = app_handle.try_state::<AppState>() {
                                    if let Ok(hs) = state.huddle_state.lock() {
                                        hs.ptt_active
                                            .store(false, std::sync::atomic::Ordering::Release);
                                    }
                                }
                                // Emit ptt-state=false — React plays the release audio cue.
                                let _ = app_handle.emit("ptt-state", false);
                            });
                        }
                    }
                })
                .build()
        });

    // Only register the updater in release builds that were compiled with a
    // real updater configuration. Local unsigned builds omit that config and
    // should still launch for debugging.
    #[cfg(sprout_updater_enabled)]
    let builder = if cfg!(debug_assertions) {
        builder
    } else {
        builder.plugin(tauri_plugin_updater::Builder::new().build())
    };

    #[cfg(not(sprout_updater_enabled))]
    let builder = builder;

    let shutdown_started = Arc::new(AtomicBool::new(false));
    let restore_shutdown_started = Arc::clone(&shutdown_started);
    let app = builder
        .register_asynchronous_uri_scheme_protocol("sprout-media", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let response = media_proxy::handle_sprout_media(&app, &request).await;
                responder.respond(response);
            });
        })
        .manage(build_app_state())
        .manage(commands::pairing::PairingHandle::new())
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let shutdown_started = Arc::clone(&restore_shutdown_started);

            // Sync shared agent data from the canonical dev data directory to
            // this worktree's data directory. Must run before
            // restore_managed_agents_on_launch (which reads managed-agents.json).
            migration::sync_shared_agent_data(&app_handle);
            migration::reconcile_provider_mcp_commands(&app_handle);

            // Resolve persisted identity key (env var → file → generate+save).
            // This is fatal — the app should not start with an ephemeral identity
            // that will be lost on restart, as that silently breaks channel
            // memberships, DMs, and relay identity.
            let state = app_handle.state::<AppState>();

            // Store the AppHandle so huddle commands can emit `huddle-state-changed`
            // events via `huddle::emit_huddle_state` without threading the handle
            // through every call site.
            if let Ok(mut guard) = state.app_handle.lock() {
                *guard = Some(app_handle.clone());
            }

            resolve_persisted_identity(&app_handle, &state)
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

            // Start the localhost media streaming proxy. Uses the shared HTTP
            // client so WARP tunnelling applies. The port is stored in AppState
            // and exposed to the frontend via the `get_media_proxy_port` command.
            let proxy_client = state.http_client.clone();
            let proxy_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let port = media_proxy::spawn_media_proxy(proxy_client, proxy_handle.clone()).await;
                let state = proxy_handle.state::<AppState>();
                state
                    .media_proxy_port
                    .store(port, std::sync::atomic::Ordering::Relaxed);
            });

            // Create the Sprout nest (~/.sprout) before agents are restored,
            // so default_agent_workdir() resolves to the nest directory.
            // Non-fatal: agents fall back to $HOME if nest creation fails.
            if let Err(error) = ensure_nest() {
                eprintln!("sprout-desktop: failed to create nest: {error}");
            }

            // Create/update ~/.local/bin/sprout symlink pointing to the
            // bundled CLI binary. Non-fatal: agents find CLI via PATH.
            if let Ok(exe) = std::env::current_exe() {
                if let Some(parent) = exe.parent() {
                    if let Err(error) = managed_agents::ensure_cli_symlink(parent) {
                        eprintln!("sprout-desktop: failed to create CLI symlink: {error}");
                    }
                }
            }

            try_regenerate_nest(&app_handle);

            if let Some(mgr) = huddle::models::global_model_manager() {
                mgr.start_stt_download(state.http_client.clone());
                mgr.start_tts_download(state.http_client.clone());
            }

            // Non-fatal: huddle works without the shortcut (user can switch to VAD mode).
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
                let shortcut = Shortcut::new(Some(Modifiers::CONTROL), Code::Space);
                if let Err(e) = app.handle().global_shortcut().register(shortcut) {
                    eprintln!("sprout-desktop: failed to register PTT shortcut: {e}");
                }
            }

            // Handle deep link URLs received while the app is running (macOS)
            // and on cold start. The single-instance plugin handles forwarding
            // from duplicate launches on Windows/Linux.
            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let dl_handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        handle_deep_link_url(&dl_handle, url.as_str());
                    }
                });
            }

            // Keep launch-time agent restoration off the synchronous setup path
            // so the frontend can mount and reveal the window promptly.
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) =
                    restore_managed_agents_on_launch(&app_handle, shutdown_started.as_ref())
                {
                    eprintln!("sprout-desktop: failed to restore managed agents: {error}");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_identity,
            get_nsec,
            import_identity,
            get_profile,
            update_profile,
            get_user_profile,
            get_users_batch,
            get_user_notes,
            search_users,
            get_presence,
            set_presence,
            get_default_relay_url,
            is_shared_identity,
            get_relay_ws_url,
            get_relay_http_url,
            get_media_proxy_port,
            discover_acp_providers,
            install_acp_runtime,
            discover_managed_agent_prereqs,
            sign_event,
            decrypt_observer_event,
            build_observer_control_event,
            create_auth_event,
            nip44_encrypt_to_self,
            nip44_decrypt_from_self,
            get_channels,
            create_channel,
            open_dm,
            hide_dm,
            get_channel_details,
            get_channel_members,
            update_channel,
            set_channel_topic,
            set_channel_purpose,
            archive_channel,
            unarchive_channel,
            delete_channel,
            add_channel_members,
            remove_channel_member,
            change_channel_member_role,
            join_channel,
            leave_channel,
            get_canvas,
            set_canvas,
            get_feed,
            search_messages,
            send_channel_message,
            get_forum_posts,
            get_forum_thread,
            edit_message,
            delete_message,
            add_reaction,
            remove_reaction,
            get_event,
            upload_media,
            pick_and_upload_media,
            upload_media_bytes,
            download_image,
            list_relay_members,
            get_my_relay_membership,
            add_relay_member,
            remove_relay_member,
            change_relay_member_role,
            // NIP-IA identity archival
            archive_identity,
            unarchive_identity,
            list_archived_identities,
            resolve_oa_owner,
            list_relay_agents,
            list_managed_agents,
            create_managed_agent,
            start_managed_agent,
            stop_managed_agent,
            set_managed_agent_start_on_app_launch,
            delete_managed_agent,
            get_managed_agent_log,
            get_agent_models,
            update_managed_agent,
            discover_backend_providers,
            probe_backend_provider,
            list_personas,
            create_persona,
            update_persona,
            delete_persona,
            set_persona_active,
            list_channel_templates,
            create_channel_template,
            update_channel_template,
            delete_channel_template,
            duplicate_channel_template,
            list_teams,
            create_team,
            update_team,
            delete_team,
            export_team_to_json,
            parse_team_file,
            parse_persona_files,
            export_persona_to_json,
            install_persona_pack,
            uninstall_persona_pack,
            list_persona_packs,
            get_channel_workflows,
            get_workflow,
            create_workflow,
            update_workflow,
            delete_workflow,
            get_workflow_runs,
            get_run_approvals,
            trigger_workflow,
            grant_approval,
            deny_approval,
            publish_note,
            get_contact_list,
            set_contact_list,
            get_notes_timeline,
            get_global_notes,
            get_note,
            get_note_reactions,
            get_liked_notes,
            start_huddle,
            join_huddle,
            leave_huddle,
            end_huddle,
            get_huddle_state,
            push_audio_pcm,
            start_stt_pipeline,
            download_voice_models,
            get_model_status,
            set_tts_enabled,
            speak_agent_message,
            add_agent_to_huddle,
            check_pipeline_hotstart,
            confirm_huddle_active,
            get_huddle_agent_pubkeys,
            set_voice_input_mode,
            get_voice_input_mode,
            list_audio_output_devices,
            set_audio_output_device,
            get_audio_output_device,
            start_pairing,
            confirm_pairing_sas,
            cancel_pairing,
            apply_workspace,
            get_active_workspace,
            set_prevent_sleep_active,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    let shutdown_done = AtomicBool::new(false);
    app.run(move |app_handle, event| match event {
        RunEvent::ExitRequested { .. } | RunEvent::Exit => {
            shutdown_started.store(true, Ordering::SeqCst);
            if !shutdown_done.swap(true, Ordering::SeqCst) {
                prevent_sleep::release(&app_handle.state::<AppState>().prevent_sleep);
                if let Err(error) = shutdown_managed_agents(app_handle) {
                    eprintln!("sprout-desktop: failed to stop managed agents: {error}");
                }
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use url::Url;

    use crate::models::ChannelInfo;
    use crate::parse_message_deep_link;

    #[test]
    fn channel_info_defaults_is_member_for_legacy_payloads() {
        let channel: ChannelInfo = serde_json::from_value(json!({
            "id": "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
            "name": "general",
            "channel_type": "stream",
            "visibility": "open",
            "description": "General discussion",
            "topic": null,
            "purpose": null,
            "member_count": 3,
            "last_message_at": null,
            "archived_at": null,
            "participants": [],
            "participant_pubkeys": []
        }))
        .expect("legacy payload should deserialize");

        assert!(channel.is_member);
    }

    #[test]
    fn parse_message_deep_link_extracts_required_params() {
        let url = Url::parse("sprout://message?channel=abc&id=xyz").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert_eq!(payload["channelId"], "abc");
        assert_eq!(payload["messageId"], "xyz");
        assert!(payload["threadRootId"].is_null());
    }

    #[test]
    fn parse_message_deep_link_includes_thread_root() {
        let url = Url::parse("sprout://message?channel=abc&id=xyz&thread=root1").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert_eq!(payload["threadRootId"], "root1");
    }

    #[test]
    fn parse_message_deep_link_rejects_missing_id() {
        let url = Url::parse("sprout://message?channel=abc").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_rejects_empty_channel() {
        // Regression: `channel=&id=foo` previously produced channelId: "".
        let url = Url::parse("sprout://message?channel=&id=foo").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_rejects_empty_id() {
        let url = Url::parse("sprout://message?channel=abc&id=").unwrap();
        assert!(parse_message_deep_link(&url).is_none());
    }

    #[test]
    fn parse_message_deep_link_treats_empty_thread_as_absent() {
        let url = Url::parse("sprout://message?channel=abc&id=xyz&thread=").unwrap();
        let payload = parse_message_deep_link(&url).expect("required params present");
        assert!(payload["threadRootId"].is_null());
    }
}
