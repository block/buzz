//! The desktop's sole BW projection bridge. No transport, signing or local latch.
use buzz_core_pkg::bw::{Consumer, Decision, Evidence, Trust};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Explicit, transient snapshot inputs; events retain their original JSON bytes.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub trust: Trust,
    pub external: Evidence,
    pub now: u64,
    pub events: Vec<String>,
}

/// Execute the exact Core consumer used by the desktop, including step outcomes.
pub fn replay(input: &Input) -> (Consumer, Vec<Decision>) {
    let mut consumer = Consumer::new(input.trust.clone(), input.external.clone(), input.now);
    let steps = input
        .events
        .iter()
        .map(|raw| consumer.ingest(raw.as_bytes()))
        .collect();
    (consumer, steps)
}

/// The issue this pending/conflicted event belongs to, for a 46100 BW
/// record (its `issue` tag) or a kind:1 assignment/unassignment candidate
/// over the existing wire (its `e` tag) — the only two event shapes a
/// per-issue notice makes sense for. A conflicted event of either kind
/// never reaches `accept`, so it never appears in `records`; without this,
/// its refusal/fork would be invisible to the desktop UI entirely (P4E: a
/// forked assignment chain must surface the same way a forked 46100 chain
/// does, not silently disappear).
fn notice_issue(wire: &Value) -> Option<&str> {
    let tags = wire["tags"].as_array()?;
    match wire["kind"].as_u64()? {
        46100 => tags.iter().find(|t| t[0] == "issue")?[1].as_str(),
        1 if tags.iter().any(|t| {
            t[0] == "t" && matches!(t[1].as_str(), Some("assignment" | "unassignment"))
        }) =>
        {
            tags.iter().find(|t| t[0] == "e")?[1].as_str()
        }
        _ => None,
    }
}

/// Project all history anew. Inspection rechecks early pending events after their
/// dependencies arrive. Only Core-approved records provide authoritative fields.
pub fn snapshot(input: &Input) -> Value {
    let (consumer, _) = replay(input);
    let projection = consumer.projection();
    let mut decisions = BTreeMap::new();
    let mut records = BTreeMap::new();
    let mut notices: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (raw, result) in input.events.iter().zip(consumer.inspect_all()) {
        if matches!(result.outcome.as_str(), "pending" | "conflict") {
            if let Ok(wire) = buzz_core_pkg::bw::parse_json(raw.as_bytes()) {
                if let Some(issue) = notice_issue(&wire) {
                    notices.entry(issue.to_owned()).or_default().push(json!({"event_id":result.event_id,"outcome":result.outcome,"stage":result.stage,"code":result.code}));
                }
            }
        }
        if let Some(id) = &result.event_id {
            // Invalid copies never overwrite a verified event sharing its ID.
            if result.outcome == "accept" || !decisions.contains_key(id) {
                if result.outcome == "accept" {
                    if let Ok(wire) = buzz_core_pkg::bw::parse_json(raw.as_bytes()) {
                        records.insert(id.clone(), wire);
                    }
                }
                decisions.insert(
                    id.clone(),
                    json!({"outcome":result.outcome,"stage":result.stage,"code":result.code}),
                );
            }
        }
    }
    for rows in notices.values_mut() {
        rows.sort_by_key(Value::to_string);
        rows.dedup();
    }
    json!({"notices":notices,"projection":projection,"decisions":decisions,"records":records,
           "activation":consumer.activation().ok(),"repo":input.trust.repo})
}
