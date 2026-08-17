//! Minimal conversion from terminal ACP turn outcomes to bounded experience records.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use buzz_core::agent_experience::{
    build_experience_event, experience_projection_payload, redact_task_summary, ExperienceError,
    ExperienceOutcome, ExperienceRecordV1, MemoryScope, SkillVersionV1, ValidationResultV1,
};
use nostr::{Keys, PublicKey};

use crate::{
    experience_outbox::{ExperienceOutbox, ExperienceOutboxError},
    experience_projection::ExperienceProjector,
    pool::{PromptOutcome, PromptSource, TimeoutKind},
    relay::RestClient,
    skill_learning::{
        materialize::materialize_active_skills, rebuild::rebuild_registry,
        registry::PublicationKind, LearningOutcome, SkillLearningError, SkillLearningRuntime,
        TurnLearningEvidence,
    },
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
    pub skill_versions: Vec<SkillVersionV1>,
    pub validation_results: Vec<ValidationResultV1>,
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
            skill_versions: evidence.skill_versions,
            validation_results: evidence.validation_results,
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
    projector: Option<ExperienceProjector>,
    skill_learning: SkillLearningRuntime,
    skill_root: PathBuf,
    active_turns: Mutex<HashMap<String, RuntimeEvidence>>,
}

pub(crate) fn initialize_experience_runtime(
    path: &Path,
    agent_keys: Keys,
    owner_pubkey: PublicKey,
    rest_client: RestClient,
) -> Result<(), ExperienceRuntimeError> {
    if EXPERIENCE_RUNTIME.get().is_some() {
        return Ok(());
    }
    let projector = std::env::var("COMMAND_ADVISER_MEMORY_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .and_then(|endpoint| match ExperienceProjector::from_endpoint(&endpoint) {
            Ok(projector) => Some(projector),
            Err(error) => {
                tracing::warn!(%error, "experience_learning_degraded: Memory MCP endpoint rejected");
                None
            }
        });
    let specialist_id = agent_keys.public_key().to_hex();
    let skill_registry_path = std::env::var_os("BUZZ_ACP_SKILL_REGISTRY")
        .map(PathBuf::from)
        .unwrap_or_else(|| path.with_file_name(format!("{specialist_id}.skills.sqlite3")));
    let skill_root = std::env::var_os("BUZZ_ACP_SKILL_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".agents")
                .join("skills")
        });
    let skill_learning = SkillLearningRuntime::open(
        &skill_registry_path,
        agent_keys.clone(),
        owner_pubkey,
        &specialist_id,
    )?;
    let runtime = Arc::new(ExperienceRuntime {
        outbox: ExperienceOutbox::open(path)?,
        agent_keys,
        owner_pubkey,
        rest_client,
        projector,
        skill_learning,
        skill_root,
        active_turns: Mutex::new(HashMap::new()),
    });
    if EXPERIENCE_RUNTIME.set(Arc::clone(&runtime)).is_ok() {
        tokio::spawn(async move {
            runtime.publish_pending().await;
            runtime.rebuild_skills_if_settled().await;
        });
    }
    Ok(())
}

pub(crate) struct BeginTurn<'a> {
    pub turn_id: &'a str,
    pub occurred_at: &'a str,
    pub source: &'a PromptSource,
    pub source_event_ids: &'a [String],
    pub specialist_id: &'a str,
    pub task_text: &'a str,
    pub model_identity: Option<&'a str>,
    pub harness_name: &'a str,
}

