use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use buzz_core::agent_job::{
    parse_agent_job_event, AgentJobAccepted, AgentJobAcceptedState, AgentJobError,
    AgentJobErrorState, AgentJobPayload, AgentJobProgress, AgentJobProgressState, AgentJobRequest,
    AgentJobResult, AgentJobResultState, ParsedAgentJobEvent, AGENT_JOB_SCHEMA,
};
use buzz_core::kind::{
    KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_ERROR, KIND_JOB_PROGRESS, KIND_JOB_REQUEST,
    KIND_JOB_RESULT,
};
use buzz_runtime::{
    argv_sha256, canonicalize_workspace, job_attempt_dir, process_matches_marker,
    process_start_marker, read_runner_receipt, runner_receipt_health, tail_rotating_log,
    write_job_spec, AssignmentRecord, AssignmentSetStateRequest, AssignmentState,
    AuthorizedCapability, CancelledRemoteJob, ControlError, ControlHandler, ControlOperation,
    ControlPayload, CreateJobOutcome, HandlerFuture, JobId, JobListFilter, JobLogs, JobRecord,
    JobRunnerReceiptHealth, JobSpec, JobStartRequest, JobState, JobStatus, JobTransition, NewJob,
    OutboxEvent, PublicationState, RemoteCancelTombstone, RunnerIdentity, RunnerReceipt,
    RunnerReceiptState, RuntimeStatus, StoreHandle, MAX_LOG_TAIL_BYTES, MAX_LOG_TAIL_LINES,
};
use chrono::Utc;
use nostr::{Event, EventId, Keys, PublicKey};
use tokio::sync::watch;
use uuid::Uuid;

use crate::config::ManagedRuntimeConfig;

const RUNNER_READY_TIMEOUT: Duration = Duration::from_secs(5);
const RUNNER_READY_POLL: Duration = Duration::from_millis(25);
const TERMINAL_RECEIPT_SETTLE_POLLS: usize = 4;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(15);
const LOG_TAIL_STREAM_BUDGET: usize = (MAX_LOG_TAIL_BYTES / 2) - (16 * 1024);

#[derive(Clone)]
pub(crate) struct JobSupervisor {
    inner: Arc<Inner>,
}

struct Inner {
    runtime: ManagedRuntimeConfig,
    store: StoreHandle,
    keys: Keys,
    generation: Uuid,
    runner_executable: PathBuf,
    shutdown_tx: watch::Sender<bool>,
    lifecycle_lock: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for JobSupervisor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JobSupervisor")
            .field("runtime_id", &self.inner.runtime.runtime_id)
            .field("generation", &self.inner.generation)
            .finish_non_exhaustive()
    }
}

impl JobSupervisor {
    pub(crate) fn new(
        runtime: ManagedRuntimeConfig,
        store: StoreHandle,
        keys: Keys,
        generation: Uuid,
        shutdown_tx: watch::Sender<bool>,
    ) -> Result<Self> {
        let runner_executable = std::env::current_exe()
            .context("resolve current buzz-acp executable")?
            .canonicalize()
            .context("canonicalize current buzz-acp executable")?;
        Self::new_with_runner_executable(
            runtime,
            store,
            keys,
            generation,
            shutdown_tx,
            runner_executable,
        )
    }

    pub(crate) fn new_with_runner_executable(
        runtime: ManagedRuntimeConfig,
        store: StoreHandle,
        keys: Keys,
        generation: Uuid,
        shutdown_tx: watch::Sender<bool>,
        runner_executable: PathBuf,
    ) -> Result<Self> {
        let runner_executable = runner_executable
            .canonicalize()
            .context("canonicalize configured buzz-acp runner executable")?;
        Ok(Self {
            inner: Arc::new(Inner {
                runtime,
                store,
                keys,
                generation,
                runner_executable,
                shutdown_tx,
                lifecycle_lock: tokio::sync::Mutex::new(()),
            }),
        })
    }

    pub(crate) fn runtime_id(&self) -> &str {
        &self.inner.runtime.runtime_id
    }

    pub(crate) fn generation(&self) -> Uuid {
        self.inner.generation
    }

    pub(crate) async fn start(&self, request: JobStartRequest) -> Result<JobStatus, ControlError> {
        self.start_local(request, None).await
    }

