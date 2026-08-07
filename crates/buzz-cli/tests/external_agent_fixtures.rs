use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/external_agent_v1")
}

#[derive(Debug, Deserialize)]
struct Contract {
    contract: String,
    agent_pubkey: String,
    owner_pubkey: String,
    allowlisted_pubkeys: Vec<String>,
    records: Vec<Fact>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct Fact {
    fixture: String,
    event_id: Option<String>,
    author_pubkey: Option<String>,
    channel_id: Option<String>,
    immediate_parent_id: Option<String>,
    thread_root_id: Option<String>,
    explicitly_mentions_agent: bool,
    is_self_authored: bool,
    author_policy_result: String,
    activation_result: String,
    conversation_lane: Option<String>,
}

fn null_fact(fixture: &str, author_policy: &str, activation: &str) -> Fact {
    Fact {
        fixture: fixture.to_string(),
        event_id: None,
        author_pubkey: None,
        channel_id: None,
        immediate_parent_id: None,
        thread_root_id: None,
        explicitly_mentions_agent: false,
        is_self_authored: false,
        author_policy_result: author_policy.to_string(),
        activation_result: activation.to_string(),
        conversation_lane: None,
    }
}

fn is_hex64(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn tag_values<'a>(event: &'a Value, tag_name: &str) -> Vec<&'a str> {
    event["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .filter(|tag| tag.first().and_then(Value::as_str) == Some(tag_name))
        .filter_map(|tag| tag.get(1).and_then(Value::as_str))
        .collect()
}

fn channel_fact(event: &Value) -> Result<String, &'static str> {
    let channels: HashSet<&str> = tag_values(event, "h").into_iter().collect();
    match channels.len() {
        0 => Err("invalid_missing_channel"),
        1 => Ok(channels.into_iter().next().unwrap().to_string()),
        _ => Err("invalid_conflicting_channel"),
    }
}

