//! Trusted Desktop lifecycle adapter. Historical projection never launches.
use super::{
    desktop_profiles::scope,
    desktop_stop::{local_id, owned_local},
};
use crate::{
    app_state::AppState,
    managed_agents::{self, broker_launch, placement, retention::open_retention_db},
};
use buzz_core_pkg::{
    desktop_lifecycle::{Action, Outcome, Request, ResultMessage},
    desktop_stop::StopTarget,
};
use nostr::{Event, JsonUtil};
use rusqlite::{params, OptionalExtension};
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn prepare_desktop_lifecycle(
    app: AppHandle,
    owner: String,
    community: String,
    desktop: String,
    agent: String,
    action: Action,
    observed: Option<String>,
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
        for event in events {
            agents.insert(placement::observe(
                &conn,
                &event,
                &scope.owner_keys,
                &community,
            )?);
        }
        if !reconcile {
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

fn generation(
    app: &AppHandle,
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
    let records = managed_agents::load_managed_agents(app)?;
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

fn status(
    app: &AppHandle,
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
fn current_observation(
    app: &AppHandle,
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
                if request.action == Action::Status {
                    status(&app, conn, &state, &event, request)
                } else {
                    execute(&app, &state, conn, &owner, request)
                }
            },
        )
    })
    .await
    .map_err(|e| format!("Desktop lifecycle task failed: {e}"))?
}

fn execute(
    app: &AppHandle,
    state: &AppState,
    conn: &rusqlite::Connection,
    owner: &str,
    request: &Request,
) -> Result<Outcome, String> {
    let target = &request.target;
    if request.action == Action::Restart && !current_observation(app, conn, state, request)? {
        return Ok(Outcome::Unknown);
    }
    if request.action == Action::Start
        && generation(app, state, &target.agent, &target.community)?.is_some()
    {
        return Ok(Outcome::Running);
    }
    let record = {
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        managed_agents::load_managed_agents(app)?
            .into_iter()
            .find(|r| r.pubkey == target.agent)
            .ok_or("Agent is not provisioned on this Desktop")?
    };
    // Provision before destructive Restart Stop. No supported session: leave the
    // existing child running and return the precise non-secret missing capability.
    let broker = match broker_launch::provision(
        broker_launch::LaunchScope {
            owner,
            community: &target.community,
            agent: &target.agent,
        },
        &record,
    ) {
        Ok(b) => b,
        Err(outcome) => return Ok(outcome),
    };
    if request.action == Action::Restart
        && managed_agents::stop_pair_locked(
            target.agent.clone(),
            target.community.clone(),
            app.clone(),
        )
        .is_err()
    {
        return Ok(Outcome::Failed);
    }
    // Existing transition lock spans final intent check, ordinary launch and registration.
    if placement::blocked(conn, &target.agent, &target.desktop)? {
        return Ok(Outcome::Unknown);
    }
    match managed_agents::start_pair_locked(
        target.agent.clone(),
        target.community.clone(),
        true,
        None,
        true,
        Some(&broker),
        app.clone(),
    ) {
        Ok(_) => Ok(Outcome::Running),
        Err(_) => Ok(Outcome::Failed),
    }
}

#[tauri::command]
pub fn read_desktop_lifecycle_results(
    app: AppHandle,
    owner: String,
    community: String,
    request: Event,
    events: Vec<Event>,
) -> Result<Outcome, String> {
    let state = app.state::<AppState>();
    let scope = scope(&app, &state, &owner, &community)?;
    Request::read(&request, &scope.owner_keys, &community)?;
    if events.len() > 16 {
        return Err("Too many lifecycle results".into());
    }
    let mut outcome = Outcome::Unknown;
    for event in events {
        let result = ResultMessage::read(&event, &scope.owner_keys, &request, &community)?;
        if result.outcome != Outcome::Unknown {
            outcome = result.outcome;
        }
    }
    Ok(outcome)
}