    async fn start_for_model(&self, request: JobStartRequest) -> Result<JobStatus, ControlError> {
        let assignment = self
            .inner
            .store
            .active_assignment()
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                control(
                    "assignment_required",
                    "model job start requires an active assignment",
                )
            })?;
        let request = bind_model_request(request, &assignment)?;
        self.start_local(request, Some(assignment.assignment_id))
            .await
    }

    async fn start_local(
        &self,
        mut request: JobStartRequest,
        assignment_id: Option<String>,
    ) -> Result<JobStatus, ControlError> {
        let _lifecycle = self.inner.lifecycle_lock.lock().await;
        request.validate().map_err(protocol_error)?;
        request.cwd = self.validate_cwd(&request.cwd)?;
        let executable = self.validate_driver_executable()?;

        let job_id = Uuid::new_v4();
        let attempt = 1;
        let created_at = Utc::now();
        let source_event_id = request
            .source_event_id
            .as_deref()
            .map(EventId::from_hex)
            .transpose()
            .map_err(|_| control("invalid_source_event", "source event id is invalid"))?;
        let public_request = AgentJobRequest {
            schema: AGENT_JOB_SCHEMA,
            driver: request.driver.clone(),
            argv: request.argv.clone(),
            cwd: request.cwd.clone(),
            summary: request.summary.clone(),
        };
        public_request
            .validate()
            .map_err(|error| control("invalid_job_request", error.to_string()))?;
        let self_pubkey = self.inner.keys.public_key();
        let builder = buzz_sdk::builders::build_agent_job_request(
            request.channel_id,
            self_pubkey,
            job_id,
            source_event_id,
            None,
            &public_request,
        )
        .map_err(|error| control("job_event_failed", error.to_string()))?;
        let request_event = builder
            .sign_with_keys(&self.inner.keys)
            .map_err(|error| control("job_event_failed", error.to_string()))?;
        let request_event_id = request_event.id.to_hex();
        let new_job = NewJob {
            job_id,
            request_event_id: request_event_id.clone(),
            requester_pubkey: self_pubkey.to_hex(),
            executable,
            request: request.clone(),
            attempt,
            created_at,
        };
        let outbox = outbox_event(
            &request_event,
            job_id,
            request.channel_id,
            KIND_JOB_REQUEST as u16,
            false,
            created_at,
        )?;
        let outcome = match assignment_id {
            Some(assignment_id) => {
                self.inner
                    .store
                    .create_local_job_for_assignment(&assignment_id, new_job, outbox)
                    .await
            }
            None => self.inner.store.create_local_job(new_job, outbox).await,
        }
        .map_err(store_error)?;
        let record = match outcome {
            CreateJobOutcome::Created(record) | CreateJobOutcome::Duplicate(record) => record,
        };
        self.launch_requested_job(record, request).await
    }

    /// Admits one already-received signed kind-43001 request after the relay
    /// ingress has proved author policy and channel membership. Both proofs
    /// are rechecked here before persistence or process creation.
    pub(crate) async fn start_remote_request(
        &self,
        event: &Event,
        inbound_author_authorized: bool,
        channel_membership_verified: bool,
    ) -> Result<JobStatus, ControlError> {
        let parsed = self.validate_remote_event(
            event,
            KIND_JOB_REQUEST,
            inbound_author_authorized,
            channel_membership_verified,
        )?;
        let AgentJobPayload::Request(payload) = parsed.payload else {
            return Err(control(
                "invalid_job_request",
                "kind 43001 did not contain a request payload",
            ));
        };

        let _lifecycle = self.inner.lifecycle_lock.lock().await;
        if let Some(existing) = self
            .inner
            .store
            .get_job(parsed.job)
            .await
            .map_err(store_error)?
        {
            self.link_current_assignment(&existing).await?;
            return Ok(status(&existing));
        }

        let mut request = JobStartRequest {
            channel_id: parsed.channel_id,
            source_event_id: parsed.linked_event_id.as_ref().map(EventId::to_hex),
            driver: payload.driver,
            argv: payload.argv,
            cwd: payload.cwd,
            summary: payload.summary,
        };
        request.validate().map_err(protocol_error)?;
        request.cwd = self.validate_cwd(&request.cwd)?;
        let executable = self.validate_driver_executable()?;
        let request_event_id = event.id.to_hex();
        let new_job = NewJob {
            job_id: parsed.job,
            request_event_id: request_event_id.clone(),
            requester_pubkey: event.pubkey.to_hex(),
            executable,
            request: request.clone(),
            attempt: 1,
            created_at: Utc::now(),
        };

        let tombstones = self
            .inner
            .store
            .remote_cancels(parsed.job, request_event_id.clone())
            .await
            .map_err(store_error)?;
        if let Some(cancel) = tombstones.iter().find(|cancel| {
            cancel.channel_id == parsed.channel_id
                && (cancel.authorized_without_request
                    || cancel.canceller_pubkey == event.pubkey.to_hex())
        }) {
            return self
                .create_pre_cancelled_remote_job(new_job, cancel.cancel_event_id.clone())
                .await;
        }
        if !tombstones.is_empty() {
            self.inner
                .store
                .discard_remote_cancels(parsed.job, request_event_id)
                .await
                .map_err(store_error)?;
        }

        let snapshot = self
            .inner
            .store
            .assignment_snapshot()
            .await
            .map_err(store_error)?;
        if let Some(assignment) = snapshot.active_assignment.as_ref() {
            let linked_source = parsed.linked_event_id.as_ref().map(EventId::to_hex);
            let source_matches = assignment.source_event_id.as_ref().is_some_and(|source| {
                linked_source.as_ref() == Some(source) || event.id.to_hex() == *source
            });
            if assignment.active_job_id.is_some() || !source_matches {
                return Err(control(
                    "assignment_busy",
                    "an unrelated assignment or durable job is already active",
                ));
            }
        } else if !snapshot.active_jobs.is_empty() {
            return Err(control(
                "assignment_busy",
                "a durable job is already active",
            ));
        }
        if snapshot.active_assignment.is_none() {
            let assignment = self
                .inner
                .store
                .claim_assignment(
                    parsed.channel_id,
                    Some(event.id.to_hex()),
                    request.summary.clone(),
                    None,
                    Utc::now(),
                )
                .await
                .map_err(store_error)?;
            if assignment.channel_id != parsed.channel_id
                || assignment.source_event_id.as_deref() != Some(event.id.to_hex().as_str())
            {
                return Err(control(
                    "assignment_busy",
                    "another assignment became active before job admission",
                ));
            }
        }

        let record = match self
            .inner
            .store
            .create_remote_job(new_job)
            .await
            .map_err(store_error)?
        {
            CreateJobOutcome::Created(record) | CreateJobOutcome::Duplicate(record) => record,
        };
        self.launch_requested_job(record, request).await
    }

    /// Applies one signed kind-43005 cancellation only after rechecking the
    /// original request linkage and requester/owner/target authority.
    pub(crate) async fn apply_remote_cancel(
        &self,
        event: &Event,
        inbound_author_authorized: bool,
        channel_membership_verified: bool,
        owner_pubkey: Option<&PublicKey>,
    ) -> Result<JobStatus, ControlError> {
        let parsed = self.validate_remote_event(
            event,
            KIND_JOB_CANCEL,
            inbound_author_authorized,
            channel_membership_verified,
        )?;
        if !matches!(parsed.payload, AgentJobPayload::Cancel(_)) {
            return Err(control(
                "invalid_job_cancel",
                "kind 43005 did not contain a cancel payload",
            ));
        }
        let linked_request = parsed
            .linked_event_id
            .as_ref()
            .map(EventId::to_hex)
            .ok_or_else(|| control("invalid_job_cancel", "cancel is missing request linkage"))?;

        let lifecycle = self.inner.lifecycle_lock.lock().await;
        let job = self
            .inner
            .store
            .get_job(parsed.job)
            .await
            .map_err(store_error)?;
        if let Some(job) = job {
            if parsed.channel_id != job.channel_id {
                return Err(control(
                    "unauthorized_job_cancel",
                    "cancel channel does not match the original request",
                ));
            }
            if job.request_event_id.as_deref() != Some(linked_request.as_str()) {
                return Err(control(
                    "unauthorized_job_cancel",
                    "cancel request link does not match the original request",
                ));
            }
            let requester = PublicKey::from_hex(&job.requester_pubkey)
                .map_err(|_| control("invalid_job_state", "stored requester key is invalid"))?;
            let target = self.inner.keys.public_key();
            let authorized = event.pubkey == requester
                || owner_pubkey.is_some_and(|owner| owner == &event.pubkey)
                || event.pubkey == target;
            if !authorized {
                return Err(control(
                    "unauthorized_job_cancel",
                    "cancel author is not requester, owner, or target agent",
                ));
            }
            drop(lifecycle);
            return self.cancel(parsed.job).await;
        }

        let target = self.inner.keys.public_key();
        let authorized_without_request =
            event.pubkey == target || owner_pubkey.is_some_and(|owner| owner == &event.pubkey);
        self.inner
            .store
            .record_remote_cancel(RemoteCancelTombstone {
                job_id: parsed.job,
                request_event_id: linked_request.clone(),
                channel_id: parsed.channel_id,
                cancel_event_id: event.id.to_hex(),
                canceller_pubkey: event.pubkey.to_hex(),
                authorized_without_request,
                created_at: Utc::now(),
            })
            .await
            .map_err(store_error)?;
        let now = Utc::now();
        Ok(JobStatus {
            job_id: parsed.job,
            request_event_id: Some(linked_request),
            source_event_id: None,
            channel_id: parsed.channel_id,
            state: JobState::Cancelled,
            attempt: 1,
            progress_seq: 0,
            summary: "cancel recorded pending request replay".into(),
            started_at: None,
            finished_at: Some(now),
            exit_code: None,
            error_code: Some("cancel_pending_request".into()),
            publication_state: PublicationState::NotStarted,
            runner_pid: None,
            runner_start_marker: None,
        })
    }

    async fn create_pre_cancelled_remote_job(
        &self,
        new_job: NewJob,
        cancel_event_id: String,
    ) -> Result<JobStatus, ControlError> {
        let finished_at = Utc::now();
        let draft = JobRecord {
            job_id: new_job.job_id,
            request_event_id: Some(new_job.request_event_id.clone()),
            source_event_id: new_job.request.source_event_id.clone(),
            channel_id: new_job.request.channel_id,
            requester_pubkey: new_job.requester_pubkey.clone(),
            driver: new_job.request.driver.clone(),
            executable: new_job.executable.to_string_lossy().into_owned(),
            argv: new_job.request.argv.clone(),
            cwd: new_job.request.cwd.clone(),
            summary: new_job.request.summary.clone(),
            state: JobState::Requested,
            runner: None,
            attempt: new_job.attempt,
            progress_seq: 0,
            exit_code: None,
            result_json: None,
            error_code: None,
            terminal_event_id: None,
            publication_state: PublicationState::NotStarted,
            publication_error: None,
            created_at: new_job.created_at,
            started_at: None,
            finished_at: None,
            updated_at: new_job.created_at,
        };
        let payload = AgentJobError {
            schema: AGENT_JOB_SCHEMA,
            job: draft.job_id,
            attempt: draft.attempt,
            state: AgentJobErrorState::Cancelled,
            code: "cancelled_before_request".into(),
            summary: "job cancelled before request admission".into(),
            retryable: false,
            artifacts: Vec::new(),
            finished_at,
        };
        let event = self
            .build_error_event(&draft, &payload)
            .map_err(|error| control("job_event_failed", error.to_string()))?;
        let terminal_event = outbox_event(
            &event,
            draft.job_id,
            draft.channel_id,
            KIND_JOB_ERROR as u16,
            true,
            finished_at,
        )?;
        let result_json = serde_json::to_string(&payload)
            .map_err(|error| control("job_event_failed", error.to_string()))?;
        let record = match self
            .inner
            .store
            .create_cancelled_remote_job(CancelledRemoteJob {
                job: new_job,
                cancel_event_id,
                result_json,
                terminal_event,
            })
            .await
            .map_err(store_error)?
        {
            CreateJobOutcome::Created(record) | CreateJobOutcome::Duplicate(record) => record,
        };
        self.project_terminal_assignment(&record).await;
        Ok(status(&record))
    }

    fn validate_remote_event(
        &self,
        event: &Event,
        expected_kind: u32,
        inbound_author_authorized: bool,
        channel_membership_verified: bool,
    ) -> Result<ParsedAgentJobEvent, ControlError> {
        if !inbound_author_authorized || !channel_membership_verified {
            return Err(control(
                "unauthorized_remote_job",
                "remote job author or channel membership is not authorized",
            ));
        }
        event
            .verify()
            .map_err(|_| control("invalid_job_event", "job event signature is invalid"))?;
        let parsed = parse_agent_job_event(event)
            .map_err(|error| control("invalid_job_event", error.to_string()))?;
        if parsed.kind != expected_kind {
            return Err(control(
                "invalid_job_event",
                "job event kind does not match the requested operation",
            ));
        }
        if parsed.peer != self.inner.keys.public_key() {
            return Err(control(
                "unauthorized_remote_job",
                "job event does not target this agent",
            ));
        }
        Ok(parsed)
    }

    async fn launch_requested_job(
        &self,
        record: JobRecord,
        request: JobStartRequest,
    ) -> Result<JobStatus, ControlError> {
        if record.state != JobState::Requested {
            self.link_current_assignment(&record).await?;
            return Ok(status(&record));
        }
        self.link_current_assignment(&record).await?;

        let job_id = record.job_id;
        let attempt = record.attempt;
        let created_at = record.created_at;
        let spec = JobSpec {
            runtime_id: self.inner.runtime.runtime_id.clone(),
            job_id,
            attempt,
            executable: record.executable.clone().into(),
            argv_sha256: argv_sha256(&request.argv)
                .map_err(|error| control("job_spec_failed", error.to_string()))?,
            request,
            created_at,
        };
        let spec_path = match write_job_spec(&self.inner.runtime.state_dir, &spec) {
            Ok(path) => path,
            Err(error) => {
                return self
                    .fail_before_accept(&record, "job_spec_failed", &error.to_string())
                    .await;
            }
        };
        let runner_pid = match self.spawn_runner(&spec_path) {
            Ok(pid) => pid,
            Err(error) => {
                return self
                    .fail_before_accept(&record, "runner_spawn_failed", &error.to_string())
                    .await;
            }
        };
        let receipt = match self
            .wait_for_runner(&record, runner_pid, &spec.argv_sha256)
            .await
        {
            Ok(receipt) => receipt,
            Err(error) => {
                self.kill_spawned_runner_if_verified(&record, runner_pid)
                    .await;
                return self
                    .fail_before_accept(&record, "runner_not_ready", &error.message)
                    .await;
            }
        };
        let runner = RunnerIdentity {
            pid: receipt.runner_pid,
            start_marker: receipt.runner_start_marker.clone(),
            process_group: receipt.process_group.clone(),
        };
        let accepted_at = Utc::now();
        let accepted_event = self
            .build_accepted_event(&record, accepted_at)
            .map_err(|error| control("job_event_failed", error.to_string()))?;
        let accepted_outbox = outbox_event(
            &accepted_event,
            job_id,
            record.channel_id,
            KIND_JOB_ACCEPTED as u16,
            false,
            accepted_at,
        )?;
        let accepted = self
            .inner
            .store
            .transition_job(
                transition(
                    &record,
                    JobState::Accepted,
                    Some(runner.clone()),
                    accepted_at,
                ),
                Some(accepted_outbox),
            )
            .await
            .map_err(store_error)?;
        let running_at = Utc::now();
        let running = self
            .record_progress(
                &accepted,
                JobState::Running,
                AgentJobProgressState::Running,
                Some(runner),
                "Legacy Harness runner running (elapsed 0s)".into(),
                running_at,
            )
            .await?;

        if receipt.state == RunnerReceiptState::Ready {
            Ok(status(&running))
        } else {
            let terminal = self.import_terminal(&running, receipt).await?;
            Ok(status(&terminal))
        }
    }

    pub(crate) async fn reconcile(&self) -> Result<Vec<JobStatus>, ControlError> {
        let _lifecycle = self.inner.lifecycle_lock.lock().await;
        let jobs = self
            .inner
            .store
            .list_jobs(JobListFilter::default())
            .await
            .map_err(store_error)?;
        let mut output = Vec::with_capacity(jobs.len());
        for job in jobs {
            let reconciled = if job.state.is_terminal() {
                job
            } else {
                self.reconcile_one(job).await?
            };
            output.push(status(&reconciled));
        }
        Ok(output)
    }

    async fn reconcile_one(&self, job: JobRecord) -> Result<JobRecord, ControlError> {
        match read_runner_receipt(&self.inner.runtime.state_dir, job.job_id, job.attempt) {
            Ok(receipt) => {
                if receipt.argv_sha256
                    != argv_sha256(&job.argv)
                        .map_err(|error| control("runner_receipt_invalid", error.to_string()))?
                {
                    return self.mark_lost(&job, "runner_argv_mismatch").await;
                }
                let identity = RunnerIdentity {
                    pid: receipt.runner_pid,
                    start_marker: receipt.runner_start_marker.clone(),
                    process_group: receipt.process_group.clone(),
                };
                if receipt.state == RunnerReceiptState::Ready {
                    if self.verify_job_identity(&job, &identity).is_err() {
                        if let Some(terminal) = self.retry_terminal_receipt(&job).await? {
                            return Ok(terminal);
                        }
                        return self.mark_lost(&job, "runner_identity_unverified").await;
                    }
                    if job.state == JobState::Cancelling {
                        terminate_verified_tree(&identity).await?;
                        return self
                            .terminal_error(
                                &job,
                                JobState::Cancelled,
                                AgentJobErrorState::Cancelled,
                                "cancelled",
                                "job cancellation completed during runtime recovery",
                                false,
                            )
                            .await;
                    }
                    let running = self.ensure_running(job, identity).await?;
                    self.emit_periodic_progress(running).await
                } else {
                    let prepared = if matches!(job.state, JobState::Requested | JobState::Accepted)
                    {
                        self.ensure_running(job, identity).await?
                    } else {
                        job
                    };
                    self.import_terminal(&prepared, receipt).await
                }
            }
            Err(_) => match job.runner.as_ref().map(verify_identity) {
                Some(Ok(())) => Ok(job),
                _ => {
                    if let Some(terminal) = self.retry_terminal_receipt(&job).await? {
                        return Ok(terminal);
                    }
                    self.mark_lost(&job, "runner_receipt_missing").await
                }
            },
        }
    }

    async fn retry_terminal_receipt(
        &self,
        job: &JobRecord,
    ) -> Result<Option<JobRecord>, ControlError> {
        for _ in 0..TERMINAL_RECEIPT_SETTLE_POLLS {
            tokio::time::sleep(RUNNER_READY_POLL).await;
            let Ok(receipt) =
                read_runner_receipt(&self.inner.runtime.state_dir, job.job_id, job.attempt)
            else {
                continue;
            };
            if receipt.state == RunnerReceiptState::Ready {
                continue;
            }
            if receipt.argv_sha256
                != argv_sha256(&job.argv)
                    .map_err(|error| control("runner_receipt_invalid", error.to_string()))?
            {
                return Ok(Some(self.mark_lost(job, "runner_argv_mismatch").await?));
            }
            let identity = RunnerIdentity {
                pid: receipt.runner_pid,
                start_marker: receipt.runner_start_marker.clone(),
                process_group: receipt.process_group.clone(),
            };
            let prepared = if matches!(job.state, JobState::Requested | JobState::Accepted) {
                self.ensure_running(job.clone(), identity).await?
            } else {
                job.clone()
            };
            return self.import_terminal(&prepared, receipt).await.map(Some);
        }
        Ok(None)
    }

    async fn ensure_running(
        &self,
        mut job: JobRecord,
        identity: RunnerIdentity,
    ) -> Result<JobRecord, ControlError> {
        if job.state == JobState::Requested {
            let accepted_at = Utc::now();
            let event = self
                .build_accepted_event(&job, accepted_at)
                .map_err(|error| control("job_event_failed", error.to_string()))?;
            let outbox = outbox_event(
                &event,
                job.job_id,
                job.channel_id,
                KIND_JOB_ACCEPTED as u16,
                false,
                accepted_at,
            )?;
            job = self
                .inner
                .store
                .transition_job(
                    transition(
                        &job,
                        JobState::Accepted,
                        Some(identity.clone()),
                        accepted_at,
                    ),
                    Some(outbox),
                )
                .await
                .map_err(store_error)?;
        }
        if job.state == JobState::Accepted {
            return self
                .record_progress(
                    &job,
                    JobState::Running,
                    AgentJobProgressState::Running,
                    Some(identity),
                    "Legacy Harness runner running (elapsed 0s)".into(),
                    Utc::now(),
                )
                .await;
        }
        if job.state == JobState::Running && job.runner.as_ref() == Some(&identity) {
            return Ok(job);
        }
        self.mark_lost(&job, "runner_identity_changed").await
    }

    pub(crate) async fn cancel(&self, job_id: JobId) -> Result<JobStatus, ControlError> {
        let _lifecycle = self.inner.lifecycle_lock.lock().await;
        let job = self.get(job_id).await?;
        if job.state.is_terminal() {
            return Ok(status(&job));
        }
        let runner = job
            .runner
            .clone()
            .ok_or_else(|| control("runner_identity_missing", "runner identity is unavailable"))?;
        self.verify_job_identity(&job, &runner).map_err(|_| {
            control(
                "runner_identity_mismatch",
                "runner identity could not be verified; no process was signalled",
            )
        })?;
        let cancelling_at = Utc::now();
        let cancelling = self
            .record_progress(
                &job,
                JobState::Cancelling,
                AgentJobProgressState::Cancelling,
                Some(runner.clone()),
                self.progress_summary(&job, AgentJobProgressState::Cancelling, cancelling_at),
                cancelling_at,
            )
            .await?;
        terminate_verified_tree(&runner).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline && verify_identity(&runner).is_ok() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if verify_identity(&runner).is_ok() {
            return Err(control(
                "runner_cancel_failed",
                "verified runner did not exit",
            ));
        }
        let terminal = self
            .terminal_error(
                &cancelling,
                JobState::Cancelled,
                AgentJobErrorState::Cancelled,
                "cancelled",
                "job cancelled by local authenticated request",
                false,
            )
            .await?;
        Ok(status(&terminal))
    }

    pub(crate) async fn logs(
        &self,
        job_id: JobId,
        lines: Option<u16>,
    ) -> Result<JobLogs, ControlError> {
        let job = self.get(job_id).await?;
        let lines = lines.unwrap_or(100).min(MAX_LOG_TAIL_LINES);
        let attempt_dir = job_attempt_dir(&self.inner.runtime.state_dir, job.job_id, job.attempt)
            .map_err(|error| control("logs_unavailable", error.to_string()))?;
        let stdout_path = attempt_dir.join("stdout.log");
        let stderr_path = attempt_dir.join("stderr.log");
        let (stdout, stderr) = tokio::task::spawn_blocking(move || {
            let stdout = tail_rotating_log(stdout_path, lines, LOG_TAIL_STREAM_BUDGET)?;
            let stderr = tail_rotating_log(stderr_path, lines, LOG_TAIL_STREAM_BUDGET)?;
            Ok::<_, std::io::Error>((stdout, stderr))
        })
        .await
        .map_err(|error| control("logs_unavailable", error.to_string()))?
        .map_err(|error| control("logs_unavailable", error.to_string()))?;
        let mut output = Vec::with_capacity(stdout.len() + stderr.len());
        output.extend(stdout.into_iter().map(|line| format!("stdout: {line}")));
        output.extend(stderr.into_iter().map(|line| format!("stderr: {line}")));
        if output.len() > usize::from(lines) {
            output.drain(..output.len() - usize::from(lines));
        }
        Ok(JobLogs {
            job_id,
            local_only: true,
            lines: output,
        })
    }

    async fn get(&self, job_id: JobId) -> Result<JobRecord, ControlError> {
        self.inner
            .store
            .get_job(job_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| control("job_not_found", "job does not exist"))
    }

    async fn list(&self, filter: JobListFilter) -> Result<Vec<JobStatus>, ControlError> {
        self.inner
            .store
            .list_jobs(filter)
            .await
            .map_err(store_error)
            .map(|jobs| jobs.iter().map(status).collect())
    }

    async fn runtime_status(&self) -> Result<RuntimeStatus, ControlError> {
        let snapshot = self
            .inner
            .store
            .assignment_snapshot()
            .await
            .map_err(store_error)?;
        let active_job_id = snapshot
            .active_assignment
            .as_ref()
            .and_then(|assignment| assignment.active_job_id)
            .or_else(|| snapshot.active_jobs.first().copied());
        let active_job = match active_job_id {
            Some(job_id) => self
                .inner
                .store
                .get_job(job_id)
                .await
                .map_err(store_error)?,
            None => None,
        };
        let mut status = snapshot.runtime_status(
            self.inner.runtime.runtime_id.clone(),
            self.inner.generation,
            true,
            false,
            snapshot.queue_depths.in_turn > 0,
            active_job.as_ref(),
        );
        let diagnostics = self
            .inner
            .store
            .operational_diagnostics()
            .await
            .map_err(store_error)?;
        status.diagnostics.store_schema_version = diagnostics.schema_version;
        status.diagnostics.last_relay_progress_published_at =
            diagnostics.last_relay_progress_published_at;
        for job_id in &snapshot.active_jobs {
            if let Some(job) = self
                .inner
                .store
                .get_job(*job_id)
                .await
                .map_err(store_error)?
            {
                status
                    .diagnostics
                    .runner_receipts
                    .push(JobRunnerReceiptHealth {
                        job_id: job.job_id,
                        attempt: job.attempt,
                        health: runner_receipt_health(
                            &self.inner.runtime.state_dir,
                            job.job_id,
                            job.attempt,
                        ),
                    });
            }
        }
        Ok(status)
    }

    async fn shutdown_runtime(&self) -> Result<(), ControlError> {
        let _lifecycle = self.inner.lifecycle_lock.lock().await;
        let jobs = self
            .inner
            .store
            .list_jobs(JobListFilter::default())
            .await
            .map_err(store_error)?;
        let mut first_error = jobs
            .iter()
            .find(|job| {
                job.state.is_terminal()
                    && matches!(
                        job.error_code.as_deref(),
                        Some(
                            "shutdown_runner_identity_missing"
                                | "shutdown_runner_identity_unverified"
                                | "shutdown_process_tree_survived"
                        )
                    )
                    && match job.runner.as_ref() {
                        Some(identity) => recorded_tree_is_empty(identity) != Ok(true),
                        None => true,
                    }
            })
            .map(|_| {
                control(
                    "shutdown_reconciliation_required",
                    "a prior shutdown could not verify runner-tree termination",
                )
            });

        for job in jobs.into_iter().filter(|job| !job.state.is_terminal()) {
            let Some(runner) = job.runner.clone() else {
                if self.retry_terminal_receipt(&job).await?.is_some() {
                    continue;
                }
                if let Err(error) = self
                    .mark_lost(&job, "shutdown_runner_identity_missing")
                    .await
                {
                    first_error.get_or_insert(error);
                } else {
                    first_error.get_or_insert_with(|| {
                        control(
                            "shutdown_runner_identity_missing",
                            "runner identity is unavailable; runtime remains active",
                        )
                    });
                }
                continue;
            };
            if self.verify_job_identity(&job, &runner).is_err() {
                if self.retry_terminal_receipt(&job).await?.is_some() {
                    continue;
                }
                if let Err(error) = self
                    .mark_lost(&job, "shutdown_runner_identity_unverified")
                    .await
                {
                    first_error.get_or_insert(error);
                } else {
                    first_error.get_or_insert_with(|| {
                        control(
                            "shutdown_runner_identity_unverified",
                            "runner identity could not be verified; runtime remains active",
                        )
                    });
                }
                continue;
            }
            if let Err(error) = terminate_verified_tree(&runner).await {
                let terminal_error = self
                    .mark_lost(&job, "shutdown_process_tree_survived")
                    .await
                    .err();
                first_error.get_or_insert(error);
                if let Some(error) = terminal_error {
                    first_error.get_or_insert(error);
                }
                continue;
            }
            if job.state == JobState::Requested {
                if let Err(error) = self.mark_lost(&job, "runtime_shutdown_before_accept").await {
                    first_error.get_or_insert(error);
                }
                continue;
            }
            let terminal_source = if job.state == JobState::Accepted {
                match self
                    .inner
                    .store
                    .transition_job(
                        transition(&job, JobState::Cancelling, Some(runner), Utc::now()),
                        None,
                    )
                    .await
                    .map_err(store_error)
                {
                    Ok(job) => job,
                    Err(error) => {
                        first_error.get_or_insert(error);
                        continue;
                    }
                }
            } else {
                job
            };
            if let Err(error) = self
                .terminal_error(
                    &terminal_source,
                    JobState::Cancelled,
                    AgentJobErrorState::Cancelled,
                    "runtime_shutdown",
                    "job cancelled by explicit runtime shutdown",
                    false,
                )
                .await
            {
                first_error.get_or_insert(error);
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }
        self.inner
            .shutdown_tx
            .send(true)
            .map_err(|_| control("shutdown_unavailable", "runtime shutdown channel closed"))
    }

    async fn link_current_assignment(&self, job: &JobRecord) -> Result<(), ControlError> {
        let Some(assignment) = self
            .inner
            .store
            .active_assignment()
            .await
            .map_err(store_error)?
        else {
            return Ok(());
        };
        let source_matches = assignment.source_event_id.as_ref().is_some_and(|source| {
            job.source_event_id.as_ref() == Some(source)
                || job.request_event_id.as_ref() == Some(source)
        });
        if assignment.channel_id == job.channel_id && source_matches {
            self.inner
                .store
                .link_assignment_job(&assignment.assignment_id, job.job_id, Utc::now())
                .await
                .map_err(store_error)?;
        }
        Ok(())
    }

    fn validate_cwd(&self, cwd: &str) -> Result<String, ControlError> {
        canonicalize_workspace(Path::new(cwd), &self.inner.runtime.workspace_roots)
            .map_err(|_| {
                control(
                    "workspace_not_allowed",
                    "cwd is outside operator-approved workspace roots",
                )
            })?
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| control("workspace_not_allowed", "cwd is not valid UTF-8"))
    }

    fn validate_driver_executable(&self) -> Result<PathBuf, ControlError> {
        let configured = self
            .inner
            .runtime
            .lh_executable
            .as_ref()
            .ok_or_else(|| control("driver_unavailable", "LH executable is not configured"))?;
        let canonical = configured
            .canonicalize()
            .map_err(|_| control("driver_unavailable", "LH executable is unavailable"))?;
        if canonical != *configured || !canonical.is_file() || !is_executable(&canonical) {
            return Err(control(
                "driver_unavailable",
                "LH executable no longer matches validated operator configuration",
            ));
        }
        Ok(canonical)
    }
    fn verify_job_identity(&self, _job: &JobRecord, identity: &RunnerIdentity) -> Result<()> {
        #[cfg(windows)]
        {
            let expected = crate::job_windows::job_name(
                &self.inner.runtime.runtime_id,
                _job.job_id,
                _job.attempt,
            );
            if identity.process_group != expected {
                anyhow::bail!("runner Job Object identity mismatch");
            }
        }
        verify_identity(identity)
    }

    fn spawn_runner(&self, spec_path: &Path) -> Result<u32> {
        let mut command = Command::new(&self.inner.runner_executable);
        command
            .arg("__job-runner")
            .arg(spec_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_remove("BUZZ_PRIVATE_KEY")
            .env_remove("BUZZ_AUTH_TAG")
            .env_remove("BUZZ_RUNTIME_RECEIPT")
            .env_remove("BUZZ_RUNTIME_CONTROL_TOKEN")
            .env_remove("BUZZ_RUNTIME_MODEL_TOKEN");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0000_0200);
        }
        let mut child = command.spawn().context("spawn independent job runner")?;
        let pid = child.id();
        std::thread::Builder::new()
            .name(format!("buzz-job-runner-{pid}"))
            .spawn(move || {
                let _ = child.wait();
            })
            .context("spawn runner reap thread")?;
        Ok(pid)
    }

    async fn wait_for_runner(
        &self,
        job: &JobRecord,
        expected_pid: u32,
        expected_argv_sha256: &str,
    ) -> Result<RunnerReceipt, ControlError> {
        let deadline = tokio::time::Instant::now() + RUNNER_READY_TIMEOUT;
        loop {
            if let Ok(receipt) =
                read_runner_receipt(&self.inner.runtime.state_dir, job.job_id, job.attempt)
            {
                if receipt.runner_pid != expected_pid
                    || receipt.argv_sha256 != expected_argv_sha256
                    || receipt.validate(job.job_id, job.attempt).is_err()
                {
                    return Err(control(
                        "runner_identity_mismatch",
                        "runner receipt mismatch",
                    ));
                }
                if receipt.state == RunnerReceiptState::Ready {
                    let identity = RunnerIdentity {
                        pid: receipt.runner_pid,
                        start_marker: receipt.runner_start_marker.clone(),
                        process_group: receipt.process_group.clone(),
                    };
                    self.verify_job_identity(job, &identity).map_err(|_| {
                        control(
                            "runner_identity_mismatch",
                            "runner identity could not be verified",
                        )
                    })?;
                }
                return Ok(receipt);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(control(
                    "runner_not_ready",
                    "runner did not become ready within 5 seconds",
                ));
            }
            tokio::time::sleep(RUNNER_READY_POLL).await;
        }
    }

    async fn kill_spawned_runner_if_verified(&self, job: &JobRecord, pid: u32) {
        let Ok(start_marker) = process_start_marker(pid) else {
            return;
        };
        #[cfg(unix)]
        let process_group = pid.to_string();
        #[cfg(windows)]
        let process_group =
            crate::job_windows::job_name(&self.inner.runtime.runtime_id, job.job_id, job.attempt);
        let identity = RunnerIdentity {
            pid,
            start_marker,
            process_group,
        };
        if self.verify_job_identity(job, &identity).is_ok() {
            let _ = terminate_verified_tree(&identity).await;
        }
    }

    async fn fail_before_accept(
        &self,
        job: &JobRecord,
        code: &str,
        summary: &str,
    ) -> Result<JobStatus, ControlError> {
        let terminal = self
            .terminal_error(
                job,
                JobState::Failed,
                AgentJobErrorState::Failed,
                code,
                summary,
                true,
            )
            .await?;
        Ok(status(&terminal))
    }

    async fn mark_lost(&self, job: &JobRecord, code: &str) -> Result<JobRecord, ControlError> {
        let (state, wire_state) = if job.state == JobState::Requested {
            (JobState::Failed, AgentJobErrorState::Failed)
        } else {
            (JobState::Lost, AgentJobErrorState::Lost)
        };
        self.terminal_error(
            job,
            state,
            wire_state,
            code,
            "runtime could not verify the durable runner identity",
            false,
        )
        .await
    }

    async fn import_terminal(
        &self,
        job: &JobRecord,
        receipt: RunnerReceipt,
    ) -> Result<JobRecord, ControlError> {
        match receipt.state {
            RunnerReceiptState::Ready => Ok(job.clone()),
            RunnerReceiptState::Succeeded => {
                let finished_at = receipt.finished_at.unwrap_or_else(Utc::now);
                let exit_code = receipt.exit_code.unwrap_or(0);
                let payload = AgentJobResult {
                    schema: AGENT_JOB_SCHEMA,
                    job: job.job_id,
                    attempt: job.attempt,
                    state: AgentJobResultState::Succeeded,
                    exit_code,
                    summary: job.summary.clone(),
                    artifacts: Vec::new(),
                    finished_at,
                };
                let event = self
                    .build_result_event(job, &payload)
                    .map_err(|error| control("job_event_failed", error.to_string()))?;
                let outbox = outbox_event(
                    &event,
                    job.job_id,
                    job.channel_id,
                    KIND_JOB_RESULT as u16,
                    true,
                    finished_at,
                )?;
                let mut next =
                    transition(job, JobState::Succeeded, job.runner.clone(), finished_at);
                next.exit_code = Some(exit_code);
                next.result_json = Some(
                    serde_json::to_string(&payload)
                        .map_err(|error| control("job_event_failed", error.to_string()))?,
                );
                next.terminal_event_id = Some(event.id.to_hex());
                next.publication_state = Some(PublicationState::Pending);
                let committed = self
                    .inner
                    .store
                    .transition_job(next, Some(outbox))
                    .await
                    .map_err(store_error)?;
                self.project_terminal_assignment(&committed).await;
                Ok(committed)
            }
            RunnerReceiptState::Failed
                if receipt.error_code.as_deref() == Some("orphan_suspected") =>
            {
                self.terminal_error(
                    job,
                    JobState::Lost,
                    AgentJobErrorState::Lost,
                    "orphan_suspected",
                    "Legacy Harness runner left an unverified command descendant",
                    false,
                )
                .await
            }
            RunnerReceiptState::Failed => {
                self.terminal_error(
                    job,
                    JobState::Failed,
                    AgentJobErrorState::Failed,
                    receipt.error_code.as_deref().unwrap_or("driver_failed"),
                    "Legacy Harness runner failed",
                    true,
                )
                .await
            }
            RunnerReceiptState::Cancelled => {
                self.terminal_error(
                    job,
                    JobState::Cancelled,
                    AgentJobErrorState::Cancelled,
                    "cancelled",
                    "Legacy Harness runner was cancelled",
                    false,
                )
                .await
            }
        }
    }

    async fn terminal_error(
        &self,
        job: &JobRecord,
        next_state: JobState,
        wire_state: AgentJobErrorState,
        code: &str,
        summary: &str,
        retryable: bool,
    ) -> Result<JobRecord, ControlError> {
        let finished_at = Utc::now();
        let bounded_summary = truncate_utf8(summary, buzz_core::agent_job::MAX_JOB_SUMMARY_BYTES);
        let payload = AgentJobError {
            schema: AGENT_JOB_SCHEMA,
            job: job.job_id,
            attempt: job.attempt,
            state: wire_state,
            code: code.to_string(),
            summary: bounded_summary,
            retryable,
            artifacts: Vec::new(),
            finished_at,
        };
        let event = self
            .build_error_event(job, &payload)
            .map_err(|error| control("job_event_failed", error.to_string()))?;
        let outbox = outbox_event(
            &event,
            job.job_id,
            job.channel_id,
            KIND_JOB_ERROR as u16,
            true,
            finished_at,
        )?;
        let mut next = transition(job, next_state, job.runner.clone(), finished_at);
        next.error_code = Some(code.to_string());
        next.result_json = Some(
            serde_json::to_string(&payload)
                .map_err(|error| control("job_event_failed", error.to_string()))?,
        );
        next.terminal_event_id = Some(event.id.to_hex());
        next.publication_state = Some(PublicationState::Pending);
        let committed = self
            .inner
            .store
            .transition_job(next, Some(outbox))
            .await
            .map_err(store_error)?;
        self.project_terminal_assignment(&committed).await;
        Ok(committed)
    }
    async fn project_terminal_assignment(&self, job: &JobRecord) {
        let assignment = match self.inner.store.active_assignment().await {
            Ok(Some(assignment)) if assignment.active_job_id == Some(job.job_id) => assignment,
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(
                    job_id = %job.job_id,
                    error = %error,
                    "terminal job committed but linked assignment lookup failed"
                );
                return;
            }
        };
        let result = match job.state {
            JobState::Succeeded => {
                self.inner
                    .store
                    .complete_assignment(&assignment.assignment_id, None, Utc::now())
                    .await
            }
            JobState::Failed | JobState::Lost => {
                self.inner
                    .store
                    .set_assignment_state(
                        &assignment.assignment_id,
                        AssignmentSetStateRequest {
                            state: AssignmentState::Failed,
                            summary: None,
                            reason: Some(if job.state == JobState::Lost {
                                "durable job ended without a verified runner result".into()
                            } else {
                                "durable job failed".into()
                            }),
                            blocker: None,
                            approval_gate_id: None,
                            delivery_evidence: None,
                            reply_event_id: None,
                        },
                        Utc::now(),
                    )
                    .await
            }
            JobState::Cancelled => {
                self.inner
                    .store
                    .set_assignment_state(
                        &assignment.assignment_id,
                        AssignmentSetStateRequest {
                            state: AssignmentState::Cancelled,
                            summary: None,
                            reason: Some("durable job was cancelled".into()),
                            blocker: None,
                            approval_gate_id: None,
                            delivery_evidence: None,
                            reply_event_id: None,
                        },
                        Utc::now(),
                    )
                    .await
            }
            _ => return,
        };
        if let Err(error) = result {
            tracing::warn!(
                job_id = %job.job_id,
                assignment_id = %assignment.assignment_id,
                state = ?job.state,
                error = %error,
                "terminal job committed but linked assignment projection failed"
            );
        }
    }

    async fn emit_periodic_progress(&self, job: JobRecord) -> Result<JobRecord, ControlError> {
        if job.state != JobState::Running {
            return Ok(job);
        }
        let now = Utc::now();
        let elapsed_since_progress = now
            .signed_duration_since(job.updated_at)
            .to_std()
            .unwrap_or_default();
        if elapsed_since_progress < PROGRESS_INTERVAL {
            return Ok(job);
        }
        self.record_progress(
            &job,
            JobState::Running,
            AgentJobProgressState::Running,
            job.runner.clone(),
            self.progress_summary(&job, AgentJobProgressState::Running, now),
            now,
        )
        .await
    }

    async fn record_progress(
        &self,
        job: &JobRecord,
        next_state: JobState,
        wire_state: AgentJobProgressState,
        runner: Option<RunnerIdentity>,
        summary: String,
        occurred_at: chrono::DateTime<Utc>,
    ) -> Result<JobRecord, ControlError> {
        let seq = job.progress_seq.checked_add(1).ok_or_else(|| {
            control(
                "progress_sequence_exhausted",
                "job progress sequence exhausted",
            )
        })?;
        let outbox = self.build_progress_outbox(job, seq, wire_state, summary, occurred_at)?;
        let mut next = transition(job, next_state, runner, occurred_at);
        next.progress_seq = Some(seq);
        self.inner
            .store
            .transition_job(next, Some(outbox))
            .await
            .map_err(store_error)
    }

    fn build_progress_outbox(
        &self,
        job: &JobRecord,
        seq: u64,
        state: AgentJobProgressState,
        summary: String,
        created_at: chrono::DateTime<Utc>,
    ) -> Result<OutboxEvent, ControlError> {
        let payload = AgentJobProgress {
            schema: AGENT_JOB_SCHEMA,
            job: job.job_id,
            attempt: job.attempt,
            seq,
            state,
            summary,
            artifacts: Vec::new(),
        };
        let event = buzz_sdk::builders::build_agent_job_progress(
            job.channel_id,
            parse_requester(job).map_err(|error| control("job_event_failed", error.to_string()))?,
            parse_request_event(job)
                .map_err(|error| control("job_event_failed", error.to_string()))?,
            &payload,
        )
        .map_err(|error| control("job_event_failed", error.to_string()))?
        .sign_with_keys(&self.inner.keys)
        .map_err(|error| control("job_event_failed", error.to_string()))?;
        let mut outbox = outbox_event(
            &event,
            job.job_id,
            job.channel_id,
            KIND_JOB_PROGRESS as u16,
            false,
            created_at,
        )?;
        outbox.seq = Some(seq);
        Ok(outbox)
    }

    fn progress_summary(
        &self,
        job: &JobRecord,
        state: AgentJobProgressState,
        now: chrono::DateTime<Utc>,
    ) -> String {
        let started_at = job.started_at.unwrap_or(job.created_at);
        let elapsed = now.signed_duration_since(started_at).num_seconds().max(0);
        format!(
            "Legacy Harness runner {} (elapsed {elapsed}s)",
            state.as_str()
        )
    }

    fn build_accepted_event(
        &self,
        job: &JobRecord,
        accepted_at: chrono::DateTime<Utc>,
    ) -> Result<Event> {
        let payload = AgentJobAccepted {
            schema: AGENT_JOB_SCHEMA,
            job: job.job_id,
            attempt: job.attempt,
            state: AgentJobAcceptedState::Accepted,
            accepted_at,
        };
        Ok(buzz_sdk::builders::build_agent_job_accepted(
            job.channel_id,
            parse_requester(job)?,
            parse_request_event(job)?,
            &payload,
        )?
        .sign_with_keys(&self.inner.keys)?)
    }

    fn build_result_event(&self, job: &JobRecord, payload: &AgentJobResult) -> Result<Event> {
        Ok(buzz_sdk::builders::build_agent_job_result(
            job.channel_id,
            parse_requester(job)?,
            parse_request_event(job)?,
            payload,
        )?
        .sign_with_keys(&self.inner.keys)?)
    }

    fn build_error_event(&self, job: &JobRecord, payload: &AgentJobError) -> Result<Event> {
        Ok(buzz_sdk::builders::build_agent_job_error(
            job.channel_id,
            parse_requester(job)?,
            parse_request_event(job)?,
            payload,
        )?
        .sign_with_keys(&self.inner.keys)?)
    }
}

