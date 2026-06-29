#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-workflow` — Workflow engine for Buzz.
//!
//! Channel-scoped automations with sequential execution, variable substitution,
//! conditional logic, and execution traces.
//!
//! ## Architecture
//!
//! - [`WorkflowEngine`] — top-level handle; lives in `AppState`
//! - [`schema`] — YAML/JSON definition types (`WorkflowDef`, `TriggerDef`, `ActionDef`, `Step`)
//! - [`executor`] — sequential execution, template resolution, condition evaluation
//! - [`error`] — [`WorkflowError`] enum
//!
//! ## Usage
//!
//! ```rust,ignore
//! let engine = Arc::new(WorkflowEngine::new(db, WorkflowConfig::default()));
//!
//! // Parse and validate a YAML definition.
//! let (def, json) = WorkflowEngine::parse_yaml(yaml_str)?;
//!
//! // React to an incoming event (called from event handler post-store hook).
//! // The community is the event's server-resolved tenant, threaded from the
//! // relay's bound `TenantContext` — the same workflow UUID can exist in two
//! // communities, so execution is always scoped to its owner.
//! engine.on_event(community_id, &stored_event).await?;
//!
//! // Run the background scheduler (cron triggers).
//! tokio::spawn(async move { engine.run().await });
//! ```

pub mod action_sink;
pub mod error;
pub mod executor;
pub mod schema;

pub use action_sink::{ActionSink, ActionSinkError};
pub use error::{PartialProgress, WorkflowError};
pub use executor::ExecutionResult;
pub use schema::{ActionDef, Step, TriggerDef, WorkflowDef};

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use buzz_core::kind::{event_kind_u32, is_workflow_execution_kind, KIND_REACTION};
use buzz_core::tenant::CommunityId;
use buzz_db::workflow::RunStatus;
use buzz_db::Db;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use tokio::sync::Semaphore;
use uuid::Uuid;

/// Runtime configuration for the workflow engine.
#[derive(Clone, Debug)]
pub struct WorkflowConfig {
    /// Maximum number of concurrently executing workflow runs. Default: 100.
    pub max_concurrent: usize,
    /// Default per-step timeout in seconds. Default: 300 (5 minutes).
    pub default_timeout_secs: u64,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 100,
            default_timeout_secs: 300,
        }
    }
}

/// The workflow engine. Clone is cheap (Arc-backed DB pool + semaphore).
pub struct WorkflowEngine {
    pub(crate) db: Db,
    pub(crate) config: WorkflowConfig,
    /// Semaphore enforcing `config.max_concurrent` simultaneous workflow runs.
    pub(crate) run_semaphore: Arc<Semaphore>,
    /// Last-fired timestamps for interval-triggered workflows, keyed by
    /// `(community_id, workflow_id)`. The same workflow UUID can exist in two
    /// communities (the PK is `(community_id, id)`); keying by bare id would let
    /// one community's interval fire suppress the other's for the interval.
    /// In-memory only — lost on restart. Missed fires during downtime are
    /// not replayed (acceptable for MVP).
    pub(crate) last_fired: DashMap<(CommunityId, Uuid), DateTime<Utc>>,
    /// Action sink for executing side-effects (SendMessage, etc.).
    /// Late-initialized via [`set_action_sink`] after `AppState` construction.
    pub(crate) action_sink: OnceLock<Arc<dyn ActionSink>>,
}

impl WorkflowEngine {
    /// Create a new `WorkflowEngine`.
    pub fn new(db: Db, config: WorkflowConfig) -> Self {
        let permits = config.max_concurrent.max(1);
        let run_semaphore = Arc::new(Semaphore::new(permits));
        Self {
            db,
            config,
            run_semaphore,
            last_fired: DashMap::new(),
            action_sink: OnceLock::new(),
        }
    }

    /// Set the action sink. Called once after `AppState` construction.
    ///
    /// # Panics
    /// Panics if called more than once.
    pub fn set_action_sink(&self, sink: Arc<dyn ActionSink>) {
        if self.action_sink.set(sink).is_err() {
            panic!("action_sink already initialized");
        }
    }

    /// Get the action sink reference.
    ///
    /// Returns `Err(WorkflowError)` if the sink has not been initialized via
    /// [`set_action_sink`]. This avoids a panic if the engine is used before
    /// wiring is complete.
    pub(crate) fn action_sink(&self) -> Result<&dyn ActionSink, WorkflowError> {
        self.action_sink.get().map(|s| s.as_ref()).ok_or_else(|| {
            WorkflowError::InvalidDefinition(
                "action_sink not initialized — call set_action_sink() before executing workflows"
                    .into(),
            )
        })
    }

    /// Parse and validate a YAML workflow definition.
    ///
    /// Returns `(WorkflowDef, canonical_json)` on success. The canonical JSON
    /// is suitable for storage in the `definition` column.
    pub fn parse_yaml(yaml: &str) -> Result<(WorkflowDef, String), WorkflowError> {
        schema::parse_yaml(yaml)
    }

