//! Rechecking a saved [`WorkTrail`](super::WorkTrail) from its own evidence,
//! without the origin.
//!
//! Every kept envelope is verified again, and every event the trail cites
//! must have one that verifies. This shows the cited events are intact; it
//! does not re-derive the interpretation, which a new read reproduces from
//! the same events, scope, and policy.

use std::collections::{BTreeMap, BTreeSet};

use nostr::Event;
use serde::Serialize;
use serde_json::Value;

use crate::SdkError;

/// What rechecking a saved trail found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceCheck {
    /// Envelopes kept.
    pub events: usize,
    /// Envelopes whose id and signature verify now.
    pub verified: usize,
    /// Envelopes that fail now, or whose result differs from the one
    /// recorded when fetched, with why.
    pub failed: Vec<String>,
    /// Events the trail cites with no envelope that verifies.
    pub unsupported: Vec<String>,
}

impl EvidenceCheck {
    /// Whether every envelope rechecks as recorded and every cited event is
    /// supported.
    pub fn holds(&self) -> bool {
        self.failed.is_empty() && self.unsupported.is_empty()
    }
}

/// Recheck a saved trail (the JSON of a [`WorkTrail`](super::WorkTrail)).
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] when the JSON has no evidence section.
pub fn check_evidence(saved: &Value) -> Result<EvidenceCheck, SdkError> {
    let kept = saved["evidence"]["events"]
        .as_array()
        .ok_or_else(|| SdkError::InvalidInput("no evidence.events array".into()))?;
    let mut failed = Vec::new();
    let mut good: BTreeMap<String, bool> = BTreeMap::new();
    for entry in kept {
        let id = entry["id"].as_str().unwrap_or("(no id)").to_owned();
        let recorded = entry["valid"].as_bool();
        let now = serde_json::from_value::<Event>(entry["event"].clone())
            .is_ok_and(|e| e.id.to_hex() == id && e.verify().is_ok());
        if recorded != Some(now) {
            failed.push(format!(
                "{id}: recorded valid={}, rechecks {now}",
                recorded.map_or("missing".to_owned(), |v| v.to_string())
            ));
        }
        let ok = good.entry(id).or_insert(false);
        *ok |= now;
    }
    let verified = good.values().filter(|ok| **ok).count();
    let unsupported = cited(saved)
        .into_iter()
        .filter(|id| !good.get(id).copied().unwrap_or(false))
        .collect();
    Ok(EvidenceCheck {
        events: kept.len(),
        verified,
        failed,
        unsupported,
    })
}

/// Every event id the trail's tasks cite.
fn cited(saved: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut add = |v: &Value| {
        if let Some(id) = v["id"].as_str() {
            out.insert(id.to_owned());
        }
    };
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    for task in list(&saved["tasks"]) {
        add(&task["request"]);
        for c in list(&task["contributions"]) {
            add(&c);
            list(&c["outcomes"]).iter().for_each(&mut add);
        }
        list(&task["unbound"]).iter().for_each(&mut add);
        list(&task["conversation"]).iter().for_each(&mut add);
    }
    out
}
