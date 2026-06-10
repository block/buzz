#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `sprout-workflow` — Workflow engine for Sprout.
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
//! engine.on_event(&stored_event).await?;
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

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use sprout_core::kind::{event_kind_u32, is_workflow_execution_kind, KIND_REACTION};
use sprout_db::workflow::RunStatus;
use sprout_db::Db;
use tokio::sync::Semaphore;
use uuid::Uuid;

// ── Configuration ─────────────────────────────────────────────────────────────

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

// ── Engine ────────────────────────────────────────────────────────────────────

/// The workflow engine. Clone is cheap (Arc-backed DB pool + semaphore).
pub struct WorkflowEngine {
    pub(crate) db: Db,
    pub(crate) config: WorkflowConfig,
    /// Semaphore enforcing `config.max_concurrent` simultaneous workflow runs.
    pub(crate) run_semaphore: Arc<Semaphore>,
    /// Last-fired timestamps for interval-triggered workflows.
    /// In-memory only — lost on restart. Missed fires during downtime are
    /// not replayed (acceptable for MVP).
    pub(crate) last_fired: DashMap<Uuid, DateTime<Utc>>,
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
    /// The method takes `self: &Arc<Self>` so that the spawned task can hold a
    /// clone of the `Arc` without requiring `'static` on `&self`.
    pub async fn on_event(
        self: &Arc<Self>,
        event: &sprout_core::StoredEvent,
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
            .list_enabled_channel_workflows(channel_id)
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
                let result = executor::execute_run(&engine, run_id, &def_clone, &ctx_clone).await;
                engine.finalize_run(run_id, result, None).await;
            });
        }

        Ok(())
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
    /// Interval tracking is in-memory (`last_fired` DashMap). Lost on restart —
    /// missed fires during downtime are not replayed.
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

                let (should_fire, trigger_type) = match &def.trigger {
                    schema::TriggerDef::Schedule {
                        cron: Some(expr),
                        interval: None,
                    } => {
                        // Fix 7: delegate to pure helper for testability.
                        (cron_should_fire(expr, now, 60, workflow.id), "cron")
                    }
                    schema::TriggerDef::Schedule {
                        cron: None,
                        interval: Some(dur),
                    } => {
                        // Fix 7: delegate to pure helper for testability.
                        let last = self.last_fired.get(&workflow.id).map(|t| *t);
                        (
                            interval_should_fire(dur, last, now, workflow.id),
                            "interval",
                        )
                    }
                    _ => (false, ""), // Non-schedule triggers handled by on_event()
                };

                if !should_fire {
                    continue;
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
                        continue;
                    }
                };

                // Update last_fired AFTER successful DB insert so that a
                // failed insert doesn't suppress the next tick for the full interval.
                // Only needed for interval triggers — cron uses window-based matching
                // which already prevents double-fire within the same minute.
                if trigger_type == "interval" {
                    self.last_fired.insert(workflow.id, now);
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
                    let result =
                        executor::execute_run(&engine, run_id, &def_clone, &ctx_clone).await;
                    engine.finalize_run(run_id, result, None).await;
                });
            }

            // Fix 1: prune stale last_fired entries for workflows that are no longer
            // active/enabled. Without this the DashMap grows monotonically as
            // workflows are deleted or disabled.
            let active_ids: std::collections::HashSet<Uuid> =
                workflows.iter().map(|w| w.id).collect();
            self.last_fired.retain(|id, _| active_ids.contains(id));
        }
    }
}

// ── Cron/interval helpers ─────────────────────────────────────────────────────

