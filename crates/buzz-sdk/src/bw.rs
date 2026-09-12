//! NIP-BW offline draft construction and the readback-bound publish boundary.
//! Publication requires resolved activation and an exact confirmed readback;
//! there is no unauthorized path and no transport that can bypass either.
use crate::SdkError;
use buzz_core::bw::{Activation, Consumer, Refusal};
use nostr::JsonUtil;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The closed NIP-BW record namespace. Artifacts retain their separate 1063 kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecordType {
    /// Owner-signed historical roles and cutover.
    RolePolicy,
    /// Issue text and classification patch.
    IssueUpdate,
    /// Human triage action.
    TriageAction,
    /// Causal source-work state.
    IssueState,
    /// Causal relation operation.
    IssueRelation,
    /// Immutable pipeline binding.
    ReleasePipeline,
    /// Immutable freeze or terminal close.
    ReleaseSet,
    /// Explicit build authorization.
    BuildRequest,
    /// Externally confirmed provider snapshot.
    BuildRun,
    /// Artifact-bound human handoff.
    TestReady,
    /// Individual human verdict.
    MemberVerdict,
}
/// Build an unsigned record draft from a typed discriminator and closed content.
/// Required authority/reference tags must be supplied; no role is inferred.
pub fn record(
    record: RecordType,
    mut tags: Vec<Vec<String>>,
    content: Value,
) -> Result<Draft, SdkError> {
    let label = serde_json::to_value(record).map_err(|e| SdkError::InvalidInput(e.to_string()))?;
    let name = label
        .as_str()
        .ok_or_else(|| SdkError::InvalidInput("record discriminator".into()))?;
    tags.insert(0, vec!["record".into(), name.into()]);
    Draft::new(
        buzz_core::kind::KIND_BUZZ_WORKFLOW_RECORD as u64,
        json!(tags),
        content.to_string(),
    )
}
/// Verify exact confirmed readback without performing I/O. Equality includes all
/// seven signed fields, original content bytes, signer and ordered tags.
pub fn verify_readback(
    expected: &nostr::Event,
    readback: Option<&nostr::Event>,
) -> Result<(), SdkError> {
    let observed = readback.ok_or_else(|| SdkError::InvalidInput("bw:readback:missing".into()))?;
    if !expected.verify_id()
        || !expected.verify_signature()
        || expected != observed
        || !observed.verify_id()
        || !observed.verify_signature()
    {
        return Err(SdkError::InvalidInput("bw:readback:mismatch".into()));
    }
    Ok(())
}

/// A shape-checked, unsigned BW draft. This is not a publishable event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    kind: u64,
    tags: Value,
    content: String,
}
impl Draft {
    /// Construct any of the eleven closed record profiles, NIP-94 artifacts,
    /// issue roots or the existing assignment profile. No normalization is applied.
    pub fn new(kind: u64, tags: Value, content: String) -> Result<Self, SdkError> {
        if !matches!(kind, 46100 | 1063 | 1621 | 1) {
            return Err(SdkError::InvalidInput("unsupported BW draft kind".into()));
        }
        buzz_core::bw::validate_shape(kind, &tags, &content).map_err(SdkError::InvalidInput)?;
        Ok(Self {
            kind,
            tags,
            content,
        })
    }
    /// Revalidate a deserialized draft and produce JSON for inspection, without signing.
    pub fn dry_run(&self) -> Result<Value, SdkError> {
        if !matches!(self.kind, 46100 | 1063 | 1621 | 1) {
            return Err(SdkError::InvalidInput("unsupported BW draft kind".into()));
        }
        buzz_core::bw::validate_shape(self.kind, &self.tags, &self.content)
            .map_err(SdkError::InvalidInput)?;
        Ok(
            json!({"kind":self.kind,"tags":self.tags,"content":self.content,"publish_enabled":false,"activation_gate":"activation-readback"}),
        )
    }
}
/// Publication without prepared activation evidence is unavailable and stays so.
/// There is no flag, environment variable or local assertion that opens it: the
/// only producer path is [`Publication::prepare`], which requires an owner-signed
/// genesis, a current policy and a validated operation.
pub fn publish() -> Result<(), SdkError> {
    Err(SdkError::InvalidInput("bw:activation:required".into()))
}