fn thread_facts(event: &Value) -> Result<(Option<String>, Option<String>), ()> {
    let mut root = None;
    let mut reply = None;
    for tag in event["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
    {
        if tag.first().and_then(Value::as_str) != Some("e") {
            continue;
        }
        let Some(marker) = tag.get(3).and_then(Value::as_str) else {
            continue;
        };
        if marker != "root" && marker != "reply" {
            continue;
        }
        let Some(event_id) = tag.get(1).and_then(Value::as_str) else {
            return Err(());
        };
        if !is_hex64(event_id) {
            return Err(());
        }
        let destination = if marker == "root" {
            &mut root
        } else {
            &mut reply
        };
        if destination
            .as_ref()
            .is_some_and(|existing| existing != event_id)
        {
            return Err(());
        }
        *destination = Some(event_id.to_string());
    }
    Ok((reply.clone(), root.or(reply)))
}

fn normalize_event(fixture: &str, envelope: &Value, contract: &Contract) -> Fact {
    let event = &envelope["event"];
    let event_id = event["id"].as_str().map(str::to_string);
    let author = event["pubkey"].as_str().map(str::to_string);
    let is_self_authored = author.as_deref() == Some(contract.agent_pubkey.as_str());
    let explicitly_mentions_agent = tag_values(event, "p")
        .into_iter()
        .any(|pubkey| pubkey == contract.agent_pubkey);

    if envelope["schema_version"].as_u64() != Some(1) {
        return Fact {
            fixture: fixture.to_string(),
            event_id,
            author_pubkey: author,
            channel_id: channel_fact(event).ok(),
            immediate_parent_id: None,
            thread_root_id: None,
            explicitly_mentions_agent,
            is_self_authored,
            author_policy_result: "unknown".to_string(),
            activation_result: "unsupported_schema_version".to_string(),
            conversation_lane: None,
        };
    }

    let author_policy = if is_self_authored {
        "self"
    } else if author.as_deref() == Some(contract.owner_pubkey.as_str()) {
        "owner"
    } else if author
        .as_ref()
        .is_some_and(|pubkey| contract.allowlisted_pubkeys.contains(pubkey))
    {
        "allowlisted"
    } else {
        "denied"
    };

    let channel = channel_fact(event);
    let thread = thread_facts(event);
    let activation = if !matches!(event["kind"].as_u64(), Some(9 | 40002)) {
        "ignored_unsupported_kind"
    } else if let Err(reason) = channel {
        reason
    } else if thread.is_err() {
        "invalid_malformed_thread"
    } else if is_self_authored {
        "ignored_self_authored"
    } else if author_policy == "denied" {
        "ignored_non_owner"
    } else if !explicitly_mentions_agent {
        "ignored_missing_mention"
    } else {
        "accepted"
    };
    let channel_id = channel.ok();
    let (immediate_parent_id, thread_root_id) = thread.unwrap_or_default();
    let conversation_lane = if activation == "accepted" {
        channel_id
            .as_ref()
            .zip(thread_root_id.as_ref().or(event_id.as_ref()))
            .map(|(channel, root)| format!("{channel}:{root}"))
    } else {
        None
    };

    Fact {
        fixture: fixture.to_string(),
        event_id,
        author_pubkey: author,
        channel_id,
        immediate_parent_id,
        thread_root_id,
        explicitly_mentions_agent,
        is_self_authored,
        author_policy_result: author_policy.to_string(),
        activation_result: activation.to_string(),
        conversation_lane,
    }
}

fn normalize_fixture(fixture: &str, raw: &str, contract: &Contract) -> Vec<Fact> {
    if fixture == "malformed_json_line.ndjson" {
        assert!(
            raw.lines()
                .all(|line| serde_json::from_str::<Value>(line).is_err()),
            "malformed JSON fixture must fail parsing"
        );
        return vec![null_fact(fixture, "not_applicable", "invalid_json")];
    }

    let envelopes: Vec<Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).expect("fixture line should parse"))
        .collect();
    if fixture == "lifecycle_sequence.ndjson" {
        let states: Vec<&str> = envelopes
            .iter()
            .map(|envelope| {
                assert_eq!(envelope["schema_version"], 1);
                assert_eq!(envelope["type"], "lifecycle");
                envelope["state"].as_str().expect("lifecycle state")
            })
            .collect();
        assert_eq!(states, ["connected", "eose", "closed"]);
        return vec![null_fact(fixture, "not_applicable", "lifecycle_only")];
    }

    let mut facts: Vec<Fact> = envelopes
        .iter()
        .map(|envelope| {
            assert_eq!(envelope["type"], "event");
            normalize_event(fixture, envelope, contract)
        })
        .collect();
    if fixture == "duplicate_event_id.ndjson" {
        assert_eq!(
            envelopes.len(),
            2,
            "duplicate fixture must contain two deliveries"
        );
        assert_eq!(
            envelopes[0], envelopes[1],
            "a verified Nostr replay duplicate must be byte-identical"
        );
        assert_eq!(facts[0], facts[1]);
        facts.truncate(1);
        facts[0].activation_result = "accepted_then_duplicate_suppressed".to_string();
    }
    facts
}

#[test]
fn external_agent_v1_fixtures_match_expected_normalized_facts() {
    let dir = fixtures_dir();
    let expected_raw = fs::read_to_string(dir.join("expected_facts.json")).unwrap();
    let contract: Contract = serde_json::from_str(&expected_raw).unwrap();
    assert_eq!(contract.contract, "external_agent_v1");

    let mut fixture_names = HashSet::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("ndjson") {
            fixture_names.insert(path.file_name().unwrap().to_string_lossy().to_string());
        }
    }
    let expected_names: HashSet<String> = contract
        .records
        .iter()
        .map(|record| record.fixture.clone())
        .collect();
    assert_eq!(
        fixture_names, expected_names,
        "fixtures and expected_facts must reference the same file set"
    );

    let mut actual_by_fixture: HashMap<String, Vec<Fact>> = HashMap::new();
    for fixture in &fixture_names {
        let raw = fs::read_to_string(dir.join(fixture)).unwrap();
        actual_by_fixture.insert(fixture.clone(), normalize_fixture(fixture, &raw, &contract));
    }
    let mut expected_by_fixture: HashMap<String, Vec<Fact>> = HashMap::new();
    for record in &contract.records {
        expected_by_fixture
            .entry(record.fixture.clone())
            .or_default()
            .push(record.clone());
    }
    assert_eq!(actual_by_fixture, expected_by_fixture);
}