/// Check whether a cron expression should fire within the `window_secs`-wide
/// window ending at `now`.
///
/// Uses window-based matching: finds the next scheduled time after
/// `(now - window_secs)` and checks whether it falls at or before `now`.
/// This tolerates tick drift gracefully — a 61s tick won't miss a
/// minute-granularity cron expression.
///
/// Returns `false` (and logs a warning) if the expression is invalid.
fn cron_should_fire(expr: &str, now: DateTime<Utc>, window_secs: i64, workflow_id: Uuid) -> bool {
    let normalized = schema::normalize_cron(expr);
    match normalized.parse::<cron::Schedule>() {
        Ok(sched) => {
            let window_start = now - chrono::Duration::seconds(window_secs);
            sched
                .after(&window_start)
                .next()
                .map(|t| t <= now)
                .unwrap_or(false)
        }
        Err(e) => {
            tracing::warn!(
                workflow_id = %workflow_id,
                "Cron tick: invalid cron expression '{expr}': {e}"
            );
            false
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

// ── Pre-trigger filtering ─────────────────────────────────────────────────────

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

// ── Trigger context builder ───────────────────────────────────────────────────

/// Build a [`executor::TriggerContext`] from a [`sprout_core::StoredEvent`].
///
/// - `text` — event content (message body or reaction emoji character)
/// - `author` — pubkey hex string
/// - `channel_id` — channel UUID as string (empty if no channel scope)
/// - `timestamp` — Unix timestamp as string
/// - `emoji` — for `KIND_REACTION` events, the content is the emoji; otherwise empty
/// - `message_id` — for reactions, the target message's event ID (from `e` tag);
///   for all other events, the event's own ID
pub fn build_trigger_context(event: &sprout_core::StoredEvent) -> executor::TriggerContext {
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

// ── Trigger matching ──────────────────────────────────────────────────────────

/// Returns `true` if the trigger type matches the given event kind.
fn trigger_matches_event(trigger: &TriggerDef, kind_u32: u32) -> bool {
    use sprout_core::kind::{KIND_REACTION, KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_DIFF};
    match trigger {
        TriggerDef::MessagePosted { .. } => kind_u32 == KIND_STREAM_MESSAGE,
        TriggerDef::ReactionAdded { .. } => kind_u32 == KIND_REACTION,
        TriggerDef::DiffPosted { .. } => kind_u32 == KIND_STREAM_MESSAGE_DIFF,
        // Schedule and Webhook triggers are not fired by channel events.
        TriggerDef::Schedule { .. } | TriggerDef::Webhook => false,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── cron_should_fire helper ───────────────────────────────────────────────

    #[test]
    fn cron_should_fire_matches_within_window() {
        // "every minute" cron — should always fire within a 60s window.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:30Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert!(
            cron_should_fire("* * * * *", now, 60, wf_id),
            "every-minute cron should fire within 60s window"
        );
    }

    #[test]
    fn cron_should_fire_returns_false_for_invalid_expr() {
        let now = Utc::now();
        let wf_id = Uuid::new_v4();
        assert!(
            !cron_should_fire("not-a-cron", now, 60, wf_id),
            "invalid cron should return false"
        );
    }

    #[test]
    fn cron_should_fire_returns_false_outside_window() {
        // Fixed time: 2026-06-15 14:30:00 UTC (a Sunday in June)
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        // "0 0 1 1 *" = midnight on Jan 1 only — June 15 is definitely outside.
        assert!(
            !cron_should_fire("0 0 1 1 *", now, 60, wf_id),
            "Jan-1-only cron should not fire on June 15"
        );
    }

    #[test]
    fn cron_should_fire_at_exact_minute_boundary() {
        // Fixed time: exactly 09:00:00 UTC. Cron "0 9 * * *" fires at 09:00.
        // Window [08:59:00, 09:00:00] should contain the fire time.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert!(
            cron_should_fire("0 9 * * *", now, 60, wf_id),
            "cron should fire at exact minute boundary"
        );
    }

    #[test]
    fn cron_should_fire_within_drift_window() {
        // Fixed time: 09:00:45 UTC (45s drift). Cron "0 9 * * *" fires at 09:00.
        // Window [08:59:45, 09:00:45] should still contain 09:00:00.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:00:45Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert!(
            cron_should_fire("0 9 * * *", now, 60, wf_id),
            "cron should fire even with 45s drift"
        );
    }

    #[test]
    fn cron_should_fire_returns_false_just_outside_window() {
        // Fixed time: 09:01:01 UTC. Cron "0 9 * * *" fires at 09:00:00.
        // Window [09:00:01, 09:01:01] does NOT contain 09:00:00.
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T09:01:01Z")
            .unwrap()
            .with_timezone(&Utc);
        let wf_id = Uuid::new_v4();
        assert!(
            !cron_should_fire("0 9 * * *", now, 60, wf_id),
            "cron should not fire 61s after the scheduled time"
        );
    }

    // ── interval_should_fire helper ───────────────────────────────────────────

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
            sprout_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(
            &trigger,
            sprout_core::kind::KIND_REACTION
        ));
    }

    #[test]
    fn trigger_matches_reaction() {
        let trigger = TriggerDef::ReactionAdded { emoji: None };
        assert!(trigger_matches_event(
            &trigger,
            sprout_core::kind::KIND_REACTION
        ));
        assert!(!trigger_matches_event(
            &trigger,
            sprout_core::kind::KIND_STREAM_MESSAGE
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
            sprout_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(
            &trigger,
            sprout_core::kind::KIND_REACTION
        ));
        assert!(!trigger_matches_event(
            &trigger,
            sprout_core::kind::KIND_WORKFLOW_TRIGGERED
        ));
    }

    #[test]
    fn webhook_trigger_never_matches_events() {
        let trigger = TriggerDef::Webhook;
        assert!(!trigger_matches_event(
            &trigger,
            sprout_core::kind::KIND_STREAM_MESSAGE
        ));
        assert!(!trigger_matches_event(&trigger, 0));
    }

    // ── Trigger matching edge cases ───────────────────────────────────────────

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

        for kind in sprout_core::kind::KIND_WORKFLOW_TRIGGERED
            ..=sprout_core::kind::KIND_WORKFLOW_APPROVAL_DENIED
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

    // ── build_trigger_context ─────────────────────────────────────────────────

    fn make_message_event() -> sprout_core::StoredEvent {
        use nostr::{EventBuilder, Keys, Kind};
        use uuid::Uuid;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "hello world")
            .tags([])
            .sign_with_keys(&keys)
            .expect("sign");
        sprout_core::StoredEvent::new(event, Some(Uuid::new_v4()))
    }

    /// Create a reaction event with an `e` tag pointing to a target message.
    fn make_reaction_event() -> (sprout_core::StoredEvent, String) {
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
            sprout_core::StoredEvent::new(event, Some(Uuid::new_v4())),
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
        let stored = sprout_core::StoredEvent::new(event, None);
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

        let stored = sprout_core::StoredEvent::new(event, Some(Uuid::new_v4()));
        let ctx = build_trigger_context(&stored);

        // Should pick the LAST e tag (direct target), not the first (thread root)
        assert_eq!(ctx.message_id, direct_target_id.to_hex());
    }
}
