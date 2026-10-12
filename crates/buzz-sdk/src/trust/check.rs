//! Rechecking a saved [`WorkTrail`](super::WorkTrail) from its own evidence,
//! without the origin.
//!
//! Every kept envelope is verified again, and every event the trail cites
//! must have one that verifies. This authenticates the cited events only.
//! It does not check anything the reader wrote around them: summaries and
//! the interpretation, authority and relationship labels, fetch provenance
//! (origin, coverage records, observation times are unsigned), or coverage
//! and completeness. A new read reproduces those from the same events,
//! scope, and policy. [`NOT_CHECKED`] says so in every result.

use std::collections::{BTreeMap, BTreeSet};

use nostr::Event;
use serde::Serialize;
use serde_json::Value;

use crate::SdkError;

/// What a recheck does not establish, carried in every [`EvidenceCheck`] so
/// a passing result cannot be read as a verified trail.
pub const NOT_CHECKED: &[&str] = &[
    "summaries and the interpretation of events",
    "authority and relationship labels",
    "fetch provenance: origin, coverage records and observation times are unsigned",
    "coverage or completeness: only events the viewer was served are kept",
];

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
    /// What this check does not establish ([`NOT_CHECKED`]).
    pub not_checked: &'static [&'static str],
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
        not_checked: NOT_CHECKED,
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