impl ControlHandler for JobSupervisor {
    fn handle(
        &self,
        capability: AuthorizedCapability,
        operation: ControlOperation,
    ) -> HandlerFuture<'_> {
        Box::pin(async move {
            match operation {
                ControlOperation::Hello => Ok(ControlPayload::Hello(buzz_runtime::HelloResponse {
                    runtime_id: self.runtime_id().to_string(),
                    generation: self.generation(),
                    capability: match capability {
                        AuthorizedCapability::Controller => "controller",
                        AuthorizedCapability::Model => "model",
                    }
                    .into(),
                })),
                ControlOperation::Status => self.runtime_status().await.map(ControlPayload::Status),
                ControlOperation::JobsList(filter) => {
                    self.list(filter).await.map(ControlPayload::Jobs)
                }
                ControlOperation::JobsStart(request) => match capability {
                    AuthorizedCapability::Controller => self.start(request).await,
                    AuthorizedCapability::Model => self.start_for_model(request).await,
                }
                .map(ControlPayload::Job),
                ControlOperation::JobsStatus { job_id } => self
                    .get(job_id)
                    .await
                    .map(|job| ControlPayload::Job(status(&job))),
                ControlOperation::JobsCancel { job_id } => {
                    self.cancel(job_id).await.map(ControlPayload::Job)
                }
                ControlOperation::JobsLogs { job_id, tail_lines } => self
                    .logs(job_id, tail_lines)
                    .await
                    .map(ControlPayload::Logs),
                ControlOperation::AssignmentSetState {
                    assignment_id,
                    request,
                } => self
                    .inner
                    .store
                    .set_assignment_state(&assignment_id, request, Utc::now())
                    .await
                    .map(ControlPayload::Assignment)
                    .map_err(store_error),
                ControlOperation::Reconcile => self.reconcile().await.map(ControlPayload::Jobs),
                ControlOperation::Shutdown => {
                    self.shutdown_runtime().await?;
                    Ok(ControlPayload::Ack)
                }
            }
        })
    }
}

