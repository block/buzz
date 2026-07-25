//! Startup, wake, and readiness-transition wiring for the Daily Command Brief.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::command_brief::audit::{EncryptedBriefAudit, RelayBriefAuditPublisher};
use crate::command_brief::orchestrator::{
    BriefPersistence, CommandBriefOrchestrator, CommandBriefRequest, OrchestratorAdmissionState,
};
use crate::command_brief::schedule::{
    current_macos_timezone, due_local_date, idempotency_key, load_or_create_schedule,
    process_due_schedule, DeferredReason, ReadinessSnapshot, ScheduleRunOutcome, ScheduleTrigger,
    ScheduledRunPresence, ScheduledRunStarter, ScheduledStartError,
};
use crate::command_brief::scheduler::LocalModelScheduler;
use crate::command_brief::sources::{ProductionSourceBackend, SourceBackend};
use crate::command_brief::store::{
    has_spooled_terminal, open_command_brief_store, validate_command_brief_store_schema,
};
#[cfg(target_os = "macos")]
use crate::command_brief::wake::{MacWorkspaceWakeSource, WakeEventSource};
use crate::command_services::apple_inputs::{bundled_helper_identity, AppleBriefSelection};
use crate::commands::LmStudioReadinessState;
use tokio_util::sync::CancellationToken;

const SCHEDULED_CO_REQUEST: &str =
    "Prepare the Daily Command Brief from the admitted current local sources.";
const MODEL_TIMEOUT: Duration = Duration::from_secs(120);
const COMMAND_BRIEF_POLICY_REVISION: &str = "command-brief-policy-v1";

#[cfg(target_os = "macos")]
pub(crate) fn install_system_wake_source(app: AppHandle) -> Result<(), &'static str> {
    let callback_app = app.clone();
    let subscription = MacWorkspaceWakeSource.subscribe(Arc::new(move || {
        let wake_app = callback_app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = run_command_brief_schedule(wake_app, ScheduleTrigger::Wake).await;
        });
    }))?;
    let state = app.state::<AppState>();
    *state
        .command_brief_wake_subscription
        .lock()
        .map_err(|_| "wake_source_unavailable")? = Some(subscription);
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeConfigIdentity {
    token: String,
    model: String,
    capacity: u8,
}

impl RuntimeConfigIdentity {
    fn new(
        owner_pubkey: &str,
        model: &str,
        snapshot_id: &str,
        apple_config_id: &str,
        capacity: u8,
        policy_revision: &str,
    ) -> Result<Self, String> {
        if owner_pubkey.is_empty()
            || model.is_empty()
            || snapshot_id.is_empty()
            || apple_config_id.is_empty()
            || !matches!(capacity, 1 | 2)
            || policy_revision.is_empty()
        {
            return Err("command brief runtime configuration unavailable".to_string());
        }
        let basis = format!(
            "{owner_pubkey}\0{model}\0{snapshot_id}\0{apple_config_id}\0{capacity}\0{policy_revision}"
        );
        Ok(Self {
            token: format!("config:{}", hex::encode(Sha256::digest(basis.as_bytes()))),
            model: model.to_string(),
            capacity,
        })
    }

    #[cfg(test)]
    fn new_for_test(
        owner_pubkey: &str,
        model: &str,
        snapshot_id: &str,
        apple_config_id: &str,
        capacity: u8,
        policy_revision: &str,
    ) -> Self {
        Self::new(
            owner_pubkey,
            model,
            snapshot_id,
            apple_config_id,
            capacity,
            policy_revision,
        )
        .expect("runtime identity")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeReadiness {
    transition_token: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ReadinessSignalSource {
    Knowledge,
    Model,
}

#[derive(Default)]
pub(crate) struct CommandBriefReadinessTransitions {
    observed: BTreeMap<ReadinessSignalSource, String>,
}

pub(crate) fn observe_readiness_transition(
    transitions: &mut CommandBriefReadinessTransitions,
    source: ReadinessSignalSource,
    token: &str,
) -> bool {
    if !valid_readiness_token(token)
        || transitions
            .observed
            .get(&source)
            .is_some_and(|observed| observed == token)
    {
        return false;
    }
    transitions.observed.insert(source, token.to_string());
    true
}

pub(crate) fn notify_command_brief_readiness(
    app: &AppHandle,
    source: ReadinessSignalSource,
    basis: &[u8],
) {
    let token = format!("signal:{}", hex::encode(Sha256::digest(basis)));
    let transitions = app.state::<Mutex<CommandBriefReadinessTransitions>>();
    let changed = transitions.lock().is_ok_and(|mut transitions| {
        observe_readiness_transition(&mut transitions, source, &token)
    });
    if changed {
        let transition_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = run_command_brief_schedule(transition_app, ScheduleTrigger::Readiness).await;
        });
    }
}

pub(crate) struct InstalledCommandBriefRuntime {
    config: RuntimeConfigIdentity,
    generation: u64,
    orchestrator: CommandBriefOrchestrator,
}

#[derive(Default)]
pub(crate) struct CommandBriefRuntimeSet {
    current: Option<Arc<InstalledCommandBriefRuntime>>,
    retired: Vec<Arc<InstalledCommandBriefRuntime>>,
}

impl CommandBriefRuntimeSet {
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.current.is_none() && self.retired.is_empty()
    }