/// Machine-readable rendering of a refusal. Carries only the closed
/// outcome/stage/code vocabulary — never event content, keys or signatures.
fn refused(refusal: &Refusal) -> SdkError {
    SdkError::InvalidInput(refusal.to_string())
}

/// Relay transport for the narrow BW producer. Implementations perform I/O only;
/// every trust decision is taken before and after the call, never inside it.
///
/// `submit` returning `Err` means the delivery status is *unknown*, not failed —
/// the pinned readback is what decides, so an implementation must never retry by
/// building a different event.
pub trait Transport {
    /// Submit an already signed event to the relay.
    fn submit(&mut self, event: &nostr::Event) -> Result<(), String>;
    /// Read the event the relay stores under `id`, if any.
    fn readback(&mut self, id: &str) -> Result<Option<nostr::Event>, String>;
}

/// An activation-bound, ID-pinned BW operation.
///
/// Constructing one proves — against externally confirmed trust and the signed
/// history — that the repository genesis is owner-signed, that exactly one role
/// policy is in force, and that this operation passes the same role, reference,
/// causality and external-fact validation an offline consumer applies. The event
/// ID is fixed here, before signing, so no retry can produce a second transition.
#[derive(Debug, Clone)]
pub struct Publication {
    activation: Activation,
    event_id: String,
    candidate: Value,
}
impl Publication {
    /// Validate an unsigned candidate against the consumer's activation and
    /// history. The consumer is not modified and nothing is signed or sent.
    ///
    /// `candidate` is the unsigned wire form: exactly `pubkey`, `created_at`,
    /// `kind`, `tags` and `content`.
    pub fn prepare(consumer: &Consumer, candidate: Value) -> Result<Self, SdkError> {
        let (activation, event_id) = consumer.preflight(&candidate).map_err(|d| refused(&d))?;
        Ok(Self {
            activation,
            event_id,
            candidate,
        })
    }
    /// The activation this operation was validated under.
    pub fn activation(&self) -> &Activation {
        &self.activation
    }
    /// The pinned event ID. It is known before signing and never changes.
    pub fn event_id(&self) -> &str {
        &self.event_id
    }
    /// The exact signed fields to hand to an existing signer. Callers sign this
    /// with their own signing abstraction; no key material enters this module.
    pub fn candidate(&self) -> &Value {
        &self.candidate
    }
    /// Bind a signed event to this publication before any transport.
    ///
    /// Fails unless the event verifies and every signed field is byte-identical
    /// to the pinned candidate — so a signer that rewrote, re-tagged or re-timed
    /// the operation is refused instead of being published.
    pub fn seal(&self, signed: &nostr::Event) -> Result<(), SdkError> {
        if !signed.verify_id() || !signed.verify_signature() {
            return Err(SdkError::InvalidInput("bw:seal:unverified".into()));
        }
        let wire: Value = serde_json::from_str(&signed.as_json())
            .map_err(|_| SdkError::InvalidInput("bw:seal:mismatch".into()))?;
        if signed.id.to_hex() != self.event_id
            || ["pubkey", "created_at", "kind", "tags", "content"]
                .iter()
                .any(|k| wire[*k] != self.candidate[*k])
        {
            return Err(SdkError::InvalidInput("bw:seal:mismatch".into()));
        }
        Ok(())
    }
    /// Accept a relay readback as proof of publication, and only then record it.
    ///
    /// The readback must be byte-exact against the sealed event *and* still
    /// validate semantically in the live history under the same pinned ID. Any
    /// other result leaves the consumer unchanged, so a failed publication never
    /// enters the local history.
    pub fn confirm(
        &self,
        consumer: &mut Consumer,
        signed: &nostr::Event,
        readback: Option<&nostr::Event>,
    ) -> Result<(), SdkError> {
        self.seal(signed)?;
        verify_readback(signed, readback)?;
        let decision = consumer.ingest(signed.as_json().as_bytes());
        if decision.event_id.as_deref() != Some(self.event_id.as_str())
            || decision.outcome != "accept"
        {
            return Err(SdkError::InvalidInput(format!(
                "bw:{}:{}:{}",
                decision.outcome, decision.stage, decision.code
            )));
        }
        Ok(())
    }
}

