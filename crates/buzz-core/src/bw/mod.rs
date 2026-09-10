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
/// A refused producer request, in the closed vocabulary [`Decision`] already uses.
///
/// It deliberately carries no projection, content, tags or key material: only the
/// stable reason and, where one exists, the public event ID the request was bound
/// to. That keeps refusals safe to return, log and branch on.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Refusal {
    /// The pinned event ID, when the request got far enough to have one.
    pub event_id: Option<String>,
    /// reject, pending, conflict or replay.
    pub outcome: &'static str,
    /// First failing validation stage.
    pub stage: &'static str,
    /// Stable protocol reason.
    pub code: &'static str,
}
impl Refusal {
    fn new(event_id: Option<String>, f: Fault) -> Self {
        Self {
            event_id,
            outcome: f.outcome,
            stage: f.stage,
            code: f.code,
        }
    }
}
impl std::fmt::Display for Refusal {
    /// The canonical machine-readable form: `bw:<outcome>:<stage>:<code>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bw:{}:{}:{}", self.outcome, self.stage, self.code)
    }
}
/// Resolved producer activation. Every field is derived from externally confirmed
/// trust plus the signed history — never from a local flag, an asserted event ID
/// or an environment variable. Holding one proves that the repository genesis is
/// signed by the externally confirmed owner and that exactly one role policy is
/// currently in force.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Activation {
    /// Externally confirmed NIP-34 repository coordinate.
    pub repo: String,
    /// Externally confirmed repository owner.
    pub owner: String,
    /// Event ID of the owner-signed repository genesis record.
    pub genesis: String,
    /// Event ID of the single current role-policy head.
    pub policy: String,
    /// Version of the current role policy.
    pub policy_version: u64,
    /// Time from which the current role policy is in force.
    pub effective_at: u64,
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
    /// Resolve producer activation from the supplied trust and signed history.
    ///
    /// Fails when the owner-signed repository genesis is absent or does not match
    /// the externally confirmed coordinate, when no role policy is in force yet,
    /// or when the policy history has more than one current head. The failure is
    /// a [`Refusal`] in the same machine-readable outcome/stage/code vocabulary
    /// validation uses, so a caller never receives a bare boolean.
    pub fn activation(&self) -> Result<Activation, Refusal> {
        let h = history::History::new(
            &self.records,
            &self.invalid,
            &self.trust,
            &self.evidence,
            self.now,
        );
        let refuse = |f: Fault| Refusal::new(None, f);
        // Absent evidence stays absent; a genesis under a foreign key is a refusal.
        let repositories: Vec<_> = self
            .records
            .values()
            .filter(|r| r.typ() == "repository")
            .collect();
        let genesis = repositories
            .iter()
            .find(|r| r.signer() == self.trust.owner && h.validate(r.id()).is_ok())
            .ok_or_else(|| {
                refuse(if repositories.is_empty() {
                    pending("references", "missing-reference")
                } else {
                    fail("references", "repository")
                })
            })?;
        // Only policies already in force can activate a producer; a future policy
        // is not yet current and a superseded one is not the head.
        let policies: Vec<_> = self
            .records
            .values()
            .filter(|r| {
                r.typ() == "role-policy"
                    && n(&r.body["effective_at"]) <= self.now
                    && h.historical(r.id()).is_ok()
            })
            .collect();
        let heads: Vec<_> = policies
            .iter()
            .filter(|p| !policies.iter().any(|q| h.descendant(q, p.id())))
            .copied()
            .collect();
        // Competing heads are a disputed policy history, not a tie to break.
        let [policy] = heads[..] else {
            return Err(refuse(if heads.is_empty() {
                pending("policy", "missing-policy")
            } else {
                Fault {
                    outcome: "conflict",
                    stage: "policy",
                    code: "head-conflict",
                }
            }));
        };
        h.validate(policy.id()).map_err(refuse)?;
        Ok(Activation {
            repo: self.trust.repo.clone(),
            owner: self.trust.owner.clone(),
            genesis: genesis.id().to_owned(),
            policy: policy.id().to_owned(),
            policy_version: n(&policy.body["version"]),
            effective_at: n(&policy.body["effective_at"]),
        })
    }
    /// Validate an unsigned candidate operation against the current activation,
    /// roles, references, causality and external facts, and pin the event ID the
    /// signature will carry.
    ///
    /// Nothing is signed, sent or recorded: the candidate is evaluated in a copy
    /// of the history and the consumer is left untouched. On success the caller
    /// receives the activation it was validated under and the pinned ID; a retry
    /// therefore re-reads that one ID instead of generating a second transition.
    /// An ID already present in the history is reported as a replay, not accepted.
    pub fn preflight(&self, candidate: &Value) -> Result<(Activation, String), Refusal> {
        let activation = self.activation()?;
        let record = shape::draft(candidate).map_err(|f| Refusal::new(None, f))?;
        let id = record.id().to_owned();
        if self.records.contains_key(&id) {
            return Err(Refusal::new(
                Some(id),
                Fault {
                    outcome: "replay",
                    stage: "projection",
                    code: "seen-id",
                },
            ));
        }
        let mut records = self.records.clone();
        records.insert(id.clone(), record);
        let h = history::History::new(
            &records,
            &self.invalid,
            &self.trust,
            &self.evidence,
            self.now,
        );
        match h.validate(&id) {
            Ok(()) => Ok((activation, id)),
            Err(f) => Err(Refusal::new(Some(id), f)),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../../../../docs/nips/NIP-BW.fixtures.json"))
            .expect("fixtures")
    }
    /// Rebuild one pinned corpus case: its trust, its external observations, its
    /// explicit now and the named prefix of its signed history.
    fn case(f: &Value, name: &str, labels: &[&str]) -> Consumer {
        let c = a(&f["cases"])
            .iter()
            .find(|c| s(&c["name"]) == name)
            .expect("case");
        let mut consumer = Consumer::new(
            serde_json::from_value(c["trust"].clone()).expect("trust"),
            serde_json::from_value(c["external"].clone()).expect("evidence"),
            n(&c["now"]),
        );
        for label in labels {
            consumer.ingest(&serde_json::to_vec(&f["events"][label]["event"]).expect("event"));
        }
        consumer
    }
    /// The five signed fields of a pinned event, before its signature exists.
    fn unsigned(f: &Value, label: &str) -> Value {
        let e = &f["events"][label]["event"];
        json!({"pubkey":e["pubkey"],"created_at":e["created_at"],"kind":e["kind"],"tags":e["tags"],"content":e["content"]})
    }

    #[test]
    fn activation_binds_owner_signed_genesis_and_current_policy() {
        let f = fixtures();
        let a = case(&f, "role-policy-positive", &["repo", "policy"])
            .activation()
            .expect("activation");
        assert_eq!(a.genesis, s(&f["events"]["repo"]["event"]["id"]));
        assert_eq!(a.policy, s(&f["events"]["policy"]["event"]["id"]));
        assert_eq!((a.policy_version, a.effective_at), (1, 1800000000));
        // The later policy supersedes the genesis policy without any local flag.
        let b = case(
            &f,
            "policy-new-positive",
            &["repo", "policy", "policy-next"],
        )
        .activation()
        .expect("activation");
        assert_eq!(b.policy, s(&f["events"]["policy-next"]["event"]["id"]));
        assert_eq!(b.policy_version, 2);
    }
    #[test]
    fn activation_refuses_without_genesis_or_policy() {
        let f = fixtures();
        // A policy alone proves nothing: the repository genesis is simply absent.
        let d = case(&f, "role-policy-positive", &["policy"])
            .activation()
            .expect_err("no genesis");
        assert_eq!(
            (d.outcome, d.stage, d.code),
            ("pending", "references", "missing-reference")
        );
        // Genesis alone leaves no policy in force.
        let d = case(&f, "role-policy-positive", &["repo"])
            .activation()
            .expect_err("no policy");
        assert_eq!(
            (d.outcome, d.stage, d.code),
            ("pending", "policy", "missing-policy")
        );
    }
    #[test]
    fn activation_refuses_foreign_owner_and_disputed_policy() {
        let f = fixtures();
        // An owner asserted locally cannot take over a repository it never signed.
        let mut consumer = case(&f, "role-policy-positive", &["repo", "policy"]);
        let foreign = s(&f["events"]["root_a"]["event"]["pubkey"]).to_owned();
        consumer.trust = Trust {
            community: consumer.trust.community.clone(),
            repo: format!("30617:{foreign}:fictional-bw"),
            owner: foreign,
        };
        let d = consumer.activation().expect_err("foreign owner");
        assert_eq!((d.stage, d.code), ("references", "repository"));
        // Two competing genesis policies are a disputed history, not a tie-break.
        let d = case(
            &f,
            "policy-genesis-fork",
            &["repo", "policy", "policy-fork"],
        )
        .activation()
        .expect_err("policy fork");
        assert_eq!(
            (d.outcome, d.stage, d.code),
            ("conflict", "policy", "head-conflict")
        );
    }
    #[test]
    fn preflight_pins_the_signed_id_of_an_authorized_operation() {
        let f = fixtures();
        let mut consumer = case(
            &f,
            "issue-update-positive",
            &["repo", "policy", "root_a", "enroll_a"],
        );
        let before = consumer.projection();
        let (activation, id) = consumer
            .preflight(&unsigned(&f, "update_a"))
            .expect("preflight");
        // The pinned ID is the ID the real signature carries — known before signing.
        assert_eq!(id, s(&f["events"]["update_a"]["event"]["id"]));
        assert_eq!(activation.policy, s(&f["events"]["policy"]["event"]["id"]));
        // Preflight records nothing: neither the history nor the projection moves.
        assert_eq!(consumer.projection(), before);
        assert_eq!(consumer.archived_inputs().len(), 4);
        // The candidate's own effect is visible only once it is really observed.
        let root = s(&f["events"]["root_a"]["event"]["id"]).to_owned();
        assert_eq!(before["issue_fields"][&root]["priority"], json!("P2"));
        consumer.ingest(&serde_json::to_vec(&f["events"]["update_a"]["event"]).expect("event"));
        assert_eq!(
            consumer.projection()["issue_fields"][&root]["priority"],
            json!("P1")
        );
    }
    #[test]
    fn preflight_refuses_unauthorized_stale_and_replayed_operations() {
        let f = fixtures();
        // Wrong role: an unauthorized signer cannot patch classified fields.
        let d = case(
            &f,
            "issue-update-wrong-role",
            &["repo", "policy", "root_a", "enroll_a"],
        )
        .preflight(&unsigned(&f, "issue-update-wrong-role"))
        .expect_err("wrong role");
        assert_eq!(d.stage, "role");
        // Stale policy: the operation binds a policy that is no longer current.
        let d = case(
            &f,
            "policy-boundary",
            &["repo", "policy", "policy-next", "root_a"],
        )
        .preflight(&unsigned(&f, "old-policy-binding"))
        .expect_err("stale policy");
        assert_eq!((d.stage, d.code), ("policy", "policy-binding"));
        // A known ID is a replay, never a fresh accepted transition.
        let d = case(
            &f,
            "issue-update-positive",
            &["repo", "policy", "root_a", "enroll_a", "update_a"],
        )
        .preflight(&unsigned(&f, "update_a"))
        .expect_err("replay");
        assert_eq!(
            (d.outcome, d.code, d.event_id.as_deref()),
            (
                "replay",
                "seen-id",
                Some(s(&f["events"]["update_a"]["event"]["id"]))
            )
        );
    }
    #[test]
    fn preflight_refuses_a_candidate_that_is_not_an_unsigned_envelope() {
        let f = fixtures();
        let consumer = case(
            &f,
            "issue-update-positive",
            &["repo", "policy", "root_a", "enroll_a"],
        );
        // The signed envelope is not a candidate: a producer pins before signing.
        let d = consumer
            .preflight(&f["events"]["update_a"]["event"])
            .expect_err("signed envelope");
        assert_eq!((d.stage, d.code), ("envelope", "fields"));
        let mut short = unsigned(&f, "update_a");
        short["content"] = json!(42);
        assert!(consumer.preflight(&short).is_err());
    }
}
