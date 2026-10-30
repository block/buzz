//! `buzz agents metrics` — the caller's decrypted NIP-AM agent turn metrics
//! (kind:44200). Only the owner can read them: the relay gates reads to the
//! `p`-tagged owner and the payload is NIP-44 encrypted to that key.

use std::collections::HashSet;

use buzz_core::agent_turn_metric::decrypt_agent_turn_metric;
use buzz_core::kind::{KIND_AGENT_TURN_METRIC, KIND_MANAGED_AGENT};
use nostr::Event;
use serde_json::{json, Value};

use super::catalog::single_tag;
use crate::client::{extract_d_tag, BuzzClient};
use crate::error::CliError;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

fn is_pubkey_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// A 64-hex pubkey as given, or the unique managed agent of `owner` with that
/// (case-insensitive) name.
async fn resolve_agent(client: &BuzzClient, owner: &str, agent: &str) -> Result<String, CliError> {
    let agent = agent.trim();
    if is_pubkey_hex(agent) {
        return Ok(agent.to_ascii_lowercase());
    }
    if agent.is_empty() {
        return Err(CliError::Usage("--agent must not be empty".into()));
    }
    let filter = json!({ "kinds": [KIND_MANAGED_AGENT], "authors": [owner] });
    let events = client.query_all_bounded(filter, 5_000).await?;
    let mut matches: Vec<String> = events
        .iter()
        .filter(|event| {
            event
                .get("content")
                .and_then(Value::as_str)
                .and_then(|content| serde_json::from_str::<Value>(content).ok())
                .and_then(|content| content.get("name")?.as_str().map(str::to_lowercase))
                .is_some_and(|name| name == agent.to_lowercase())
        })
        .map(extract_d_tag)
        .filter(|pubkey| is_pubkey_hex(pubkey))
        .collect();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [pubkey] => Ok(pubkey.to_ascii_lowercase()),
        [] => Err(CliError::NotFound(format!(
            "no managed agent named \"{agent}\"; pass its pubkey instead"
        ))),
        _ => Err(CliError::Usage(format!(
            "more than one managed agent is named \"{agent}\"; pass its pubkey instead"
        ))),
    }
}

fn read_metric(event: &Event, keys: &nostr::Keys) -> Result<Value, String> {
    event.verify().map_err(|e| format!("bad signature: {e}"))?;
    let owner = keys.public_key().to_hex();
    if single_tag(event, "p").as_deref() != Some(owner.as_str()) {
        return Err("p tag is not the reader".into());
    }
    let pubkey = event.pubkey.to_hex();
    if single_tag(event, "agent").as_deref() != Some(pubkey.as_str()) {
        return Err("agent tag is not the author".into());
    }
    let metric = decrypt_agent_turn_metric(keys, event).map_err(|e| e.to_string())?;
    Ok(json!({
        "event_id": event.id.to_hex(),
        "agent": pubkey,
        "created_at": event.created_at.as_secs(),
        "metric": metric,
    }))
}

