use tauri::Manager;

use crate::app_state::AppState;
use crate::managed_agents::{self, load_managed_agents, BackendKind};
use crate::prevent_sleep;

pub(crate) fn is_restart_request(code: Option<i32>) -> bool {
    code == Some(tauri::RESTART_EXIT_CODE)
}

pub(crate) fn shut_down_app(app: &tauri::AppHandle, shutdown_done: &std::sync::atomic::AtomicBool) {
    use std::sync::atomic::Ordering;

    app.state::<AppState>()
        .shutdown_started
        .store(true, Ordering::SeqCst);
    if !shutdown_done.swap(true, Ordering::SeqCst) {
        prevent_sleep::release(&app.state::<AppState>().prevent_sleep);
        app.state::<crate::terminal_runtime::TerminalSessions>()
            .shutdown_all();
        disconnect_managed_agent_controllers(app);
        #[cfg(feature = "mesh-llm")]
        shutdown_mesh_runtime(app);
    }
}

/// Install SIGINT/SIGTERM/SIGHUP cleanup on ctrlc's dedicated handler thread.
#[cfg(unix)]
pub(crate) fn install_signal_handler(
    app: tauri::AppHandle,
    shutdown_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    if let Err(error) = ctrlc::set_handler(move || {
        app.state::<AppState>()
            .shutdown_started
            .store(true, Ordering::SeqCst);
        if !shutdown_done.swap(true, Ordering::SeqCst) {
            app.state::<crate::terminal_runtime::TerminalSessions>()
                .shutdown_all();
            disconnect_managed_agent_controllers(&app);
            #[cfg(feature = "mesh-llm")]
            shutdown_mesh_runtime(&app);
        }
        #[cfg(all(feature = "mesh-llm", target_os = "macos"))]
        hard_exit_after_mesh_shutdown();
        #[cfg(not(all(feature = "mesh-llm", target_os = "macos")))]
        std::process::exit(0);
    }) {
        eprintln!("buzz-desktop: failed to register signal handler: {error}");
    }
}

#[cfg(all(feature = "mesh-llm", target_os = "macos"))]
fn updated_macos_binary(current_binary: &std::path::Path) -> Option<std::path::PathBuf> {
    let macos_directory = current_binary.parent()?;
    if macos_directory.file_name()? != "MacOS" {
        return None;
    }
    let contents_directory = macos_directory.parent()?;
    if contents_directory.file_name()? != "Contents" {
        return None;
    }
    let info_plist =
        plist::from_file::<_, plist::Dictionary>(contents_directory.join("Info.plist")).ok()?;
    let binary_name = info_plist.get("CFBundleExecutable")?.as_string()?;
    Some(macos_directory.join(binary_name))
}

#[cfg(all(feature = "mesh-llm", target_os = "macos"))]
pub(crate) fn relaunch_after_mesh_shutdown(app: &tauri::AppHandle) -> ! {
    use std::process::Command;

    tauri_plugin_single_instance::destroy(app);
    let env = app.env();
    match tauri::process::current_binary(&env) {
        Ok(current_binary) => {
            let binary = updated_macos_binary(&current_binary).unwrap_or(current_binary);
            if let Err(error) = Command::new(binary)
                .args(env.args_os.iter().skip(1))
                .spawn()
            {
                eprintln!("buzz-desktop: failed to relaunch app: {error}");
            }
        }
        Err(error) => eprintln!("buzz-desktop: failed to locate app for relaunch: {error}"),
    }
    hard_exit_after_mesh_shutdown();
}

#[cfg(all(feature = "mesh-llm", target_os = "macos"))]
pub(crate) fn hard_exit_after_mesh_shutdown() -> ! {
    // SAFETY: Desktop-owned resources and the embedded Mesh runtime have
    // been stopped. Durable managed runtimes are deliberately detached and
    // do not own handles whose destructors are required here.
    unsafe { libc::_exit(0) }
}

#[cfg(feature = "mesh-llm")]
pub(crate) fn shutdown_mesh_runtime(app: &tauri::AppHandle) {
    let app = app.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let runtime = state.mesh_llm_runtime.lock().await.take();
        let result = match runtime {
            Some(runtime) => runtime.stop().await,
            None => Ok(()),
        };
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("buzz-desktop: failed to stop Mesh runtime: {error}"),
        Err(error) => eprintln!("buzz-desktop: timed out stopping Mesh runtime: {error}"),
    }
}

/// Drop Desktop's local schema-v2 control handles without stopping durable
/// runtimes. A Phase-0 schema-v1 harness remains Desktop-owned and is stopped
/// only when its tracked child and anti-PID-reuse marker still agree.
pub(crate) fn disconnect_managed_agent_controllers(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let Ok(_transition) = state.managed_agent_runtime_transition.lock() else {
        return;
    };
    if let Ok(mut runtimes) = state.managed_agent_processes.lock() {
        for runtime in runtimes.values_mut().filter(|runtime| runtime.is_legacy()) {
            let Some(receipt) = runtime.legacy_receipt.as_ref() else {
                continue;
            };
            let tracked_child_matches = runtime
                .process
                .as_ref()
                .is_some_and(|process| process.child.id() == receipt.pid);
            if tracked_child_matches
                && buzz_runtime_pkg::process_matches_marker(
                    receipt.pid,
                    &receipt.process_start_marker,
                )
            {
                let _ = managed_agents::terminate_process(receipt.pid);
                if let Some(process) = runtime.process.as_mut() {
                    let _ = process.child.wait();
                }
            }
        }
        runtimes.clear();
    };
}

pub(crate) fn shutdown_managed_agents(app: &tauri::AppHandle) -> Result<(), String> {
    // Consequential shutdown (sign-out/delete/update) is explicit and routes
    // every pair through the authenticated generation-fenced control path.
    // Ordinary app exit calls `disconnect_managed_agent_controllers` instead.
    let records = load_managed_agents(app)?;
    let mut targets = {
        let state = app.state::<AppState>();
        let runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        runtimes.keys().cloned().collect::<Vec<_>>()
    };
    for (_, receipt) in managed_agents::read_all_schema_v2_runtime_receipts(app) {
        let Ok(key) =
            managed_agents::ManagedAgentRuntimeKey::new(receipt.key.pubkey, &receipt.key.relay_url)
        else {
            continue;
        };
        if !targets.contains(&key) {
            targets.push(key);
        }
    }
    for record in records
        .iter()
        .filter(|record| record.backend == BackendKind::Local)
    {
        if let Some(key) = managed_agents::workspace_pair_key(app, record) {
            if !targets.contains(&key) {
                targets.push(key);
            }
        }
    }

    let mut errors = Vec::new();
    for key in targets {
        if let Err(error) = managed_agents::stop_managed_agent_runtime(
            key.pubkey.clone(),
            key.relay_url.clone(),
            app.clone(),
        ) {
            errors.push(format!("{} on {}: {error}", key.pubkey, key.relay_url));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to stop one or more authenticated managed runtimes: {}",
            errors.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::is_restart_request;

    #[test]
    fn only_tauri_restart_exit_code_requests_a_relaunch() {
        assert!(is_restart_request(Some(tauri::RESTART_EXIT_CODE)));
        assert!(!is_restart_request(None));
        assert!(!is_restart_request(Some(0)));
    }
}