fn transition(
    job: &JobRecord,
    next_state: JobState,
    runner: Option<RunnerIdentity>,
    occurred_at: chrono::DateTime<Utc>,
) -> JobTransition {
    JobTransition {
        job_id: job.job_id,
        attempt: job.attempt,
        next_state,
        runner,
        progress_seq: None,
        exit_code: None,
        result_json: None,
        error_code: None,
        terminal_event_id: None,
        publication_state: None,
        publication_error: None,
        occurred_at,
    }
}

fn status(job: &JobRecord) -> JobStatus {
    JobStatus {
        job_id: job.job_id,
        request_event_id: job.request_event_id.clone(),
        source_event_id: job.source_event_id.clone(),
        channel_id: job.channel_id,
        state: job.state,
        attempt: job.attempt,
        progress_seq: job.progress_seq,
        summary: job.summary.clone(),
        started_at: job.started_at,
        finished_at: job.finished_at,
        exit_code: job.exit_code,
        error_code: job.error_code.clone(),
        publication_state: job.publication_state,
        runner_pid: job.runner.as_ref().map(|runner| runner.pid),
        runner_start_marker: job
            .runner
            .as_ref()
            .map(|runner| runner.start_marker.clone()),
    }
}

fn outbox_event(
    event: &Event,
    job_id: JobId,
    channel_id: Uuid,
    kind: u16,
    is_terminal: bool,
    created_at: chrono::DateTime<Utc>,
) -> Result<OutboxEvent, ControlError> {
    Ok(OutboxEvent {
        event_id: event.id.to_hex(),
        job_id: Some(job_id),
        channel_id,
        ordering_key: format!("job:{job_id}"),
        kind,
        seq: None,
        is_terminal,
        event_json: serde_json::to_string(event)
            .map_err(|error| control("job_event_failed", error.to_string()))?,
        created_at,
    })
}

