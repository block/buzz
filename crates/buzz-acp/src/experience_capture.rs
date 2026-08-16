//! Minimal conversion from terminal ACP turn outcomes to bounded experience records.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

use buzz_core::agent_experience::{
    build_experience_event, ExperienceError, ExperienceOutcome, ExperienceRecordV1, MemoryScope,
};
use nostr::{Keys, PublicKey};

use crate::{
    experience_outbox::{ExperienceOutbox, ExperienceOutboxError},
    pool::{PromptOutcome, PromptSource, TimeoutKind},
    relay::RestClient,
};

static EXPERIENCE_RUNTIME: OnceLock<Arc<ExperienceRuntime>> = OnceLock::new();

/// Terminal turn information retained by the learning path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TurnOutcome {
    Completed,
    Failed {
        code: String,
    },
    #[allow(dead_code)]
    OwnerCorrection {
        decision: String,
        supersedes: Vec<String>,
    },
    Cancelled,
    NonSubstantive,
}

/// Bounded runtime facts needed to describe a turn without storing its transcript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeEvidence {
    pub record_id: String,
    pub memory_key: String,
    pub occurred_at: String,
    pub task_summary: String,
    pub specialist_id: String,
    pub team_id: String,
    pub source_ids: Vec<String>,
    pub model_identity: String,
    pub prompt_template_id: String,
    pub memory_view_revision: String,
    pub rag_snapshot_id: String,
}

/// Builds an immutable experience from bounded runtime evidence.
pub(crate) struct ExperienceCapture;

impl ExperienceCapture {
    pub(crate) fn from_turn(
        outcome: TurnOutcome,
        evidence: RuntimeEvidence,
    ) -> Result<Option<ExperienceRecordV1>, ExperienceError> {
        if outcome == TurnOutcome::NonSubstantive {
            return Ok(None);
        }

        let (outcome, decision, supersedes, limitations, confidence) = match outcome {
            TurnOutcome::Completed => (ExperienceOutcome::Succeeded, None, vec![], vec![], 1.0),
            TurnOutcome::Failed { code } => (
                ExperienceOutcome::Failed,
                None,
                vec![],
                vec![format!("Turn ended with failure code: {code}")],
                0.0,
            ),
            TurnOutcome::OwnerCorrection {
                decision,
                supersedes,
            } => (
                ExperienceOutcome::Corrected,
                Some(decision),
                supersedes,
                vec![],
                1.0,
            ),
            TurnOutcome::Cancelled => (
                ExperienceOutcome::Cancelled,
                None,
                vec![],
                vec!["Turn was cancelled before completion.".to_string()],
                0.0,
            ),
            TurnOutcome::NonSubstantive => return Ok(None),
        };

        let record = ExperienceRecordV1 {
            record_id: evidence.record_id,
            memory_key: evidence.memory_key,
            scope: MemoryScope::SpecialistPrivate,
            specialist_id: Some(evidence.specialist_id),
            team_id: Some(evidence.team_id),
            occurred_at: evidence.occurred_at,
            task_summary: evidence.task_summary,
            decision,
            assumptions: vec![],
            dissent: vec![],
            limitations,
            outcome,
            tool_evidence: vec![],
            source_ids: evidence.source_ids,
            model_identity: evidence.model_identity,
            prompt_template_id: evidence.prompt_template_id,
            memory_view_revision: evidence.memory_view_revision,
            rag_snapshot_id: evidence.rag_snapshot_id,
            skill_versions: vec![],
            validation_results: vec![],
            supersedes,
            confidence,
        };
        record.validate()?;
        Ok(Some(record))
    }
}

/// Local durability and best-effort relay publication for captured turns.
pub(crate) struct ExperienceRuntime {
    outbox: ExperienceOutbox,
    agent_keys: Keys,
    owner_pubkey: PublicKey,
    rest_client: RestClient,
    active_turns: Mutex<HashMap<String, RuntimeEvidence>>,
}

pub(crate) fn initialize_experience_runtime(
    path: &Path,
    agent_keys: Keys,
    owner_pubkey: PublicKey,
    rest_client: RestClient,
) -> Result<(), ExperienceOutboxError> {
    if EXPERIENCE_RUNTIME.get().is_some() {
        return Ok(());
    }
    let runtime = Arc::new(ExperienceRuntime {
        outbox: ExperienceOutbox::open(path)?,
        agent_keys,
        owner_pubkey,
        rest_client,
        active_turns: Mutex::new(HashMap::new()),
    });
    if EXPERIENCE_RUNTIME.set(Arc::clone(&runtime)).is_ok() {
        tokio::spawn(async move {
            runtime.publish_pending().await;
        });
    }
    Ok(())
}

pub(crate) fn begin_turn(
    turn_id: &str,
    occurred_at: &str,
    source: &PromptSource,
    source_event_ids: &[String],
    specialist_id: &str,
    model_identity: Option<&str>,
    harness_name: &str,
) {
    let Some(runtime) = EXPERIENCE_RUNTIME.get() else {
        return;
    };
    let team_id = match source {
        PromptSource::Channel(channel_id) => channel_id.to_string(),
        PromptSource::Heartbeat => "heartbeat".to_string(),
    };
    let source_ids = source_event_ids
        .iter()
        .map(|event_id| format!("buzz:{event_id}"))
        .collect::<Vec<_>>();
    let task_summary = if source_ids.is_empty() {
        "Completed a scheduled adviser turn.".to_string()
    } else {
        format!("Processed {} triggering Buzz event(s).", source_ids.len())
    };
    let evidence = RuntimeEvidence {
        record_id: turn_id.to_string(),
        memory_key: format!("turn.{turn_id}"),
        occurred_at: occurred_at.to_string(),
        task_summary,
        specialist_id: specialist_id.to_string(),
        team_id,
        source_ids,
        model_identity: model_identity
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("unreported:{harness_name}")),
        prompt_template_id: "buzz-acp-v1".to_string(),
        memory_view_revision: "runtime-active".to_string(),
        rag_snapshot_id: "runtime-current".to_string(),
    };
    match runtime.active_turns.lock() {
        Ok(mut active_turns) => {
            active_turns.insert(turn_id.to_string(), evidence);
        }
        Err(_) => tracing::warn!(
            turn_id,
            "experience_learning_degraded: active turn lock failed"
        ),
    }
}

