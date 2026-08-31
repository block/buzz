//! Opt-in local two-process native tracer. Synthetic fixture keys only. Does not
//! expose a production IPC bypass or start normal Desktop restore/sweep services.
use crate::{
    app_state::{build_app_state, AppState},
    commands, managed_agents, relay,
};
use nostr::{Event, JsonUtil, Keys, ToBech32};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::Manager;
#[cfg(all(target_os = "macos", feature = "system-keyring"))]
mod keychain;
#[path = "host_move_tracer.rs"]
mod move_trace;

/// Run `init|source|destination FIXTURE_DIR ws://127.0.0.1:PORT` in an isolated
/// debug build. Init writes fresh synthetic keys with restricted permissions.
pub fn run() -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("tracer requires a debug build".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err(
            "usage: host-start-tracer init|source|destination FIXTURE_DIR ws://127.0.0.1:PORT"
                .into(),
        );
    }
    let role = args[1].clone();
    let root = PathBuf::from(&args[2]);
    if !root.is_absolute() {
        return Err("fixture path must be absolute".into());
    }
    let url = url::Url::parse(&args[3]).map_err(|_| "invalid relay")?;
    if url.scheme() != "ws" || url.host_str() != Some("127.0.0.1") {
        return Err("tracer requires an isolated loopback relay".into());
    }
    if role == "init" || role == "move-init" {
        std::fs::create_dir(&root).map_err(|_| "fixture directory must be new")?;
        for name in if role == "move-init" {
            vec!["owner", "agent", "peer"]
        } else {
            vec!["owner", "agent"]
        } {
            managed_agents::atomic_write_json_restricted(
                &root.join(format!("{name}.key")),
                Keys::generate().secret_key().to_secret_hex().as_bytes(),
            )
            .map_err(|_| "fixture key write failed")?;
        }
        println!("PASS synthetic fixture initialized (no keys printed)");
        return Ok(());
    }
    if !matches!(
        role.as_str(),
        "source" | "destination" | "move-source" | "move-destination"
    ) {
        return Err("unknown role".into());
    }
    // Unique per fixture and per executor: do not touch the real Desktop keyring.
    use sha2::{Digest, Sha256};
    let scope = hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()));
    std::env::set_var(
        "BUZZ_DEV_KEYRING_SERVICE",
        format!("buzz-desktop-dev.start-tracer.{scope}.{role}"),
    );
    let home = root.join(format!("{role}-home"));
    std::fs::create_dir_all(&home).map_err(|_| "fixture home")?;
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
    if dirs::home_dir().as_ref() != Some(&home) {
        return Err("cannot isolate fixture home".into());
    }
    #[cfg(all(target_os = "macos", feature = "system-keyring"))]
    keychain::install(&home)?;
    managed_agents::init_nest_dir(true);
    std::env::remove_var("BUZZ_PRIVATE_KEY");
    std::env::remove_var("BUZZ_AUTH_TAG");
    let state = build_app_state();
    *state.keys.lock().map_err(|_| "identity lock")? = keys(&root, "owner")?;
    *state.relay_url_override.lock().map_err(|_| "relay lock")? = Some(args[3].clone());
    let mut context = crate::native_context();
    context.config_mut().identifier = format!("xyz.block.buzz.start-tracer.{scope}.{role}");
    context.config_mut().app.windows.clear();
    let app = tauri::Builder::default()
        .manage(state)
        .build(context)
        .map_err(|_| "native app build failed")?;
    let handle = app.handle().clone();
    let relay = args[3].clone();
    // Use run_return: Wry's non-returning run can discard AppHandle::exit's
    // requested code. Success also requires that the trace actually completed.
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let task_completed = completed.clone();
    tauri::async_runtime::spawn(async move {
        let result = if role.starts_with("move-") {
            move_trace::trace(&handle, &role, &root, &relay).await
        } else {
            trace(&handle, &role, &root, &relay).await
        };
        match result {
            Ok(()) => {
                task_completed.store(true, std::sync::atomic::Ordering::Release);
                handle.exit(0);
            }
            Err(error) => {
                eprintln!("Start tracer failed: {error}");
                handle.exit(1);
            }
        }
    });
    let exit_code = app.run_return(|_, _| {});
    if exit_code == 0 && completed.load(std::sync::atomic::Ordering::Acquire) {
        Ok(())
    } else {
        Err("tracer did not complete successfully; inspect result files and logs".into())
    }
}