fn parse_request_event(job: &JobRecord) -> Result<EventId> {
    let value = job
        .request_event_id
        .as_deref()
        .context("job has no request event id")?;
    EventId::from_hex(value).context("job request event id is invalid")
}

fn parse_requester(job: &JobRecord) -> Result<PublicKey> {
    PublicKey::from_hex(&job.requester_pubkey).context("job requester pubkey is invalid")
}

fn protocol_error(error: buzz_runtime::ProtocolError) -> ControlError {
    match error {
        buzz_runtime::ProtocolError::UnsupportedDriver => {
            control("unsupported_driver", "only driver lh is supported")
        }
        other => control("invalid_job_request", other.to_string()),
    }
}
fn bind_model_request(
    mut request: JobStartRequest,
    assignment: &AssignmentRecord,
) -> Result<JobStartRequest, ControlError> {
    let source_event_id = assignment.source_event_id.clone().ok_or_else(|| {
        control(
            "assignment_required",
            "active assignment has no source event",
        )
    })?;
    if request.channel_id != assignment.channel_id
        || request
            .source_event_id
            .as_ref()
            .is_some_and(|source| source != &source_event_id)
    {
        return Err(control(
            "assignment_mismatch",
            "job channel or source does not match the active assignment",
        ));
    }
    request.source_event_id = Some(source_event_id);
    Ok(request)
}

fn store_error(error: buzz_runtime::StoreError) -> ControlError {
    match error {
        buzz_runtime::StoreError::ActiveJobExists => {
            control("job_busy", "a privileged job is already active")
        }
        buzz_runtime::StoreError::AssignmentJobMismatch => control(
            "assignment_mismatch",
            "job no longer matches the current active assignment",
        ),
        other => control("persistence_failed", other.to_string()),
    }
}

fn control(code: impl Into<String>, message: impl Into<String>) -> ControlError {
    ControlError::new(code, message)
}

fn truncate_utf8(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_string();
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn verify_identity(identity: &RunnerIdentity) -> Result<()> {
    if identity.pid == 0 || !process_matches_marker(identity.pid, &identity.start_marker) {
        anyhow::bail!("runner start marker mismatch");
    }
    #[cfg(unix)]
    {
        let expected = identity.pid.to_string();
        if identity.process_group != expected {
            anyhow::bail!("runner process-group receipt mismatch");
        }
        let actual = nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(identity.pid as i32)))?;
        if actual.as_raw() != identity.pid as i32 {
            anyhow::bail!("runner is not process-group leader");
        }
    }
    #[cfg(windows)]
    crate::job_windows::verify_member(&identity.process_group, identity.pid)
        .context("runner is not a verified member of its named Job Object")?;
    Ok(())
}