    /// Finalize a workflow run after execution completes or fails.
    ///
    /// This is the **single** place that maps an executor result to a DB status
    /// update. All execution paths (event-triggered, manual trigger/webhook,
    /// approval resume) call this instead of duplicating the 3-way match.
    ///
    /// `existing_trace` is prepended to the executor's trace — used by the
    /// approval-resume path where pre-approval steps already have trace entries.
    pub async fn finalize_run(
        &self,
        community_id: CommunityId,
        run_id: uuid::Uuid,
        result: Result<ExecutionResult, (WorkflowError, PartialProgress)>,
        existing_trace: Option<Vec<serde_json::Value>>,
    ) {
        let prefix = existing_trace.unwrap_or_default();

        match result {
            Ok(result) => {
                let mut full_trace = prefix;
                full_trace.extend(result.trace);
                let trace_json = serde_json::Value::Array(full_trace);
                let step_count = result.step_index as i32;

                if result.approval_token.is_some() {
                    // Approval gates are not yet implemented (WF-08).
                    // Fail explicitly rather than creating unreachable WaitingApproval rows.
                    tracing::warn!(
                        run_id = %run_id,
                        step_index = result.step_index,
                        "Workflow hit approval gate — not yet implemented, marking as failed"
                    );
                    if let Err(e) = self
                        .db
                        .update_workflow_run(
                            community_id,
                            run_id,
                            RunStatus::Failed,
                            step_count,
                            &trace_json,
                            Some("approval gates not yet implemented — see WF-08"),
                        )
                        .await
                    {
                        tracing::error!(
                            run_id = %run_id,
                            "Failed to update run to Failed (approval gate): {e}"
                        );
                    }
                } else {
                    tracing::info!(run_id = %run_id, "Workflow run completed");
                    if let Err(e) = self
                        .db
                        .update_workflow_run(
                            community_id,
                            run_id,
                            RunStatus::Completed,
                            step_count,
                            &trace_json,
                            None,
                        )
                        .await
                    {
                        tracing::error!(
                            run_id = %run_id,
                            "Failed to update run to Completed: {e}"
                        );
                    }
                }
            }
            Err((e, progress)) => {
                tracing::error!(run_id = %run_id, "Workflow run failed: {e}");
                let mut full_trace = prefix;
                full_trace.extend(progress.trace);
                let trace_json = serde_json::Value::Array(full_trace);
                if let Err(db_err) = self
                    .db
                    .update_workflow_run(
                        community_id,
                        run_id,
                        RunStatus::Failed,
                        progress.step_index as i32,
                        &trace_json,
                        Some(&e.to_string()),
                    )
                    .await
                {
                    tracing::error!(
                        run_id = %run_id,
                        "Failed to update run to Failed: {db_err}"
                    );
                }
            }
        }
    }

