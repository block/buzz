//! Trusted Desktop lifecycle adapter. Historical projection never launches.
use super::{
    desktop_profiles::scope,
    desktop_stop::{local_id, owned_local},
};
use crate::{
    app_state::AppState,
    managed_agents::{self, placement, retention::open_retention_db},
};
use buzz_core_pkg::{
    desktop_lifecycle::{Action, Outcome, Request, ResultMessage, RuntimeConfigurationRef},
    desktop_stop::StopTarget,
};
use nostr::{Event, JsonUtil};
use rusqlite::{params, OptionalExtension};
use tauri::{AppHandle, Manager};

mod configurations;

#[tauri::command]
pub fn prepare_desktop_lifecycle(
    app: AppHandle,
    owner: String,
    community: String,
    desktop: String,
    agent: String,
    action: Action,
    observed: Option<String>,
    configuration: Option<RuntimeConfigurationRef>,
    cursor: Option<String>,
) -> Result<Event, String> {
    let state = app.state::<AppState>();
    let scope = scope(&app, &state, &owner, &community)?;
    let event = Request {
        target: StopTarget {
            v: 1,
            community,
            desktop,
            agent,
        },
        action,
        observed,
        configuration,
        cursor,
    }
    .sign(&scope.owner_keys)?;
    let conn = open_retention_db(&scope.db_path)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS desktop_lifecycle_outgoing (slot INTEGER PRIMARY KEY CHECK(slot=1),raw TEXT NOT NULL)").map_err(|e|e.to_string())?;
    conn.execute("INSERT INTO desktop_lifecycle_outgoing VALUES(1,?1) ON CONFLICT(slot) DO UPDATE SET raw=excluded.raw",[event.as_json()]).map_err(|e|e.to_string())?;
    Ok(event)
}

/// Authenticated projection batches only; no Stop/Restart command replay. Stops
/// caused by superseded placement reuse the ordinary local lifecycle owner.
#[tauri::command]
pub async fn observe_desktop_placement(
    app: AppHandle,
    owner: String,
    community: String,
    events: Vec<Event>,
    reconcile: bool,
) -> Result<(), String> {
    if events.len() > 256 {
        return Err("Too many placement events".into());
    }
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _transition = state
            .managed_agent_runtime_transition
            .lock()
            .map_err(|e| e.to_string())?;
        let scope = scope(&app, &state, &owner, &community)?;
        let mut conn = open_retention_db(&scope.db_path)?;
        let desktop = local_id(&mut conn, &scope)?;
        let mut agents = std::collections::BTreeSet::new();
        let mut projection = events.is_empty();
        for event in events {
            if event.kind.as_u16() as u32 == buzz_core_pkg::kind::KIND_DESKTOP_LIFECYCLE
                && matches!(
                    Request::read(&event, &scope.owner_keys, &community)?.action,
                    Action::Catalog | Action::Preflight
                )
            {
                continue;
            }
            projection = true;
            agents.insert(placement::observe(
                &conn,
                &event,
                &scope.owner_keys,
                &community,
            )?);
        }
        if !reconcile || !projection {
            return Ok(());
        }
        placement::schema(&conn)?;
        let mut query = conn
            .prepare("SELECT DISTINCT agent FROM desktop_placement")
            .map_err(|e| e.to_string())?;
        for row in query
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
        {
            agents.insert(row.map_err(|e| e.to_string())?);
        }
        for agent in agents {
            if placement::blocked(&conn, &agent, &desktop)?
                && owned_local(&app, &state, &owner, &agent)?
            {
                // Merely learning old Stop while no child exists performs no effect.
                if generation(&app, &state, &agent, &community).map_or(true, |g| g.is_some()) {
                    managed_agents::stop_pair_locked(agent, community.clone(), app.clone())?;
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("Placement task failed: {e}"))?
}

#[tauri::command]
pub fn read_desktop_placement(
    app: AppHandle,
    owner: String,
    community: String,
    agent: String,
) -> Result<Option<(String, String)>, String> {
    let state = app.state::<AppState>();
    let scope = scope(&app, &state, &owner, &community)?;
    placement::latest_start(&open_retention_db(&scope.db_path)?, &agent)
}

fn generation<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    agent: &str,
    community: &str,
) -> Result<Option<String>, String> {
    let key = managed_agents::ManagedAgentRuntimeKey::new(agent, community)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some(runtime) = runtimes.get_mut(&key) {
        if runtime
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_none()
        {
            return Ok(Some(runtime.start_nonce.clone()));
        }
    }
    drop(runtimes);
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let records = managed_agents::storage::load_agent_store(app)?;
    let legacy = records
        .iter()
        .find(|r| r.pubkey == agent)
        .and_then(|r| r.runtime_pid);
    let dir = managed_agents::managed_agents_base_dir(app)?.join("agent-pids");
    // A receipt may represent a surviving untracked child. Never turn a missing
    // in-memory handle, unreadable receipt, or legacy live PID into Stopped.
    if legacy.is_some_and(managed_agents::process_is_running)
        || dir
            .join(format!("{}.json", key.runtime_id()))
            .try_exists()
            .map_err(|e| e.to_string())?
        || dir
            .join(format!("{agent}.pid"))
            .try_exists()
            .map_err(|e| e.to_string())?
    {
        return Err("Local process state is untracked; use ordinary Desktop Stop".into());
    }
    Ok(None)
}

fn status<R: tauri::Runtime>(
    app: &AppHandle<R>,
    conn: &rusqlite::Connection,
    state: &AppState,
    event: &Event,
    request: &Request,
) -> Result<Outcome, String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS desktop_status_generation (id TEXT PRIMARY KEY,nonce TEXT NOT NULL,observed INTEGER NOT NULL)").map_err(|e|e.to_string())?;
    let Some(nonce) = generation(app, state, &request.target.agent, &request.target.community)?
    else {
        return Ok(Outcome::Stopped);
    };
    conn.execute(
        "INSERT OR REPLACE INTO desktop_status_generation VALUES(?1,?2,?3)",
        params![event.id.to_hex(), nonce, nostr::Timestamp::now().as_secs()],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM desktop_status_generation WHERE rowid NOT IN (SELECT rowid FROM desktop_status_generation ORDER BY rowid DESC LIMIT 256)",[]).map_err(|e|e.to_string())?;
    Ok(Outcome::Running)
}
fn current_observation<R: tauri::Runtime>(
    app: &AppHandle<R>,
    conn: &rusqlite::Connection,
    state: &AppState,
    request: &Request,
) -> Result<bool, String> {
    let Some(id) = request.observed.as_deref() else {
        return Ok(false);
    };
    // A request cannot create this local record; only a real Status observation can.
    conn.execute_batch("CREATE TABLE IF NOT EXISTS desktop_status_generation (id TEXT PRIMARY KEY,nonce TEXT NOT NULL,observed INTEGER NOT NULL)").map_err(|e|e.to_string())?;
    let saved: Option<(String, u64)> = conn
        .query_row(
            "SELECT nonce,observed FROM desktop_status_generation WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(match saved {
        Some((nonce, stamp)) if nostr::Timestamp::now().as_secs().saturating_sub(stamp) <= 30 => {
            generation(app, state, &request.target.agent, &request.target.community)?.as_deref()
                == Some(nonce.as_str())
        }
        _ => false,
    })
}

