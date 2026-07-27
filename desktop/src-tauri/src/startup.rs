//! Startup, wake, and readiness-transition wiring for the Daily Command Brief.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    ScheduledRunPresence, ScheduledRunStarter, ScheduledStartError, DEFAULT_SCHEDULE_ID,
};
use crate::command_brief::scheduler::LocalModelScheduler;
use crate::command_brief::sources::{
    ProductionSourceBackend, SourceBackend, TrustedLanSourceBackend,
};
use crate::command_brief::store::{open_command_brief_store, validate_command_brief_store_schema};
#[cfg(target_os = "macos")]
use crate::command_brief::wake::{MacWorkspaceWakeSource, WakeEventSource};
use crate::command_services::apple_inputs::{bundled_helper_identity, AppleBriefSelection};
use crate::commands::LmStudioReadiness;
use tokio_util::sync::CancellationToken;

// Evidence-backed Qwen advisers can exceed two minutes on Apple Silicon even
// with native reasoning disabled. Five minutes is the native client's bounded
// maximum and leaves enough time for one structured contribution to complete.
const MODEL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MODEL_READINESS_POLL_INTERVAL: Duration = Duration::from_secs(15);
const READINESS_DISPATCH_BACKOFF: Duration = Duration::from_millis(250);
const COMMAND_BRIEF_POLICY_REVISION: &str = "command-brief-policy-v1";
const IDENTITY_UNAVAILABLE: &str = "identity_unavailable";

mod scheduled_owner;
mod trusted_model;

pub(crate) use trusted_model::{
    admitted_model, model_readiness_for_schedule, trusted_lan_mode_enabled,
    trusted_model_readiness_observation, TrustedModelReadinessObservation,
};

#[cfg(test)]
use scheduled_owner::OrchestratorStarter;
use scheduled_owner::{
    active_owner_matches, active_owner_pubkey, current_runtime_admission_token,
    defer_identity_claim, identity_transition_token, process_scheduled_runtime,
    ScheduledRuntimeRequest,
};

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
struct ReadinessTransitionState {
    handled: Option<String>,
    in_flight: Option<String>,
    retry_not_before: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct CommandBriefReadinessTransitions {
    sources: BTreeMap<ReadinessSignalSource, ReadinessTransitionState>,
}

impl CommandBriefReadinessTransitions {
    fn try_begin(&mut self, source: ReadinessSignalSource, token: &str, now: Instant) -> bool {
        if !valid_readiness_token(token) {
            return false;
        }
        let transition = self.sources.entry(source).or_default();
        if transition.handled.as_deref() == Some(token)
            || transition.in_flight.is_some()
            || transition
                .retry_not_before
                .is_some_and(|deadline| deadline > now)
        {
            return false;
        }
        transition.in_flight = Some(token.to_string());
        true
    }

    fn complete(
        &mut self,
        source: ReadinessSignalSource,
        token: &str,
        handled: bool,
        retry_not_before: Option<Instant>,
    ) {
        let Some(transition) = self.sources.get_mut(&source) else {
            return;
        };
        if transition.in_flight.as_deref() != Some(token) {
            return;
        }
        transition.in_flight = None;
        if handled {
            transition.handled = Some(token.to_string());
            transition.retry_not_before = None;
        } else {
            transition.retry_not_before = retry_not_before;
        }
    }

    #[cfg(test)]
    pub(crate) fn is_handled(&self, source: ReadinessSignalSource, token: &str) -> bool {
        self.sources
            .get(&source)
            .and_then(|transition| transition.handled.as_deref())
            == Some(token)
    }