    /// Called from the event handler post-store hook for every stored event.
    ///
    /// Checks whether any workflow in the event's channel has a matching trigger.
    /// Workflow execution events (kinds 46001–46012) are excluded to prevent loops.
    ///
    /// `community_id` is the server-resolved community the event was stored
    /// under — `StoredEvent` does not carry it, and the same channel UUID can
    /// exist in two communities, so the workflow lookup/run-creation must be
    /// scoped to the caller's tenant or community B could trigger community A's
    /// workflow on a colliding channel id.
    ///
    /// The method takes `self: &Arc<Self>` so that the spawned task can hold a
    /// clone of the `Arc` without requiring `'static` on `&self`.
    pub async fn on_event(
        self: &Arc<Self>,
        community_id: CommunityId,
        event: &buzz_core::StoredEvent,
    ) -> Result<(), WorkflowError> {
        let Some(channel_id) = event.channel_id else {
            tracing::debug!(
                event_id = %event.event.id.to_hex(),
                kind = event_kind_u32(&event.event),
                "Skipping workflow trigger — event has no channel_id"
            );
            return Ok(());
        };

        let kind_u32 = event_kind_u32(&event.event);

        // Exclude workflow execution events to prevent infinite loops.
        if is_workflow_execution_kind(kind_u32) {
            return Ok(());
        }

        let workflows = self
            .db
            .list_enabled_channel_workflows(community_id, channel_id)
            .await
            .map_err(WorkflowError::from)?;

        if workflows.is_empty() {
            return Ok(());
        }

        let trigger_ctx = build_trigger_context(event);

        let trigger_ctx_json: serde_json::Value = match serde_json::to_value(&trigger_ctx) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Failed to serialize trigger context: {e}");
                return Ok(());
            }
        };

        for workflow in &workflows {
            let def: WorkflowDef = match serde_json::from_value(workflow.definition.clone()) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(workflow_id = %workflow.id, "Failed to parse definition: {e}");
                    continue;
                }
            };

            if !def.enabled || !trigger_matches_event(&def.trigger, kind_u32) {
                continue;
            }

            if !should_fire_workflow(&def, &trigger_ctx, workflow.id).await {
                continue;
            }

            let trigger_event_id_bytes = event.event.id.as_bytes().to_vec();
            let run_id = match self
                .db
                .create_workflow_run(
                    community_id,
                    workflow.id,
                    Some(&trigger_event_id_bytes),
                    Some(&trigger_ctx_json),
                )
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!(workflow_id = %workflow.id, "Failed to create run: {e}");
                    continue;
                }
            };

            tracing::debug!(
                workflow_id = %workflow.id,
                run_id = %run_id,
                "Workflow triggered — spawning execution"
            );

            let engine = Arc::clone(self);
            let def_clone = def.clone();
            let ctx_clone = trigger_ctx.clone();

            tokio::spawn(async move {
                let result =
                    executor::execute_run(&engine, community_id, run_id, &def_clone, &ctx_clone)
                        .await;
                engine
                    .finalize_run(community_id, run_id, result, None)
                    .await;
            });
        }

        Ok(())
    }

    /// Interval prefilter: decide whether the interval workflow should fire this
    /// tick, applying the cold-start anchor seed as a side effect.
    ///
    /// `last` is the resolved anchor (in-memory entry if present, else the
    /// durable `latest_scheduled_workflow_fire` read). Returns `true` to proceed
    /// to the durable claim, `false` to suppress this tick.
    ///
    /// Cold-start liveness: a brand-new interval workflow has no in-memory entry
    /// AND no prior claim, so `last` is `None`. `interval_should_fire` then reads
    /// `last = now` and suppresses — correct for the first tick (wait a full
    /// interval), but the in-memory anchor is only written *after* a successful
    /// claim, and no claim is attempted until the prefilter passes. Without
    /// seeding, every subsequent tick repeats with `last = None` and the workflow
    /// suppresses forever. So on the `None` suppress path we seed `now`: the next
    /// tick counts from a real anchor and the workflow fires after one interval.
    /// We seed ONLY when `last` was `None`; when `last` is `Some` we are correctly
    /// mid-interval and must not advance the anchor, or it would never elapse.
    fn interval_prefilter_should_fire(
        &self,
        community_id: CommunityId,
        workflow_id: Uuid,
        dur: &str,
        last: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> bool {
        interval_prefilter_should_fire(&self.last_fired, community_id, workflow_id, dur, last, now)
    }

    /// Background loop for scheduled (cron/interval) triggers.
    ///
    /// Ticks every 60 seconds. For each active workflow with a `Schedule`
    /// trigger, checks whether the cron expression or interval has elapsed
    /// and spawns execution if so.
    ///
    /// Uses window-based matching for cron expressions to handle tick drift:
    /// `schedule.after(&(now - 60s)).next() <= now` instead of `includes(now)`.
    ///
    /// Interval tracking is anchored on the durable scheduled-fire claim:
    /// `last_fired` is an in-memory pre-filter, but the
    /// `(community_id, workflow_id, scheduled_for)` claim row is the
    /// at-most-once boundary across pods and restarts. On the first tick after
    /// a restart the interval anchor is seeded from
    /// `latest_scheduled_workflow_fire` so a process bounce cannot double-fire
    /// within an interval.
    pub async fn run(self: &Arc<Self>) {
        tracing::info!("WorkflowEngine cron loop started (60s tick)");

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;

            let now = Utc::now();

            let workflows = match self.db.list_all_enabled_workflows().await {
                Ok(wf) => wf,
                Err(e) => {
                    tracing::error!("Cron tick: failed to load workflows: {e}");
                    continue;
                }
            };

            for workflow in &workflows {
                // The same workflow UUID may exist in another community; carry
                // the row's owning community through fire-tracking, run creation,
                // and execution so a fire/run never crosses tenants.
                let community_id = workflow.community_id;
                let def: schema::WorkflowDef =
                    match serde_json::from_value(workflow.definition.clone()) {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::warn!(
                                workflow_id = %workflow.id,
                                "Cron tick: failed to parse workflow definition: {e}"
                            );
                            continue;
                        }
                    };

                if !def.enabled {
                    continue;
                }

                // Fix 2: skip workflows with no channel_id — an empty channel_id
                // causes silent downstream failures when the run tries to act on a channel.
                let Some(channel_id) = workflow.channel_id else {
                    tracing::warn!(
                        workflow_id = %workflow.id,
                        "Cron tick: skipping schedule workflow with no channel_id"
                    );
                    continue;
                };

                // Resolve the *deterministic* schedule instant this tick is
                // firing for. `scheduled_for` is computed identically on every
                // pod (cron's own scheduled time, or the interval bucket
                // boundary) so all pods collide on a single durable claim —
                // never `now`, which is per-pod and would let every pod fire.
                let (scheduled_for, trigger_type) = match &def.trigger {
                    schema::TriggerDef::Schedule {
                        cron: Some(expr),
                        interval: None,
                    } => match cron_fire_instant(expr, now, 60, workflow.id) {
                        Some(instant) => (instant, "cron"),
                        None => continue,
                    },
                    schema::TriggerDef::Schedule {
                        cron: None,
                        interval: Some(dur),
                    } => {
                        // Cheap pre-filter: skip the claim attempt when the
                        // in-memory clock says we're clearly mid-interval. The
                        // durable claim below is the real at-most-once boundary;
                        // this only avoids a DB write every tick. Seed the
                        // anchor from the DB on the first tick after restart so
                        // a process bounce can't double-fire within an interval.
                        let last = match self.last_fired.get(&(community_id, workflow.id)) {
                            Some(t) => Some(*t),
                            None => match self
                                .db
                                .latest_scheduled_workflow_fire(community_id, workflow.id)
                                .await
                            {
                                Ok(anchor) => anchor,
                                Err(e) => {
                                    // Fail closed: a missing anchor reads as
                                    // last_fired = now in interval_should_fire,
                                    // so this tick is suppressed and the next
                                    // tick retries. Surface the read failure so
                                    // a persistently-unreadable anchor is visible
                                    // rather than silently stalling the schedule.
                                    tracing::warn!(
                                        community_id = %community_id,
                                        workflow_id = %workflow.id,
                                        "Cron tick: failed to read interval restart anchor, \
                                         suppressing this tick: {e}"
                                    );
                                    None
                                }
                            },
                        };
                        if !self.interval_prefilter_should_fire(
                            community_id,
                            workflow.id,
                            dur,
                            last,
                            now,
                        ) {
                            continue;
                        }
                        match interval_fire_instant(dur, now, workflow.id) {
                            Some(instant) => (instant, "interval"),
                            None => continue,
                        }
                    }
                    _ => continue, // Non-schedule triggers handled by on_event()
                };

                // Durable at-most-once claim — the cross-pod fire boundary.
                // The loser receives `None` and skips BEFORE any run creation or
                // side effect. `community_id` is the workflow row's own
                // community (server provenance from the scan), never client
                // input; the claim binds `(community_id, workflow_id,
                // scheduled_for)` so a duplicate workflow UUID in another
                // community claims independently.
                match self
                    .db
                    .claim_scheduled_workflow_fire(community_id, workflow.id, scheduled_for)
                    .await
                {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        // Another pod (or an earlier tick this pod) already
                        // claimed this instant. Still advance the in-memory
                        // interval clock so we don't re-attempt the claim every
                        // tick for the rest of the interval.
                        if trigger_type == "interval" {
                            self.last_fired.insert((community_id, workflow.id), now);
                        }
                        continue;
                    }
                    Err(e) => {
                        tracing::error!(
                            workflow_id = %workflow.id,
                            "Cron tick: scheduled-fire claim failed: {e}"
                        );
                        continue;
                    }
                }

                // Fix 5: handle serialization errors explicitly rather than silently
                // dropping the trigger context with .ok().
                let trigger_ctx = executor::TriggerContext {
                    channel_id: channel_id.to_string(),
                    timestamp: now.timestamp().to_string(),
                    ..Default::default()
                };
                let trigger_ctx_json = match serde_json::to_value(&trigger_ctx) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::error!(
                            workflow_id = %workflow.id,
                            "Cron tick: failed to serialize trigger context: {e}"
                        );
                        continue;
                    }
                };

                let run_id = match self
                    .db
                    .create_workflow_run(
                        community_id,
                        workflow.id,
                        None, // no trigger event for cron
                        trigger_ctx_json.as_ref(),
                    )
                    .await
                {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::error!(
                            workflow_id = %workflow.id,
                            "Cron tick: failed to create workflow run: {e}"
                        );
                        // The claim is held but the run failed to create. The
                        // claim row intentionally stays (its `workflow_run_id`
                        // NULL) so this instant is not re-fired: at-most-once is
                        // preserved over exactly-once on transient run-insert
                        // failures.
                        continue;
                    }
                };

                // Link the won claim to its run for ops/audit forensics. The
                // claim row already guarantees dedupe; this is best-effort.
                if let Err(e) = self
                    .db
                    .attach_scheduled_workflow_run(community_id, workflow.id, scheduled_for, run_id)
                    .await
                {
                    tracing::warn!(
                        workflow_id = %workflow.id,
                        run_id = %run_id,
                        "Cron tick: failed to attach run to scheduled-fire claim: {e}"
                    );
                }

                // Update last_fired AFTER a successful claim+insert so that a
                // failure doesn't suppress the next tick for the full interval.
                // Only needed for interval triggers — cron uses window-based
                // matching which already prevents double-fire within the same
                // minute, and the durable claim backstops both.
                if trigger_type == "interval" {
                    self.last_fired.insert((community_id, workflow.id), now);
                }

                // Fix 6: log the specific trigger type (cron vs interval).
                tracing::info!(
                    workflow_id = %workflow.id,
                    run_id = %run_id,
                    trigger = trigger_type,
                    "Cron trigger fired"
                );

                let engine = Arc::clone(self);
                let def_clone = def.clone();
                let ctx_clone = trigger_ctx.clone();
                tokio::spawn(async move {
                    let result = executor::execute_run(
                        &engine,
                        community_id,
                        run_id,
                        &def_clone,
                        &ctx_clone,
                    )
                    .await;
                    engine
                        .finalize_run(community_id, run_id, result, None)
                        .await;
                });
            }

            // Fix 1: prune stale last_fired entries for workflows that are no longer
            // active/enabled. Without this the DashMap grows monotonically as
            // workflows are deleted or disabled. Keyed by `(community_id, id)` so
            // entries are matched to the same scope they were inserted under.
            let active_ids: std::collections::HashSet<(CommunityId, Uuid)> =
                workflows.iter().map(|w| (w.community_id, w.id)).collect();
            self.last_fired.retain(|key, _| active_ids.contains(key));
        }
    }
}