#[tauri::command]
pub async fn receive_desktop_lifecycle(
    app: AppHandle,
    owner: String,
    community: String,
    event: Event,
) -> Result<Option<Event>, String> {
    let preflight_app = app.clone();
    receive_desktop_lifecycle_with(app, owner, community, event, move |model, allow| {
        let app = preflight_app.clone();
        async move {
            #[cfg(feature = "mesh-llm")]
            {
                crate::commands::ensure_relay_mesh_for_record(&app, model.as_deref(), allow).await
            }
            #[cfg(not(feature = "mesh-llm"))]
            {
                let _ = (app, model, allow);
                Ok(())
            }
        }
    })
    .await
}

// Production receiver with only ordinary provider I/O replaceable in isolated tests.
pub(crate) async fn receive_desktop_lifecycle_with<R, F, Fut>(
    app: AppHandle<R>,
    owner: String,
    community: String,
    event: Event,
    preflight: F,
) -> Result<Option<Event>, String>
where
    R: tauri::Runtime,
    F: Fn(Option<String>, bool) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    // Authenticate and fence the destination before reading any configuration or
    // doing ordinary async preflight. Never hold the transition lock across await.
    let request = {
        let state = app.state::<AppState>();
        let scope = scope(&app, &state, &owner, &community)?;
        let request = Request::read(&event, &scope.owner_keys, &community)?;
        let mut conn = open_retention_db(&scope.db_path)?;
        if local_id(&mut conn, &scope)? != request.target.desktop {
            return Ok(None);
        }
        if let Some(saved) = placement::saved(&conn, &event.id.to_hex())? {
            ResultMessage::read(&saved, &scope.owner_keys, &event, &community)?;
            return Ok(Some(saved));
        }
        request
    };
    let owned = owned_local(
        &app,
        &app.state::<AppState>(),
        &owner,
        &request.target.agent,
    )?;
    let fresh = nostr::Timestamp::now().as_secs() < event.created_at.as_secs().saturating_add(30);
    let mut prepared = if owned
        && fresh
        && matches!(
            request.action,
            Action::Start | Action::Restart | Action::Preflight
        ) {
        configurations::prepare(&app, &owner, &request, &preflight).await
    } else {
        Err(Outcome::Ineligible)
    };
    if let Ok(captured) = &mut prepared {
        captured
            .plan
            .expire_at(event.created_at.as_secs().saturating_add(30));
    }
    let catalog = if owned && fresh && request.action == Action::Catalog {
        configurations::catalog(&app, &owner, &request, &preflight).await
    } else {
        Err("Catalog is unavailable".into())
    };
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _transition = state
            .managed_agent_runtime_transition
            .lock()
            .map_err(|e| e.to_string())?;
        let scope = scope(&app, &state, &owner, &community)?;
        let request = Request::read(&event, &scope.owner_keys, &community)?;
        let mut conn = open_retention_db(&scope.db_path)?;
        let desktop = local_id(&mut conn, &scope)?;
        let owned = owned_local(&app, &state, &owner, &request.target.agent)?;
        placement::receive(
            &mut conn,
            &event,
            &scope.owner_keys,
            &community,
            &desktop,
            owned,
            |conn, request| {
                let (outcome, page) = match request.action {
                    Action::Status => (status(&app, conn, &state, &event, request)?, None),
                    Action::Catalog => match catalog {
                        Ok(page) => (Outcome::Ready, Some(page)),
                        Err(_) => (Outcome::Ineligible, None),
                    },
                    Action::Preflight => (
                        match prepared.as_ref() {
                            Ok(plan)
                                if configurations::revalidate(&app, &owner, request, plan)
                                    .is_ok() =>
                            {
                                Outcome::Ready
                            }
                            _ => Outcome::Ineligible,
                        },
                        None,
                    ),
                    _ => (
                        execute(&app, &state, conn, &owner, request, prepared.as_ref())?,
                        None,
                    ),
                };
                let running =
                    if matches!(outcome, Outcome::Running | Outcome::DifferentConfiguration) {
                        configurations::running(&state, request)?
                    } else {
                        None
                    };
                Ok((
                    outcome,
                    Some(configurations::observation(&event, running, page)),
                ))
            },
        )
    })
    .await
    .map_err(|e| format!("Desktop lifecycle task failed: {e}"))?
}

