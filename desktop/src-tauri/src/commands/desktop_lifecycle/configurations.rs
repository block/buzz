//! Adapter only: configuration storage, async preflight and spawn belong to the shared launcher.
use super::*;
use buzz_core_pkg::desktop_lifecycle::{CatalogPage, Observation, RuntimeConfigurationSummary};
use managed_agents::runtime_configurations::{self as configurations, PreparedLaunch};

/// The shared launch plan and its pre-await lifecycle fences; never wire data.
pub(super) struct CapturedLaunch {
    pub(super) plan: PreparedLaunch,
    pub(super) resume: Option<managed_agents::remote_stop::ResumeTicket>,
    pub(super) generation: Option<String>,
}

fn record<R: tauri::Runtime>(
    app: &AppHandle<R>,
    agent: &str,
) -> Result<managed_agents::ManagedAgentRecord, String> {
    let state = app.state::<AppState>();
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    managed_agents::storage::load_managed_agents_for_launch(app)?
        .into_iter()
        .find(|r| r.pubkey == agent)
        .ok_or_else(|| "Agent is not provisioned on this Desktop".into())
}

pub(super) async fn prepare<R, F, Fut>(
    app: &AppHandle<R>,
    owner: &str,
    request: &Request,
    preflight: &F,
) -> Result<CapturedLaunch, Outcome>
where
    R: tauri::Runtime,
    F: Fn(Option<String>, bool) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let reference = request.configuration.as_ref().ok_or(Outcome::Ineligible)?;
    let mut captured = (|| -> Result<CapturedLaunch, String> {
        let state = app.state::<AppState>();
        let _transition = state
            .managed_agent_runtime_transition
            .lock()
            .map_err(|e| e.to_string())?;
        scope(app, &state, owner, &request.target.community)?;
        let record = record(app, &request.target.agent)?;
        let plan = configurations::prepare_for_app(
            app,
            &record,
            Some(reference),
            owner,
            &request.target.community,
        )?;
        let key = managed_agents::ManagedAgentRuntimeKey::new(
            &request.target.agent,
            &request.target.community,
        )?;
        // Only explicit Start can supersede an older Stop. Capture its existing
        // shared fence before await; probes and Restart receive no resume authority.
        let resume = if request.action == Action::Start {
            Some(managed_agents::remote_stop::capture_resume(
                app,
                &key,
                &request.target.community,
                owner,
            )?)
        } else {
            None
        };
        let generation = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?
            .get(&key)
            .map(|runtime| runtime.start_nonce.clone());
        Ok(CapturedLaunch {
            plan,
            resume,
            generation,
        })
    })()
    .map_err(|_| Outcome::Ineligible)?;
    configurations::preflight_with(
        &mut captured.plan,
        owner,
        &request.target.community,
        false,
        preflight,
    )
    .await
    .map_err(|_| Outcome::Ineligible)?;
    Ok(captured)
}

pub(super) fn revalidate<R: tauri::Runtime>(
    app: &AppHandle<R>,
    owner: &str,
    request: &Request,
    captured: &CapturedLaunch,
) -> Result<(), String> {
    let plan = &captured.plan;
    plan.require_preflight()?;
    plan.check_scope(Some(owner), &request.target.community)?;
    if plan.configuration().as_ref() != request.configuration.as_ref() {
        return Err("Prepared configuration does not match the request".into());
    }
    plan.revalidate(
        &record(app, &request.target.agent)?,
        &managed_agents::load_personas(app)?,
        &managed_agents::load_global_agent_config(app)?,
    )
}

pub(super) async fn catalog<R, F, Fut>(
    app: &AppHandle<R>,
    owner: &str,
    request: &Request,
    preflight: &F,
) -> Result<CatalogPage, String>
where
    R: tauri::Runtime,
    F: Fn(Option<String>, bool) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let record = record(app, &request.target.agent)?;
    let mut entries =
        configurations::catalog_for_app(app, &record, owner, &request.target.community)?
            .into_iter()
            .filter_map(|entry| {
                Some(RuntimeConfigurationSummary {
                    configuration: entry.configuration?,
                    name: entry.name,
                    host: entry.host,
                    runtime: entry.runtime,
                    model: entry.model?,
                    provider: entry.provider,
                    eligible: entry.eligible,
                })
            })
            .filter(|entry| entry.host == request.target.desktop)
            .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.configuration.id.cmp(&b.configuration.id));
    if entries.len() > 32
        || entries
            .windows(2)
            .any(|w| w[0].configuration.id == w[1].configuration.id)
    {
        return Err("Configuration catalog is invalid".into());
    }
    // An unknown/deleted cursor is not an authoritative empty tail.
    let index = match &request.cursor {
        Some(cursor) => {
            entries
                .iter()
                .position(|e| &e.configuration.id == cursor)
                .ok_or("Configuration catalog changed; refresh")?
                + 1
        }
        None => 0,
    };
    let next = entries
        .get(index + 1)
        .and_then(|_| entries.get(index))
        .map(|e| e.configuration.id.clone());
    let mut entry = entries.into_iter().nth(index);
    if let Some(entry) = &mut entry {
        entry.validate(&request.target.desktop)?;
        if entry.eligible {
            let probe = Request {
                action: Action::Preflight,
                configuration: Some(entry.configuration.clone()),
                cursor: None,
                ..request.clone()
            };
            entry.eligible = match prepare(app, owner, &probe, preflight).await {
                Ok(captured) => {
                    let state = app.state::<AppState>();
                    let _transition = state
                        .managed_agent_runtime_transition
                        .lock()
                        .map_err(|e| e.to_string())?;
                    scope(app, &state, owner, &request.target.community)?;
                    revalidate(app, owner, &probe, &captured).is_ok()
                }
                Err(_) => false,
            };
        }
    }
    Ok(CatalogPage { entry, next })
}

pub(super) fn observation(
    event: &Event,
    running_configuration: Option<RuntimeConfigurationRef>,
    catalog: Option<CatalogPage>,
) -> Observation {
    Observation {
        valid_until: event.created_at.as_secs().saturating_add(30),
        running_configuration,
        catalog,
    }
}

/// Read only the live process snapshot, never the saved next-launch selection.
pub(super) fn running(
    state: &AppState,
    request: &Request,
) -> Result<Option<RuntimeConfigurationRef>, String> {
    let key = managed_agents::ManagedAgentRuntimeKey::new(
        &request.target.agent,
        &request.target.community,
    )?;
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(runtimes
        .get(&key)
        .and_then(|runtime| runtime.spawn_config.runtime_configuration.clone()))
}