/// Find the cron schedule instant that fired within the `window_secs`-wide
/// window ending at `now`, if any.
///
/// Uses window-based matching: finds the next scheduled time after
/// `(now - window_secs)` and returns it when it falls at or before `now`.
/// This tolerates tick drift gracefully — a 61s tick won't miss a
/// minute-granularity cron expression. The returned instant is the cron's own
/// scheduled time (not `now`), so every pod evaluating the same expression in
/// the same window computes the *same* value — making it a safe, deterministic
/// claim anchor for cross-pod at-most-once firing.
///
/// Returns `None` (and logs a warning) if the expression is invalid or nothing
/// is due in the window.
fn cron_fire_instant(
    expr: &str,
    now: DateTime<Utc>,
    window_secs: i64,
    workflow_id: Uuid,
) -> Option<DateTime<Utc>> {
    let normalized = schema::normalize_cron(expr);
    match normalized.parse::<cron::Schedule>() {
        Ok(sched) => {
            let window_start = now - chrono::Duration::seconds(window_secs);
            sched.after(&window_start).next().filter(|t| *t <= now)
        }
        Err(e) => {
            tracing::warn!(
                workflow_id = %workflow_id,
                "Cron tick: invalid cron expression '{expr}': {e}"
            );
            None
        }
    }
}

/// Quantize `now` to the interval bucket boundary, yielding a deterministic
/// claim anchor that every pod computes identically within the same bucket.
///
/// The boundary is `floor(now / interval) * interval` from the Unix epoch.
/// Because the scheduler ticks every 60s and interval schedules are minutes or
/// longer, bounded cross-pod clock skew keeps all pods inside the same bucket,
/// so they collide on one `(community, workflow, scheduled_for)` claim — only
/// one wins and creates the run. Returns `None` if the duration is unparseable
/// or non-positive (the caller skips firing).
fn interval_fire_instant(
    dur: &str,
    now: DateTime<Utc>,
    workflow_id: Uuid,
) -> Option<DateTime<Utc>> {
    match executor::parse_duration_secs(dur) {
        Ok(interval_secs) if interval_secs > 0 => {
            let secs = interval_secs as i64;
            let bucket = (now.timestamp().div_euclid(secs)) * secs;
            DateTime::from_timestamp(bucket, 0)
        }
        Ok(_) => {
            tracing::warn!(
                workflow_id = %workflow_id,
                "Cron tick: interval duration is zero — skipping"
            );
            None
        }
        Err(e) => {
            tracing::warn!(
                workflow_id = %workflow_id,
                "Cron tick: invalid interval '{dur}': {e}"
            );
            None
        }
    }
}

/// Check whether an interval trigger should fire based on the last-fired time.
///
/// `last_fired` is `None` on the first tick after startup — in that case we
/// default to `now`, which prevents an immediate fire and waits a full interval.
///
/// Returns `false` (and logs a warning) if the duration string is invalid.
fn interval_should_fire(
    dur: &str,
    last_fired: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    workflow_id: Uuid,
) -> bool {
    match executor::parse_duration_secs(dur) {
        Ok(interval_secs) => {
            // Default to now on first tick — prevents immediate fire after startup.
            let last = last_fired.unwrap_or(now);
            let elapsed = (now - last).num_seconds().unsigned_abs();
            elapsed >= interval_secs
        }
        Err(e) => {
            tracing::warn!(
                workflow_id = %workflow_id,
                "Cron tick: invalid interval '{dur}': {e}"
            );
            false
        }
    }
}

/// Interval prefilter decision + cold-start anchor seed. See the
/// [`WorkflowEngine::interval_prefilter_should_fire`] wrapper for the liveness
/// rationale. Free function over the `last_fired` map so it is unit-testable
/// without a `Db`/Postgres: the only state it touches is the in-memory anchor.
///
/// Returns `true` to fire, `false` to suppress. On the cold-start `None` suppress
/// path it seeds `now` so the next tick has a real anchor; it never advances an
/// existing (`Some`) anchor, which is mid-interval and must elapse on its own.
fn interval_prefilter_should_fire(
    last_fired: &DashMap<(CommunityId, Uuid), DateTime<Utc>>,
    community_id: CommunityId,
    workflow_id: Uuid,
    dur: &str,
    last: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    if interval_should_fire(dur, last, now, workflow_id) {
        return true;
    }
    if last.is_none() {
        last_fired.insert((community_id, workflow_id), now);
    }
    false
}