    fn matching(
        &self,
        config: &RuntimeConfigIdentity,
    ) -> Option<Arc<InstalledCommandBriefRuntime>> {
        self.current
            .as_ref()
            .filter(|runtime| runtime.config == *config)
            .cloned()
    }

    fn install(&mut self, runtime: Arc<InstalledCommandBriefRuntime>) {
        self.retired
            .retain(|candidate| candidate.orchestrator.has_nonterminal_runs());
        if let Some(previous) = self.current.replace(runtime) {
            if previous.orchestrator.has_nonterminal_runs() {
                self.retired.push(previous);
            }
        }
    }

    fn all(&self) -> Vec<Arc<InstalledCommandBriefRuntime>> {
        self.current
            .iter()
            .chain(self.retired.iter())
            .cloned()
            .collect()
    }
}

struct ProductionPreflight {
    config: RuntimeConfigIdentity,
    owner_keys: nostr::Keys,
}

impl RuntimeReadiness {
    fn ready(
        identity: &RuntimeConfigIdentity,
        generation: u64,
        admission: OrchestratorAdmissionState,
    ) -> Self {
        let admission = match admission {
            OrchestratorAdmissionState::Available {
                tracked_nonterminal,
                capacity,
            } => format!("available:{tracked_nonterminal}/{capacity}"),
            OrchestratorAdmissionState::Unavailable => "unavailable".to_string(),
        };
        Self::from_basis(&format!(
            "ready|{}|generation:{generation}|admission:{admission}",
            identity.token
        ))
    }

    fn unavailable(reason: &str, generation: u64) -> Self {
        Self::from_basis(&format!("unavailable|{reason}|generation:{generation}"))
    }

    fn from_basis(basis: &str) -> Self {
        Self {
            transition_token: format!("local:{}", hex::encode(Sha256::digest(basis.as_bytes()))),
        }
    }

    fn transition_token(&self) -> &str {
        &self.transition_token
    }
}

struct OrchestratorStarter<'a> {
    current: &'a CommandBriefOrchestrator,
    runtimes: &'a [Arc<InstalledCommandBriefRuntime>],
    store_path: &'a std::path::Path,
    owner_pubkey: &'a str,
}

impl ScheduledRunStarter for OrchestratorStarter<'_> {
    fn start_scheduled(
        &self,
        run_id: &str,
        _idempotency_key: &str,
        schedule_id: &str,
        observed_at: &str,
    ) -> Result<String, ScheduledStartError> {
        let request = CommandBriefRequest::new(schedule_id, SCHEDULED_CO_REQUEST, observed_at)
            .map_err(|_| ScheduledStartError)?;
        self.current
            .start_exact(run_id, request)
            .map_err(|_| ScheduledStartError)
    }

    fn presence(&self, run_id: &str) -> ScheduledRunPresence {
        if open_command_brief_store(self.store_path)
            .and_then(|conn| has_spooled_terminal(&conn, self.owner_pubkey, run_id))
            .unwrap_or(false)
        {
            ScheduledRunPresence::Terminal
        } else if self
            .runtimes
            .iter()
            .any(|runtime| runtime.orchestrator.status(run_id).is_some())
        {
            ScheduledRunPresence::Active
        } else {
            ScheduledRunPresence::Absent
        }
    }
}