fn keys(root: &Path, name: &str) -> Result<Keys, String> {
    Keys::parse(
        &std::fs::read_to_string(root.join(format!("{name}.key")))
            .map_err(|_| "missing fixture key")?,
    )
    .map_err(|_| "invalid fixture key".into())
}
fn save(root: &Path, name: &str, value: &Value) -> Result<(), String> {
    managed_agents::atomic_write_json_restricted(
        &root.join(name),
        &serde_json::to_vec_pretty(value).map_err(|_| "serialize evidence")?,
    )
    .map_err(|_| "save evidence".into())
}
async fn trace(
    app: &tauri::AppHandle,
    role: &str,
    root: &Path,
    relay_url: &str,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let owner = state.signing_keys()?;
    let owner_hex = owner.public_key().to_hex();
    let registration = commands::create_host_registration(state.clone(), owner_hex.clone()).await?;
    let reg = Event::from_json(registration.to_string()).map_err(|_| "registration")?;
    relay::submit_signed_event_with_keys(&reg, &state, &owner, None).await?;
    println!("PASS {role} registered {}", reg.id);
    if role == "destination" {
        let agent = keys(root, "agent")?;
        let record: managed_agents::ManagedAgentRecord = serde_json::from_value(json!({
            "pubkey": agent.public_key().to_hex(), "name":"Start tracer fixture", "private_key_nsec":agent.secret_key().to_bech32().map_err(|_| "fixture key encoding")?,
            "auth_tag":buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").map_err(|_| "fixture owner attestation")?,
            "relay_url":relay_url, "acp_command":"buzz-acp", "agent_command":"buzz-agent", "agent_command_override":"buzz-agent", "agent_args":[], "mcp_command":"buzz-dev-mcp",
            "turn_timeout_seconds":320, "system_prompt":"Synthetic local tracer. Do not perform work.", "model":"fixture", "provider":"openai",
            "env_vars":{"OPENAI_COMPAT_API_KEY":"synthetic-not-a-credential", "OPENAI_COMPAT_BASE_URL":"http://127.0.0.1:18991/v1"},
            "start_on_app_launch":false, "auto_restart_on_config_change":false, "created_at":"2026-08-31T00:00:00Z", "updated_at":"2026-08-31T00:00:00Z",
            "last_started_at":null, "last_stopped_at":null, "last_exit_code":null, "last_error":null
        })).map_err(|_| "fixture record schema mismatch")?;
        managed_agents::save_managed_agents(app, &[record])?;
        let config = commands::inspect_local_execution_config(
            app.clone(),
            state.clone(),
            owner_hex.clone(),
            relay_url.into(),
            agent.public_key().to_hex(),
        )
        .await?;
        save(
            root,
            "destination.json",
            &json!({"registration":registration,"agent":agent.public_key().to_hex(),"config":config}),
        )?;
        for _ in 0..120 {
            let snapshot = commands::pump_host_start(
                app.clone(),
                state.clone(),
                owner_hex.clone(),
                relay_url.into(),
            )
            .await?;
            save(
                root,
                "destination-progress.json",
                &serde_json::to_value(snapshot).map_err(|_| "snapshot")?,
            )?;
            if root.join("finish").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        // Only the child owned by this fixture app; never broad PID sweeps.
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|_| "runtime lock")?;
        for runtime in runtimes.values_mut() {
            let _ = runtime.child.kill();
            let _ = runtime.child.wait();
        }
        println!(
            "PASS destination fixture loop ended; root children reaped (not a Stop certificate)"
        );
        return Ok(());
    }
    let destination: Value = serde_json::from_slice(
        &std::fs::read(root.join("destination.json")).map_err(|_| "destination not ready")?,
    )
    .map_err(|_| "destination fixture invalid")?;
    let text = |field: &Value| {
        field
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "destination field missing".to_string())
    };
    let agent = text(&destination["agent"])?;
    let operation = commands::queue_host_start(
        app.clone(),
        state.clone(),
        owner_hex.clone(),
        relay_url.into(),
        destination["registration"].clone(),
        agent.clone(),
        text(&destination["config"]["runtime"])?,
        text(&destination["config"]["revision"])?,
        None,
    )
    .await?;
    println!("PASS source queued immutable operation {operation}");
    for _ in 0..90 {
        let snapshot = serde_json::to_value(
            commands::pump_host_start(
                app.clone(),
                state.clone(),
                owner_hex.clone(),
                relay_url.into(),
            )
            .await?,
        )
        .map_err(|_| "snapshot")?;
        save(root, "source-progress.json", &snapshot)?;
        let spawned = snapshot["operations"].as_array().is_some_and(|ops| {
            ops.iter()
                .any(|op| op["operation"] == operation && op["status"] == "spawned")
        });
        if spawned {
            let runs = commands::get_presence_runs(
                state.clone(),
                owner_hex.clone(),
                relay_url.into(),
                vec![agent.clone()],
            )
            .await?;
            let value = serde_json::to_value(&runs).map_err(|_| "presence")?;
            save(root, "presence.json", &value)?;
            // Public ACP run ID must equal the native Start generation.
            let destination_registration =
                Event::from_json(destination["registration"].to_string())
                    .map_err(|_| "destination registration")?;
            let destination_host = buzz_core_pkg::host::validate(&destination_registration)?
                .host
                .to_hex();
            if runs.get(&agent).is_some_and(|runs| {
                runs.iter().any(|run| {
                    run.run == operation
                        && run.location.as_ref().is_some_and(|location| {
                            location.host == destination_host && !location.label.is_empty()
                        })
                })
            }) {
                save(
                    root,
                    "source-success.json",
                    &json!({"operation":operation,"snapshot":snapshot,"presence":value,"physical_hosts":1,"executors":2,"provider":"synthetic fixture; no model turn asserted"}),
                )?;
                println!("PASS source -> destination spawn -> signed correlated receipt -> public live run {operation}");
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err("timed out awaiting spawned receipt and matching live run; inspect private fixture progress".into())
}