/// Check emoji and filter-expression conditions that determine whether a
/// matched workflow should actually fire. Extracted from `on_event` to keep
/// the per-workflow loop body small.
///
/// Returns `true` if the workflow should fire, `false` to skip.
async fn should_fire_workflow(
    def: &WorkflowDef,
    trigger_ctx: &executor::TriggerContext,
    workflow_id: uuid::Uuid,
) -> bool {
    if let TriggerDef::ReactionAdded {
        emoji: Some(ref expected),
    } = def.trigger
    {
        if &trigger_ctx.emoji != expected {
            tracing::debug!(
                workflow_id = %workflow_id,
                expected_emoji = %expected,
                actual_emoji = %trigger_ctx.emoji,
                "Reaction emoji mismatch — skipping workflow"
            );
            return false;
        }
    }

    if let TriggerDef::MessagePosted {
        filter: Some(ref expr),
    } = def.trigger
    {
        match executor::evaluate_condition(expr, trigger_ctx, &HashMap::new()).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    workflow_id = %workflow_id,
                    "Trigger filter evaluated false — skipping workflow"
                );
                return false;
            }
            Err(e) => {
                tracing::warn!(
                    workflow_id = %workflow_id,
                    "Trigger filter error: {e} — skipping workflow"
                );
                return false;
            }
        }
    }

    if let TriggerDef::DiffPosted {
        filter: Some(ref expr),
    } = def.trigger
    {
        match executor::evaluate_condition(expr, trigger_ctx, &HashMap::new()).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    workflow_id = %workflow_id,
                    "Trigger filter evaluated false — skipping workflow"
                );
                return false;
            }
            Err(e) => {
                tracing::warn!(
                    workflow_id = %workflow_id,
                    "Trigger filter error: {e} — skipping workflow"
                );
                return false;
            }
        }
    }

    true
}

