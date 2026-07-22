#![recursion_limit = "256"] // Deep Tauri command futures exceed the default layout query depth.
mod app_state;
mod archive;
mod builderlab;
mod commands;
mod deep_link;
mod event_sync;
mod events;
mod huddle;
mod managed_agents;
mod media_proxy;
#[cfg(feature = "mesh-llm")]
mod mesh_llm;
#[cfg(not(feature = "mesh-llm"))]
mod mesh_llm_stubs;
mod migration;
#[cfg(test)]
mod model_tests;
mod models;
mod native_websocket;
mod nostr_bind;
pub mod nostr_convert;
mod prevent_sleep;
mod ptt_shortcut;
mod relay;
mod relay_admission;
mod reset;
mod secret_store;
mod shutdown;
mod templates;
mod util;
use app_state::{build_app_state, resolve_persisted_identity, AppState};
use builderlab::*;
use commands::*;
use deep_link::{
    acknowledge_pending_community_deep_link, handle_deep_link_url,
    take_pending_community_deep_link, PendingCommunityDeepLinks,
};
use huddle::audio_output::{
    get_audio_output_device, list_audio_output_devices, set_audio_output_device,
};
use huddle::reconnect::reconnect_huddle_audio;
use huddle::{
    add_agent_to_huddle, check_pipeline_hotstart, confirm_huddle_active, download_voice_models,
    end_huddle, get_huddle_agent_pubkeys, get_huddle_state, get_model_status, get_voice_input_mode,
    join_huddle, leave_huddle, push_audio_pcm, set_huddle_transcription_enabled, set_tts_enabled,
    set_voice_input_mode, speak_agent_message, start_huddle, start_stt_pipeline,
};
use managed_agents::{
    backfill_persona_snapshots, ensure_nest, list_managed_agent_runtimes,
    put_managed_agent_runtime_lifecycle, reconcile_managed_agent_runtimes,
    restart_managed_agent_runtime, start_managed_agent_runtime, stop_managed_agent_runtime,
    try_regenerate_nest,
};
#[cfg(not(feature = "mesh-llm"))]
use mesh_llm_stubs::*;
#[cfg(all(feature = "mesh-llm", target_os = "macos"))]
use shutdown::{hard_exit_after_mesh_shutdown, relaunch_after_mesh_shutdown};
use shutdown::{is_restart_request, shut_down_app};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[cfg(target_os = "macos")]
use tauri::Listener;
use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_window_state::StateFlags;

#[cfg(target_os = "macos")]
const INITIAL_RENDER_READY_EVENT: &str = "initial-render-ready";

fn reveal_initial_window<R: tauri::Runtime>(window: &tauri::Window<R>) {
    if let Err(error) = window.show() {
        eprintln!("buzz-desktop: failed to reveal main window: {error}");
        return;
    }
    if let Err(error) = window.set_focus() {
        eprintln!("buzz-desktop: failed to focus main window: {error}");
    }
}

#[cfg(target_os = "macos")]
fn set_initial_window_backing<R: tauri::Runtime>(window: &tauri::Window<R>) {
    // The window remains transparent at runtime for vibrancy. Use an opaque
    // native backing only across the first visible frames so the previous app
    // cannot show through before WebKit has submitted its first surface.
    if let Err(error) = window.set_background_color(Some(tauri::window::Color(17, 21, 24, 255))) {
        eprintln!("buzz-desktop: failed to set initial window backing: {error}");
    }
}

#[cfg(target_os = "macos")]
async fn clear_initial_window_backing<R: tauri::Runtime>(window: &tauri::Window<R>) {
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    if let Err(error) = window.set_background_color(None) {
        eprintln!("buzz-desktop: failed to clear initial window backing: {error}");
    }
}