/// Query, validate, and decrypt metrics. Events that fail any check are
/// counted and warned about; if every returned event fails, that is an error
/// rather than an empty success.
pub(crate) async fn run(
    client: &BuzzClient,
    agent: Option<&str>,
    since: Option<i64>,
    limit: Option<u32>,
) -> Result<Value, CliError> {
    if client.auth_tag_owner_hex().is_some() {
        return Err(CliError::Auth(
            "agent metrics are encrypted to the owner; run this with the owner's key, not an agent's".into(),
        ));
    }
    let limit = limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    if limit == 0 {
        return Err(CliError::Usage("--limit must be at least 1".into()));
    }
    let since = since
        .map(|since| {
            u64::try_from(since).map_err(|_| {
                CliError::Usage("--since must be a non-negative unix timestamp".into())
            })
        })
        .transpose()?;
    let me = client.keys().public_key().to_hex();
    let mut filter = json!({ "kinds": [KIND_AGENT_TURN_METRIC], "#p": [me] });
    if let Some(agent) = agent {
        filter["authors"] = json!([resolve_agent(client, &me, agent).await?]);
    }
    if let Some(since) = since {
        filter["since"] = json!(since);
    }

    let raw = client.query_paginated(filter, limit).await?;
    let mut seen = HashSet::new();
    let mut metrics = Vec::new();
    let mut failed = 0usize;
    let mut first_failure = None;
    for value in raw {
        let result = serde_json::from_value::<Event>(value)
            .map_err(|e| format!("unparsable event: {e}"))
            .and_then(|event| {
                if !seen.insert(event.id) {
                    return Ok(None);
                }
                read_metric(&event, client.keys()).map(Some)
            });
        match result {
            Ok(Some(metric)) => metrics.push(metric),
            Ok(None) => {}
            Err(reason) => {
                failed += 1;
                first_failure.get_or_insert(reason);
            }
        }
    }
    if let Some(reason) = first_failure {
        if metrics.is_empty() {
            return Err(CliError::Other(format!(
                "all {failed} metric events failed validation or decryption (first: {reason}); only the owner key can read agent metrics"
            )));
        }
        eprintln!("warning: skipped {failed} metric events that failed validation or decryption (first: {reason})");
    }
    metrics.sort_by(|a, b| {
        b["created_at"]
            .as_u64()
            .cmp(&a["created_at"].as_u64())
            .then_with(|| a["event_id"].as_str().cmp(&b["event_id"].as_str()))
    });
    Ok(Value::Array(metrics))
}