pub(crate) fn finish_turn(turn_id: &str, outcome: &PromptOutcome) {
    let Some(runtime) = EXPERIENCE_RUNTIME.get().cloned() else {
        return;
    };
    let evidence = match runtime.active_turns.lock() {
        Ok(mut active_turns) => active_turns.remove(turn_id),
        Err(_) => {
            tracing::warn!(
                turn_id,
                "experience_learning_degraded: active turn lock failed"
            );
            None
        }
    };
    let Some(evidence) = evidence else {
        return;
    };
    let turn_outcome = match outcome {
        PromptOutcome::Ok(_) => TurnOutcome::Completed,
        PromptOutcome::Cancelled | PromptOutcome::CancelDrainTimeout(_) => TurnOutcome::Cancelled,
        PromptOutcome::AgentExited => TurnOutcome::Failed {
            code: "agent_exited".to_string(),
        },
        PromptOutcome::Timeout(TimeoutKind::Idle) => TurnOutcome::Failed {
            code: "idle_timeout".to_string(),
        },
        PromptOutcome::Timeout(TimeoutKind::Hard { .. }) => TurnOutcome::Failed {
            code: "hard_timeout".to_string(),
        },
        PromptOutcome::Error(_) => TurnOutcome::Failed {
            code: "agent_error".to_string(),
        },
    };
    let record = match ExperienceCapture::from_turn(turn_outcome, evidence) {
        Ok(Some(record)) => record,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(turn_id, %error, "experience_learning_degraded: capture rejected");
            return;
        }
    };
    if let Err(error) = runtime.enqueue_record(&record) {
        tracing::warn!(turn_id, %error, "experience_learning_degraded: durable enqueue failed");
        return;
    }
    tokio::spawn(async move {
        runtime.publish_pending().await;
    });
}

impl ExperienceRuntime {
    fn enqueue_record(&self, record: &ExperienceRecordV1) -> Result<(), ExperienceRuntimeError> {
        let created_at = chrono::Utc::now().timestamp().max(0) as u64;
        let event =
            build_experience_event(&self.agent_keys, &self.owner_pubkey, record, created_at)?;
        let status = if matches!(
            record.outcome,
            ExperienceOutcome::Succeeded | ExperienceOutcome::Corrected
        ) {
            "active"
        } else {
            "inactive"
        };
        let projection = serde_json::json!({
            "source_event_id": event.id.to_hex(),
            "timestamp": record.occurred_at,
            "agent": record.specialist_id,
            "event_type": "command_experience",
            "content": record.task_summary,
            "metadata": {
                "memory_key": record.memory_key,
                "status": status,
                "scope": record.scope,
                "owner_id": self.owner_pubkey.to_hex(),
                "team_id": record.team_id,
                "specialist_id": record.specialist_id,
                "confidence": record.confidence,
                "supersedes": record.supersedes,
                "source_event_id": event.id.to_hex(),
                "source_created_at": created_at
            }
        });
        self.outbox
            .enqueue(&record.record_id, &event, &projection)?;
        Ok(())
    }

    async fn publish_pending(&self) {
        let entries = match self.outbox.ready_for_publish() {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(%error, "experience_learning_degraded: outbox read failed");
                return;
            }
        };
        for entry in entries {
            match self.rest_client.submit_event(&entry.signed_event).await {
                Ok(response) if relay_accepted(&response) => {
                    if let Err(error) = self.outbox.mark_published(&entry.record_id) {
                        tracing::warn!(record_id = %entry.record_id, %error, "experience_learning_degraded: publish checkpoint failed");
                    }
                }
                Ok(response) => tracing::warn!(
                    record_id = %entry.record_id,
                    response = %response,
                    "experience_learning_degraded: relay rejected experience"
                ),
                Err(error) => tracing::warn!(
                    record_id = %entry.record_id,
                    %error,
                    "experience_learning_degraded: experience publish delayed"
                ),
            }
        }
        if let Ok(health) = self.outbox.health() {
            if health.pending > 0 || health.published > 0 {
                tracing::warn!(
                    pending = health.pending,
                    awaiting_projection = health.published,
                    "experience_learning_degraded: durable work remains"
                );
            }
        }
    }
}

fn relay_accepted(response: &serde_json::Value) -> bool {
    match response
        .get("accepted")
        .and_then(serde_json::Value::as_bool)
    {
        Some(true) | None => true,
        Some(false) => response
            .get("message")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| {
                let message = message.to_ascii_lowercase();
                message.contains("duplicate") || message.contains("already")
            }),
    }
}

#[derive(Debug, thiserror::Error)]
enum ExperienceRuntimeError {
    #[error(transparent)]
    Experience(#[from] ExperienceError),
    #[error(transparent)]
    Outbox(#[from] ExperienceOutboxError),
}