/// Submit a sealed publication and succeed only on an exact confirmed readback.
///
/// Every attempt re-sends and re-reads the same pinned event: an unclear delivery
/// status is resolved by reading that one ID back, never by generating a new
/// transition. Failure is machine-readable and leaves the consumer unchanged.
pub fn publish_via<T: Transport>(
    transport: &mut T,
    consumer: &mut Consumer,
    publication: &Publication,
    signed: &nostr::Event,
    attempts: u32,
) -> Result<(), SdkError> {
    publication.seal(signed)?;
    let mut reason = "bw:readback:missing";
    for _ in 0..attempts.max(1) {
        if transport.submit(signed).is_err() {
            reason = "bw:transport:delivery-unknown";
        }
        match transport.readback(publication.event_id()) {
            Ok(Some(observed)) => {
                return publication.confirm(consumer, signed, Some(&observed));
            }
            Ok(None) => {}
            Err(_) => reason = "bw:transport:readback-unavailable",
        }
    }
    Err(SdkError::InvalidInput(reason.into()))
}

// The producer boundary is exercised end to end against the pinned corpus: no
// transport, retry or local flag can reach acceptance without an exact readback.
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Event, EventBuilder, Keys, Kind};

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../../../docs/nips/NIP-BW.fixtures.json"))
            .expect("corpus")
    }
    /// Rebuild a pinned corpus case as an offline consumer.
    fn case(f: &Value, name: &str, labels: &[&str]) -> Consumer {
        let c = f["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|c| c["name"] == name)
            .expect("case");
        let mut consumer = Consumer::new(
            serde_json::from_value(c["trust"].clone()).expect("trust"),
            serde_json::from_value(c["external"].clone()).expect("evidence"),
            c["now"].as_u64().expect("now"),
        );
        for label in labels {
            consumer.ingest(&serde_json::to_vec(&f["events"][label]["event"]).expect("event"));
        }
        consumer
    }
    fn unsigned(f: &Value, label: &str) -> Value {
        let e = &f["events"][label]["event"];
        json!({"pubkey":e["pubkey"],"created_at":e["created_at"],"kind":e["kind"],"tags":e["tags"],"content":e["content"]})
    }
    fn signed(f: &Value, label: &str) -> Event {
        Event::from_json(f["events"][label]["event"].to_string()).expect("signed")
    }
    /// A history and a validated operation over it, with the real signed event.
    fn prepared(f: &Value) -> (Consumer, Publication, Event) {
        let consumer = case(
            f,
            "issue-update-positive",
            &["repo", "policy", "root_a", "enroll_a"],
        );
        let publication =
            Publication::prepare(&consumer, unsigned(f, "update_a")).expect("prepare");
        (consumer, publication, signed(f, "update_a"))
    }
    /// Records every submission so a retry cannot hide a second transition.
    struct Fake {
        submitted: Vec<String>,
        reads: usize,
        delivery: Vec<bool>,
        stored: Option<Event>,
    }
    impl Fake {
        fn new(stored: Option<Event>, delivery: &[bool]) -> Self {
            Self {
                submitted: Vec::new(),
                reads: 0,
                delivery: delivery.to_vec(),
                stored,
            }
        }
    }
    impl Transport for Fake {
        fn submit(&mut self, event: &Event) -> Result<(), String> {
            self.submitted.push(event.id.to_hex());
            let attempt = self.submitted.len() - 1;
            match self.delivery.get(attempt).copied().unwrap_or(true) {
                true => Ok(()),
                false => Err("timeout".into()),
            }
        }
        fn readback(&mut self, id: &str) -> Result<Option<Event>, String> {
            self.reads += 1;
            Ok(self
                .stored
                .clone()
                .filter(|e| e.id.to_hex() == id && self.reads >= self.delivery.len().max(1)))
        }
    }

    /// Regression for the desktop write commands (`project_bw_write.rs`,
    /// `project_bw_assignment.rs`): loading a repository's history over a
    /// real relay takes real network time, so a `Consumer`'s `now` captured
    /// before that load can trail the `created_at` of a candidate signed
    /// after it — which reads as "from the future" to Core's zero-tolerance
    /// `references:future` check. `Consumer::observe` refreshing `now` right
    /// before `Publication::prepare` (not at load time) is what fixes that;
    /// this proves the same consumer flips from refused to accepted purely
    /// by that refresh, with no change to the candidate or the history.
    #[test]
    fn a_candidate_newer_than_a_stale_consumer_clock_is_pending_until_now_is_refreshed() {
        let f = fixtures();
        let mut consumer = case(
            &f,
            "issue-update-positive",
            &["repo", "policy", "root_a", "enroll_a"],
        );
        let candidate = unsigned(&f, "update_a");
        let created_at = candidate["created_at"].as_u64().expect("created_at");
        let evidence: buzz_core::bw::Evidence = serde_json::from_value(
            f["cases"]
                .as_array()
                .expect("cases")
                .iter()
                .find(|c| c["name"] == "issue-update-positive")
                .expect("case")["external"]
                .clone(),
        )
        .expect("evidence");

        // Simulate `now` captured before a slow history load: three seconds
        // behind the candidate's own `created_at`. Same evidence either way —
        // only `now` moves, exactly as the fix's `consumer.observe` call does.
        consumer.observe(evidence.clone(), created_at - 3);
        let refusal = Publication::prepare(&consumer, candidate.clone())
            .expect_err("a stale consumer clock refuses a newer candidate");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:pending:references:future"
        );

        // Refresh `now` to the signing instant, exactly as the fix does right
        // after `bw_projection::replay` and before `Publication::prepare`.
        consumer.observe(evidence, created_at);
        Publication::prepare(&consumer, candidate)
            .expect("refreshing now accepts the exact same candidate");
    }

    #[test]
    fn mock_publish_succeeds_only_through_an_exact_readback() {
        let f = fixtures();
        let (mut consumer, publication, event) = prepared(&f);
        let root = f["events"]["root_a"]["event"]["id"].as_str().expect("root");
        assert_eq!(
            consumer.projection()["issue_fields"][root]["priority"],
            "P2"
        );
        let mut transport = Fake::new(Some(event.clone()), &[true]);
        publish_via(&mut transport, &mut consumer, &publication, &event, 3).expect("published");
        // Confirmed publication, and only then, becomes part of the local history.
        assert_eq!(
            consumer.projection()["issue_fields"][root]["priority"],
            "P1"
        );
        assert_eq!(transport.submitted, vec![publication.event_id().to_owned()]);
    }
    #[test]
    fn missing_or_wrong_readback_is_never_success() {
        let f = fixtures();
        for (label, stored) in [
            ("missing", None),
            ("foreign", Some(signed(&f, "issue-update-wrong-role"))),
        ] {
            let (mut consumer, publication, event) = prepared(&f);
            let before = consumer.projection();
            let mut transport = Fake::new(stored, &[true]);
            publish_via(&mut transport, &mut consumer, &publication, &event, 2).expect_err(label);
            // A refused publication leaves no trace in the projection.
            assert_eq!(consumer.projection(), before, "{label}");
        }
        // A readback that differs in any signed field is a mismatch, not a match.
        let (_, publication, event) = prepared(&f);
        for key in ["id", "pubkey", "created_at", "tags", "content", "sig"] {
            let mut wire: Value = serde_json::from_str(&event.as_json()).expect("json");
            wire[key] = match key {
                "tags" => json!([["a", "wrong"]]),
                "content" => json!("wrong"),
                "created_at" => json!(1800000004),
                "sig" => json!("0".repeat(128)),
                _ => json!("0".repeat(64)),
            };
            if let Ok(other) = Event::from_json(wire.to_string()) {
                let mut consumer = prepared(&f).0;
                assert!(
                    publication
                        .confirm(&mut consumer, &event, Some(&other))
                        .is_err(),
                    "{key}"
                );
            }
        }
        assert!(publication
            .confirm(&mut prepared(&f).0, &event, None)
            .is_err());
    }
    #[test]
    fn unclear_delivery_reuses_the_pinned_id_and_never_forks() {
        let f = fixtures();
        let (mut consumer, publication, event) = prepared(&f);
        // Two lost sends, then a confirmed readback of the very same event.
        let mut transport = Fake::new(Some(event.clone()), &[false, false, true]);
        publish_via(&mut transport, &mut consumer, &publication, &event, 4).expect("published");
        assert!(transport.submitted.len() > 1);
        assert!(transport
            .submitted
            .iter()
            .all(|id| id == publication.event_id()));
        // A permanently unclear delivery stays unclear and stays unpublished.
        let (mut consumer, publication, event) = prepared(&f);
        let before = consumer.projection();
        let mut transport = Fake::new(None, &[false, false]);
        let e = publish_via(&mut transport, &mut consumer, &publication, &event, 2)
            .expect_err("unclear");
        assert_eq!(
            e.to_string(),
            "invalid input: bw:transport:delivery-unknown"
        );
        assert_eq!(consumer.projection(), before);
        assert!(transport
            .submitted
            .iter()
            .all(|id| id == publication.event_id()));
    }
    #[test]
    fn preparation_refuses_unauthorized_operations_and_sealing_refuses_substitutes() {
        let f = fixtures();
        // No activation evidence at all: the historical gate stays closed.
        assert_eq!(
            publish().expect_err("disabled").to_string(),
            "invalid input: bw:activation:required"
        );
        let bare = case(&f, "issue-update-positive", &[]);
        assert_eq!(
            Publication::prepare(&bare, unsigned(&f, "update_a"))
                .expect_err("no genesis")
                .to_string(),
            "invalid input: bw:pending:references:missing-reference"
        );
        // Wrong role under a real activation is still refused before signing.
        let consumer = case(
            &f,
            "issue-update-wrong-role",
            &["repo", "policy", "root_a", "enroll_a"],
        );
        assert_eq!(
            Publication::prepare(&consumer, unsigned(&f, "issue-update-wrong-role"))
                .expect_err("wrong role")
                .to_string(),
            "invalid input: bw:reject:role:unauthorized"
        );
        // A signer that returns a different event than the pinned one is refused.
        let (_, publication, event) = prepared(&f);
        assert_eq!(
            publication
                .seal(&signed(&f, "enroll_a"))
                .expect_err("substitute")
                .to_string(),
            "invalid input: bw:seal:mismatch"
        );
        let unrelated = EventBuilder::new(Kind::Custom(46100), "fixture only")
            .sign_with_keys(&Keys::generate())
            .expect("sign");
        assert!(publication.seal(&unrelated).is_err());
        publication.seal(&event).expect("pinned event seals");
    }
    #[test]
    fn draft_profiles_reject_bad_shapes() {
        let corpus: Value =
            serde_json::from_str(include_str!("../../../docs/nips/NIP-BW.fixtures.json"))
                .expect("corpus");
        for item in corpus["events"].as_object().expect("events").values() {
            let e = &item["event"];
            let kind = e["kind"].as_u64().expect("kind");
            if matches!(kind, 46100 | 1063 | 1621 | 1) && item["shape"] == true {
                let draft = Draft::new(
                    kind,
                    e["tags"].clone(),
                    e["content"].as_str().expect("content").into(),
                )
                .expect("draft");
                assert_eq!(draft.dry_run().expect("dry run")["publish_enabled"], false);
            }
        }
        assert!(Draft::new(46100, json!([["record", "unknown"]]), "{}".into()).is_err());
    }
}
