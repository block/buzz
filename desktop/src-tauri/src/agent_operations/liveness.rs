use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::managed_agents::ManagedAgentSummary;

use super::{
    sanitize::sanitize_name,
    types::{AlertBatch, OutageEpisode, ScopeDeliveryState},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Health {
    Healthy,
    Unhealthy(AlertKind),
    Ineligible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AlertKind {
    Stopped,
    Error,
}

#[derive(Debug, Clone)]
struct AgentObservation {
    pubkey: String,
    health: Health,
    last_exit_code: Option<i32>,
    last_error_code: Option<i64>,
}

impl From<&ManagedAgentSummary> for AgentObservation {
    fn from(agent: &ManagedAgentSummary) -> Self {
        let health = if !agent.start_on_app_launch
            || matches!(agent.status.as_str(), "deployed" | "not_deployed")
        {
            Health::Ineligible
        } else if agent.status != "stopped" {
            Health::Healthy
        } else if agent
            .last_error
            .as_deref()
            .is_some_and(|error| !error.trim().is_empty())
            || agent.last_error_code.is_some()
            || agent.last_exit_code.is_some_and(|code| code != 0)
        {
            Health::Unhealthy(AlertKind::Error)
        } else {
            Health::Unhealthy(AlertKind::Stopped)
        };
        Self {
            pubkey: agent.pubkey.to_ascii_lowercase(),
            health,
            last_exit_code: agent.last_exit_code.filter(|code| *code != 0),
            last_error_code: agent.last_error_code,
        }
    }
}

fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

pub(crate) fn observe(
    delivery: &mut ScopeDeliveryState,
    agents: &[ManagedAgentSummary],
    channel_id: &str,
    detected_at: i64,
) -> Option<String> {
    let observations = agents
        .iter()
        .map(AgentObservation::from)
        .collect::<Vec<_>>();
    observe_inner(delivery, observations, channel_id, detected_at)
}

fn observe_inner(
    delivery: &mut ScopeDeliveryState,
    observations: Vec<AgentObservation>,
    channel_id: &str,
    detected_at: i64,
) -> Option<String> {
    let by_pubkey = observations
        .iter()
        .map(|observation| (observation.pubkey.clone(), observation))
        .collect::<BTreeMap<_, _>>();
    for episode in delivery.episodes.values_mut() {
        if episode.active
            && by_pubkey
                .get(&episode.agent_pubkey)
                .is_some_and(|observation| observation.health == Health::Healthy)
        {
            episode.active = false;
        }
    }
    while delivery.episodes.len() >= 256 {
        let inactive_id = delivery
            .episodes
            .iter()
            .find(|(_, episode)| !episode.active)
            .map(|(id, _)| id.clone())?;
        delivery.episodes.remove(&inactive_id);
    }
    while delivery.alert_batches.len() >= 128 {
        let delivered_index = delivery
            .alert_batches
            .iter()
            .position(|batch| batch.event_id.is_some())?;
        delivery.alert_batches.remove(delivered_index);
    }

    let mut newly_unhealthy = Vec::new();
    for observation in observations {
        let kind = match observation.health.clone() {
            Health::Unhealthy(kind) => kind,
            _ => continue,
        };
        let already_active = delivery
            .episodes
            .values()
            .any(|episode| episode.agent_pubkey == observation.pubkey && episode.active);
        if already_active {
            continue;
        }
        let id = digest(&[channel_id, &observation.pubkey, &detected_at.to_string()]);
        delivery.episodes.insert(
            id.clone(),
            OutageEpisode {
                id: id.clone(),
                agent_pubkey: observation.pubkey.clone(),
                first_detected_at: detected_at,
                active: true,
                classification: match &kind {
                    AlertKind::Stopped => "Stopped",
                    AlertKind::Error => "Error",
                }
                .to_string(),
                last_exit_code: observation.last_exit_code,
                last_error_code: observation.last_error_code,
            },
        );
        newly_unhealthy.push((id, observation, kind));
    }
    if newly_unhealthy.is_empty() {
        return None;
    }
    newly_unhealthy.sort_by(|left, right| left.0.cmp(&right.0));
    let episode_ids = newly_unhealthy
        .iter()
        .map(|(id, _, _)| id.clone())
        .collect::<Vec<_>>();
    let joined_ids = episode_ids.join(":");
    let marker = format!(
        "buzz:ops-agent-outage:v1:{}",
        digest(&[channel_id, &joined_ids])
    );
    delivery.alert_batches.push(AlertBatch {
        marker: marker.clone(),
        episode_ids,
        event_id: None,
    });
    Some(marker)
}

pub(crate) fn render_pending_batch(
    delivery: &ScopeDeliveryState,
    batch: &AlertBatch,
    agents: &[ManagedAgentSummary],
) -> String {
    let names = agents
        .iter()
        .map(|agent| {
            (
                agent.pubkey.to_ascii_lowercase(),
                sanitize_name(&agent.name),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let detected_at = batch
        .episode_ids
        .iter()
        .filter_map(|id| delivery.episodes.get(id))
        .map(|episode| episode.first_detected_at)
        .min()
        .unwrap_or_default();
    let mut lines = vec![format!(
        "Start-on-launch agent outage detected at {}.",
        chrono::DateTime::from_timestamp(detected_at, 0)
            .unwrap_or_default()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )];
    for id in &batch.episode_ids {
        let Some(episode) = delivery.episodes.get(id) else {
            continue;
        };
        let name = names
            .get(&episode.agent_pubkey)
            .cloned()
            .unwrap_or_else(|| {
                format!(
                    "Agent {}",
                    &episode.agent_pubkey[..episode.agent_pubkey.len().min(12)]
                )
            });
        let mut line = format!("- {name}: {}", episode.classification);
        if let Some(code) = episode.last_exit_code {
            line.push_str(&format!("; exit code {code}"));
        }
        if let Some(code) = episode.last_error_code {
            line.push_str(&format!("; error code {code}"));
        }
        lines.push(line);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(pubkey: &str, health: Health) -> AgentObservation {
        AgentObservation {
            pubkey: pubkey.into(),
            health,
            last_exit_code: None,
            last_error_code: None,
        }
    }

    #[test]
    fn syn79_liveness_batches_suppresses_recovers_and_recurs_per_agent() {
        let mut state = ScopeDeliveryState::default();
        let first = observe_inner(
            &mut state,
            vec![
                observation("b", Health::Unhealthy(AlertKind::Stopped)),
                observation("a", Health::Unhealthy(AlertKind::Error)),
            ],
            "channel",
            10,
        )
        .unwrap();
        assert_eq!(state.alert_batches.len(), 1);
        assert!(observe_inner(
            &mut state,
            vec![observation("a", Health::Unhealthy(AlertKind::Error))],
            "channel",
            20
        )
        .is_none());
        assert!(observe_inner(
            &mut state,
            vec![observation("a", Health::Healthy)],
            "channel",
            30
        )
        .is_none());
        let second = observe_inner(
            &mut state,
            vec![observation("a", Health::Unhealthy(AlertKind::Stopped))],
            "channel",
            40,
        )
        .unwrap();
        assert_ne!(first, second);
        assert_eq!(state.alert_batches.len(), 2);
    }

    #[test]
    fn syn79_liveness_ignores_ineligible_records() {
        let mut state = ScopeDeliveryState::default();
        assert!(observe_inner(
            &mut state,
            vec![observation("a", Health::Ineligible)],
            "channel",
            10
        )
        .is_none());
    }

    #[test]
    fn syn79_liveness_recovers_and_reuses_bounded_delivery_capacity() {
        let mut state = ScopeDeliveryState::default();
        for index in 0..256 {
            let pubkey = format!("{index:064x}");
            let marker = observe_inner(
                &mut state,
                vec![observation(&pubkey, Health::Unhealthy(AlertKind::Stopped))],
                "channel",
                index,
            )
            .unwrap();
            state.alert_batches.last_mut().unwrap().event_id = Some(marker);
        }
        assert_eq!(state.episodes.len(), 256);
        assert!(observe_inner(
            &mut state,
            vec![observation(&format!("{:064x}", 255), Health::Healthy,)],
            "channel",
            300,
        )
        .is_none());
        assert!(observe_inner(
            &mut state,
            vec![observation(
                &format!("{:064x}", 256),
                Health::Unhealthy(AlertKind::Stopped),
            )],
            "channel",
            301,
        )
        .is_some());
        assert_eq!(state.episodes.len(), 256);
        assert_eq!(state.alert_batches.len(), 128);
    }
}
