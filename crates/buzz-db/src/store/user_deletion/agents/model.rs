//! Saved, bounded per-agent progress. Phase routing belongs to buzz-deletion.
use super::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Internal database cleanup operation, not a workflow stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDbPhase {
    /// Reuse message removal and dependent-record cleanup.
    Messages,
    /// Memory, metrics, lists, reminders and read progress.
    PrivateState,
    /// Agent-authored job protocol events.
    JobRecords,
    /// Agent-owned workflow definitions and execution records.
    OwnedWorkflows,
    /// Subscriptions, push state and recipient delivery records.
    DeliveryState,
    /// Independent absence checks for the supported database scope.
    VerifyDbContent,
}

/// Saved outcome for a single approved agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDbStatus {
    /// No work started.
    Pending,
    /// Cleanup is in progress.
    Running,
    /// Last attempt failed; phase is the resume position.
    Failed,
    /// Supported database content was verified absent.
    Completed,
}

/// One agent's durable resume position; never contains message bodies.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDbProgress {
    /// Current state of this agent's database cleanup.
    pub status: AgentDbStatus,
    /// Internal phase to resume.
    pub phase: AgentDbPhase,
    /// Database-clock time of successful verification.
    pub completed_at: Option<DateTime<Utc>>,
    /// Most recent failure, bounded to 2048 characters.
    pub last_error: Option<String>,
}

/// Progress stored under `entity_deletion_stage_history.metadata_json.progress`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentsDbProgress {
    /// Serialization contract version.
    pub version: u8,
    /// Current agent, or null before execution/after all agents finish.
    pub current_agent: Option<String>,
    /// Status for every approved agent in this community.
    pub agents: BTreeMap<String, AgentDbProgress>,
}

impl AgentsDbProgress {
    pub(super) fn read(stage: &EntityStage, agents: &[String]) -> Result<Self> {
        let progress: Self = match stage.metadata_json.get("progress") {
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|_| safety("invalid saved agent progress"))?,
            None => Self {
                version: 1,
                current_agent: None,
                agents: agents
                    .iter()
                    .map(|key| {
                        (
                            key.clone(),
                            AgentDbProgress {
                                status: AgentDbStatus::Pending,
                                phase: AgentDbPhase::Messages,
                                completed_at: None,
                                last_error: None,
                            },
                        )
                    })
                    .collect(),
            },
        };
        if progress.version != 1
            || progress.agents.len() != agents.len()
            || agents.iter().any(|key| !progress.agents.contains_key(key))
        {
            return Err(safety(
                "saved agent progress differs from approved inventory",
            ));
        }
        let first = progress.next_agent(agents);
        if progress
            .current_agent
            .as_deref()
            .is_some_and(|key| Some(key) != first)
        {
            return Err(safety(
                "saved current agent is not the next unfinished agent",
            ));
        }
        let mut unfinished = false;
        for key in agents {
            let item = &progress.agents[key];
            if item.status == AgentDbStatus::Completed {
                if unfinished
                    || item.phase != AgentDbPhase::VerifyDbContent
                    || item.completed_at.is_none()
                    || item.last_error.is_some()
                {
                    return Err(safety("invalid completed agent progress"));
                }
            } else {
                if item.completed_at.is_some()
                    || (unfinished && item.status != AgentDbStatus::Pending)
                    || (item.status == AgentDbStatus::Pending
                        && item.phase != AgentDbPhase::Messages)
                    || (matches!(item.status, AgentDbStatus::Running | AgentDbStatus::Failed)
                        && progress.current_agent.as_deref() != Some(key.as_str()))
                {
                    return Err(safety("invalid unfinished agent progress"));
                }
                unfinished = true;
            }
        }
        Ok(progress)
    }

    pub(super) fn next_agent<'a>(&self, agents: &'a [String]) -> Option<&'a str> {
        agents
            .iter()
            .find(|key| self.agents[*key].status != AgentDbStatus::Completed)
            .map(String::as_str)
    }
}

/// Result of one bounded operation; the caller repeats while not exhausted.
#[derive(Debug, Serialize)]
pub struct AgentsDbBatch {
    /// Exact progress committed with this batch.
    pub progress: AgentsDbProgress,
    /// All approved agents completed supported database cleanup.
    pub exhausted: bool,
}