fn execute<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    conn: &rusqlite::Connection,
    owner: &str,
    request: &Request,
    prepared: Result<&configurations::CapturedLaunch, &Outcome>,
) -> Result<Outcome, String> {
    let target = &request.target;
    if request.configuration.is_none() {
        // Old requests remain readable for placement history, never implicit launches.
        return Ok(Outcome::Ineligible);
    }
    if request.action == Action::Restart && !current_observation(app, conn, state, request)? {
        return Ok(Outcome::Unknown);
    }
    if request.action == Action::Start
        && generation(app, state, &target.agent, &target.community)?.is_some()
    {
        return Ok(
            if configurations::running(state, request)? == request.configuration {
                Outcome::Running
            } else {
                Outcome::DifferentConfiguration
            },
        );
    }
    let plan = match prepared {
        Ok(plan) => plan,
        Err(outcome) => return Ok(*outcome),
    };
    if configurations::revalidate(app, owner, request, plan).is_err() {
        return Ok(Outcome::Ineligible);
    }
    // Shared admission validates the captured Stop fence and generation BEFORE
    // destructive Restart. The transition lock spans that Stop, launch and receipt.
    if placement::blocked(conn, &target.agent, &target.desktop)? {
        return Ok(Outcome::Unknown);
    }
    match managed_agents::start_pair_captured_locked(
        target.agent.clone(),
        target.community.clone(),
        true,
        None,
        plan.resume.as_ref(),
        &plan.plan,
        false, // Explicit wire reference, not the destination's next selection.
        request.action == Action::Restart,
        Some(&plan.generation),
        app.clone(),
    ) {
        Ok(status)
            if status.running_configuration == request.configuration
                && !matches!(
                    status.lifecycle,
                    managed_agents::ManagedAgentRuntimeLifecycle::Failed
                        | managed_agents::ManagedAgentRuntimeLifecycle::Stopped
                ) =>
        {
            Ok(Outcome::Running)
        }
        Ok(_) | Err(_) => Ok(Outcome::Failed),
    }
}

#[tauri::command]
pub fn read_desktop_lifecycle_results(
    app: AppHandle,
    owner: String,
    community: String,
    request: Event,
    events: Vec<Event>,
) -> Result<Option<ResultMessage>, String> {
    let state = app.state::<AppState>();
    let scope = scope(&app, &state, &owner, &community)?;
    Request::read(&request, &scope.owner_keys, &community)?;
    if events.len() > 16 {
        return Err("Too many lifecycle results".into());
    }
    let mut outcome = None;
    for event in events {
        let result = ResultMessage::read(&event, &scope.owner_keys, &request, &community)?;
        if result.outcome != Outcome::Unknown {
            outcome = Some(result);
        }
    }
    Ok(outcome)
}