/// Evaluate the protected schedule after startup, wake, or readiness refresh.
pub(crate) async fn run_command_brief_schedule(
    app: AppHandle,
    trigger: ScheduleTrigger,
) -> Result<ScheduleRunOutcome, String> {
    let store_path = command_brief_store_path()?;
    let conn = open_command_brief_store(&store_path)?;
    if trigger == ScheduleTrigger::Startup {
        validate_command_brief_store_schema(&conn)?;
    }
    let timezone = current_macos_timezone()?;
    let schedule = load_or_create_schedule(&conn, &timezone, Utc::now().timestamp())?;
    let now = Utc::now();
    let state = app.state::<AppState>();
    if let Some(outcome) = timer_claim_fast_path(&conn, &schedule, now, trigger, || {
        current_runtime_admission_token(&state)
    })? {
        return Ok(outcome);
    }

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

    let model_readiness = match crate::commands::read_lmstudio_readiness(app.clone()).await {
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

    let preflight = match production_preflight(&app, &schedule, &model).await {
        Ok(preflight) => preflight,
        Err(reason) => {
            let generation = state
                .command_brief_runtime_generation
                .load(Ordering::Acquire);
            let local = RuntimeReadiness::unavailable(reason, generation);
            let readiness = ReadinessSnapshot::deferred(
                DeferredReason::LocalStateUnavailable,
                local.transition_token(),
            );
            return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
        }
    };
    let runtime = match ensure_production_runtime(&app, &preflight, store_path.clone()).await {
        Ok(runtime) => runtime,
        Err(reason) => {
            let generation = state
                .command_brief_runtime_generation
                .load(Ordering::Acquire);
            let local = RuntimeReadiness::unavailable(reason, generation);
            let readiness = ReadinessSnapshot::deferred(
                DeferredReason::LocalStateUnavailable,
                local.transition_token(),
            );
            return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
        }
    };
    let runtimes = state.command_brief_runtimes.read().await.all();
    if runtimes.is_empty() {
        let readiness = ReadinessSnapshot::deferred(
            DeferredReason::LocalStateUnavailable,
            RuntimeReadiness::unavailable("runtime_unavailable", runtime.generation)
                .transition_token(),
        );
        return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
    }
    let admission = runtime.orchestrator.admission_state();
    let local = RuntimeReadiness::ready(&runtime.config, runtime.generation, admission);
    let readiness = match admission {
        OrchestratorAdmissionState::Available {
            tracked_nonterminal,
            capacity,
        } if tracked_nonterminal < capacity => ReadinessSnapshot::ready(local.transition_token()),
        OrchestratorAdmissionState::Available { .. } | OrchestratorAdmissionState::Unavailable => {
            ReadinessSnapshot::deferred(
                DeferredReason::LocalStateUnavailable,
                local.transition_token(),
            )
        }
    };
    let owner_pubkey = state.signing_keys()?.public_key().to_hex();
    process_due_schedule(
        &conn,
        &schedule,
        now,
        trigger,
        &readiness,
        &OrchestratorStarter {
            current: &runtime.orchestrator,
            runtimes: &runtimes,
            store_path: &store_path,
            owner_pubkey: &owner_pubkey,
        },
    )
}

fn timer_claim_fast_path(
    conn: &rusqlite::Connection,
    schedule: &crate::command_brief::types::BriefSchedule,
    now: chrono::DateTime<Utc>,
    trigger: ScheduleTrigger,
    current_admission_token: impl FnOnce() -> Option<String>,
) -> Result<Option<ScheduleRunOutcome>, String> {
    let Some(date) = due_local_date(schedule, now, trigger) else {
        return Ok(Some(ScheduleRunOutcome::NotDue));
    };
    if trigger != ScheduleTrigger::Timer {
        return Ok(None);
    }
    let key = idempotency_key(schedule, date);
    let claim = conn
        .query_row(
            "SELECT state,transition_token
             FROM command_brief_schedule_claims WHERE idempotency_key=?1",
            [key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|_| "command brief schedule unavailable".to_string())?;
    let Some((state, stored_token)) = claim else {
        return Ok(None);
    };
    if state == "deferred" {
        let stored_token =
            stored_token.ok_or_else(|| "command brief schedule unavailable".to_string())?;
        if current_admission_token()
            .as_deref()
            .is_some_and(|current| current != stored_token)
        {
            return Ok(None);
        }
    }
    Ok(Some(ScheduleRunOutcome::AlreadyClaimed))
}

fn current_runtime_admission_token(state: &AppState) -> Option<String> {
    let runtimes = state.command_brief_runtimes.try_read().ok()?;
    let runtime = runtimes.current.as_ref()?;
    Some(
        RuntimeReadiness::ready(
            &runtime.config,
            runtime.generation,
            runtime.orchestrator.admission_state(),
        )
        .transition_token,
    )
}

fn valid_readiness_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

async fn production_preflight(
    app: &AppHandle,
    schedule: &crate::command_brief::types::BriefSchedule,
    model: &str,
) -> Result<ProductionPreflight, &'static str> {
    let state = app.state::<AppState>();
    let owner_keys = state.signing_keys().map_err(|_| "identity_unavailable")?;
    let owner_pubkey = owner_keys.public_key().to_hex();
    let backend = ProductionSourceBackend::from_app(app.clone())
        .await
        .map_err(|_| "rag_unavailable")?;
    let snapshot = backend
        .verify_active_rag_snapshot()
        .map_err(|_| "rag_invalid")?;
    let expected = snapshot.clone();
    tokio::task::spawn_blocking(move || {
        backend.recheck_rag_snapshot(&expected, &CancellationToken::new())
    })
    .await
    .map_err(|_| "rag_unavailable")?
    .map_err(|_| "rag_unavailable")?;
    let config_path = app
        .path()
        .app_config_dir()
        .map_err(|_| "apple_config_unavailable")?
        .join("command-apple-inputs.json");
    let (apple_config_id, helper_id) = tokio::task::spawn_blocking(move || {
        let selection = AppleBriefSelection::load_protected(&config_path)
            .map_err(|_| "apple_config_unavailable")?;
        let helper = bundled_helper_identity()?;
        Ok::<_, &'static str>((selection.configuration_identity(), helper))
    })
    .await
    .map_err(|_| "apple_config_unavailable")??;
    let apple_identity = format!("{apple_config_id}:{helper_id}");
    let config = RuntimeConfigIdentity::new(
        &owner_pubkey,
        model,
        snapshot.snapshot_id(),
        &apple_identity,
        schedule.concurrency(),
        COMMAND_BRIEF_POLICY_REVISION,
    )
    .map_err(|_| "runtime_config_unavailable")?;
    Ok(ProductionPreflight { config, owner_keys })
}

async fn ensure_production_runtime(
    app: &AppHandle,
    preflight: &ProductionPreflight,
    store_path: std::path::PathBuf,
) -> Result<Arc<InstalledCommandBriefRuntime>, &'static str> {
    let state = app.state::<AppState>();
    let mut runtimes = state.command_brief_runtimes.write().await;
    if let Some(runtime) = runtimes.matching(&preflight.config) {
        return Ok(runtime);
    }
    let scheduler =
        LocalModelScheduler::new(preflight.config.capacity).map_err(|_| "scheduler_unavailable")?;
    let persistence: Arc<dyn BriefPersistence> = Arc::new(EncryptedBriefAudit::new(
        store_path,
        preflight.owner_keys.clone(),
        Arc::new(RelayBriefAuditPublisher::new(app.clone())),
    ));
    let orchestrator = CommandBriefOrchestrator::production(
        app.clone(),
        scheduler.clone(),
        &preflight.config.model,
        MODEL_TIMEOUT,
        persistence,
    )
    .await
    .map_err(|_| "orchestrator_unavailable")?;
    let generation = state
        .command_brief_runtime_generation
        .fetch_add(1, Ordering::AcqRel)
        + 1;
    let runtime = Arc::new(InstalledCommandBriefRuntime {
        config: preflight.config.clone(),
        generation,
        orchestrator,
    });
    runtimes.install(Arc::clone(&runtime));
    Ok(runtime)
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
        _run_id: &str,
        _idempotency_key: &str,
        _schedule_id: &str,
        _observed_at: &str,
    ) -> Result<String, ScheduledStartError> {
        Err(ScheduledStartError)
    }

    fn presence(&self, _run_id: &str) -> ScheduledRunPresence {
        ScheduledRunPresence::Absent
    }
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
