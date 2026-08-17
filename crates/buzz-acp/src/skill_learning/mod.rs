pub(crate) mod candidate;
pub(crate) mod evaluate;
pub(crate) mod registry;

use std::path::Path;

use buzz_core::agent_skill::{
    build_skill_pointer_event, build_skill_version_event, SkillPointerReason, SkillPointerV1,
};
use chrono::DateTime;
use nostr::{Keys, PublicKey};
use sha2::{Digest, Sha256};

use self::{
    candidate::{build_candidate, normalize_task, skill_id_for_task, CandidateInput},
    evaluate::evaluate_candidate,
    registry::{InsertObservation, RegistryError, SkillRegistry},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LearningOutcome {
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TurnLearningEvidence {
    pub experience_id: String,
    pub occurred_at: String,
    pub task_text: String,
    pub outcome: LearningOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LearningAction {
    None,
    Promoted {
        skill_id: String,
        version_id: String,
    },
    RolledBack {
        skill_id: String,
        version_id: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SkillLearningError {
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error("skill contract construction failed")]
    Contract,
    #[error("skill learning evidence is invalid")]
    InvalidEvidence,
}

pub(crate) struct SkillLearningRuntime {
    registry: SkillRegistry,
    agent_keys: Keys,
    owner_pubkey: PublicKey,
    specialist_id: String,
}

impl SkillLearningRuntime {
    pub(crate) fn open(
        path: &Path,
        agent_keys: Keys,
        owner_pubkey: PublicKey,
        specialist_id: &str,
    ) -> Result<Self, SkillLearningError> {
        if specialist_id.is_empty() {
            return Err(SkillLearningError::InvalidEvidence);
        }
        Ok(Self {
            registry: SkillRegistry::open(path)?,
            agent_keys,
            owner_pubkey,
            specialist_id: specialist_id.to_string(),
        })
    }

    pub(crate) fn registry(&self) -> &SkillRegistry {
        &self.registry
    }

    pub(crate) fn observe_turn(
        &self,
        evidence: TurnLearningEvidence,
    ) -> Result<LearningAction, SkillLearningError> {
        let normalized_task = normalize_task(&evidence.task_text);
        if normalized_task.is_empty()
            || evidence.experience_id.is_empty()
            || DateTime::parse_from_rfc3339(&evidence.occurred_at).is_err()
        {
            return Err(SkillLearningError::InvalidEvidence);
        }
        let task_hash = hex::encode(Sha256::digest(normalized_task.as_bytes()));
        let skill_id = skill_id_for_task(&normalized_task);
        let active_version_id = self.registry.active_version(&skill_id)?;
        let outcome = match evidence.outcome {
            LearningOutcome::Succeeded => "succeeded",
            LearningOutcome::Failed => "failed",
        };
        let inserted = self.registry.insert_observation(
            &evidence.experience_id,
            &task_hash,
            &normalized_task,
            outcome,
            &evidence.occurred_at,
            active_version_id.as_deref(),
        )?;
        if inserted == InsertObservation::Duplicate {
            return Ok(LearningAction::None);
        }

        match evidence.outcome {
            LearningOutcome::Succeeded => self.observe_success(&task_hash, &normalized_task),
            LearningOutcome::Failed => {
                self.observe_failure(&task_hash, &skill_id, active_version_id.as_deref())
            }
        }
    }

    fn observe_success(
        &self,
        task_hash: &str,
        normalized_task: &str,
    ) -> Result<LearningAction, SkillLearningError> {
        let skill_id = skill_id_for_task(normalized_task);
        if self.registry.has_inflight_for_skill(&skill_id)? {
            return Ok(LearningAction::None);
        }
        let observations = self.registry.unconsumed_successes(task_hash)?;
        if observations.len() < 2 {
            return Ok(LearningAction::None);
        }
        let parent = self
            .registry
            .active_version(&skill_id)?
            .map(|version_id| self.registry.version(&version_id))
            .transpose()?
            .flatten();
        let source_ids = observations
            .iter()
            .map(|observation| observation.experience_id.clone())
            .collect::<Vec<_>>();
        let created_at = observations
            .last()
            .map(|observation| observation.occurred_at.as_str())
            .ok_or(SkillLearningError::InvalidEvidence)?;
        let candidate = build_candidate(CandidateInput {
            normalized_task,
            source_experience_ids: source_ids.clone(),
            specialist_id: &self.specialist_id,
            created_at,
            parent: parent.as_ref(),
        });
        let evaluation = match evaluate_candidate(&candidate, parent.as_ref()) {
            Ok(evaluation) => evaluation,
            Err(_) => {
                self.registry.consume_observations(&source_ids)?;
                return Ok(LearningAction::None);
            }
        };
        let created_at = DateTime::parse_from_rfc3339(&candidate.created_at)
            .map_err(|_| SkillLearningError::InvalidEvidence)?
            .timestamp()
            .max(0) as u64;
        let version_event =
            build_skill_version_event(&self.agent_keys, &self.owner_pubkey, &candidate, created_at)
                .map_err(|_| SkillLearningError::Contract)?;
        let pointer = SkillPointerV1 {
            skill_id: candidate.skill_id.clone(),
            active_version_id: candidate.version_id.clone(),
            previous_version_id: candidate.parent_version_id.clone(),
            scope: candidate.scope,
            specialist_id: candidate.specialist_id.clone(),
            team_id: candidate.team_id.clone(),
            changed_at: candidate.created_at.clone(),
            reason: SkillPointerReason::Promotion,
            evaluation_ids: vec![evaluation.evaluation_id.clone()],
        };
        let pointer_event =
            build_skill_pointer_event(&self.agent_keys, &self.owner_pubkey, &pointer, created_at)
                .map_err(|_| SkillLearningError::Contract)?;
        self.registry.queue_promotion(
            &candidate,
            &evaluation.evaluation_id,
            &evaluation.check_ids,
            &version_event,
            &pointer_event,
        )?;
        self.registry.consume_observations(&source_ids)?;
        Ok(LearningAction::Promoted {
            skill_id: candidate.skill_id,
            version_id: candidate.version_id,
        })
    }

    fn observe_failure(
        &self,
        task_hash: &str,
        skill_id: &str,
        active_version_id: Option<&str>,
    ) -> Result<LearningAction, SkillLearningError> {
        let Some(active_version_id) = active_version_id else {
            return Ok(LearningAction::None);
        };
        let active = self
            .registry
            .version(active_version_id)?
            .ok_or(RegistryError::NotFound)?;
        let Some(parent_version_id) = active.parent_version_id.as_deref() else {
            return Ok(LearningAction::None);
        };
        let failures = self
            .registry
            .unconsumed_failures(task_hash, active_version_id)?;
        if failures.len() < 2 {
            return Ok(LearningAction::None);
        }
        let failure_ids = failures
            .iter()
            .map(|failure| failure.experience_id.clone())
            .collect::<Vec<_>>();
        let seed = format!(
            "{}\0{}\0{}\0{}",
            skill_id,
            active_version_id,
            parent_version_id,
            failure_ids.join("\0")
        );
        let hash = hex::encode(Sha256::digest(seed.as_bytes()));
        let operation_id = format!("rollback-{hash}");
        let evaluation_id = format!("evaluation-{hash}");
        let changed_at = failures
            .last()
            .map(|failure| failure.occurred_at.as_str())
            .ok_or(SkillLearningError::InvalidEvidence)?;
        let pointer = SkillPointerV1 {
            skill_id: skill_id.to_string(),
            active_version_id: parent_version_id.to_string(),
            previous_version_id: Some(active_version_id.to_string()),
            scope: active.scope,
            specialist_id: active.specialist_id,
            team_id: active.team_id,
            changed_at: changed_at.to_string(),
            reason: SkillPointerReason::Rollback,
            evaluation_ids: vec![evaluation_id],
        };
        let created_at = DateTime::parse_from_rfc3339(changed_at)
            .map_err(|_| SkillLearningError::InvalidEvidence)?
            .timestamp()
            .max(0) as u64;
        let pointer_event =
            build_skill_pointer_event(&self.agent_keys, &self.owner_pubkey, &pointer, created_at)
                .map_err(|_| SkillLearningError::Contract)?;
        self.registry
            .queue_rollback(&operation_id, skill_id, parent_version_id, &pointer_event)?;
        self.registry.consume_observations(&failure_ids)?;
        Ok(LearningAction::RolledBack {
            skill_id: skill_id.to_string(),
            version_id: parent_version_id.to_string(),
        })
    }
}