pub async fn dispatch(
    client: &BuzzClient,
    agent: Option<&str>,
    since: Option<i64>,
    limit: Option<u32>,
) -> Result<(), CliError> {
    println!("{}", run(client, agent, since, limit).await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::catalog::test_support::*;
    use buzz_core::agent_turn_metric::{encrypt_agent_turn_metric, AgentTurnMetricPayload};
    use nostr::Keys;

    fn payload(session: &str, seq: u64, ts: &str) -> AgentTurnMetricPayload {
        serde_json::from_value(json!({
            "harness": "goose", "model": "m", "sessionId": session, "turnSeq": seq,
            "timestamp": ts,
            "turn": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15, "costUsd": 0.5},
            "cumulative": {"inputTokens": 10 * seq, "outputTokens": 5, "totalTokens": 15, "costUsd": 0.5},
        }))
        .unwrap()
    }

    fn metric(agent: &Keys, owner: &Keys, seq: u64, at: u64) -> Value {
        let content = encrypt_agent_turn_metric(
            agent,
            &owner.public_key(),
            &payload("s", seq, "2026-01-01T00:00:00Z"),
        )
        .unwrap();
        let (agent_pk, owner_pk) = (agent.public_key().to_hex(), owner.public_key().to_hex());
        event(
            agent,
            44200,
            &content,
            &[&["p", &owner_pk], &["agent", &agent_pk]],
            at,
        )
    }

    #[tokio::test]
    async fn reads_decrypted_metrics_filtered_by_agent_name_or_pubkey_and_time() {
        let (client, relay) = setup().await;
        let owner = client.keys().clone();
        let (scout, rover) = (Keys::generate(), Keys::generate());
        seed(&relay, metric(&scout, &owner, 1, 100));
        seed(&relay, metric(&scout, &owner, 2, 200));
        seed(&relay, metric(&rover, &owner, 1, 150));
        // Someone else's metrics are never returned.
        seed(&relay, metric(&rover, &Keys::generate(), 9, 150));
        let agent_event = |keys: &Keys, name: &str| {
            event(
                &owner,
                30177,
                &json!({"name": name}).to_string(),
                &[&["d", &keys.public_key().to_hex()]],
                1,
            )
        };
        seed(&relay, agent_event(&scout, "Scout"));
        seed(&relay, agent_event(&rover, "Rover"));
        seed(&relay, agent_event(&Keys::generate(), "Twin"));
        seed(&relay, agent_event(&Keys::generate(), "twin"));

        let all = run(&client, None, None, None).await.unwrap();
        let ats: Vec<u64> = all
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["created_at"].as_u64().unwrap())
            .collect();
        assert_eq!(ats, [200, 150, 100]);
        assert_eq!(all[0]["metric"]["harness"], "goose");
        assert_eq!(all[0]["metric"]["turn"]["totalTokens"], 15);
        assert_eq!(all[0]["metric"]["turnSeq"], 2);
        assert_eq!(all[0]["agent"], scout.public_key().to_hex());

        let by_name = run(&client, Some("scout"), None, None).await.unwrap();
        assert_eq!(by_name.as_array().unwrap().len(), 2);
        let by_key = run(&client, Some(&rover.public_key().to_hex()), None, None)
            .await
            .unwrap();
        assert_eq!(by_key.as_array().unwrap().len(), 1);
        let since = run(&client, None, Some(150), None).await.unwrap();
        assert_eq!(since.as_array().unwrap().len(), 2);

        assert!(matches!(
            run(&client, Some("twin"), None, None).await,
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            run(&client, Some("nobody"), None, None).await,
            Err(CliError::NotFound(_))
        ));
        assert!(matches!(
            run(&client, None, None, Some(0)).await,
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            run(&client, None, Some(-1), None).await,
            Err(CliError::Usage(_))
        ));

        relay.lock().unwrap().filters.clear();
        run(&client, None, None, Some(10_000)).await.unwrap();
        let filters = relay.lock().unwrap().filters.clone();
        assert_eq!(filters[0]["limit"], json!(MAX_LIMIT));
        assert_eq!(filters[0]["#p"], json!([owner.public_key().to_hex()]));
        assert!(every_filter_names_kinds(&relay));
    }

    #[tokio::test]
    async fn bad_events_are_reported_and_never_become_an_empty_success() {
        let (client, relay) = setup().await;
        let owner = client.keys().clone();
        let agent = Keys::generate();
        let (agent_pk, owner_pk) = (agent.public_key().to_hex(), owner.public_key().to_hex());

        // Undecryptable and mismatched agent tag arrive through the relay; a p
        // tag for another reader is refused by the CLI even if a relay returned it.
        let garbage = event(
            &agent,
            44200,
            "x",
            &[&["p", &owner_pk], &["agent", &agent_pk]],
            10,
        );
        let stranger = Keys::generate().public_key().to_hex();
        let content = encrypt_agent_turn_metric(
            &agent,
            &owner.public_key(),
            &payload("s", 1, "2026-01-01T00:00:00Z"),
        )
        .unwrap();
        let wrong_p = event(
            &agent,
            44200,
            &content,
            &[&["p", &stranger], &["agent", &agent_pk]],
            12,
        );
        let wrong_p: Event = serde_json::from_value(wrong_p).unwrap();
        assert!(read_metric(&wrong_p, &owner).unwrap_err().contains("p tag"));
        let wrong_agent = event(
            &agent,
            44200,
            &content,
            &[&["p", &owner_pk], &["agent", &owner_pk]],
            11,
        );
        let unchecked: Event = serde_json::from_value(wrong_agent.clone()).unwrap();
        assert!(read_metric(&unchecked, &owner)
            .unwrap_err()
            .contains("agent tag"));
        for bad in [garbage, wrong_agent] {
            seed(&relay, bad);
        }
        let err = run(&client, None, None, None).await.unwrap_err();
        assert!(
            matches!(err, CliError::Other(ref m) if m.contains("all 2")),
            "{err:?}"
        );

        // One good event among bad ones still succeeds.
        seed(&relay, metric(&agent, &owner, 1, 20));
        let ok = run(&client, None, None, None).await.unwrap();
        assert_eq!(ok.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_agent_identity_is_refused_not_given_an_empty_list() {
        let (owner_client, _relay) = setup().await;
        let tag_vec = ["auth", &"a".repeat(64), "conditions", &"b".repeat(128)].map(String::from);
        let tag = nostr::Tag::parse(tag_vec.clone()).unwrap();
        let json = serde_json::to_string(&tag_vec).unwrap();
        let agent_client = BuzzClient::new(
            "http://127.0.0.1:1".into(),
            owner_client.keys().clone(),
            Some(tag),
            Some(json),
        )
        .unwrap();
        let err = run(&agent_client, None, None, None).await.unwrap_err();
        assert!(matches!(err, CliError::Auth(_)), "{err:?}");
    }
}