    #[cfg(test)]
    pub(crate) fn is_in_flight(&self, source: ReadinessSignalSource, token: &str) -> bool {
        self.sources
            .get(&source)
            .and_then(|transition| transition.in_flight.as_deref())
            == Some(token)
    }
}

#[cfg(test)]
pub(crate) fn observe_readiness_transition(
    transitions: &mut CommandBriefReadinessTransitions,
    source: ReadinessSignalSource,
    token: &str,
) -> bool {
    if !transitions.try_begin(source, token, Instant::now()) {
        return false;
    }
    transitions.complete(source, token, true, None);
    true
}

pub(crate) async fn dispatch_readiness_with_retry<Dispatch, DispatchFuture>(
    transitions: Arc<Mutex<CommandBriefReadinessTransitions>>,
    source: ReadinessSignalSource,
    token: String,
    backoff: Duration,
    mut dispatch: Dispatch,
) -> bool
where
    Dispatch: FnMut() -> DispatchFuture,
    DispatchFuture: Future<Output = Result<ScheduleRunOutcome, &'static str>>,
{
    for attempt in 0..2 {
        let began = transitions
            .lock()
            .is_ok_and(|mut state| state.try_begin(source, &token, Instant::now()));
        if !began {
            return false;
        }

        match dispatch().await {
            Ok(_) => {
                if let Ok(mut state) = transitions.lock() {
                    state.complete(source, &token, true, None);
                }
                return true;
            }
            Err(_) => {
                let retry_at = Instant::now().checked_add(backoff);
                if let Ok(mut state) = transitions.lock() {
                    state.complete(source, &token, false, retry_at);
                }
                if attempt == 0 {
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    false
}

pub(crate) fn notify_command_brief_readiness(
    app: &AppHandle,
    source: ReadinessSignalSource,
    basis: &[u8],
) {
    let token = readiness_transition_token(basis);
    let transitions = Arc::clone(
        app.state::<Arc<Mutex<CommandBriefReadinessTransitions>>>()
            .inner(),
    );
    let transition_app = app.clone();
    tauri::async_runtime::spawn(async move {
        dispatch_readiness_with_retry(
            transitions,
            source,
            token,
            READINESS_DISPATCH_BACKOFF,
            move || {
                let transition_app = transition_app.clone();
                async move {
                    run_command_brief_schedule(transition_app, ScheduleTrigger::Readiness)
                        .await
                        .map_err(|_| "command brief readiness dispatch unavailable")
                }
            },
        )
        .await;
    });
}

fn readiness_transition_token(basis: &[u8]) -> String {
    format!("signal:{}", hex::encode(Sha256::digest(basis)))
}

pub(crate) fn notify_lmstudio_readiness(
    app: &AppHandle,
    readiness: &Result<LmStudioReadiness, String>,
) {
    let observation = trusted_model_readiness_observation(readiness.clone());
    let token = observation.transition_token().to_string();
    let transitions = Arc::clone(
        app.state::<Arc<Mutex<CommandBriefReadinessTransitions>>>()
            .inner(),
    );
    let transition_app = app.clone();
    tauri::async_runtime::spawn(async move {
        dispatch_readiness_with_retry(
            transitions,
            ReadinessSignalSource::Model,
            token,
            READINESS_DISPATCH_BACKOFF,
            move || {
                let transition_app = transition_app.clone();
                let observation = observation.clone();
                async move {
                    run_command_brief_schedule_with_model_observation(
                        transition_app,
                        ScheduleTrigger::Readiness,
                        Some(observation),
                    )
                    .await
                    .map_err(|_| "command brief readiness dispatch unavailable")
                }
            },
        )
        .await;
    });
}

pub(crate) struct CommandBriefModelReadinessObserver {
    cancellation: CancellationToken,
    _task: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl CommandBriefModelReadinessObserver {
    pub(crate) fn stop(&self) {
        self.cancellation.cancel();
    }

    #[cfg(test)]
    pub(crate) async fn stop_and_wait(mut self) {
        self.stop();
        if let Some(task) = self._task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for CommandBriefModelReadinessObserver {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn start_model_readiness_observer_with_poll<Poll, PollFuture>(
    interval: Duration,
    mut poll: Poll,
) -> CommandBriefModelReadinessObserver
where
    Poll: FnMut() -> PollFuture + Send + 'static,
    PollFuture: Future<Output = ()> + Send + 'static,
{
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                () = task_cancellation.cancelled() => break,
                () = poll() => {}
            }
            tokio::select! {
                () = task_cancellation.cancelled() => break,
                () = tokio::time::sleep(interval) => {}
            }
        }
    });
    CommandBriefModelReadinessObserver {
        cancellation,
        _task: Some(task),
    }
}

pub(crate) fn start_model_readiness_observer(app: AppHandle) -> CommandBriefModelReadinessObserver {
    start_model_readiness_observer_with_poll(MODEL_READINESS_POLL_INTERVAL, move || {
        let app = app.clone();
        async move {
            let readiness = crate::commands::read_lmstudio_readiness(app.clone()).await;
            notify_lmstudio_readiness(&app, &readiness);
        }
    })
}

pub(crate) struct InstalledCommandBriefRuntime {
    owner_pubkey: String,
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
        owner_pubkey: &str,
        config: &RuntimeConfigIdentity,
    ) -> Option<Arc<InstalledCommandBriefRuntime>> {
        self.current
            .as_ref()
            .filter(|runtime| runtime.owner_pubkey == owner_pubkey && runtime.config == *config)
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

    pub(crate) fn all(&self) -> Vec<Arc<InstalledCommandBriefRuntime>> {
        self.current
            .iter()
            .chain(self.retired.iter())
            .cloned()
            .collect()
    }

    fn all_for_owner(&self, owner_pubkey: &str) -> Vec<Arc<InstalledCommandBriefRuntime>> {
        self.all()
            .into_iter()
            .filter(|runtime| runtime.owner_pubkey == owner_pubkey)
            .collect()
    }

    pub(crate) fn latest_status_and_history(
        &self,
        owner_pubkey: &str,
    ) -> Option<(
        crate::command_brief::types::BriefRunStatus,
        Vec<crate::command_brief::types::BriefRunStatus>,
    )> {
        self.all()
            .into_iter()
            .filter(|runtime| runtime.owner_pubkey == owner_pubkey)
            .filter_map(|runtime| runtime.orchestrator.latest_status_history_result())
            .max_by(|left, right| {
                left.0
                    .updated_at()
                    .cmp(right.0.updated_at())
                    .then_with(|| left.0.run_id().cmp(right.0.run_id()))
            })
            .map(|(status, history, _)| (status, history))
    }

    pub(crate) fn status(
        &self,
        owner_pubkey: &str,
        run_id: &str,
    ) -> Option<crate::command_brief::types::BriefRunStatus> {
        self.all()
            .into_iter()
            .filter(|runtime| runtime.owner_pubkey == owner_pubkey)
            .find_map(|runtime| runtime.orchestrator.status(run_id))
    }

    pub(crate) fn history_after(
        &self,
        owner_pubkey: &str,
        run_id: &str,
        cursor: Option<u64>,
    ) -> Vec<crate::command_brief::types::BriefRunStatus> {
        self.all()
            .into_iter()
            .filter(|runtime| runtime.owner_pubkey == owner_pubkey)
            .find_map(|runtime| {
                (runtime.orchestrator.status(run_id).is_some())
                    .then(|| runtime.orchestrator.history(run_id))
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|status| cursor.is_none_or(|cursor| status.sequence() > cursor))
            .collect()
    }

    pub(crate) fn cancel(&self, owner_pubkey: &str, run_id: &str) -> bool {
        self.all()
            .into_iter()
            .filter(|runtime| runtime.owner_pubkey == owner_pubkey)
            .find(|runtime| runtime.orchestrator.status(run_id).is_some())
            .is_some_and(|runtime| runtime.orchestrator.cancel(run_id))
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

/// Evaluate the protected schedule after startup, wake, or readiness refresh.
pub(crate) async fn run_command_brief_schedule(
    app: AppHandle,
    trigger: ScheduleTrigger,
) -> Result<ScheduleRunOutcome, String> {
    run_command_brief_schedule_with_model_observation(app, trigger, None).await
}

async fn run_command_brief_schedule_with_model_observation(
    app: AppHandle,
    trigger: ScheduleTrigger,
    supplied_model_readiness: Option<TrustedModelReadinessObservation>,
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
    let expected_owner_pubkey = match active_owner_pubkey(&state) {
        Some(owner_pubkey) => owner_pubkey,
        None => {
            return defer_identity_claim(&conn, &schedule, now, trigger, "identity:unavailable");
        }
    };

    let model_observation = model_readiness_for_schedule(supplied_model_readiness, || {
        crate::commands::read_lmstudio_readiness(app.clone())
    })
    .await;
    if !active_owner_matches(&state, &expected_owner_pubkey) {
        return defer_identity_claim(
            &conn,
            &schedule,
            now,
            trigger,
            &identity_transition_token(&expected_owner_pubkey),
        );
    }
    let model_transition_token = model_observation.transition_token();
    let model_readiness = match model_observation.readiness() {
        Some(readiness) => readiness,
        None => {
            let readiness = ReadinessSnapshot::deferred(
                DeferredReason::ModelUnavailable,
                model_transition_token,
            );
            return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
        }
    };
    let trusted_lan_mode = trusted_lan_mode_enabled(&app).await?;
    let Some(model) = admitted_model(model_readiness, trusted_lan_mode) else {
        let readiness =
            ReadinessSnapshot::deferred(DeferredReason::ModelUnavailable, model_transition_token);
        return process_due_schedule(&conn, &schedule, now, trigger, &readiness, &NoStarter);
    };

    let preflight = match production_preflight(&app, &schedule, &model, &expected_owner_pubkey)
        .await
    {
        Ok(preflight) => preflight,
        Err(IDENTITY_UNAVAILABLE) => {
            return defer_identity_claim(
                &conn,
                &schedule,
                now,
                trigger,
                &identity_transition_token(&expected_owner_pubkey),
            );
        }
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
    if !active_owner_matches(&state, &expected_owner_pubkey) {
        return defer_identity_claim(
            &conn,
            &schedule,
            now,
            trigger,
            &identity_transition_token(&expected_owner_pubkey),
        );
    }

    let watcher_app = app.clone();
    let watcher_owner = expected_owner_pubkey.clone();
    process_scheduled_runtime(
        ScheduledRuntimeRequest {
            state: &state,
            expected_owner_pubkey: &expected_owner_pubkey,
            conn,
            schedule: &schedule,
            now,
            trigger,
            store_path: &store_path,
        },
        ensure_production_runtime(&app, &preflight, store_path.clone(), &expected_owner_pubkey),
        move |started| {
            crate::commands::watch_command_brief_status(
                watcher_app.clone(),
                watcher_owner.clone(),
                started.to_string(),
                None,
            );
        },
    )
    .await
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
            "SELECT state,deferred_reason,transition_token
             FROM command_brief_schedule_claims WHERE idempotency_key=?1",
            [key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|_| "command brief schedule unavailable".to_string())?;
    let Some((state, deferred_reason, stored_token)) = claim else {
        return Ok(None);
    };
    if state == "deferred" && deferred_reason.as_deref() == Some("admission_unavailable") {
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

fn valid_readiness_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

async fn production_preflight(
    app: &AppHandle,
    schedule: &crate::command_brief::types::BriefSchedule,
    model: &str,
    expected_owner_pubkey: &str,
) -> Result<ProductionPreflight, &'static str> {
    let state = app.state::<AppState>();
    let owner_keys = state.signing_keys().map_err(|_| "identity_unavailable")?;
    let owner_pubkey = owner_keys.public_key().to_hex();
    if owner_pubkey != expected_owner_pubkey {
        return Err(IDENTITY_UNAVAILABLE);
    }
    let backend = production_preflight_source_backend(app, &Utc::now().to_rfc3339()).await?;
    if !active_owner_matches(&state, expected_owner_pubkey) {
        return Err(IDENTITY_UNAVAILABLE);
    }
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
    if !active_owner_matches(&state, expected_owner_pubkey) {
        return Err(IDENTITY_UNAVAILABLE);
    }
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
    if !active_owner_matches(&state, expected_owner_pubkey) {
        return Err(IDENTITY_UNAVAILABLE);
    }
    let trusted_lan_path = app
        .path()
        .app_config_dir()
        .map_err(|_| "runtime_config_unavailable")?
        .join("trusted-lan-sources.json");
    let trusted_lan_identity = tokio::task::spawn_blocking(move || {
        crate::command_services::trusted_lan::load_optional(&trusted_lan_path)
            .map_err(|_| "runtime_config_unavailable")
            .map(|config| {
                config.map_or_else(
                    || "trusted-lan-disabled".to_string(),
                    |config| config.configuration_identity(),
                )
            })
    })
    .await
    .map_err(|_| "runtime_config_unavailable")??;
    if !active_owner_matches(&state, expected_owner_pubkey) {
        return Err(IDENTITY_UNAVAILABLE);
    }
    let apple_identity = format!("{apple_config_id}:{helper_id}:{trusted_lan_identity}");
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

async fn production_preflight_source_backend(
    app: &AppHandle,
    observed_at: &str,
) -> Result<Arc<dyn SourceBackend>, &'static str> {
    let trusted_lan_path = app
        .path()
        .app_config_dir()
        .map_err(|_| "rag_unavailable")?
        .join("trusted-lan-sources.json");
    let observed_at = observed_at.to_string();
    let trusted = tokio::task::spawn_blocking(move || {
        crate::command_services::trusted_lan::load_optional(&trusted_lan_path)
            .map_err(|_| "rag_unavailable")
    })
    .await
    .map_err(|_| "rag_unavailable")??;
    if let Some(config) = trusted {
        return tokio::task::spawn_blocking(move || {
            TrustedLanSourceBackend::from_config(&config, &observed_at)
                .map(|backend| Arc::new(backend) as Arc<dyn SourceBackend>)
                .map_err(|_| "rag_unavailable")
        })
        .await
        .map_err(|_| "rag_unavailable")?;
    }
    ProductionSourceBackend::from_app(app.clone())
        .await
        .map(|backend| Arc::new(backend) as Arc<dyn SourceBackend>)
        .map_err(|_| "rag_unavailable")
}

async fn ensure_production_runtime(
    app: &AppHandle,
    preflight: &ProductionPreflight,
    store_path: std::path::PathBuf,
    expected_owner_pubkey: &str,
) -> Result<Arc<InstalledCommandBriefRuntime>, &'static str> {
    let state = app.state::<AppState>();
    if preflight.owner_keys.public_key().to_hex() != expected_owner_pubkey {
        return Err(IDENTITY_UNAVAILABLE);
    }
    let mut runtimes = state.command_brief_runtimes.write().await;
    if !active_owner_matches(&state, expected_owner_pubkey) {
        return Err(IDENTITY_UNAVAILABLE);
    }
    if let Some(runtime) = runtimes.matching(expected_owner_pubkey, &preflight.config) {
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
    if !active_owner_matches(&state, expected_owner_pubkey) {
        return Err(IDENTITY_UNAVAILABLE);
    }
    let generation = state
        .command_brief_runtime_generation
        .fetch_add(1, Ordering::AcqRel)
        + 1;
    let runtime = Arc::new(InstalledCommandBriefRuntime {
        owner_pubkey: preflight.owner_keys.public_key().to_hex(),
        config: preflight.config.clone(),
        generation,
        orchestrator,
    });
    runtimes.install(Arc::clone(&runtime));
    Ok(runtime)
}

pub(crate) fn command_brief_store_path() -> Result<std::path::PathBuf, String> {
    crate::managed_agents::nest_dir()
        .map(|nest| nest.join("command-brief").join("audit.db"))
        .ok_or_else(|| "command brief store unavailable".to_string())
}

const MANUAL_CO_REQUEST: &str =
    "Produce the current Daily Command Brief using only admitted local OFFICIAL sources.";

/// Start one fixed-policy manual brief without accepting renderer prompt,
/// persona, tool, model, endpoint, or classification input.
pub(crate) async fn start_manual_command_brief(
    app: AppHandle,
    expected_owner_pubkey: &str,
) -> Result<crate::command_brief::types::BriefRunStatus, String> {
    let store_path = command_brief_store_path()?;
    let conn = open_command_brief_store(&store_path)?;
    let timezone = current_macos_timezone()?;
    let schedule = load_or_create_schedule(&conn, &timezone, Utc::now().timestamp())?;
    let readiness = crate::commands::read_lmstudio_readiness(app.clone())
        .await
        .map_err(|_| "command brief model unavailable".to_string())?;
    let trusted_lan_mode = trusted_lan_mode_enabled(&app).await?;
    let model = admitted_model(&readiness, trusted_lan_mode)
        .ok_or_else(|| "command brief model unavailable".to_string())?;
    let preflight = production_preflight(&app, &schedule, &model, expected_owner_pubkey)
        .await
        .map_err(|code| {
            eprintln!("buzz-desktop: command brief preflight failed: {code}");
            "command brief runtime unavailable".to_string()
        })?;
    if preflight.owner_keys.public_key().to_hex() != expected_owner_pubkey {
        return Err("command brief identity unavailable".to_string());
    }
    let runtime = ensure_production_runtime(&app, &preflight, store_path, expected_owner_pubkey)
        .await
        .map_err(|code| {
            eprintln!("buzz-desktop: command brief runtime install failed: {code}");
            "command brief runtime unavailable".to_string()
        })?;
    let active_owner = app
        .state::<AppState>()
        .signing_keys()
        .map_err(|_| "command brief identity unavailable".to_string())?
        .public_key()
        .to_hex();
    if active_owner != expected_owner_pubkey {
        return Err("command brief identity unavailable".to_string());
    }
    let observed_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let request = CommandBriefRequest::new(DEFAULT_SCHEDULE_ID, MANUAL_CO_REQUEST, &observed_at)
        .map_err(|_| "command brief runtime unavailable".to_string())?;
    let run_id = runtime
        .orchestrator
        .start(request)
        .map_err(|_| "command brief runtime unavailable".to_string())?;
    runtime
        .orchestrator
        .status(&run_id)
        .ok_or_else(|| "command brief runtime unavailable".to_string())
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
        Err(ScheduledStartError::Unavailable)
    }

    fn presence(&self, _run_id: &str) -> ScheduledRunPresence {
        ScheduledRunPresence::Absent
    }
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
