//! Opt-in tracer extension; no manufactured Stopped observations. On a baseline
//! executor the expected result is blocked Move, NOT a successful migration.
use super::{keys, save};
use crate::{app_state::AppState, commands, managed_agents, relay};
use nostr::{Event, JsonUtil, ToBech32};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tauri::Manager;

async fn pump(app: &tauri::AppHandle, owner: &str, relay: &str) -> Result<Value, String> {
    serde_json::to_value(
        commands::pump_host_start(
            app.clone(),
            app.state::<AppState>(),
            owner.into(),
            relay.into(),
        )
        .await?,
    )
    .map_err(|_| "snapshot".into())
}
fn text(v: &Value) -> Result<String, String> {
    v.as_str()
        .map(str::to_owned)
        .ok_or("fixture field absent".into())
}
fn read(root: &Path, name: &str) -> Result<Value, String> {
    serde_json::from_slice(&std::fs::read(root.join(name)).map_err(|_| "fixture file absent")?)
        .map_err(|_| "fixture JSON invalid".into())
}

pub(super) async fn trace(
    app: &tauri::AppHandle,
    role: &str,
    root: &Path,
    relay_url: &str,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let owner = state.signing_keys()?;
    let owner_hex = owner.public_key().to_hex();
    let mut socket =
        buzz_ws_client_pkg::NostrWsConnection::connect_authenticated(relay_url, &owner, None)
            .await
            .map_err(|_| "fixture owner socket")?;
    // Restart reuses the exact registration, instead of changing its ID.
    let file = format!("{role}.json");
    let registration = match read(root, &file) {
        Ok(value) => value["registration"].clone(),
        Err(_) => commands::create_host_registration(state.clone(), owner_hex.clone()).await?,
    };
    let reg = Event::from_json(registration.to_string()).map_err(|_| "registration")?;
    relay::submit_signed_event_with_keys(&reg, &state, &owner, None).await?;
    let mut records = Vec::new();
    for name in if role == "move-source" {
        vec!["agent", "peer"]
    } else {
        vec!["agent"]
    } {
        let agent = keys(root, name)?;
        let record = serde_json::from_value::<managed_agents::ManagedAgentRecord>(json!({
            "pubkey":agent.public_key().to_hex(), "name":format!("Move tracer {name}"), "private_key_nsec":agent.secret_key().to_bech32().map_err(|_| "fixture encoding")?,
            "auth_tag":buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").map_err(|_| "fixture attestation")?,
            "relay_url":relay_url, "acp_command":"buzz-acp", "agent_command":"buzz-agent", "agent_command_override":"buzz-agent", "agent_args":[], "mcp_command":"buzz-dev-mcp",
            "turn_timeout_seconds":320, "system_prompt":"Synthetic local Move tracer. Do not perform work.", "model":"fixture", "provider":"openai",
            "env_vars":{"OPENAI_COMPAT_API_KEY":"synthetic-not-a-credential", "OPENAI_COMPAT_BASE_URL":"http://127.0.0.1:18992/v1"},
            "start_on_app_launch":false, "auto_restart_on_config_change":false, "created_at":"2026-08-31T00:00:00Z", "updated_at":"2026-08-31T00:00:00Z",
            "last_started_at":null, "last_stopped_at":null, "last_exit_code":null, "last_error":null
        })).map_err(|_| "fixture record")?;
        records.push(record);
    }
    // Never overwrite a running/restarted fixture's records or erase journals.
    if managed_agents::load_managed_agents(app)?.is_empty() {
        managed_agents::save_managed_agents(app, &records)?;
    }
    let agent = keys(root, "agent")?.public_key().to_hex();
    let config = commands::inspect_local_execution_config(
        app.clone(),
        state.clone(),
        owner_hex.clone(),
        relay_url.into(),
        agent.clone(),
    )
    .await?;
    pump(app, &owner_hex, relay_url).await?;
    let report = commands::create_host_report(
        app.clone(),
        state.clone(),
        owner_hex.clone(),
        registration.clone(),
    )
    .await?;
    let event = Event::from_json(report.to_string()).map_err(|_| "report")?;
    let ack = socket
        .send_event(event)
        .await
        .map_err(|_| "fixture host publication failed")?;
    if !ack.accepted {
        return Err(format!(
            "fixture host publication rejected: {}",
            ack.message
        ));
    }
    let host_run = uuid::Uuid::new_v4().simple().to_string();
    pulse(app, &owner_hex, &registration, &host_run, 0, &mut socket).await?;
    save(
        root,
        &file,
        &json!({"registration":registration,"config":config,"agent":agent}),
    )?;
    if role == "move-destination" {
        for tick in 0..240 {
            if tick % 30 == 0 {
                pulse(
                    app,
                    &owner_hex,
                    &registration,
                    &host_run,
                    tick + 1,
                    &mut socket,
                )
                .await?;
            }
            save(
                root,
                "move-destination-progress.json",
                &pump(app, &owner_hex, relay_url).await?,
            )?;
            if root.join("move-finish").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        if root.join("move-finish").exists() {
            let old = read(root, "selected-source.json")?;
            if managed_agents::stop_managed_agent_runtime(
                agent.clone(),
                relay_url.into(),
                Some(text(&old["run"])?),
                app.clone(),
            )
            .is_ok()
            {
                return Err("stale source Stop targeted destination successor".into());
            }
            save(
                root,
                "successor-stale-stop-rejected.json",
                &json!({"rejected":true}),
            )?;
        }
        return cleanup(app);
    }
    let peer = keys(root, "peer")?.public_key().to_hex();
    let peer_config = commands::inspect_local_execution_config(
        app.clone(),
        state.clone(),
        owner_hex.clone(),
        relay_url.into(),
        peer.clone(),
    )
    .await?;
    let run = commands::queue_host_start(
        app.clone(),
        state.clone(),
        owner_hex.clone(),
        relay_url.into(),
        registration.clone(),
        agent.clone(),
        text(&config["runtime"])?,
        text(&config["revision"])?,
        None,
    )
    .await?;
    let peer_run = commands::queue_host_start(
        app.clone(),
        state.clone(),
        owner_hex.clone(),
        relay_url.into(),
        registration.clone(),
        peer.clone(),
        text(&peer_config["runtime"])?,
        text(&peer_config["revision"])?,
        None,
    )
    .await?;
    let mut selected_pid = None;
    let mut peer_pid = None;
    for _ in 0..90 {
        let snapshot = pump(app, &owner_hex, relay_url).await?;
        save(root, "move-source-progress.json", &snapshot)?;
        let runs = commands::get_presence_runs(
            state.clone(),
            owner_hex.clone(),
            relay_url.into(),
            vec![agent.clone(), peer.clone()],
        )
        .await?;
        if runs
            .get(&agent)
            .is_some_and(|r| r.iter().any(|r| r.run == run))
            && runs
                .get(&peer)
                .is_some_and(|r| r.iter().any(|r| r.run == peer_run))
        {
            let runtimes = state
                .managed_agent_processes
                .lock()
                .map_err(|_| "runtime lock")?;
            selected_pid = runtimes
                .get(&managed_agents::ManagedAgentRuntimeKey::new(
                    &agent, relay_url,
                )?)
                .map(|r| r.child.id());
            peer_pid = runtimes
                .get(&managed_agents::ManagedAgentRuntimeKey::new(
                    &peer, relay_url,
                )?)
                .map(|r| r.child.id());
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if selected_pid.is_none() || peer_pid.is_none() {
        return Err("source and peer not observed live".into());
    }
    // Exercise the ordinary Desktop Stop boundary with a stale clicked nonce.
    // It must neither stop this current generation nor create a placement fence.
    if managed_agents::stop_managed_agent_runtime(
        agent.clone(),
        relay_url.into(),
        Some("00".repeat(16)),
        app.clone(),
    )
    .is_ok()
    {
        return Err("ordinary Stop accepted a stale clicked generation".into());
    }
    save(root, "selected-source.json", &json!({"run":run}))?;
    let destination = read(root, "move-destination.json")?;
    let movement = commands::queue_host_move(
        app.clone(),
        state.clone(),
        owner_hex.clone(),
        relay_url.into(),
        registration.clone(),
        run.clone(),
        destination["registration"].clone(),
        agent.clone(),
        text(&destination["config"]["runtime"])?,
        text(&destination["config"]["revision"])?,
    )
    .await?;
    // Double-click retry must return precisely the same persisted Move.
    let retry = commands::queue_host_move(
        app.clone(),
        state.clone(),
        owner_hex.clone(),
        relay_url.into(),
        registration,
        run.clone(),
        destination["registration"].clone(),
        agent.clone(),
        text(&destination["config"]["runtime"])?,
        text(&destination["config"]["revision"])?,
    )
    .await?;
    if retry != movement {
        return Err("Move duplicated on retry".into());
    }
    for _ in 0..120 {
        let snapshot = pump(app, &owner_hex, relay_url).await?;
        save(root, "move-source-progress.json", &snapshot)?;
        let movement = snapshot["moves"]
            .as_array()
            .and_then(|m| m.iter().find(|m| m["operation"] == movement));
        if let Some(movement) = movement {
            let status = movement["status"].as_str().unwrap_or("");
            if status == "stop_unconfirmed" || status == "destination_spawned" {
                let peer_alive = {
                    let mut runtimes = state
                        .managed_agent_processes
                        .lock()
                        .map_err(|_| "runtime lock")?;
                    let key = managed_agents::ManagedAgentRuntimeKey::new(&peer, relay_url)?;
                    runtimes.get_mut(&key).is_some_and(|r| {
                        Some(r.child.id()) == peer_pid && matches!(r.child.try_wait(), Ok(None))
                    })
                };
                if !peer_alive {
                    return Err("unrelated peer did not survive Move".into());
                }
                let runs = commands::get_presence_runs(
                    state.clone(),
                    owner_hex.clone(),
                    relay_url.into(),
                    vec![agent.clone(), peer.clone()],
                )
                .await?;
                let new_run = text(&movement["destination_run"])?;
                let new_host = text(&movement["destination_host"])?;
                let matched = runs.get(&agent).is_some_and(|r| {
                    r.iter().any(|r| {
                        r.run == new_run && r.location.as_ref().is_some_and(|l| l.host == new_host)
                    })
                });
                if status == "destination_spawned" && !matched {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                save(
                    root,
                    if status == "stop_unconfirmed" {
                        "move-blocked.json"
                    } else {
                        "move-success.json"
                    },
                    &json!({
                        "snapshot":snapshot,"presence":runs,"source_run":run,"selected_pid":selected_pid,"peer_run":peer_run,"peer_pid":peer_pid,"peer_alive":peer_alive,
                        "same_identity":agent,"matched_new_run_host":matched,"physical_hosts":1,"executors":2,"provider":"synthetic fixture; no inference turn",
                        "result":status,"certification":"Only native signed receipt outcomes; no manufactured Stopped"
                    }),
                )?;
                if status == "stop_unconfirmed" {
                    cleanup(app)?;
                    return Err("Move correctly blocked: source has no certified Stopped receipt (see move-blocked.json)".into());
                }
                if state
                    .managed_agent_processes
                    .lock()
                    .map_err(|_| "runtime lock")?
                    .contains_key(&managed_agents::ManagedAgentRuntimeKey::new(
                        &agent, relay_url,
                    )?)
                {
                    return Err("source runtime remains tracked after confirmed Move".into());
                }
                // Ordinary existing Stop must consume the real peer's supported
                // proof and persist Stopped through the same execution authority.
                let peer_stop = managed_agents::stop_managed_agent_runtime(
                    peer.clone(),
                    relay_url.into(),
                    Some(peer_run.clone()),
                    app.clone(),
                )?;
                save(
                    root,
                    "ordinary-stop-success.json",
                    &serde_json::to_value(peer_stop).map_err(|_| "Stop status")?,
                )?;
                exercise_ordinary_restart(app, root, &peer, relay_url, &peer_run).await?;
                save(root, "move-finish", &json!({"done":true}))?;
                cleanup(app)?;
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    cleanup(app)?;
    Err("Move remains unconfirmed; inspect persisted progress, never force Stopped".into())
}

// Exercise the existing pair Start/Restart entry points, not a fixture-only
// ledger shortcut. A stale Stop retry must not label a live successor stopped.
async fn exercise_ordinary_restart(
    app: &tauri::AppHandle,
    root: &Path,
    peer: &str,
    relay_url: &str,
    stopped_run: &str,
) -> Result<(), String> {
    let started =
        managed_agents::start_managed_agent_runtime(peer.into(), relay_url.into(), app.clone())?;
    let started_run = started
        .run_id
        .clone()
        .ok_or("ordinary Start missing generation")?;
    if started_run == stopped_run || started.pid.is_none() {
        return Err("ordinary Start did not create a fresh runtime".into());
    }
    if managed_agents::stop_managed_agent_runtime(
        peer.into(),
        relay_url.into(),
        Some(stopped_run.into()),
        app.clone(),
    )
    .is_ok()
    {
        return Err("old successful Stop retry misreported successor as stopped".into());
    }
    save(
        root,
        "ordinary-start-success.json",
        &serde_json::to_value(&started).map_err(|_| "Start status")?,
    )?;
    wait_live_run(app, peer, relay_url, &started_run).await?;
    let restarted = managed_agents::restart_managed_agent_runtime(
        peer.into(),
        relay_url.into(),
        Some(started_run.clone()),
        app.clone(),
    )?;
    let restarted_run = restarted
        .run_id
        .clone()
        .ok_or("ordinary Restart missing generation")?;
    if restarted_run == started_run || restarted.pid.is_none() {
        return Err("ordinary Restart did not create a fresh runtime".into());
    }
    wait_live_run(app, peer, relay_url, &restarted_run).await?;
    let stopped = managed_agents::stop_managed_agent_runtime(
        peer.into(),
        relay_url.into(),
        Some(restarted_run),
        app.clone(),
    )?;
    save(
        root,
        "ordinary-restart-success.json",
        &json!({
            "start_after_exact_stop": started, "restart": restarted,
            "stale_successful_stop_retry_rejected": true, "final_stop": stopped,
        }),
    )
}

// Spawn is explicitly not Ready. Exercise the happy path only after observing
// this exact new runtime on the relay; an immediate startup Stop may fail closed.
async fn wait_live_run(
    app: &tauri::AppHandle,
    agent: &str,
    relay_url: &str,
    run: &str,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let owner = state.signing_keys()?.public_key().to_hex();
    for _ in 0..60 {
        let runs = commands::get_presence_runs(
            state.clone(),
            owner.clone(),
            relay_url.into(),
            vec![agent.into()],
        )
        .await?;
        let now = nostr::Timestamp::now().as_secs();
        if runs.get(agent).is_some_and(|runs| {
            runs.iter()
                .any(|r| r.run == run && r.status != "offline" && r.expires_at > now)
        }) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err("ordinary restart generation not observed live".into())
}

async fn pulse(
    app: &tauri::AppHandle,
    owner: &str,
    registration: &Value,
    run: &str,
    seq: u64,
    socket: &mut buzz_ws_client_pkg::NostrWsConnection,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let event = commands::create_host_presence(
        state.clone(),
        owner.into(),
        registration.clone(),
        run.into(),
        seq,
        "online".into(),
    )
    .await?;
    let event = Event::from_json(event.to_string()).map_err(|_| "host pulse")?;
    let ack = socket
        .send_event(event)
        .await
        .map_err(|_| "fixture host pulse failed")?;
    if !ack.accepted {
        return Err(format!("fixture host pulse rejected: {}", ack.message));
    }
    Ok(())
}

fn cleanup(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|_| "runtime lock")?;
    for runtime in runtimes.values_mut() {
        let _ = managed_agents::terminate_exact_owned_group(&mut runtime.child);
        let _ = runtime.child.kill();
        let _ = runtime.child.wait();
    }
    println!("Tracer cleanup reaped tracked fixture roots only; cleanup is NOT certified Stop");
    Ok(())
}
