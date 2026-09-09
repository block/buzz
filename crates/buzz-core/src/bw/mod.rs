//! NIP-BW offline consumer. No network, clock, dispatch or production signing.
mod external;
mod history;
mod projection;
mod semantics;
mod shape;
mod strict;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use shape::validate_shape;
use std::collections::BTreeMap;
pub use strict::parse as parse_json;

/// Externally confirmed repository boundary, never derived from event claims.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Trust {
    /// Community HTTPS URL.
    pub community: String,
    /// NIP-34 repository coordinate.
    pub repo: String,
    /// Externally confirmed repository owner.
    pub owner: String,
}
/// Offline observations supplied by a trusted caller; absent observations stay pending.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// Canonical Relay Git observations.
    pub git_readbacks: Vec<Value>,
    /// Explicit Git ancestry observations.
    pub git_ancestry: Vec<Value>,
    /// Provider observations including request idempotency key.
    pub provider_readbacks: Vec<Value>,
    /// Downloaded raw bytes and publisher durability observations.
    pub downloads: Vec<Value>,
    /// Current Host authorization, separate from historical roles.
    pub host_authorization: Value,
}
/// Ordered validation decision with the actual wire identity and derived projection.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Decision {
    /// Actual signed ID, absent only for undecodable envelopes.
    pub event_id: Option<String>,
    /// accept, reject, pending, conflict, replay or ignore.
    pub outcome: String,
    /// First failing validation stage, or projection.
    pub stage: String,
    /// Stable protocol reason.
    pub code: String,
    /// Derived state; keys are real event IDs.
    pub projection: Value,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Fault {
    pub outcome: &'static str,
    pub stage: &'static str,
    pub code: &'static str,
}
pub(super) type Check<T = ()> = Result<T, Fault>;
pub(super) fn fail(stage: &'static str, code: &'static str) -> Fault {
    Fault {
        outcome: "reject",
        stage,
        code,
    }
}
pub(super) fn pending(stage: &'static str, code: &'static str) -> Fault {
    Fault {
        outcome: "pending",
        stage,
        code,
    }
}
pub(super) fn first_failure(mut errors: Vec<Fault>) -> Check {
    const ORDER: [&str; 11] = [
        "envelope",
        "id",
        "signature",
        "shape",
        "attestation",
        "references",
        "policy",
        "role",
        "causality",
        "external",
        "projection",
    ];
    errors.sort_by_key(|f| {
        (
            ORDER
                .iter()
                .position(|s| *s == f.stage)
                .unwrap_or(ORDER.len()),
            f.code,
        )
    });
    errors.into_iter().next().map_or(Ok(()), Err)
}
pub(super) fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
pub(super) fn n(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}
pub(super) fn a(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
#[derive(Debug, Clone)]
pub(super) struct Record {
    pub wire: Value,
    pub body: Value,
}
impl Record {
    pub fn id(&self) -> &str {
        s(&self.wire["id"])
    }
    pub fn signer(&self) -> &str {
        s(&self.wire["pubkey"])
    }
    pub fn time(&self) -> u64 {
        n(&self.wire["created_at"])
    }
    pub fn kind(&self) -> u64 {
        n(&self.wire["kind"])
    }
    pub fn tag(&self, key: &str) -> &str {
        a(&self.wire["tags"])
            .iter()
            .find(|t| s(&t[0]) == key)
            .map(|t| s(&t[1]))
            .unwrap_or("")
    }
    pub fn typ(&self) -> &str {
        match self.kind() {
            1 if matches!(self.tag("t"), "assignment" | "unassignment") => "assignment",
            1 => "unrelated",
            1063 => "artifact",
            1621 => "root",
            30617 => "repository",
            _ => self.tag("record"),
        }
    }
    pub fn issue(&self) -> &str {
        if self.kind() == 1 {
            self.tag("e")
        } else {
            self.tag("issue")
        }
    }
    pub fn field(&self, key: &str) -> &str {
        s(&self.body[key])
    }
    pub fn previous(&self) -> &str {
        self.tag(if self.kind() == 1 {
            "prior"
        } else {
            "previous"
        })
    }
    pub fn legacy(&self) -> bool {
        (1630..=1633).contains(&self.kind()) || self.tag("t") == "mybuzz-status-v1"
    }
}
/// An append-only in-memory offline history. Recomputes validity from all supplied
/// events on every observation; it contains no acceptance latch or side effects.
pub struct Consumer {
    trust: Trust,
    evidence: Evidence,
    now: u64,
    records: BTreeMap<String, Record>,
    invalid: BTreeMap<String, Fault>,
    archive: Vec<Vec<u8>>,
}
impl Consumer {
    /// Create a fresh history with explicit time and external evidence.
    pub fn new(trust: Trust, evidence: Evidence, now: u64) -> Self {
        Self {
            trust,
            evidence,
            now,
            records: BTreeMap::new(),
            invalid: BTreeMap::new(),
            archive: Vec::new(),
        }
    }
    /// Replace external observations and time; every subsequent evaluation uses them.
    pub fn observe(&mut self, evidence: Evidence, now: u64) {
        self.evidence = evidence;
        self.now = now;
    }
    /// Validate a raw signed event, retain verified bytes and recompute the history.
    pub fn ingest(&mut self, bytes: &[u8]) -> Decision {
        self.archive.push(bytes.to_vec());
        let wire = match parse_json(bytes) {
            Ok(v) => v,
            Err(_) => return self.decision(None, Err(fail("envelope", "json")), None),
        };
        let id = wire["id"].as_str().map(str::to_owned);
        let record = match shape::decode(wire) {
            Ok(r) => r,
            Err(e) => {
                if matches!(e.stage, "shape" | "attestation" | "signature") {
                    if let Some(id) = &id {
                        self.invalid.entry(id.clone()).or_insert(e.clone());
                    }
                }
                return self.decision(id, Err(e), None);
            }
        };
        // Verify before deduplication: a forged signature sharing a genuine ID is never a replay.
        let replay = self.records.contains_key(record.id());
        let target = record.id().to_owned();
        self.records.entry(target.clone()).or_insert(record);
        let h = history::History::new(
            &self.records,
            &self.invalid,
            &self.trust,
            &self.evidence,
            self.now,
        );
        let checked = h.validate(&target);
        let result = if replay && checked.is_ok() {
            Err(Fault {
                outcome: "replay",
                stage: "projection",
                code: "seen-id",
            })
        } else {
            checked
        };
        let projection = h.project(Some(&target));
        self.decision(id, result, Some(projection))
    }
    fn decision(&self, id: Option<String>, result: Check, projection: Option<Value>) -> Decision {
        let projection = projection.unwrap_or_else(|| {
            history::History::new(
                &self.records,
                &self.invalid,
                &self.trust,
                &self.evidence,
                self.now,
            )
            .project(None)
        });
        let (outcome, stage, code) = match result {
            Ok(()) => ("accept", "projection", "valid"),
            Err(f) => (f.outcome, f.stage, f.code),
        };
        Decision {
            event_id: id,
            outcome: outcome.into(),
            stage: stage.into(),
            code: code.into(),
            projection,
        }
    }
    /// Original input bytes, including rejected inputs. No event is rewritten or
    /// deleted when later evidence changes a projection.
    pub fn archived_inputs(&self) -> &[Vec<u8>] {
        &self.archive
    }
    /// Recompute the current projection without adding an event.
    pub fn projection(&self) -> Value {
        history::History::new(
            &self.records,
            &self.invalid,
            &self.trust,
            &self.evidence,
            self.now,
        )
        .project(None)
    }
}
