use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::Utc;

use super::{
    AppState, CommandBriefOrchestrator, CommandBriefRequest, InstalledCommandBriefRuntime,
    NoStarter, OrchestratorAdmissionState, RuntimeReadiness, IDENTITY_UNAVAILABLE,
};
use crate::command_brief::schedule::{
    process_due_schedule, DeferredReason, ReadinessSnapshot, ScheduleRunOutcome, ScheduleTrigger,
    ScheduledRunPresence, ScheduledRunStarter, ScheduledStartError,
};
use crate::command_brief::store::{has_spooled_terminal, open_command_brief_store};

const SCHEDULED_CO_REQUEST: &str =
    "Prepare the Daily Command Brief from the admitted current local sources.";

pub(super) struct OrchestratorStarter<'a> {
    pub(super) state: &'a AppState,
    pub(super) owner_pubkey: &'a str,
    pub(super) current: &'a CommandBriefOrchestrator,
    pub(super) runtimes: &'a [Arc<InstalledCommandBriefRuntime>],
    pub(super) store_path: &'a std::path::Path,
    pub(super) on_started: &'a dyn Fn(&str),
}

pub(super) struct ScheduledRuntimeRequest<'a> {
    pub(super) state: &'a AppState,
    pub(super) expected_owner_pubkey: &'a str,
    pub(super) conn: rusqlite::Connection,
    pub(super) schedule: &'a crate::command_brief::types::BriefSchedule,
    pub(super) now: chrono::DateTime<Utc>,
    pub(super) trigger: ScheduleTrigger,
    pub(super) store_path: &'a std::path::Path,
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
            .map_err(|_| ScheduledStartError::Unavailable)?;
        if !active_owner_matches(self.state, self.owner_pubkey) {
            return Err(ScheduledStartError::IdentityUnavailable);
        }
        let started = self
            .current
            .start_exact(run_id, request)
            .map_err(|error| match error {
                crate::command_brief::orchestrator::OrchestratorStartError::AdmissionUnavailable => {
                    ScheduledStartError::AdmissionUnavailable
                }
                crate::command_brief::orchestrator::OrchestratorStartError::Rejected => {
                    ScheduledStartError::Unavailable
                }
            })?;
        (self.on_started)(&started);
        Ok(started)
    }

    fn presence(&self, run_id: &str) -> ScheduledRunPresence {
        if !active_owner_matches(self.state, self.owner_pubkey) {
            ScheduledRunPresence::IdentityUnavailable
        } else if open_command_brief_store(self.store_path)
            .and_then(|conn| has_spooled_terminal(&conn, self.owner_pubkey, run_id))
            .unwrap_or(false)
        {
            ScheduledRunPresence::Terminal
        } else if self.runtimes.iter().any(|runtime| {
            runtime.owner_pubkey == self.owner_pubkey
                && runtime.orchestrator.status(run_id).is_some()
        }) {
            ScheduledRunPresence::Active
        } else {
            ScheduledRunPresence::Absent
        }
    }
}