async fn terminate_verified_tree(identity: &RunnerIdentity) -> Result<(), ControlError> {
    verify_identity(identity).map_err(|_| {
        control(
            "runner_identity_mismatch",
            "runner identity changed before cancellation; no process was signalled",
        )
    })?;
    #[cfg(unix)]
    {
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        let pgid = Pid::from_raw(identity.pid as i32);
        killpg(pgid, Signal::SIGTERM)
            .map_err(|error| control("runner_cancel_failed", error.to_string()))?;
        let graceful_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < graceful_deadline && process_group_alive(pgid)? {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if process_group_alive(pgid)? {
            killpg(pgid, Signal::SIGKILL)
                .map_err(|error| control("runner_cancel_failed", error.to_string()))?;
        }
        let forced_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < forced_deadline && process_group_alive(pgid)? {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if process_group_alive(pgid)? {
            return Err(control(
                "runner_cancel_failed",
                "verified runner process group still has live members",
            ));
        }
    }
    #[cfg(windows)]
    {
        crate::job_windows::terminate_verified(&identity.process_group, identity.pid)
            .map_err(|error| control("runner_cancel_failed", error.to_string()))?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline
            && !crate::job_windows::is_empty(&identity.process_group)
                .map_err(|error| control("runner_cancel_failed", error.to_string()))?
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        if !crate::job_windows::is_empty(&identity.process_group)
            .map_err(|error| control("runner_cancel_failed", error.to_string()))?
        {
            return Err(control(
                "runner_cancel_failed",
                "verified runner Job Object still has live members",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn process_group_alive(pgid: nix::unistd::Pid) -> Result<bool, ControlError> {
    crate::job_runner::process_group_has_live_members(pgid.as_raw() as u32, None)
        .map_err(|error| control("runner_cancel_failed", error.to_string()))
}

// The unix arm returns explicitly so the windows arm below can never become the
// accidental fallthrough value on a future platform edit.
#[allow(clippy::needless_return)]
fn recorded_tree_is_empty(identity: &RunnerIdentity) -> Result<bool, ControlError> {
    #[cfg(unix)]
    {
        if identity.process_group != identity.pid.to_string() {
            return Err(control(
                "shutdown_reconciliation_required",
                "recorded runner process-group identity is invalid",
            ));
        }
        return process_group_alive(nix::unistd::Pid::from_raw(identity.pid as i32))
            .map(|alive| !alive);
    }
    #[cfg(windows)]
    {
        crate::job_windows::is_empty(&identity.process_group)
            .map_err(|error| control("shutdown_reconciliation_required", error.to_string()))
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::agent_job::AgentJobCancel;
    fn remote_fixture(
        agent_keys: Keys,
    ) -> (tempfile::TempDir, JobSupervisor, ManagedRuntimeConfig) {
        let directory = tempfile::tempdir().expect("create runtime directory");
        let state_dir = directory.path().canonicalize().expect("canonical state");
        let executable = std::env::current_exe()
            .expect("current executable")
            .canonicalize()
            .expect("canonical executable");
        let runtime = ManagedRuntimeConfig {
            runtime_id: "remote-test-runtime".into(),
            receipt_path: state_dir.join("runtime.json"),
            lh_executable: Some(executable),
            workspace_roots: vec![state_dir.clone()],
            lock_path_hash: "a".repeat(64),
            state_dir: state_dir.clone(),
        };
        let store = StoreHandle::open(state_dir.join("state").join("runtime.sqlite3"))
            .expect("open runtime store");
        let (shutdown_tx, _) = watch::channel(false);
        let supervisor = JobSupervisor::new(
            runtime.clone(),
            store,
            agent_keys,
            Uuid::new_v4(),
            shutdown_tx,
        )
        .expect("create supervisor");
        (directory, supervisor, runtime)
    }

    fn supervisor_for_runtime(
        runtime: ManagedRuntimeConfig,
        agent_keys: Keys,
    ) -> (JobSupervisor, StoreHandle) {
        let store = StoreHandle::open(
            runtime
                .state_dir
                .join(format!("optional-jobs-{}.sqlite3", Uuid::new_v4())),
        )
        .expect("open optional job store");
        let (shutdown_tx, _) = watch::channel(false);
        let supervisor = JobSupervisor::new(
            runtime,
            store.clone(),
            agent_keys,
            Uuid::new_v4(),
            shutdown_tx,
        )
        .expect("create optional job supervisor");
        (supervisor, store)
    }

    fn local_request(cwd: &Path) -> JobStartRequest {
        JobStartRequest {
            channel_id: Uuid::new_v4(),
            source_event_id: None,
            driver: "lh".into(),
            argv: vec!["lockdown".into(), "run".into()],
            cwd: cwd.to_string_lossy().into_owned(),
            summary: "optional driver test".into(),
        }
    }

    #[tokio::test]
    async fn local_start_reports_unavailable_optional_job_configuration_before_persistence() {
        let agent = Keys::generate();
        let (_directory, _configured, runtime) = remote_fixture(agent.clone());

        let mut no_driver = runtime.clone();
        no_driver.lh_executable = None;
        let (supervisor, store) = supervisor_for_runtime(no_driver, agent.clone());
        let error = supervisor
            .start(local_request(&runtime.state_dir))
            .await
            .expect_err("missing LH must fail the privileged start");
        assert_eq!(error.code, "driver_unavailable");
        assert!(store
            .list_jobs(JobListFilter::default())
            .await
            .expect("list jobs")
            .is_empty());

        let mut no_roots = runtime;
        no_roots.workspace_roots.clear();
        let cwd = no_roots.state_dir.clone();
        let (supervisor, store) = supervisor_for_runtime(no_roots, agent);
        let error = supervisor
            .start(local_request(&cwd))
            .await
            .expect_err("missing roots must fail the privileged start");
        assert_eq!(error.code, "workspace_not_allowed");
        assert!(store
            .list_jobs(JobListFilter::default())
            .await
            .expect("list jobs")
            .is_empty());
    }
    #[tokio::test]
    async fn local_start_maps_durable_admission_conflict_without_orphan_event() {
        let agent = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent);
        let active_id = Uuid::new_v4();
        supervisor
            .inner
            .store
            .create_remote_job(NewJob {
                job_id: active_id,
                request_event_id: format!("request-{active_id}"),
                requester_pubkey: "requester".into(),
                executable: runtime
                    .lh_executable
                    .clone()
                    .expect("configured LH executable"),
                request: local_request(&runtime.state_dir),
                attempt: 1,
                created_at: Utc::now(),
            })
            .await
            .expect("seed active durable job");

        let error = supervisor
            .start(local_request(&runtime.state_dir))
            .await
            .expect_err("second privileged job must be rejected before runner spawn");
        assert_eq!(error.code, "job_busy");
        assert_eq!(error.message, "a privileged job is already active");
        assert!(error.message.len() <= 64);
        assert_eq!(
            supervisor
                .inner
                .store
                .list_jobs(JobListFilter::default())
                .await
                .expect("list jobs")
                .len(),
            1
        );
        assert!(
            supervisor
                .inner
                .store
                .pending_outbox(10, Utc::now())
                .await
                .expect("read outbox")
                .is_empty(),
            "rejected local request must not leave a request event chain"
        );
    }
    #[tokio::test]
    async fn model_job_start_requires_and_matches_current_assignment_before_persistence() {
        let agent = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent);

        let error = supervisor
            .handle(
                AuthorizedCapability::Model,
                ControlOperation::JobsStart(local_request(&runtime.state_dir)),
            )
            .await
            .expect_err("model start without an assignment must fail");
        assert_eq!(error.code, "assignment_required");

        let channel_id = Uuid::new_v4();
        let source_event_id = EventId::from_byte_array([11; 32]).to_hex();
        let assignment = supervisor
            .inner
            .store
            .claim_assignment(
                channel_id,
                Some(source_event_id.clone()),
                "current model assignment".into(),
                None,
                Utc::now(),
            )
            .await
            .expect("claim current assignment");
        let mut unbound = local_request(&runtime.state_dir);
        unbound.channel_id = channel_id;
        let bound = bind_model_request(unbound, &assignment)
            .expect("missing source is safely bound to the assignment");
        assert_eq!(
            bound.source_event_id.as_deref(),
            Some(source_event_id.as_str())
        );

        let mut wrong_channel = local_request(&runtime.state_dir);
        wrong_channel.source_event_id = Some(source_event_id.clone());
        let error = supervisor
            .handle(
                AuthorizedCapability::Model,
                ControlOperation::JobsStart(wrong_channel),
            )
            .await
            .expect_err("arbitrary channel must fail");
        assert_eq!(error.code, "assignment_mismatch");

        let mut wrong_source = local_request(&runtime.state_dir);
        wrong_source.channel_id = channel_id;
        wrong_source.source_event_id = Some(EventId::from_byte_array([12; 32]).to_hex());
        let error = supervisor
            .handle(
                AuthorizedCapability::Model,
                ControlOperation::JobsStart(wrong_source),
            )
            .await
            .expect_err("arbitrary source event must fail");
        assert_eq!(error.code, "assignment_mismatch");
        assert!(supervisor
            .inner
            .store
            .list_jobs(JobListFilter::default())
            .await
            .expect("list jobs")
            .is_empty());
        assert!(
            supervisor
                .inner
                .store
                .pending_outbox(10, Utc::now())
                .await
                .expect("read outbox")
                .is_empty(),
            "failed model admission must create neither request nor job event"
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_supervisors_admit_and_spawn_exactly_one_distinct_job() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("create runtime directory");
        let state_dir = directory.path().canonicalize().expect("canonical state");
        let counter_path = state_dir.join("runner-spawns");
        let runner_path = state_dir.join("counting-runner");
        std::fs::write(
            &runner_path,
            format!(
                "#!/bin/sh\nprintf x >> '{}'\nsleep 30\n",
                counter_path.display()
            ),
        )
        .expect("write counting runner");
        std::fs::set_permissions(&runner_path, std::fs::Permissions::from_mode(0o700))
            .expect("make counting runner executable");
        let executable = std::env::current_exe()
            .expect("current executable")
            .canonicalize()
            .expect("canonical executable");
        let runtime = ManagedRuntimeConfig {
            runtime_id: "concurrent-admission-test".into(),
            receipt_path: state_dir.join("runtime.json"),
            lh_executable: Some(executable),
            workspace_roots: vec![state_dir.clone()],
            lock_path_hash: "a".repeat(64),
            state_dir: state_dir.clone(),
        };
        let store = StoreHandle::open(state_dir.join("state").join("runtime.sqlite3"))
            .expect("open shared runtime store");
        let agent = Keys::generate();
        let (first_shutdown, _) = watch::channel(false);
        let (second_shutdown, _) = watch::channel(false);
        let first = JobSupervisor::new_with_runner_executable(
            runtime.clone(),
            store.clone(),
            agent.clone(),
            Uuid::new_v4(),
            first_shutdown,
            runner_path.clone(),
        )
        .expect("create first supervisor");
        let second = JobSupervisor::new_with_runner_executable(
            runtime.clone(),
            store.clone(),
            agent,
            Uuid::new_v4(),
            second_shutdown,
            runner_path,
        )
        .expect("create second supervisor");

        let (first_result, second_result) = tokio::join!(
            first.start(local_request(&state_dir)),
            second.start(local_request(&state_dir))
        );
        let results = [first_result, second_result];
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(error) if error.code == "job_busy"))
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Ok(status) if status.state == JobState::Failed))
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(&counter_path).expect("read spawn counter"),
            "x",
            "durable admission must precede runner creation"
        );
        assert_eq!(
            store
                .list_jobs(JobListFilter::default())
                .await
                .expect("list jobs")
                .len(),
            1
        );
    }

    fn signed_remote_request(
        requester: &Keys,
        target: PublicKey,
        channel_id: Uuid,
        job_id: Uuid,
        cwd: &Path,
    ) -> Event {
        let payload = AgentJobRequest {
            schema: AGENT_JOB_SCHEMA,
            driver: "lh".into(),
            argv: vec!["lockdown".into(), "run".into()],
            cwd: cwd.to_string_lossy().into_owned(),
            summary: "remote request".into(),
        };
        buzz_sdk::builders::build_agent_job_request(
            channel_id, target, job_id, None, None, &payload,
        )
        .expect("build remote request")
        .sign_with_keys(requester)
        .expect("sign remote request")
    }
    fn signed_remote_cancel(
        canceller: &Keys,
        target: PublicKey,
        channel_id: Uuid,
        job_id: Uuid,
        request_event_id: EventId,
    ) -> Event {
        let payload = AgentJobCancel {
            schema: AGENT_JOB_SCHEMA,
            job: job_id,
            reason: "stop before replay".into(),
        };
        buzz_sdk::builders::build_agent_job_cancel(channel_id, target, request_event_id, &payload)
            .expect("build remote cancel")
            .sign_with_keys(canceller)
            .expect("sign remote cancel")
    }

    #[tokio::test]
    async fn cancel_before_request_replay_is_terminal_without_runner_spawn() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent.clone());
        let channel_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let request = signed_remote_request(
            &requester,
            agent.public_key(),
            channel_id,
            job_id,
            &runtime.state_dir,
        );
        let cancel = signed_remote_cancel(
            &requester,
            agent.public_key(),
            channel_id,
            job_id,
            request.id,
        );

        let unauthorized = supervisor
            .apply_remote_cancel(&cancel, false, true, None)
            .await
            .expect_err("unproved inbound author must not create a tombstone");
        assert_eq!(unauthorized.code, "unauthorized_remote_job");
        assert!(supervisor
            .inner
            .store
            .remote_cancels(job_id, request.id.to_hex())
            .await
            .expect("query tombstones")
            .is_empty());

        let pending = supervisor
            .apply_remote_cancel(&cancel, true, true, None)
            .await
            .expect("authenticated cancel is durable");
        assert_eq!(pending.state, JobState::Cancelled);
        assert_eq!(
            pending.error_code.as_deref(),
            Some("cancel_pending_request")
        );
        let duplicate = supervisor
            .apply_remote_cancel(&cancel, true, true, None)
            .await
            .expect("replayed cancel remains idempotent");
        assert_eq!(duplicate.state, JobState::Cancelled);
        assert_eq!(
            supervisor
                .inner
                .store
                .remote_cancels(job_id, request.id.to_hex())
                .await
                .expect("query deduplicated tombstone")
                .len(),
            1
        );

        let status = supervisor
            .start_remote_request(&request, true, true)
            .await
            .expect("replayed request reconciles tombstone");
        assert_eq!(status.state, JobState::Cancelled);
        assert_eq!(
            status.error_code.as_deref(),
            Some("cancelled_before_request")
        );
        assert!(status.runner_pid.is_none());

        let stored = supervisor
            .inner
            .store
            .get_job(job_id)
            .await
            .expect("read job")
            .expect("cancelled job persisted");
        assert_eq!(stored.state, JobState::Cancelled);
        assert!(stored.runner.is_none());
        assert!(stored.started_at.is_none());
        assert!(stored.finished_at.is_some());
        assert!(!runtime
            .state_dir
            .join("jobs")
            .join(job_id.to_string())
            .exists());
        assert!(supervisor
            .inner
            .store
            .remote_cancels(job_id, request.id.to_hex())
            .await
            .expect("query consumed tombstones")
            .is_empty());
        let outbox = supervisor
            .inner
            .store
            .pending_outbox(10, Utc::now())
            .await
            .expect("read terminal outbox");
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].event.kind, KIND_JOB_ERROR as u16);
        assert!(outbox[0].event.is_terminal);

        let replay = supervisor
            .start_remote_request(&request, true, true)
            .await
            .expect("duplicate request remains terminal");
        assert_eq!(replay.state, JobState::Cancelled);
        assert!(replay.runner_pid.is_none());
    }

    #[tokio::test]
    async fn remote_request_rechecks_author_and_membership_before_persistence_or_spawn() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent.clone());
        let job_id = Uuid::new_v4();
        let event = signed_remote_request(
            &requester,
            agent.public_key(),
            Uuid::new_v4(),
            job_id,
            &runtime.state_dir,
        );

        let error = supervisor
            .start_remote_request(&event, false, true)
            .await
            .expect_err("unauthorized requester must fail");
        assert_eq!(error.code, "unauthorized_remote_job");
        assert!(
            supervisor
                .inner
                .store
                .get_job(job_id)
                .await
                .expect("read job")
                .is_none(),
            "authority rejection must precede persistence"
        );

        let nonmember_job_id = Uuid::new_v4();
        let nonmember_event = signed_remote_request(
            &requester,
            agent.public_key(),
            Uuid::new_v4(),
            nonmember_job_id,
            &runtime.state_dir,
        );
        let error = supervisor
            .start_remote_request(&nonmember_event, true, false)
            .await
            .expect_err("nonmember requester must fail");
        assert_eq!(error.code, "unauthorized_remote_job");
        assert!(
            supervisor
                .inner
                .store
                .get_job(nonmember_job_id)
                .await
                .expect("read nonmember job")
                .is_none(),
            "membership rejection must precede persistence"
        );
        assert!(supervisor
            .inner
            .store
            .list_jobs(JobListFilter::default())
            .await
            .expect("list jobs")
            .is_empty());
        assert!(
            !runtime.state_dir.join("jobs").exists(),
            "membership rejection must not create a job spec or runner"
        );
    }
    #[tokio::test]
    async fn unrelated_remote_request_cannot_replace_active_assignment() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent.clone());
        let channel_id = Uuid::new_v4();
        let active_source = EventId::from_byte_array([7; 32]).to_hex();
        supervisor
            .inner
            .store
            .claim_assignment(
                channel_id,
                Some(active_source),
                "active work".into(),
                None,
                Utc::now(),
            )
            .await
            .expect("claim active assignment");
        let job_id = Uuid::new_v4();
        let event = signed_remote_request(
            &requester,
            agent.public_key(),
            channel_id,
            job_id,
            &runtime.state_dir,
        );

        let error = supervisor
            .start_remote_request(&event, true, true)
            .await
            .expect_err("unrelated remote request must not replace active work");
        assert_eq!(error.code, "assignment_busy");
        assert!(supervisor
            .inner
            .store
            .get_job(job_id)
            .await
            .expect("read job")
            .is_none());
    }

    #[tokio::test]
    async fn member_remote_request_is_admitted_idempotently_without_runner_spawn() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent.clone());
        let channel_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let event = signed_remote_request(
            &requester,
            agent.public_key(),
            channel_id,
            job_id,
            &runtime.state_dir,
        );
        let request = JobStartRequest {
            channel_id,
            source_event_id: None,
            driver: "lh".into(),
            argv: vec!["lockdown".into(), "run".into()],
            cwd: runtime.state_dir.to_string_lossy().into_owned(),
            summary: "remote request".into(),
        };
        supervisor
            .inner
            .store
            .create_remote_job(NewJob {
                job_id,
                request_event_id: event.id.to_hex(),
                requester_pubkey: requester.public_key().to_hex(),
                executable: runtime
                    .lh_executable
                    .clone()
                    .expect("configured LH executable"),
                request,
                attempt: 1,
                created_at: Utc::now(),
            })
            .await
            .expect("seed requested job");

        let status = supervisor
            .start_remote_request(&event, true, true)
            .await
            .expect("duplicate request returns existing");
        assert_eq!(status.job_id, job_id);
        assert_eq!(status.state, JobState::Requested);
        assert_eq!(
            supervisor
                .inner
                .store
                .list_jobs(JobListFilter::default())
                .await
                .expect("list jobs")
                .len(),
            1
        );
        assert!(
            !runtime.state_dir.join("jobs").exists(),
            "idempotent admission must not spawn a second runner"
        );
    }

    #[tokio::test]
    async fn remote_cancel_rechecks_request_link_and_canceller_authority() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let intruder = Keys::generate();
        let owner = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent.clone());
        let channel_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        let request_event = signed_remote_request(
            &requester,
            agent.public_key(),
            channel_id,
            job_id,
            &runtime.state_dir,
        );
        supervisor
            .inner
            .store
            .create_remote_job(NewJob {
                job_id,
                request_event_id: request_event.id.to_hex(),
                requester_pubkey: requester.public_key().to_hex(),
                executable: runtime
                    .lh_executable
                    .clone()
                    .expect("configured LH executable"),
                request: JobStartRequest {
                    channel_id,
                    source_event_id: None,
                    driver: "lh".into(),
                    argv: vec!["lockdown".into(), "run".into()],
                    cwd: runtime.state_dir.to_string_lossy().into_owned(),
                    summary: "remote request".into(),
                },
                attempt: 1,
                created_at: Utc::now(),
            })
            .await
            .expect("seed requested job");
        let cancel_payload = AgentJobCancel {
            schema: AGENT_JOB_SCHEMA,
            job: job_id,
            reason: "stop".into(),
        };
        let wrong_link = buzz_sdk::builders::build_agent_job_cancel(
            channel_id,
            agent.public_key(),
            EventId::all_zeros(),
            &cancel_payload,
        )
        .expect("build wrong-link cancel")
        .sign_with_keys(&requester)
        .expect("sign wrong-link cancel");
        let owner_pubkey = owner.public_key();
        let error = supervisor
            .apply_remote_cancel(&wrong_link, true, true, Some(&owner_pubkey))
            .await
            .expect_err("wrong request link must fail");
        assert_eq!(error.code, "unauthorized_job_cancel");

        let unauthorized = buzz_sdk::builders::build_agent_job_cancel(
            channel_id,
            agent.public_key(),
            request_event.id,
            &cancel_payload,
        )
        .expect("build unauthorized cancel")
        .sign_with_keys(&intruder)
        .expect("sign unauthorized cancel");
        let error = supervisor
            .apply_remote_cancel(&unauthorized, true, true, Some(&owner_pubkey))
            .await
            .expect_err("non-requester, non-owner, non-target must fail");
        assert_eq!(error.code, "unauthorized_job_cancel");
        assert_eq!(
            supervisor
                .inner
                .store
                .get_job(job_id)
                .await
                .expect("read job")
                .expect("job exists")
                .state,
            JobState::Requested
        );
    }

    #[tokio::test]
    async fn progress_event_and_sequence_commit_atomically() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent);
        let job_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let created_at = Utc::now();
        supervisor
            .inner
            .store
            .create_remote_job(NewJob {
                job_id,
                request_event_id: EventId::all_zeros().to_hex(),
                requester_pubkey: requester.public_key().to_hex(),
                executable: runtime
                    .lh_executable
                    .clone()
                    .expect("configured LH executable"),
                request: JobStartRequest {
                    channel_id,
                    source_event_id: None,
                    driver: "lh".into(),
                    argv: vec!["run".into()],
                    cwd: runtime.state_dir.to_string_lossy().into_owned(),
                    summary: "progress test".into(),
                },
                attempt: 1,
                created_at,
            })
            .await
            .expect("seed requested job");
        let requested = supervisor
            .inner
            .store
            .get_job(job_id)
            .await
            .expect("read requested job")
            .expect("requested job exists");
        let accepted = supervisor
            .inner
            .store
            .transition_job(
                transition(&requested, JobState::Accepted, None, created_at),
                None,
            )
            .await
            .expect("accept seeded job");
        let running = supervisor
            .record_progress(
                &accepted,
                JobState::Running,
                AgentJobProgressState::Running,
                None,
                "Legacy Harness runner running (elapsed 0s)".into(),
                created_at,
            )
            .await
            .expect("commit running progress");
        assert_eq!(running.state, JobState::Running);
        assert_eq!(running.progress_seq, 1);
        let outbox = supervisor
            .inner
            .store
            .pending_outbox(10, Utc::now())
            .await
            .expect("read progress outbox");
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].event.kind, KIND_JOB_PROGRESS as u16);
        assert_eq!(outbox[0].event.seq, Some(1));
        let event: Event = serde_json::from_str(&outbox[0].event.event_json)
            .expect("decode signed progress event");
        let parsed = parse_agent_job_event(&event).expect("validate signed progress event");
        match parsed.payload {
            AgentJobPayload::Progress(payload) => {
                assert_eq!(payload.job, job_id);
                assert_eq!(payload.seq, 1);
                assert_eq!(payload.state, AgentJobProgressState::Running);
            }
            payload => panic!("expected progress payload, got {payload:?}"),
        }
    }

    #[tokio::test]
    async fn shutdown_persists_lost_and_refuses_ack_for_unverified_runner() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent);
        let store = supervisor.inner.store.clone();
        let shutdown_rx = supervisor.inner.shutdown_tx.subscribe();
        let job_id = Uuid::new_v4();
        let created_at = Utc::now();
        store
            .create_remote_job(NewJob {
                job_id,
                request_event_id: EventId::all_zeros().to_hex(),
                requester_pubkey: requester.public_key().to_hex(),
                executable: runtime
                    .lh_executable
                    .clone()
                    .expect("configured LH executable"),
                request: JobStartRequest {
                    channel_id: Uuid::new_v4(),
                    source_event_id: None,
                    driver: "lh".into(),
                    argv: vec!["run".into()],
                    cwd: runtime.state_dir.to_string_lossy().into_owned(),
                    summary: "unverified shutdown runner".into(),
                },
                attempt: 1,
                created_at,
            })
            .await
            .expect("seed requested job");
        let requested = store
            .get_job(job_id)
            .await
            .expect("read requested job")
            .expect("requested job exists");
        store
            .transition_job(
                transition(
                    &requested,
                    JobState::Running,
                    Some(RunnerIdentity {
                        pid: std::process::id(),
                        start_marker: "forged-start-marker".into(),
                        process_group: std::process::id().to_string(),
                    }),
                    created_at,
                ),
                None,
            )
            .await
            .expect("record unverified runner");

        let error = supervisor
            .handle(AuthorizedCapability::Controller, ControlOperation::Shutdown)
            .await
            .expect_err("unverified runner must prevent shutdown acknowledgement");
        assert_eq!(error.code, "shutdown_runner_identity_unverified");
        let terminal = store
            .get_job(job_id)
            .await
            .expect("read terminal job")
            .expect("terminal job exists");
        assert_eq!(terminal.state, JobState::Lost);
        assert_eq!(
            terminal.error_code.as_deref(),
            Some("shutdown_runner_identity_unverified")
        );
        assert!(
            !*shutdown_rx.borrow(),
            "failed reconciliation must leave the runtime active"
        );
    }

    #[tokio::test]
    async fn cancellation_refuses_unverified_process_identity_without_signalling() {
        let identity = RunnerIdentity {
            pid: std::process::id(),
            start_marker: "forged-start-marker".into(),
            process_group: std::process::id().to_string(),
        };
        let error = terminate_verified_tree(&identity)
            .await
            .expect_err("forged identity must fail closed");
        assert_eq!(error.code, "runner_identity_mismatch");
        assert!(
            process_start_marker(std::process::id()).is_ok(),
            "the current test process must still be alive"
        );
    }

    #[tokio::test]
    async fn shutdown_imports_terminal_receipt_before_marking_runner_lost() {
        let agent = Keys::generate();
        let requester = Keys::generate();
        let (_directory, supervisor, runtime) = remote_fixture(agent);
        let store = supervisor.inner.store.clone();
        let _shutdown_rx = supervisor.inner.shutdown_tx.subscribe();
        let job_id = Uuid::new_v4();
        let argv = vec!["run".into()];
        let created_at = Utc::now();
        store
            .create_remote_job(NewJob {
                job_id,
                request_event_id: EventId::all_zeros().to_hex(),
                requester_pubkey: requester.public_key().to_hex(),
                executable: runtime
                    .lh_executable
                    .clone()
                    .expect("configured LH executable"),
                request: JobStartRequest {
                    channel_id: Uuid::new_v4(),
                    source_event_id: None,
                    driver: "lh".into(),
                    argv: argv.clone(),
                    cwd: runtime.state_dir.to_string_lossy().into_owned(),
                    summary: "settled before shutdown".into(),
                },
                attempt: 1,
                created_at,
            })
            .await
            .expect("seed requested job");
        let requested = store
            .get_job(job_id)
            .await
            .expect("read requested job")
            .expect("requested job exists");
        store
            .transition_job(
                transition(
                    &requested,
                    JobState::Running,
                    Some(RunnerIdentity {
                        pid: std::process::id(),
                        start_marker: "runner-already-exited".into(),
                        process_group: std::process::id().to_string(),
                    }),
                    created_at,
                ),
                None,
            )
            .await
            .expect("record stale running projection");
        buzz_runtime::write_runner_receipt(
            &runtime.state_dir,
            &RunnerReceipt {
                schema_version: buzz_runtime::RUNNER_RECEIPT_SCHEMA_VERSION,
                job_id,
                attempt: 1,
                state: RunnerReceiptState::Succeeded,
                runner_pid: std::process::id(),
                runner_start_marker: "runner-already-exited".into(),
                process_group: std::process::id().to_string(),
                argv_sha256: argv_sha256(&argv).expect("hash argv"),
                started_at: created_at,
                finished_at: Some(Utc::now()),
                exit_code: Some(0),
                error_code: None,
            },
        )
        .expect("write terminal runner receipt");

        let response = supervisor
            .handle(AuthorizedCapability::Controller, ControlOperation::Shutdown)
            .await
            .expect("shutdown imports settled receipt");
        assert!(matches!(response, ControlPayload::Ack));
        assert_eq!(
            store
                .get_job(job_id)
                .await
                .expect("read terminal job")
                .expect("terminal job exists")
                .state,
            JobState::Succeeded
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_shutdown_reaps_active_runner_tree_before_acknowledging() {
        use std::os::unix::process::CommandExt;

        let directory = tempfile::tempdir().expect("create runtime directory");
        let state_dir = directory.path().canonicalize().expect("canonical state");
        let executable = std::env::current_exe()
            .expect("current executable")
            .canonicalize()
            .expect("canonical executable");
        let runtime = ManagedRuntimeConfig {
            runtime_id: "shutdown-test-runtime".into(),
            receipt_path: state_dir.join("runtime.json"),
            lh_executable: Some(executable.clone()),
            workspace_roots: vec![state_dir.clone()],
            lock_path_hash: "a".repeat(64),
            state_dir: state_dir.clone(),
        };
        let store = StoreHandle::open(state_dir.join("state").join("runtime.sqlite3"))
            .expect("open runtime store");
        let agent = Keys::generate();
        let requester = Keys::generate();
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let supervisor = JobSupervisor::new_with_runner_executable(
            runtime.clone(),
            store.clone(),
            agent,
            Uuid::new_v4(),
            shutdown_tx,
            executable,
        )
        .expect("create supervisor");

        let descendant_pid_path = state_dir.join("shutdown-descendant.pid");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("sleep 30 & printf '%s' \"$!\" > \"$1\"; wait")
            .arg("buzz-shutdown-test")
            .arg(&descendant_pid_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        let mut runner = command.spawn().expect("spawn runner tree");
        let runner_pid = runner.id();
        let runner_marker =
            process_start_marker(runner_pid).expect("capture runner process identity");
        let reaper = std::thread::spawn(move || runner.wait());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !descendant_pid_path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _descendant_pid: u32 = std::fs::read_to_string(&descendant_pid_path)
            .expect("runner recorded descendant pid")
            .parse()
            .expect("parse descendant pid");

        let job_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        store
            .create_remote_job(NewJob {
                job_id,
                request_event_id: EventId::all_zeros().to_hex(),
                requester_pubkey: requester.public_key().to_hex(),
                executable: runtime
                    .lh_executable
                    .clone()
                    .expect("configured LH executable"),
                request: JobStartRequest {
                    channel_id,
                    source_event_id: None,
                    driver: "lh".into(),
                    argv: vec!["run".into()],
                    cwd: state_dir.to_string_lossy().into_owned(),
                    summary: "active shutdown test".into(),
                },
                attempt: 1,
                created_at: Utc::now(),
            })
            .await
            .expect("seed requested job");
        let requested = store
            .get_job(job_id)
            .await
            .expect("read requested job")
            .expect("requested job exists");
        store
            .transition_job(
                transition(
                    &requested,
                    JobState::Running,
                    Some(RunnerIdentity {
                        pid: runner_pid,
                        start_marker: runner_marker.clone(),
                        process_group: runner_pid.to_string(),
                    }),
                    Utc::now(),
                ),
                None,
            )
            .await
            .expect("record running job");

        let response = supervisor
            .handle(AuthorizedCapability::Controller, ControlOperation::Shutdown)
            .await
            .expect("explicit shutdown succeeds");
        assert!(matches!(response, ControlPayload::Ack));
        shutdown_rx
            .changed()
            .await
            .expect("shutdown signal remains observable");
        assert!(*shutdown_rx.borrow());
        assert_eq!(
            store
                .get_job(job_id)
                .await
                .expect("read terminal job")
                .expect("terminal job exists")
                .state,
            JobState::Cancelled
        );
        let _ = reaper.join();
        assert!(
            !process_matches_marker(runner_pid, &runner_marker),
            "shutdown acknowledgement must follow runner termination"
        );
        assert!(
            !crate::job_runner::process_group_has_live_members(runner_pid, None)
                .expect("inspect terminated runner process group"),
            "shutdown acknowledgement must follow descendant termination"
        );
    }
}