#[cfg(target_os = "macos")]
async fn wait_for_stable_initial_window_geometry<R: tauri::Runtime>(window: &tauri::Window<R>) {
    const MAX_POLLS: usize = 120;
    const REQUIRED_STABLE_POLLS: usize = 4;

    let mut previous_bounds = None;
    let mut stable_polls = 0;

    for _ in 0..MAX_POLLS {
        // Accept whatever geometry the window-state plugin restores — maximized
        // or a normal saved size. macOS applies the restore asynchronously, so
        // we only need consecutive identical outer bounds to know it settled.
        // Gating on `is_maximized()` here would leave `bounds` permanently
        // `None` for restored non-maximized windows and stall the reveal until
        // the poll timeout.
        let bounds = match (window.outer_position(), window.outer_size()) {
            (Ok(position), Ok(size)) => Some((position.x, position.y, size.width, size.height)),
            _ => None,
        };

        if bounds.is_some() && bounds == previous_bounds {
            stable_polls += 1;
            if stable_polls >= REQUIRED_STABLE_POLLS {
                return;
            }
        } else {
            stable_polls = 0;
        }
        previous_bounds = bounds;

        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
    }

    eprintln!("buzz-desktop: initial window geometry did not settle before reveal timeout");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // mesh-llm's async chains (model download, node start/join) overflow
    // tokio's default 2 MiB worker stacks — a stack-guard SIGABRT, not a
    // panic. Upstream mesh-llm and mesh-console both run on 8 MiB worker
    // stacks for this reason; give Tauri's command runtime the same headroom
    // before anything else touches tauri::async_runtime.
    #[cfg(feature = "mesh-llm")]
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::mesh_llm::MESH_WORKER_STACK_SIZE)
        .build()
    {
        Ok(runtime) => {
            tauri::async_runtime::set(runtime.handle().clone());
            // Keep the runtime alive for the process lifetime; dropping it
            // would shut down the workers Tauri now depends on.
            std::mem::forget(runtime);
            eprintln!(
                "buzz-mesh: installed tokio runtime with {} MiB worker stacks",
                crate::mesh_llm::MESH_WORKER_STACK_SIZE / (1024 * 1024)
            );
        }
        Err(error) => {
            // Fall back to Tauri's default runtime: the app still works,
            // only deep mesh-llm futures are at risk of stack overflow.
            eprintln!("buzz-mesh: failed to build big-stack tokio runtime, using default: {error}");
        }
    }

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // Focus the existing window when a duplicate instance launches.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
            // Forward any deep link URLs from the duplicate launch.
            for arg in &argv {
                if arg.starts_with("buzz://") {
                    handle_deep_link_url(app, arg);
                }
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // Visibility is excluded: the native reveal plugin below
                // shows the window after saved geometry has been restored.
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .build(),
        )
        .plugin(
            tauri::plugin::Builder::<_, ()>::new("initial-window-reveal")
                .on_webview_ready(|webview| {
                    if webview.label() != "main" {
                        return;
                    }

                    // macOS applies the restored geometry asynchronously. Wait
                    // for several identical outer bounds and for React to
                    // commit the startup surface before revealing it.
                    let window = webview.window();

                    #[cfg(target_os = "macos")]
                    {
                        set_initial_window_backing(&window);

                        let (initial_render_tx, initial_render_rx) = tokio::sync::oneshot::channel();
                        window
                            .app_handle()
                            .once(INITIAL_RENDER_READY_EVENT, move |_| {
                                let _ = initial_render_tx.send(());
                            });

                        tauri::async_runtime::spawn(async move {
                            wait_for_stable_initial_window_geometry(&window).await;

                            if tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                initial_render_rx,
                            )
                            .await
                            .is_err()
                            {
                                eprintln!(
                                    "buzz-desktop: initial render did not commit before reveal timeout"
                                );
                            }

                            reveal_initial_window(&window);
                            clear_initial_window_backing(&window).await;
                        });
                    }

                    #[cfg(not(target_os = "macos"))]
                    {
                        reveal_initial_window(&window);
                    }
                })
                .build(),
        )
        .plugin(native_websocket::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init());

    // The global-shortcut plugin is omitted from test builds: linking it into
    // the lib-test binary makes it fail to load on Windows
    // (STATUS_ENTRYPOINT_NOT_FOUND) before any test runs.
    #[cfg(not(test))]
    let builder = builder.plugin({
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
                            if gen_arc.load(std::sync::atomic::Ordering::Acquire) != gen_at_release
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

    // Register the updater only in configured release builds; omit it locally.
    #[cfg(buzz_updater_enabled)]
    let builder = if cfg!(debug_assertions) {
        builder
    } else {
        builder.plugin(tauri_plugin_updater::Builder::new().build())
    };

    #[cfg(not(buzz_updater_enabled))]
    let builder = builder;

    let app = builder
        .register_asynchronous_uri_scheme_protocol("buzz-media", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let response = media_proxy::handle_buzz_media(&app, &request).await;
                responder.respond(response);
            });
        })
        .manage(build_app_state())
        .manage(ClipboardState::new())
        .manage(PendingCommunityDeepLinks::default())
        .manage(BuilderlabSession::default())
        .manage(BuilderlabLogin::default())
        .manage(commands::pairing::PairingHandle::new())
        .setup(move |app| {
            let app_handle = app.handle().clone();

            // ── Phase 2: boot-time sentinel wipe ──────────────────────────────
            // Must run before migrations and identity resolution so the wipe
            // completes atomically on crash recovery.
            //
            // init_nest_dir is called early here (normally it runs inside
            // run_boot_migrations) so reset::run_boot_reset can call nest_dir().
            let reset_outcome = if let Ok(data_dir) = app_handle.path().app_data_dir() {
                let is_dev_for_reset = data_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(crate::migration::is_dev_data_dir_name)
                    .unwrap_or(false);
                crate::managed_agents::init_nest_dir(is_dev_for_reset);
                crate::reset::run_boot_reset(&data_dir)
            } else {
                crate::reset::ResetOutcome::default()
            };

            if reset_outcome.failed {
                // Surface reset-failed state — skip identity resolution and
                // all side-effecting setup. The webview still loads so the
                // frontend can show the recovery screen via get_identity.
                let state = app_handle.state::<AppState>();
                state
                    .reset_failed
                    .store(true, std::sync::atomic::Ordering::Release);
                return Ok(());
            }

            // Run all pre-identity data migrations before state loads from disk.
            if reset_outcome.completed {
                migration::run_boot_migrations_after_reset(&app_handle);
            } else {
                migration::run_boot_migrations(&app_handle);
            }

            // Resolve persisted identity key (env var → file → generate+save).
            // This is fatal — the app should not start with an ephemeral identity
            // that will be lost on restart, as that silently breaks channel
            // memberships, DMs, and relay identity.
            let state = app_handle.state::<AppState>();
            if let Err(e) = resolve_persisted_identity(&app_handle, &state) {
                eprintln!("buzz-desktop: fatal: identity resolution failed: {e}");
                std::process::exit(1);
            }

            // When the identity is in recovery mode (lost = keyring empty after
            // migration, or keyring-locked = keyring unreachable but marker
            // present), all owner-keyed side effects (event sync, agent restore,
            // relay publish) are skipped. The frontend shows a recovery screen;
            // the user must relaunch after restoring the identity.
            let identity_lost = state
                .identity_lost
                .load(std::sync::atomic::Ordering::Acquire);
            let keyring_locked = state
                .keyring_locked
                .load(std::sync::atomic::Ordering::Acquire);
            let recovery_mode = identity_lost || keyring_locked;

            // Snapshot owner keys after identity resolution; the best-effort
            // event reconcile itself runs off the synchronous setup path below.
            let owner_keys = match state.keys.lock() {
                Ok(k) => k.clone(),
                Err(e) => {
                    eprintln!("buzz-desktop: fatal: owner keys lock poisoned: {e}");
                    std::process::exit(1);
                }
            };

            // Backfill the pinned persona snapshot for any pre-existing agent
            // that predates the record-authoritative-spawn cutover (persona_id
            // set but no source_version). Must run before
            // restore_managed_agents_on_launch so no agent spawns from an empty
            // snapshot. Synchronous and best-effort — a failure here must not
            // block launch, but a missing persona is logged loudly inside.
            if let Err(e) = backfill_persona_snapshots(&app_handle) {
                eprintln!("buzz-desktop: persona-snapshot backfill failed: {e}");
            }

            // Store the AppHandle so huddle commands can emit `huddle-state-changed`
            // events via `huddle::emit_huddle_state` without threading the handle
            // through every call site.
            if let Ok(mut guard) = state.app_handle.lock() {
                *guard = Some(app_handle.clone());
            }

            // Bring up the runtime-owned shared-compute coordinator before
            // saved agents are restored. Its lifetime is tied to the app, not
            // a UI mount; it publishes discovery and reconciles membership for
            // MeshLLM's native admission and transport.
            #[cfg(feature = "mesh-llm")]
            {
                // Route mesh-llm's download progress (model weights, runtime)
                // onto Tauri events so the UI can render real progress.
                crate::mesh_llm::install_progress_sink(&app_handle);
                let mesh_app = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    crate::mesh_llm::start_coordinator(mesh_app).await;
                });
            }

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

            // Create the Buzz nest (~/.buzz or ~/.buzz-dev for dev builds) before
            // agents are restored, so default_agent_workdir() resolves to the
            // nest directory. Non-fatal: agents fall back to $HOME if nest
            // creation fails.
            if let Err(error) = ensure_nest() {
                eprintln!("buzz-desktop: failed to create nest: {error}");
            }

            // Resolve the REPOS symlink from the persisted repos_dir BEFORE
            // agents are restored below, and decide whether restore is safe.
            // The frontend's apply_workspace runs only after React mounts —
            // later than the async agent restore — so without this an agent
            // could clone into the empty real REPOS dir, and once REPOS is
            // non-empty ensure_repos_symlink refuses forever. resolve_repos_at_boot
            // fails closed: if a repos_dir was configured but its symlink could
            // not be resolved (transiently unavailable external volume), it
            // returns false so we skip restore this launch rather than let an
            // agent clone into the wrong REPOS. See managed_agents::repos.
            let restore_agents = match managed_agents::nest_dir() {
                Some(nest) => managed_agents::resolve_repos_at_boot(&nest),
                None => true,
            };

            // Carry the agent's knowledge from the legacy nest (~/.sprout) into
            // the live nest after it exists. Must run after ensure_nest() so the
            // destination is present. Non-fatal.
            // On a real migration, emit a one-time hint so the user can delete
            // the now-inert ~/.sprout; the frontend dedupes the toast.
            // Suppressed when a reset completed this boot: the nest was wiped and
            // a fresh ~/.sprout-less state is exactly what we want.
            if !reset_outcome.completed && migration::migrate_legacy_nest() {
                let _ = app_handle.emit("legacy-nest-migrated", ());
            }

            // One-time migration for dev builds: copy accumulated knowledge
            // from the shared ~/.buzz nest into the new dedicated ~/.buzz-dev
            // nest so no work is lost when the nest is first namespaced.
            // Runs only when nest_dir() resolved to ~/.buzz-dev (dev instance).
            // Suppressed after a reset so re-importing ~/.buzz into ~/.buzz-dev
            // doesn't re-populate what was just wiped.
            let is_dev_nest = managed_agents::nest_dir()
                .and_then(|p| p.file_name().map(|n| n.to_os_string()))
                .is_some_and(|n| n == ".buzz-dev");
            if !reset_outcome.completed && is_dev_nest {
                migration::migrate_dev_nest();
            }

            // Create/update the local CLI symlink pointing to the
            // bundled CLI binary. Non-fatal: agents find CLI via PATH.
            if let Ok(exe) = std::env::current_exe() {
                if let Some(parent) = exe.parent() {
                    if let Err(error) = managed_agents::ensure_cli_symlink(parent, is_dev_nest) {
                        eprintln!("buzz-desktop: failed to create CLI symlink: {error}");
                    }
                }
            }

            try_regenerate_nest(&app_handle);

            // Sync team-dir edits and reconcile persona/team/agent events after
            // setup can continue. It is best-effort retention backfill, unlike
            // identity resolution above, so JSON/SQLite/signing work must not
            // hold the boot path hostage. Skipped in recovery mode — the owner
            // key is ephemeral.
            if !recovery_mode {
                event_sync::spawn_event_sync(app_handle.clone(), owner_keys);
            }

            if let Some(mgr) = huddle::models::global_model_manager() {
                mgr.start_stt_download(state.http_client.clone());
                mgr.start_tts_download(state.http_client.clone());
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

            // Defer launch-time agent restoration until `apply_workspace` has
            // installed the active workspace relay and identity. Starting here
            // would race React initialization and send agents whose saved record
            // has no relay override to the localhost fallback. Preserve the
            // boot-time repos and identity recovery safety gates by only marking
            // restoration pending when both allow it.
            if restore_agents && !recovery_mode {
                state
                    .managed_agent_restore_pending
                    .store(true, Ordering::Release);
            }

            // Periodic sweep: reap orphaned agents from dead instances every 60s.
            // Catches agents that escaped both the Justfile trap and boot-time
            // reaping (e.g. a `just staging` Ctrl+C leak that only gets collected
            // by a different instance's periodic sweep).
            let sweep_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use std::collections::HashSet;
                use std::time::Duration;
                use tauri::Manager;
                let instance_id = managed_agents::current_instance_id(&sweep_handle);
                let state = sweep_handle.state::<AppState>();
                // Two-tick grace: only reap same-instance orphans seen on two
                // consecutive sweeps. Prevents killing a legitimately-starting
                // agent that spawned between the skip-list snapshot and the scan.
                let mut prev_orphans: HashSet<u32> = HashSet::new();
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    // Collect PIDs of our own live agents to avoid killing them.
                    let skip_pids: Vec<u32> = state
                        .managed_agent_processes
                        .lock()
                        .map(|runtimes| runtimes.values().map(|rt| rt.child.id()).collect())
                        .unwrap_or_default();
                    let prev = prev_orphans.clone();
                    let inst = instance_id.clone();
                    // Run the blocking syscall work off the async executor.
                    let new_orphans = tauri::async_runtime::spawn_blocking(move || {
                        let orphans = managed_agents::sweep_system_agent_processes_with_grace(
                            &inst, &skip_pids, &prev,
                        );
                        managed_agents::reap_dead_instance_agents(&inst, &skip_pids);
                        orphans
                    })
                    .await
                    .unwrap_or_default();
                    prev_orphans = new_orphans;
                }
            });

            // Drain events the retention store flagged `pending_sync` (UI
            // create/edit, delete tombstones, launch reconcile) to the relay.
            // One loop is the sole publisher for persona, team, and managed-
            // agent writers; a relay-unreachable tick leaves rows pending for
            // the next sweep.
            // Skipped in recovery mode — flushing under an ephemeral key would
            // publish events attributed to an identity the user doesn't own.
            if !recovery_mode {
                let flush_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use std::time::Duration;
                    use tauri::Manager;
                    let Ok(db_path) = managed_agents::managed_agents_base_dir(&flush_handle)
                        .map(|d| d.join("retention.db"))
                    else {
                        eprintln!("buzz-desktop: event-flush: cannot resolve retention db path");
                        return;
                    };
                    loop {
                        let state = flush_handle.state::<AppState>();
                        if let Err(e) =
                            managed_agents::persona_events::flush_pending_events(&db_path, &state)
                                .await
                        {
                            eprintln!("buzz-desktop: event-flush: {e}");
                        }
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            take_pending_community_deep_link,
            acknowledge_pending_community_deep_link,
            start_builderlab_login,
            cancel_builderlab_login,
            get_builderlab_auth,
            clear_builderlab_auth,
            get_builderlab_nostr_identity,
            bind_builderlab_nostr_identity,
            delete_builderlab_nostr_identity,
            list_builderlab_communities,
            check_builderlab_community_name,
            create_builderlab_community,
            archive_builderlab_community,
            unarchive_builderlab_community,
            transfer_builderlab_community,
            title_bar_double_click,
            get_identity,
            get_nsec,
            import_identity,
            persist_current_identity,
            get_profile,
            update_profile,
            get_user_profile,
            get_users_batch,
            get_user_notes,
            get_git_identity,
            get_project_repo_snapshot,
            get_project_repo_diff,
            get_project_local_repo_diff,
            get_project_local_repo_snapshot,
            get_project_repo_sync_status,
            list_project_local_repositories,
            clone_project_repository,
            create_project_remote_branch,
            delete_project_remote_branch,
            push_project_local_repository,
            pull_project_local_repository,
            sign_project_pull_request_review_request,
            publish_project_pull_request_merged_status,
            merge_project_pull_request,
            open_project_terminal,
            open_project_merge_recovery_terminal,
            search_users,
            get_presence,
            get_os_idle_seconds,
            get_default_relay_url,
            get_legacy_workspace_storage,
            is_shared_identity,
            get_relay_ws_url,
            get_relay_http_url,
            get_media_proxy_port,
            fetch_link_preview_title,
            discover_acp_auth_methods,
            discover_acp_providers,
            discover_git_bash_prerequisite,
            install_acp_runtime,
            connect_acp_runtime,
            discover_managed_agent_prereqs,
            sign_event,
            sign_nostr_identity_binding,
            sign_out,
            decrypt_observer_event,
            build_observer_control_event,
            create_auth_event,
            nip44_encrypt_to_self,
            nip44_decrypt_from_self,
            get_channels,
            create_channel,
            ensure_starter_channels,
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
            send_managed_agent_channel_message,
            has_managed_agent_channel_message_marker,
            get_forum_posts,
            get_forum_thread,
            get_thread_replies,
            get_channel_window,
            get_channel_messages_before,
            edit_message,
            delete_message,
            add_reaction,
            remove_reaction,
            get_event,
            show_native_notification,
            upload_media,
            pick_and_upload_media,
            pick_and_upload_image,
            upload_media_bytes,
            download_image,
            save_png_data_url,
            download_file,
            fetch_media_bytes,
            copy_image_to_clipboard,
            copy_text_to_clipboard,
            fetch_snapshot_bytes,
            relay_requires_membership,
            list_relay_members,
            get_my_relay_membership,
            add_relay_member,
            remove_relay_member,
            change_relay_member_role,
            archive_identity,
            unarchive_identity,
            list_archived_identities,
            get_relay_self,
            resolve_oa_owner,
            list_relay_agents,
            list_managed_agents,
            list_managed_agent_runtimes,
            start_managed_agent_runtime,
            stop_managed_agent_runtime,
            restart_managed_agent_runtime,
            reconcile_managed_agent_runtimes,
            put_managed_agent_runtime_lifecycle,
            create_managed_agent,
            start_managed_agent,
            stop_managed_agent,
            set_agent_managed_profiles,
            set_managed_agent_start_on_app_launch,
            set_managed_agent_auto_restart,
            delete_managed_agent,
            get_managed_agent_log,
            get_agent_models,
            discover_agent_models,
            get_agent_config_surface,
            get_runtime_file_config,
            get_baked_build_env_keys,
            get_baked_build_env,
            put_agent_session_config,
            get_global_agent_config,
            set_global_agent_config,
            mesh_start_node,
            mesh_stop_node,
            mesh_node_status,
            mesh_installed_models,
            mesh_model_catalog,
            update_managed_agent,
            discover_backend_providers,
            probe_backend_provider,
            list_personas,
            create_persona,
            update_persona,
            delete_persona,
            set_persona_active,
            reconcile_inbound_persona_event,
            list_channel_templates,
            create_channel_template,
            update_channel_template,
            delete_channel_template,
            duplicate_channel_template,
            list_teams,
            create_team,
            update_team,
            delete_team,
            export_agent_snapshot,
            preview_agent_snapshot_import,
            confirm_agent_snapshot_import,
            encode_agent_snapshot_for_send,
            export_team_snapshot,
            encode_team_snapshot_for_send,
            preview_team_snapshot_import,
            confirm_team_snapshot_import,
            get_channel_workflows,
            get_channels_workflows,
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
            reconnect_huddle_audio,
            start_stt_pipeline,
            set_huddle_transcription_enabled,
            download_voice_models,
            get_model_status,
            set_tts_enabled,
            speak_agent_message,
            add_agent_to_huddle,
            check_pipeline_hotstart,
            confirm_huddle_active,
            perform_sidebar_default_haptic,
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
            validate_repos_dir,
            get_active_workspace,
            fetch_workspace_icon,
            set_prevent_sleep_active,
            get_agent_memory,
            relay_reconnect_hook,
            relay_reconnect_hook_configured,
            observer_archive_default_enabled,
            agent_metric_archive_default_enabled,
            archive::archive_events,
            archive::create_save_subscription,
            archive::merge_save_subscription_kinds,
            archive::remove_save_subscription_kind,
            archive::list_save_subscriptions,
            archive::delete_save_subscription,
            archive::read_archived_events,
            archive::read_archived_observer_events_for_channel,
            archive::index_observer_channel_id,
            archive::read_unindexed_observer_rows,
            is_auto_update_supported,
            set_window_vibrancy,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    let shutdown_done = Arc::new(AtomicBool::new(false));

    #[cfg(unix)]
    shutdown::install_signal_handler(app.handle().clone(), Arc::clone(&shutdown_done));

    let run_shutdown_done = Arc::clone(&shutdown_done);
    let restart_requested = Arc::new(AtomicBool::new(false));
    app.run(move |app_handle, event| match event {
        RunEvent::ExitRequested { code, .. } => {
            if is_restart_request(code) {
                restart_requested.store(true, Ordering::SeqCst);
            }
            shut_down_app(app_handle, &run_shutdown_done);
        }
        RunEvent::Exit => {
            shut_down_app(app_handle, &run_shutdown_done);
            app_handle.state::<ClipboardState>().release();

            #[cfg(all(feature = "mesh-llm", target_os = "macos"))]
            if restart_requested.load(Ordering::SeqCst) {
                relaunch_after_mesh_shutdown(app_handle);
            }

            // AppKit terminates through libc exit(), which runs C++ static
            // destructors. The embedded ggml/Metal runtime currently aborts in
            // that destructor phase even after its node has stopped cleanly.
            // End the process only after Buzz and Mesh shutdown above, while
            // deliberately skipping those native global destructors.
            #[cfg(all(feature = "mesh-llm", target_os = "macos"))]
            hard_exit_after_mesh_shutdown();
        }
        _ => {}
    });
}