pub(super) async fn process_scheduled_runtime<F, W>(
    request: ScheduledRuntimeRequest<'_>,
    runtime_future: F,
    on_started: W,
) -> Result<ScheduleRunOutcome, String>
where
    F: Future<Output = Result<Arc<InstalledCommandBriefRuntime>, &'static str>>,
    W: Fn(&str),
{
    let ScheduledRuntimeRequest {
        state,
        expected_owner_pubkey,
        conn,
        schedule,
        now,
        trigger,
        store_path,
    } = request;
    let runtime_result = runtime_future.await;
    if !active_owner_matches(state, expected_owner_pubkey) {
        return defer_identity_claim(
            &conn,
            schedule,
            now,
            trigger,
            &identity_transition_token(expected_owner_pubkey),
        );
    }
    let runtime = match runtime_result {
        Ok(runtime) if runtime.owner_pubkey == expected_owner_pubkey => runtime,
        Ok(_) | Err(IDENTITY_UNAVAILABLE) => {
            return defer_identity_claim(
                &conn,
                schedule,
                now,
                trigger,
                &identity_transition_token(expected_owner_pubkey),
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
            return process_due_schedule(&conn, schedule, now, trigger, &readiness, &NoStarter);
        }
    };
    let runtime_guard = state.command_brief_runtimes.read().await;
    if !active_owner_matches(state, expected_owner_pubkey) {
        drop(runtime_guard);
        return defer_identity_claim(
            &conn,
            schedule,
            now,
            trigger,
            &identity_transition_token(expected_owner_pubkey),
        );
    }
    let runtimes = runtime_guard.all_for_owner(expected_owner_pubkey);
    drop(runtime_guard);
    if runtimes.is_empty() {
        let readiness = ReadinessSnapshot::deferred(
            DeferredReason::LocalStateUnavailable,
            RuntimeReadiness::unavailable("runtime_unavailable", runtime.generation)
                .transition_token(),
        );
        return process_due_schedule(&conn, schedule, now, trigger, &readiness, &NoStarter);
    }
    let admission = runtime.orchestrator.admission_state();
    if !active_owner_matches(state, expected_owner_pubkey) {
        return defer_identity_claim(
            &conn,
            schedule,
            now,
            trigger,
            &identity_transition_token(expected_owner_pubkey),
        );
    }
    let local = RuntimeReadiness::ready(&runtime.config, runtime.generation, admission);
    let readiness = match admission {
        OrchestratorAdmissionState::Available {
            tracked_nonterminal,
            capacity,
        } if tracked_nonterminal < capacity => ReadinessSnapshot::ready(local.transition_token()),
        OrchestratorAdmissionState::Available { .. } | OrchestratorAdmissionState::Unavailable => {
            ReadinessSnapshot::deferred(
                DeferredReason::AdmissionUnavailable,
                local.transition_token(),
            )
        }
    };
    process_due_schedule(
        &conn,
        schedule,
        now,
        trigger,
        &readiness,
        &OrchestratorStarter {
            state,
            owner_pubkey: expected_owner_pubkey,
            current: &runtime.orchestrator,
            runtimes: &runtimes,
            store_path,
            on_started: &on_started,
        },
    )
}

pub(super) fn active_owner_pubkey(state: &AppState) -> Option<String> {
    state
        .signing_keys()
        .ok()
        .map(|keys| keys.public_key().to_hex())
}

pub(super) fn active_owner_matches(state: &AppState, expected_owner_pubkey: &str) -> bool {
    active_owner_pubkey(state).as_deref() == Some(expected_owner_pubkey)
}

pub(super) fn identity_transition_token(expected_owner_pubkey: &str) -> String {
    RuntimeReadiness::from_basis(&format!(
        "identity_changed|expected:{expected_owner_pubkey}"
    ))
    .transition_token
}

pub(super) fn defer_identity_claim(
    conn: &rusqlite::Connection,
    schedule: &crate::command_brief::types::BriefSchedule,
    now: chrono::DateTime<Utc>,
    trigger: ScheduleTrigger,
    transition_token: &str,
) -> Result<ScheduleRunOutcome, String> {
    process_due_schedule(
        conn,
        schedule,
        now,
        trigger,
        &ReadinessSnapshot::deferred(DeferredReason::IdentityLocked, transition_token),
        &NoStarter,
    )
}

pub(super) fn current_runtime_admission_token(state: &AppState) -> Option<String> {
    let owner_pubkey = active_owner_pubkey(state)?;
    let runtimes = state.command_brief_runtimes.try_read().ok()?;
    let runtime = runtimes.all_for_owner(&owner_pubkey).into_iter().next()?;
    let token = RuntimeReadiness::ready(
        &runtime.config,
        runtime.generation,
        runtime.orchestrator.admission_state(),
    )
    .transition_token;
    active_owner_matches(state, &owner_pubkey).then_some(token)
}