/// Build a [`executor::TriggerContext`] from a [`buzz_core::StoredEvent`].
///
/// - `text` — event content (message body or reaction emoji character)
/// - `author` — pubkey hex string
/// - `channel_id` — channel UUID as string (empty if no channel scope)
/// - `timestamp` — Unix timestamp as string
/// - `emoji` — for `KIND_REACTION` events, the content is the emoji; otherwise empty
/// - `message_id` — for reactions, the target message's event ID (from `e` tag);
///   for all other events, the event's own ID
pub fn build_trigger_context(event: &buzz_core::StoredEvent) -> executor::TriggerContext {
    let kind_u32 = event_kind_u32(&event.event);
    let content = event.event.content.clone();

    let author = event
        .event
        .tags
        .iter()
        .find_map(|tag| {
            if tag.kind().to_string() == "actor" {
                tag.content().map(|value| value.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| event.event.pubkey.to_hex());

    // For reaction events (NIP-25), the content field holds the emoji character
    // or shortcode (e.g. "👍", "+", "-"). Expose it as `emoji`.
    let emoji = if kind_u32 == KIND_REACTION {
        content.clone()
    } else {
        String::new()
    };

    // For reactions (NIP-25), `message_id` should be the target message, not
    // the reaction event itself. NIP-25 stores the target in an `e` tag whose
    // value is a 64-char hex event ID (not a UUID channel reference).
    // Per NIP-25, the last `e` tag is the direct target (earlier ones may be thread roots).
    let message_id = if kind_u32 == KIND_REACTION {
        event
            .event
            .tags
            .iter()
            .rev()
            .find_map(|tag| {
                let key = tag.kind().to_string();
                if key == "e" {
                    tag.content().and_then(|v| {
                        // Distinguish hex event IDs (64 chars) from UUID channel refs.
                        if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                            Some(v.to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            // Fallback to the reaction event's own ID if no valid `e` tag found.
            .unwrap_or_else(|| event.event.id.to_hex())
    } else {
        event.event.id.to_hex()
    };

    executor::TriggerContext {
        text: content,
        author,
        channel_id: event
            .channel_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        timestamp: event.event.created_at.as_secs().to_string(),
        emoji,
        message_id,
        webhook_fields: HashMap::new(),
    }
}

/// Returns `true` if the trigger type matches the given event kind.
fn trigger_matches_event(trigger: &TriggerDef, kind_u32: u32) -> bool {
    use buzz_core::kind::{KIND_REACTION, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_DIFF};
    match trigger {
        TriggerDef::MessagePosted { .. } => kind_u32 == KIND_STREAM_MESSAGE,
        TriggerDef::ReactionAdded { .. } => kind_u32 == KIND_REACTION,
        TriggerDef::DiffPosted { .. } => kind_u32 == KIND_STREAM_MESSAGE_DIFF,
        // Schedule and Webhook triggers are not fired by channel events.
        TriggerDef::Schedule { .. } | TriggerDef::Webhook => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_fire_instant_matches_within_window() {
        // "every minute" cron — should always fire within a 60s window.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:30Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        // The matched instant is the minute boundary 12:00:00, NOT `now`.
        assert_eq!(
            cron_fire_instant("* * * * *", now, 60, wf_id),
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            ),
            "every-minute cron should return the minute boundary as the anchor"
        );
    }

    #[test]
    fn cron_fire_instant_returns_none_for_invalid_expr() {
        let now = Utc::now();
        let wf_id = Uuid::new_v4();
        assert!(
            cron_fire_instant("not-a-cron", now, 60, wf_id).is_none(),
            "invalid cron should return None"
        );
    }

    #[test]
    fn cron_fire_instant_returns_none_outside_window() {
        // Fixed time: 2026-06-15 14:30:00 UTC (a Sunday in June)
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        // "0 0 1 1 *" = midnight on Jan 1 only — June 15 is definitely outside.
        assert!(
            cron_fire_instant("0 0 1 1 *", now, 60, wf_id).is_none(),
            "Jan-1-only cron should not fire on June 15"
        );
    }

    #[test]
    fn cron_fire_instant_at_exact_minute_boundary() {
        // Fixed time: exactly 09:00:00 UTC. Cron "0 9 * * *" fires at 09:00.
        // Window [08:59:00, 09:00:00] should contain the fire time.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert_eq!(
            cron_fire_instant("0 9 * * *", now, 60, wf_id),
            Some(now),
            "cron should fire at exact minute boundary, anchored on 09:00:00"
        );
    }

    #[test]
    fn cron_fire_instant_within_drift_window_anchors_on_scheduled_time() {
        // Fixed time: 09:00:45 UTC (45s drift). Cron "0 9 * * *" fires at 09:00.
        // Window [08:59:45, 09:00:45] should still contain 09:00:00. Critically,
        // the anchor is the *scheduled* 09:00:00 — not the drifted `now` — so a
        // second pod ticking at 09:00:50 computes the identical claim key.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:45Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert_eq!(
            cron_fire_instant("0 9 * * *", now, 60, wf_id),
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc)
            ),
            "cron anchor must be the scheduled instant, stable across pod tick drift"
        );
    }

    #[test]
    fn cron_fire_instant_returns_none_just_outside_window() {
        // Fixed time: 09:01:01 UTC. Cron "0 9 * * *" fires at 09:00:00.
        // Window [09:00:01, 09:01:01] does NOT contain 09:00:00.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:01:01Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert!(
            cron_fire_instant("0 9 * * *", now, 60, wf_id).is_none(),
            "cron should not fire 61s after the scheduled time"
        );
    }

    #[test]
    fn interval_fire_instant_quantizes_to_bucket_boundary() {
        // Two pods ticking at different sub-interval offsets must compute the
        // *same* bucket boundary so they collide on one claim. 1h interval,
        // epoch-aligned: 12:34:56 and 12:59:01 both floor to 12:00:00.
        let wf_id = Uuid::new_v4();
        let a = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc);
        let b = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:59:01Z")
            .unwrap()
            .with_timezone(&Utc);
        let bucket = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(interval_fire_instant("1h", a, wf_id), Some(bucket));
        assert_eq!(interval_fire_instant("1h", b, wf_id), Some(bucket));
        // Next hour is a distinct bucket.
        let c = chrono::DateTime::parse_from_rfc3339("2026-06-15T13:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let next_bucket = chrono::DateTime::parse_from_rfc3339("2026-06-15T13:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(interval_fire_instant("1h", c, wf_id), Some(next_bucket));
    }

    #[test]
    fn interval_fire_instant_returns_none_for_invalid_duration() {
        let now = Utc::now();
        let wf_id = Uuid::new_v4();
        assert!(interval_fire_instant("not-a-duration", now, wf_id).is_none());
    }

    #[test]
    fn interval_should_fire_returns_false_on_first_tick() {
        // When last_fired is None (first tick), defaults to now → elapsed = 0 → false.
        let now = Utc::now();
        let wf_id = Uuid::new_v4();
        assert!(
            !interval_should_fire("1h", None, now, wf_id),
            "first tick should not fire immediately"
        );
    }

    #[test]
    fn interval_should_fire_returns_true_after_interval_elapsed() {
        let wf_id = Uuid::new_v4();
        let now = Utc::now();
        // last_fired was 2 hours ago; interval is 1h → should fire.
        let last = now - chrono::Duration::hours(2);
        assert!(
            interval_should_fire("1h", Some(last), now, wf_id),
            "should fire after interval elapsed"
        );
    }

    #[test]
    fn interval_should_fire_returns_false_before_interval_elapsed() {
        let wf_id = Uuid::new_v4();
        let now = Utc::now();
        // last_fired was 30 minutes ago; interval is 1h → should not fire.
        let last = now - chrono::Duration::minutes(30);
        assert!(
            !interval_should_fire("1h", Some(last), now, wf_id),
            "should not fire before interval elapsed"
        );
    }

    #[test]
    fn interval_should_fire_returns_false_for_invalid_duration() {
        let now = Utc::now();
        let wf_id = Uuid::new_v4();
        assert!(
            !interval_should_fire("not-a-duration", None, now, wf_id),
            "invalid duration should return false"
        );
    }

    #[test]
    fn interval_should_fire_at_exact_boundary() {
        let wf_id = Uuid::new_v4();
        let now = Utc::now();
        // last_fired was exactly 1 hour ago; interval is 1h → should fire (elapsed >= interval).
        let last = now - chrono::Duration::hours(1);
        assert!(
            interval_should_fire("1h", Some(last), now, wf_id),
            "should fire at exact interval boundary"
        );
    }

    // ── Interval cold-start liveness (Max's blocker on the scheduled lane) ──
    // A brand-new interval workflow has no in-memory anchor and no prior durable
    // claim, so the prefilter resolves `last = None`. Without seeding, every tick
    // reads `None`, suppresses, and writes nothing — the workflow never fires.
    // `interval_prefilter_should_fire` must seed `now` on that first suppress so a
    // real anchor exists for the next tick.

    #[test]
    fn interval_cold_start_seeds_anchor_then_fires_after_one_interval() {
        let map: DashMap<(CommunityId, Uuid), DateTime<Utc>> = DashMap::new();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let wf = Uuid::new_v4();
        let t0 = Utc::now();

        // Tick 1 (cold start): no in-memory entry, DB anchor is None → last = None.
        let fired_1 = interval_prefilter_should_fire(&map, community, wf, "1h", None, t0);
        assert!(!fired_1, "first tick must suppress (wait a full interval)");
        let seeded = map.get(&(community, wf)).map(|v| *v);
        assert_eq!(
            seeded,
            Some(t0),
            "first suppressed tick must seed the anchor to `now`, else it suppresses forever"
        );

        // Tick 2, mid-interval: caller now passes the seeded anchor as `last`.
        let t1 = t0 + chrono::Duration::minutes(30);
        let last = map.get(&(community, wf)).map(|v| *v);
        let fired_2 = interval_prefilter_should_fire(&map, community, wf, "1h", last, t1);
        assert!(!fired_2, "still mid-interval → suppress");
        assert_eq!(
            map.get(&(community, wf)).map(|v| *v),
            Some(t0),
            "mid-interval suppress must NOT advance the anchor (or it would never elapse)"
        );

        // Tick 3, one interval elapsed → fire.
        let t2 = t0 + chrono::Duration::hours(1);
        let last = map.get(&(community, wf)).map(|v| *v);
        let fired_3 = interval_prefilter_should_fire(&map, community, wf, "1h", last, t2);
        assert!(
            fired_3,
            "after one full interval the cold-started workflow must fire"
        );
    }

    #[test]
    fn interval_prefilter_does_not_advance_existing_anchor_on_suppress() {
        // Regression for the inverse bug: if a `Some` anchor were re-seeded to
        // `now` on every suppressed tick, the interval would never elapse.
        let map: DashMap<(CommunityId, Uuid), DateTime<Utc>> = DashMap::new();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let wf = Uuid::new_v4();
        let now = Utc::now();
        let anchor = now - chrono::Duration::minutes(10); // 10m into a 1h interval
        map.insert((community, wf), anchor);

        let fired = interval_prefilter_should_fire(&map, community, wf, "1h", Some(anchor), now);
        assert!(!fired, "mid-interval suppress");
        assert_eq!(
            map.get(&(community, wf)).map(|v| *v),
            Some(anchor),
            "existing anchor must be preserved exactly, not advanced to now"
        );
    }

    #[test]
    fn interval_prefilter_passes_through_a_due_fire_without_touching_anchor() {
        // When the interval has elapsed the prefilter returns true and leaves the
        // anchor to the post-claim update path (which writes `now` only on a won
        // claim), so the prefilter must not seed here.
        let map: DashMap<(CommunityId, Uuid), DateTime<Utc>> = DashMap::new();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let wf = Uuid::new_v4();
        let now = Utc::now();
        let anchor = now - chrono::Duration::hours(2); // overdue on a 1h interval

        let fired = interval_prefilter_should_fire(&map, community, wf, "1h", Some(anchor), now);
        assert!(fired, "overdue interval must fire");
        assert!(
            map.get(&(community, wf)).is_none(),
            "a firing tick must not seed via the prefilter; the post-claim path owns the write"
        );
    }

    #[test]
    fn workflow_config_defaults() {
        let cfg = WorkflowConfig::default();
        assert_eq!(cfg.max_concurrent, 100);
        assert_eq!(cfg.default_timeout_secs, 300);
    }

    #[test]
    fn parse_yaml_roundtrip() {
        let yaml = r#"
name: "Test Workflow"
trigger:
  on: message_posted
steps:
  - id: s1
    action: send_message
    text: "Hello {{trigger.author}}"
"#;
        let (def, json) = WorkflowEngine::parse_yaml(yaml).expect("parse failed");
        assert_eq!(def.name, "Test Workflow");

        let reparsed: WorkflowDef = serde_json::from_str(&json).expect("json round-trip");
        assert_eq!(reparsed.name, def.name);
        assert_eq!(reparsed.steps.len(), 1);
    }

    #[test]
    fn trigger_matches_stream_message() {
        let trigger = TriggerDef::MessagePosted { filter: None };
        assert!(trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_REACTION
        ));
    }

    #[test]
    fn trigger_matches_reaction() {
        let trigger = TriggerDef::ReactionAdded { emoji: None };
        assert!(trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_REACTION
        ));
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_STREAM_MESSAGE
        ));
    }

    #[test]
    fn schedule_trigger_never_matches_events() {
        let trigger = TriggerDef::Schedule {
            cron: Some("0 9 * * 1-5".to_owned()),
            interval: None,
        };
        // Schedule triggers are fired by the cron loop, not by events.
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_REACTION
        ));
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_WORKFLOW_TRIGGERED
        ));
    }

    #[test]
    fn webhook_trigger_never_matches_events() {
        let trigger = TriggerDef::Webhook;
        assert!(!trigger_matches_event(
            &trigger,
            buzz_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(&trigger, 0));
    }

    #[test]
    fn message_posted_matches_kind_9_only() {
        let trigger = TriggerDef::MessagePosted { filter: None };
        // Must match KIND_STREAM_MESSAGE = 9.
        assert!(trigger_matches_event(&trigger, 9));
        // Must NOT match reaction (kind 7).
        assert!(!trigger_matches_event(&trigger, 7));
        // Must NOT match forum post (kind 45001).
        assert!(!trigger_matches_event(&trigger, 45001));
        // Must NOT match stream message v2 (kind 40002).
        assert!(!trigger_matches_event(&trigger, 40002));
    }

    #[test]
    fn reaction_added_matches_kind_7_only() {
        let trigger = TriggerDef::ReactionAdded { emoji: None };
        // Must match KIND_REACTION = 7.
        assert!(trigger_matches_event(&trigger, 7));
        // Must NOT match stream message (kind 9).
        assert!(!trigger_matches_event(&trigger, 9));
        // Must NOT match forum post (kind 45001).
        assert!(!trigger_matches_event(&trigger, 45001));
    }

    #[test]
    fn reaction_added_with_emoji_filter_still_matches_kind_7() {
        // The emoji filter is evaluated at execution time, not trigger-matching time.
        // trigger_matches_event only checks the kind number.
        let trigger = TriggerDef::ReactionAdded {
            emoji: Some("thumbsup".to_owned()),
        };
        assert!(trigger_matches_event(&trigger, 7));
        assert!(!trigger_matches_event(&trigger, 9));
    }

    #[test]
    fn message_posted_with_filter_still_matches_kind_9() {
        // The filter expression is evaluated at execution time, not trigger-matching time.
        let trigger = TriggerDef::MessagePosted {
            filter: Some("str_contains(trigger_text, \"P1\")".to_owned()),
        };
        assert!(trigger_matches_event(&trigger, 9));
        assert!(!trigger_matches_event(&trigger, 7));
    }

    #[test]
    fn workflow_execution_kinds_do_not_match_any_trigger() {
        // Workflow execution events (46001–46012) must never match triggers
        // to prevent infinite loops. The on_event() method filters these out
        // before calling trigger_matches_event, but verify the function itself
        // also returns false for these kinds.
        let msg_trigger = TriggerDef::MessagePosted { filter: None };
        let react_trigger = TriggerDef::ReactionAdded { emoji: None };

        for kind in buzz_core::kind::KIND_WORKFLOW_TRIGGERED
            ..=buzz_core::kind::KIND_WORKFLOW_APPROVAL_DENIED
        {
            assert!(
                !trigger_matches_event(&msg_trigger, kind),
                "message_posted should not match workflow execution kind {kind}"
            );
            assert!(
                !trigger_matches_event(&react_trigger, kind),
                "reaction_added should not match workflow execution kind {kind}"
            );
        }
    }

    #[test]
    fn trigger_matches_event_kind_zero_matches_nothing() {
        // Kind 0 is a profile event — no trigger should match it.
        let msg_trigger = TriggerDef::MessagePosted { filter: None };
        let react_trigger = TriggerDef::ReactionAdded { emoji: None };
        let sched_trigger = TriggerDef::Schedule {
            cron: None,
            interval: Some("1h".to_owned()),
        };
        let webhook_trigger = TriggerDef::Webhook;

        assert!(!trigger_matches_event(&msg_trigger, 0));
        assert!(!trigger_matches_event(&react_trigger, 0));
        assert!(!trigger_matches_event(&sched_trigger, 0));
        assert!(!trigger_matches_event(&webhook_trigger, 0));
    }

    #[test]
    fn diff_posted_matches_kind_40008_only() {
        let trigger = TriggerDef::DiffPosted { filter: None };
        assert!(trigger_matches_event(&trigger, 40008));
        assert!(!trigger_matches_event(&trigger, 9));
        assert!(!trigger_matches_event(&trigger, 7));
    }

    #[test]
    fn message_posted_does_not_match_kind_40008() {
        let trigger = TriggerDef::MessagePosted { filter: None };
        assert!(!trigger_matches_event(&trigger, 40008));
        assert!(trigger_matches_event(&trigger, 9));
    }

    #[test]
    fn workflow_config_custom_values() {
        let cfg = WorkflowConfig {
            max_concurrent: 50,
            default_timeout_secs: 600,
        };
        assert_eq!(cfg.max_concurrent, 50);
        assert_eq!(cfg.default_timeout_secs, 600);
    }

    fn make_message_event() -> buzz_core::StoredEvent {
        use nostr::{EventBuilder, Keys, Kind};
        use uuid::Uuid;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "hello world")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        buzz_core::StoredEvent::new(event, Some(Uuid::new_v4()))
    }

    /// Create a reaction event with an `e` tag pointing to a target message.
    fn make_reaction_event() -> (buzz_core::StoredEvent, String) {
        use nostr::{EventBuilder, Keys, Kind, Tag};
        use uuid::Uuid;
        let keys = Keys::generate();
        // Create a dummy target message ID (64-char hex).
        let target_keys = Keys::generate();
        let target_event = EventBuilder::new(Kind::Custom(9), "target msg")
            .tags([])
            .sign_with_keys(&target_keys)
            .expect("sign target");
        let target_id_hex = target_event.id.to_hex();
        // NIP-25: reaction references the target via an `e` tag.
        let e_tag = Tag::parse(["e", &target_id_hex]).expect("tag parse");
        let event = EventBuilder::new(Kind::Reaction, "👍")
            .tags([e_tag])
            .sign_with_keys(&keys)
            .expect("sign");
        (
            buzz_core::StoredEvent::new(event, Some(Uuid::new_v4())),
            target_id_hex,
        )
    }

    #[test]
    fn build_trigger_context_message_event() {
        let stored = make_message_event();
        let ctx = build_trigger_context(&stored);

        assert_eq!(ctx.text, "hello world");
        assert_eq!(ctx.author, stored.event.pubkey.to_hex());
        assert_eq!(ctx.channel_id, stored.channel_id.unwrap().to_string());
        assert_eq!(ctx.timestamp, stored.event.created_at.as_secs().to_string());
        assert_eq!(ctx.message_id, stored.event.id.to_hex());
        // Non-reaction events have empty emoji.
        assert_eq!(ctx.emoji, "");
        assert!(ctx.webhook_fields.is_empty());
    }

    #[test]
    fn build_trigger_context_reaction_event() {
        let (stored, target_id_hex) = make_reaction_event();
        let ctx = build_trigger_context(&stored);

        // For reactions, content IS the emoji.
        assert_eq!(ctx.text, "👍");
        assert_eq!(ctx.emoji, "👍");
        assert_eq!(ctx.author, stored.event.pubkey.to_hex());
        // message_id should be the TARGET message, not the reaction event itself.
        assert_eq!(ctx.message_id, target_id_hex);
        assert_ne!(ctx.message_id, stored.event.id.to_hex());
        assert!(ctx.webhook_fields.is_empty());
    }

    #[test]
    fn build_trigger_context_no_channel_id() {
        use nostr::{EventBuilder, Keys, Kind};
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "msg")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        // channel_id = None (global/DM event)
        let stored = buzz_core::StoredEvent::new(event, None);
        let ctx = build_trigger_context(&stored);

        assert_eq!(ctx.channel_id, "");
        assert_eq!(ctx.text, "msg");
    }

    #[test]
    fn build_trigger_context_author_is_hex_pubkey() {
        let stored = make_message_event();
        let ctx = build_trigger_context(&stored);
        // Pubkey hex is 64 lowercase hex characters.
        assert_eq!(ctx.author.len(), 64);
        assert!(ctx.author.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn build_trigger_context_message_id_is_hex() {
        let stored = make_message_event();
        let ctx = build_trigger_context(&stored);
        // Event ID hex is 64 lowercase hex characters.
        assert_eq!(ctx.message_id.len(), 64);
        assert!(ctx.message_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn build_trigger_context_timestamp_is_numeric_string() {
        let stored = make_message_event();
        let ctx = build_trigger_context(&stored);
        // Timestamp must parse as a u64.
        ctx.timestamp
            .parse::<u64>()
            .expect("timestamp should be a u64 string");
    }

    #[test]
    fn test_build_trigger_context_reaction_multiple_e_tags() {
        // NIP-25: last e tag is the direct target, first may be thread root
        use nostr::{EventBuilder, EventId, Keys, Kind, Tag};
        use uuid::Uuid;

        let keys = Keys::generate();
        let thread_root_id = EventId::all_zeros();
        let direct_target_id = EventId::from_byte_array([0x42; 32]);

        let event = EventBuilder::new(Kind::Reaction, "👍")
            .tags([
                Tag::parse(["e", &thread_root_id.to_hex()]).unwrap(),
                Tag::parse(["e", &direct_target_id.to_hex()]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign");

        let stored = buzz_core::StoredEvent::new(event, Some(Uuid::new_v4()));
        let ctx = build_trigger_context(&stored);

        // Should pick the LAST e tag (direct target), not the first (thread root)
        assert_eq!(ctx.message_id, direct_target_id.to_hex());
    }
}
