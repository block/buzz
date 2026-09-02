use std::collections::{BTreeMap, BTreeSet};

use buzz_core_pkg::agent_turn_metric::{AgentTurnMetricPayload, StopReason};

use crate::{archive::store::AgentMetricArchiveRow, managed_agents::ManagedAgentSummary};

use super::sanitize::sanitize_name;

#[derive(Debug, Default)]
struct Totals {
    valid_turns: u64,
    errors: u64,
    cost: f64,
    incomplete_turns: bool,
    incomplete_errors: bool,
    incomplete_cost: bool,
}

fn number(value: u64, complete: bool) -> String {
    if complete {
        value.to_string()
    } else {
        format!("≥{value}")
    }
}

fn cost(value: f64, complete: bool) -> String {
    if complete {
        format!("${value:.6}")
    } else {
        format!("≥${value:.6}")
    }
}

pub(crate) fn aggregate_activity(
    rows: Vec<AgentMetricArchiveRow>,
    agents: &[ManagedAgentSummary],
    continuous_coverage: bool,
) -> String {
    let mut names = BTreeMap::new();
    let mut totals = BTreeMap::<String, Totals>::new();
    for agent in agents {
        names.insert(
            agent.pubkey.to_ascii_lowercase(),
            sanitize_name(&agent.name),
        );
        totals.entry(agent.pubkey.to_ascii_lowercase()).or_default();
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.id) {
            continue;
        }
        let author = row.author_pubkey.to_ascii_lowercase();
        let total = totals.entry(author).or_default();
        let Ok(payload) = serde_json::from_str::<AgentTurnMetricPayload>(&row.payload_json) else {
            total.incomplete_turns = true;
            total.incomplete_errors = true;
            total.incomplete_cost = true;
            continue;
        };
        if payload.validate().is_err() {
            total.incomplete_turns = true;
            total.incomplete_errors = true;
            total.incomplete_cost = true;
            continue;
        }
        total.valid_turns += 1;
        if payload.stop_reason == Some(StopReason::Error) {
            total.errors += 1;
        }
        if !payload.delta_reliable || payload.turn.is_none() {
            total.incomplete_turns = true;
            total.incomplete_cost = true;
        }
        match payload.turn.as_ref().and_then(|turn| turn.cost_usd) {
            Some(value) if payload.delta_reliable => total.cost += value,
            _ => total.incomplete_cost = true,
        }
    }

    let overall_complete = continuous_coverage
        && totals.values().all(|total| {
            !total.incomplete_turns && !total.incomplete_errors && !total.incomplete_cost
        });
    let mut lines = vec![format!(
        "Coverage: {}",
        if overall_complete {
            "Complete"
        } else {
            "Incomplete"
        }
    )];
    for (pubkey, total) in totals {
        let name = names
            .get(&pubkey)
            .cloned()
            .unwrap_or_else(|| format!("Agent {}", &pubkey[..pubkey.len().min(12)]));
        let turns_complete = continuous_coverage && !total.incomplete_turns;
        let errors_complete = continuous_coverage && !total.incomplete_errors;
        let cost_complete = continuous_coverage && !total.incomplete_cost;
        lines.push(format!(
            "- {name}: turns {}, estimated cost {}, errors {}",
            number(total.valid_turns, turns_complete),
            cost(total.cost, cost_complete),
            number(total.errors, errors_complete),
        ));
    }
    if lines.len() == 1 {
        lines.push("- None".to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, author: &str, payload: serde_json::Value) -> AgentMetricArchiveRow {
        AgentMetricArchiveRow {
            id: id.into(),
            author_pubkey: author.into(),
            created_at: 1,
            payload_json: payload.to_string(),
        }
    }

    #[test]
    fn syn79_activity_uses_unique_valid_per_turn_metrics_only() {
        let payload = serde_json::json!({
            "harness":"test", "model":null, "channelId":null,
            "sessionId":null, "turnId":"one", "turnSeq":null,
            "timestamp":"2026-09-02T00:30:00Z",
            "turn":{"inputTokens":1,"outputTokens":1,"totalTokens":2,"costUsd":0.25},
            "cumulative":{"inputTokens":100,"outputTokens":100,"totalTokens":200,"costUsd":99.0},
            "deltaReliable":true, "stopReason":"error"
        });
        let rows = vec![row("one", "aa", payload.clone()), row("one", "aa", payload)];
        let output = aggregate_activity(rows, &[], true);
        assert!(output.contains("turns 1, estimated cost $0.250000, errors 1"));
    }

    #[test]
    fn syn79_activity_never_guesses_zero_without_continuous_coverage() {
        let output = aggregate_activity(Vec::new(), &[], false);
        assert_eq!(output, "Coverage: Incomplete\n- None");
    }
}
