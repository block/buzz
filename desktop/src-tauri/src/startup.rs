//! Startup, wake, and readiness-transition wiring for the Daily Command Brief.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::command_brief::audit::{EncryptedBriefAudit, RelayBriefAuditPublisher};
use crate::command_brief::orchestrator::{
    BriefPersistence, CommandBriefOrchestrator, CommandBriefRequest,
};
use crate::command_brief::schedule::{
    current_macos_timezone, due_local_date, load_or_create_schedule, process_due_schedule,
    DeferredReason, ReadinessSnapshot, ScheduleRunOutcome, ScheduleTrigger, ScheduledRunStarter,
    ScheduledStartError,
};
use crate::command_brief::scheduler::LocalModelScheduler;
use crate::command_brief::store::open_command_brief_store;
use crate::commands::LmStudioReadinessState;

const SCHEDULED_CO_REQUEST: &str =
    "Prepare the Daily Command Brief from the admitted current local sources.";
const MODEL_TIMEOUT: Duration = Duration::from_secs(120);

struct OrchestratorStarter<'a> {
    orchestrator: &'a CommandBriefOrchestrator,
}

impl ScheduledRunStarter for OrchestratorStarter<'_> {
    fn start_scheduled(
        &self,
        schedule_id: &str,
        observed_at: &str,
    ) -> Result<String, ScheduledStartError> {
        let request = CommandBriefRequest::new(schedule_id, SCHEDULED_CO_REQUEST, observed_at)
            .map_err(|_| ScheduledStartError)?;
        self.orchestrator
            .start(request)
            .map_err(|_| ScheduledStartError)
    }
}

/// Evaluate the protected schedule after startup, wake, or readiness refresh.
pub(crate) async fn run_command_brief_schedule(
    app: AppHandle,
    trigger: ScheduleTrigger,
) -> Result<ScheduleRunOutcome, String> {
    let store_path = command_brief_store_path()?;
    let conn = open_command_brief_store(&store_path)?;
    let timezone = current_macos_timezone()?;
    let schedule = load_or_create_schedule(&conn, &timezone, Utc::now().timestamp())?;
    let now = Utc::now();
    if due_local_date(&schedule, now, trigger).is_none() {
        return Ok(ScheduleRunOutcome::NotDue);
    }

    let state = app.state::<AppState>();
    if state.keyring_locked.load(Ordering::Acquire)
        || state.identity_lost.load(Ordering::Acquire)
        || state.reset_failed.load(Ordering::Acquire)
    {
        let readiness = ReadinessSnapshot::deferred(
            DeferredReason::IdentityLocked,
            "identity:locked|model:unknown|local:unknown",
        );
        return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
    }

    let model_readiness = match crate::commands::get_lmstudio_readiness(app.clone()).await {
        Ok(readiness) => readiness,
        Err(_) => {
            let readiness = ReadinessSnapshot::deferred(
                DeferredReason::ModelUnavailable,
                "identity:ready|model:probe_failed|local:unknown",
            );
            return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
        }
    };
    if model_readiness.status != LmStudioReadinessState::Ready {
        let token = format!(
            "identity:ready|model:{:?}|local:unknown",
            model_readiness.status
        );
        let readiness = ReadinessSnapshot::deferred(DeferredReason::ModelUnavailable, &token);
        return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
    }
    let model = model_readiness
        .configured_model
        .or_else(|| model_readiness.loaded_models.into_iter().next())
        .ok_or_else(|| "command brief model unavailable".to_string())?;

    if ensure_production_orchestrator(&app, &schedule, &model, store_path)
        .await
        .is_err()
    {
        let readiness = ReadinessSnapshot::deferred(
            DeferredReason::LocalStateUnavailable,
            "identity:ready|model:ready|local:unavailable",
        );
        return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
    }
    let orchestrator = state
        .command_brief_orchestrator
        .read()
        .await
        .clone()
        .ok_or_else(|| "command brief orchestrator unavailable".to_string())?;
    let readiness = ReadinessSnapshot::ready("identity:ready|model:ready|local:ready");
    process_due_schedule(
        &conn,
        &schedule,
        now,
        trigger,
        &readiness,
        &OrchestratorStarter {
            orchestrator: &orchestrator,
        },
    )
}

async fn ensure_production_orchestrator(
    app: &AppHandle,
    schedule: &crate::command_brief::types::BriefSchedule,
    model: &str,
    store_path: std::path::PathBuf,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    // Startup, resume, and readiness refreshes can overlap. Hold the install
    // guard through construction so the app-owned scheduler and orchestrator
    // are always installed as one coherent pair.
    let mut slot = state.command_brief_orchestrator.write().await;
    if slot.is_some() {
        return Ok(());
    }
    let scheduler = LocalModelScheduler::new(schedule.concurrency())
        .map_err(|_| "command brief scheduler unavailable".to_string())?;
    {
        let mut installed = state
            .command_brief_scheduler
            .write()
            .map_err(|_| "command brief scheduler unavailable".to_string())?;
        *installed = scheduler.clone();
    }
    let owner_keys = state
        .keys
        .lock()
        .map_err(|_| "command brief identity unavailable".to_string())?
        .clone();
    let persistence: Arc<dyn BriefPersistence> = Arc::new(EncryptedBriefAudit::new(
        store_path,
        owner_keys,
        Arc::new(RelayBriefAuditPublisher::new(app.clone())),
    ));
    let orchestrator = CommandBriefOrchestrator::production(
        app.clone(),
        scheduler,
        model,
        MODEL_TIMEOUT,
        persistence,
    )
    .await
    .map_err(|_| "command brief local state unavailable".to_string())?;
    *slot = Some(orchestrator);
    Ok(())
}

fn command_brief_store_path() -> Result<std::path::PathBuf, String> {
    crate::managed_agents::nest_dir()
        .map(|nest| nest.join("command-brief").join("audit.db"))
        .ok_or_else(|| "command brief store unavailable".to_string())
}

struct NoStarter;

impl ScheduledRunStarter for NoStarter {
    fn start_scheduled(
        &self,
        _schedule_id: &str,
        _observed_at: &str,
    ) -> Result<String, ScheduledStartError> {
        Err(ScheduledStartError)
    }
}