pub(crate) fn begin_turn(input: BeginTurn<'_>) {
    let Some(runtime) = EXPERIENCE_RUNTIME.get() else {
        return;
    };
    let team_id = match input.source {
        PromptSource::Channel(channel_id) => channel_id.to_string(),
        PromptSource::Heartbeat => "heartbeat".to_string(),
    };
    let source_ids = input
        .source_event_ids
        .iter()
        .map(|event_id| format!("buzz:{event_id}"))
        .collect::<Vec<_>>();
    let task_summary = redact_task_summary(input.task_text);
    let skill_versions = runtime
        .skill_learning
        .registry()
        .active_versions()
        .map(|versions| {
            versions
                .into_iter()
                .map(|version| SkillVersionV1 {
                    skill_id: version.skill_id,
                    version: version.version_id,
                })
                .collect()
        })
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "skill_learning_degraded: active skill snapshot failed");
            Vec::new()
        });
    let evidence = RuntimeEvidence {
        record_id: input.turn_id.to_string(),
        memory_key: format!("turn.{}", input.turn_id),
        occurred_at: input.occurred_at.to_string(),
        task_summary,
        specialist_id: input.specialist_id.to_string(),
        team_id,
        source_ids,
        model_identity: input
            .model_identity
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("unreported:{}", input.harness_name)),
        prompt_template_id: "buzz-acp-v1".to_string(),
        memory_view_revision: "runtime-active".to_string(),
        rag_snapshot_id: "runtime-current".to_string(),
        skill_versions,
        validation_results: vec![],
    };
    match runtime.active_turns.lock() {
        Ok(mut active_turns) => {
            active_turns.insert(input.turn_id.to_string(), evidence);
        }
        Err(_) => tracing::warn!(
            turn_id = input.turn_id,
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
    let learning_outcome = match &turn_outcome {
        TurnOutcome::Completed => Some(LearningOutcome::Succeeded),
        TurnOutcome::Failed { .. } => Some(LearningOutcome::Failed),
        _ => None,
    };
    let learning_evidence = learning_outcome.map(|outcome| TurnLearningEvidence {
        experience_id: evidence.record_id.clone(),
        occurred_at: evidence.occurred_at.clone(),
        task_text: evidence.task_summary.clone(),
        outcome,
    });
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
    if let Some(evidence) = learning_evidence {
        if let Err(error) = runtime.skill_learning.observe_turn(evidence) {
            tracing::warn!(turn_id, %error, "skill_learning_degraded: turn observation failed");
        }
    }
    tokio::spawn(async move {
        runtime.publish_pending().await;
        runtime.publish_skill_pending().await;
    });
}

impl ExperienceRuntime {
    fn enqueue_record(&self, record: &ExperienceRecordV1) -> Result<(), ExperienceRuntimeError> {
        let created_at = chrono::Utc::now().timestamp().max(0) as u64;
        let event =
            build_experience_event(&self.agent_keys, &self.owner_pubkey, record, created_at)?;
        let projection = experience_projection_payload(&event, &self.owner_pubkey, record)?;
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
        if let Some(projector) = &self.projector {
            let report = projector.project_pending(&self.outbox).await;
            if report.delayed > 0 || report.poisoned > 0 {
                tracing::warn!(
                    projected = report.projected,
                    delayed = report.delayed,
                    poisoned = report.poisoned,
                    "experience_learning_degraded: Memory MCP projection incomplete"
                );
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

    async fn publish_skill_pending(&self) {
        loop {
            let work = match self.skill_learning.registry().ready_for_publish() {
                Ok(work) => work,
                Err(error) => {
                    tracing::warn!(%error, "skill_learning_degraded: publication read failed");
                    return;
                }
            };
            if work.is_empty() {
                break;
            }
            let mut advanced = false;
            for item in work {
                match self.rest_client.submit_event(&item.event).await {
                    Ok(response) if relay_accepted(&response) => {
                        let result = match item.kind {
                            PublicationKind::Version => self
                                .skill_learning
                                .registry()
                                .mark_version_published(&item.operation_id),
                            PublicationKind::Pointer => self
                                .skill_learning
                                .registry()
                                .mark_pointer_published(&item.operation_id),
                        };
                        if let Err(error) = result {
                            tracing::warn!(operation_id = %item.operation_id, %error, "skill_learning_degraded: publication checkpoint failed");
                        } else {
                            advanced = true;
                        }
                    }
                    Ok(response) => tracing::warn!(
                        operation_id = %item.operation_id,
                        response = %response,
                        "skill_learning_degraded: relay rejected skill event"
                    ),
                    Err(error) => tracing::warn!(
                        operation_id = %item.operation_id,
                        %error,
                        "skill_learning_degraded: skill publication delayed"
                    ),
                }
            }
            if !advanced {
                break;
            }
        }

        let operations = match self.skill_learning.registry().pending_materializations() {
            Ok(operations) => operations,
            Err(error) => {
                tracing::warn!(%error, "skill_learning_degraded: materialization read failed");
                return;
            }
        };
        for operation_id in operations {
            let versions = match self.skill_learning.registry().active_versions() {
                Ok(versions) => versions,
                Err(error) => {
                    tracing::warn!(%error, "skill_learning_degraded: active skill read failed");
                    continue;
                }
            };
            if let Err(error) = materialize_active_skills(&self.skill_root, &versions) {
                tracing::warn!(%error, "skill_learning_degraded: projection failed");
                continue;
            }
            if let Err(error) = self
                .skill_learning
                .registry()
                .mark_materialized(&operation_id)
            {
                tracing::warn!(%error, "skill_learning_degraded: materialization checkpoint failed");
            }
        }
    }

    async fn rebuild_skills_if_settled(&self) {
        match self.skill_learning.registry().has_inflight() {
            Ok(false) => {}
            Ok(true) => {
                self.publish_skill_pending().await;
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "skill_learning_degraded: startup state read failed");
                return;
            }
        }
        if let Err(error) = rebuild_registry(
            &self.rest_client,
            &self.agent_keys,
            &self.owner_pubkey,
            self.skill_learning.registry(),
            &self.skill_root,
        )
        .await
        {
            tracing::warn!(%error, "skill_learning_degraded: signed rebuild unavailable");
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
pub(crate) enum ExperienceRuntimeError {
    #[error(transparent)]
    Experience(#[from] ExperienceError),
    #[error(transparent)]
    Outbox(#[from] ExperienceOutboxError),
    #[error(transparent)]
    SkillLearning(#[from] SkillLearningError),
}
