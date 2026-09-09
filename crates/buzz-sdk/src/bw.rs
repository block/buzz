//! NIP-BW offline draft construction and future exact-readback boundary.
//! Productive BW producers remain disabled until full P3C readback.
use crate::SdkError;
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
    let observed = readback.ok_or_else(|| SdkError::InvalidInput("missing-readback".into()))?;
    if !expected.verify_id()
        || !expected.verify_signature()
        || expected != observed
        || !observed.verify_id()
        || !observed.verify_signature()
    {
        return Err(SdkError::InvalidInput("readback-mismatch".into()));
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
            json!({"kind":self.kind,"tags":self.tags,"content":self.content,"publish_enabled":false,"activation_gate":"P3C-readback"}),
        )
    }
}
/// Production publication is deliberately unavailable. P2H source PASS cannot
/// unlock this gate; a later reviewed P3C change is required.
pub fn publish() -> Result<(), SdkError> {
    Err(SdkError::InvalidInput(
        "BW publication disabled: full P3C readback required".into(),
    ))
}

// Private transport boundary: no public implementation or transport injection can
// bypass the gate. Tests exercise the future exact signed-event readback contract.
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind};
    trait Transport {
        fn submit(&mut self, event: &Event) -> bool;
        fn readback(&mut self, id: &str) -> Option<Event>;
    }
    fn publish_and_readback(t: &mut impl Transport, event: &Event) -> Result<(), &'static str> {
        if !event.verify_id() || !event.verify_signature() {
            return Err("invalid-event");
        }
        if !t.submit(event) {
            return Err("transport-rejected");
        }
        let read = t.readback(&event.id.to_hex()).ok_or("missing-readback")?;
        verify_readback(event, Some(&read)).map_err(|_| "readback-mismatch")
    }
    struct Fake {
        accepted: bool,
        read: Option<Event>,
    }
    impl Transport for Fake {
        fn submit(&mut self, _: &Event) -> bool {
            self.accepted
        }
        fn readback(&mut self, _: &str) -> Option<Event> {
            self.read.clone()
        }
    }
    #[test]
    fn exact_and_failed_readbacks() {
        let e = EventBuilder::new(Kind::Custom(46100), "fixture only")
            .sign_with_keys(&Keys::generate())
            .expect("sign");
        assert!(publish_and_readback(
            &mut Fake {
                accepted: true,
                read: Some(e.clone())
            },
            &e
        )
        .is_ok());
        assert!(publish_and_readback(
            &mut Fake {
                accepted: false,
                read: Some(e.clone())
            },
            &e
        )
        .is_err());
        assert!(publish_and_readback(
            &mut Fake {
                accepted: true,
                read: None
            },
            &e
        )
        .is_err());
        for key in ["id", "pubkey", "tags", "content", "sig"] {
            let mut wire: Value = serde_json::from_str(&e.as_json()).expect("json");
            wire[key] = match key {
                "tags" => json!([["a", "wrong"]]),
                "content" => json!("wrong"),
                "sig" => json!("0".repeat(128)),
                _ => json!("0".repeat(64)),
            };
            if let Ok(other) = Event::from_json(wire.to_string()) {
                assert!(publish_and_readback(
                    &mut Fake {
                        accepted: true,
                        read: Some(other)
                    },
                    &e
                )
                .is_err());
            }
        }
        assert!(publish().is_err());
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
