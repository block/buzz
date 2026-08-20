//! Runs the shared NIP-AD conformance corpora against the Rust verifier.
//!
//! The TypeScript mirror (`desktop/src/shared/lib/disposition.test.mjs`) runs
//! the same files. That is the whole point: the two implementations previously
//! drifted because each tested itself with its own fixtures, so the
//! divergence only surfaced in review. A case added here obliges both.
//!
//! Two corpora, doing different jobs:
//!
//! - `docs/nips/nip-ad-conformance.json` — hand-written, expressive cases.
//!   Documents intent, and pins the two implementations to each other.
//! - `docs/nips/nip-ad-lifecycle-exhaustive.json` — every bound history up to
//!   length 4, with expectations computed from declarative rules in
//!   `scripts/gen-nip-ad-corpus.mjs` rather than from either implementation.
//!   This is what makes the corpus more than a mutual-agreement pin: the
//!   generator is a third, independent statement of the lifecycle, and the
//!   spec's transition table is generated from it too.

use std::collections::HashMap;

use buzz_core::disposition::{
    account, bind_disposition, classify_request, derive_obligation, is_marked_request, BindFailure,
    Coverage, DispositionState, EffectiveOutcome, EventView, HistoryWarning, InvalidDisposition,
    InvalidRequest, RequestClass, UnsupportedRequest, REQUEST_KINDS,
};
use serde_json::Value;

fn read_corpus(name: &str) -> Value {
    let path = format!("{}/../../docs/nips/{name}", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read conformance corpus at {path}: {e}"));
    serde_json::from_str(&raw).expect("corpus is valid JSON")
}

fn corpus() -> Value {
    read_corpus("nip-ad-conformance.json")
}

/// Resolve `$name` placeholders against the corpus constants. `$AGENT_UPPER`
/// is a derived value so the corpus can express the uppercase-target case
/// without duplicating a 64-char literal.
fn constants(corpus: &Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (key, value) in corpus["constants"].as_object().expect("constants object") {
        map.insert(
            format!("${key}"),
            value.as_str().expect("constant is a string").to_string(),
        );
    }
    let agent = map["$agent"].clone();
    map.insert("$AGENT_UPPER".to_string(), agent.to_ascii_uppercase());
    map
}

fn resolve(raw: &str, consts: &HashMap<String, String>) -> String {
    consts.get(raw).cloned().unwrap_or_else(|| {
        // Placeholders can also appear embedded in JSON content strings.
        let mut out = raw.to_string();
        for (placeholder, value) in consts {
            out = out.replace(placeholder, value);
        }
        out
    })
}

fn resolve_tags(raw: &Value, consts: &HashMap<String, String>) -> Vec<Vec<String>> {
    raw.as_array()
        .expect("tags array")
        .iter()
        .map(|row| {
            row.as_array()
                .expect("tag row")
                .iter()
                .map(|cell| resolve(cell.as_str().expect("tag cell"), consts))
                .collect()
        })
        .collect()
}

fn invalid_reason_str(reason: InvalidRequest) -> &'static str {
    match reason {
        InvalidRequest::MissingAgentTarget => "missing_agent_target",
        InvalidRequest::MalformedAgentTarget => "malformed_agent_target",
        InvalidRequest::MissingChannel => "missing_channel",
        InvalidRequest::MultipleChannels => "multiple_channels",
        InvalidRequest::TargetNotMentioned => "target_not_mentioned",
        InvalidRequest::DuplicateAgentTarget => "duplicate_agent_target",
        InvalidRequest::UnsupportedKind => "unsupported_kind",
    }
}

fn unsupported_reason_str(reason: UnsupportedRequest) -> &'static str {
    match reason {
        UnsupportedRequest::MultipleAgentTargets => "multiple_agent_targets",
    }
}

fn bind_failure_str(failure: BindFailure) -> &'static str {
    match failure {
        // Structural invalidity is flattened to its precise reason: the
        // corpus names the rule that was broken, not merely "invalid".
        BindFailure::Invalid(reason) => invalid_disposition_str(reason),
        BindFailure::RequestMismatch => "request_mismatch",
        BindFailure::ChannelMismatch => "channel_mismatch",
        BindFailure::RequesterMismatch => "requester_mismatch",
        BindFailure::NotTargetAgent => "not_target_agent",
    }
}

fn invalid_disposition_str(reason: InvalidDisposition) -> &'static str {
    match reason {
        InvalidDisposition::WrongKind => "wrong_kind",
        InvalidDisposition::RequestIdCardinality => "request_id_cardinality",
        InvalidDisposition::ChannelCardinality => "channel_cardinality",
        InvalidDisposition::RequesterCardinality => "requester_cardinality",
        InvalidDisposition::StateCardinality => "state_cardinality",
        InvalidDisposition::MalformedRequestId => "malformed_request_id",
        InvalidDisposition::MalformedRequester => "malformed_requester",
        InvalidDisposition::UnknownState => "unknown_state",
        InvalidDisposition::ContentNotObject => "content_not_object",
        InvalidDisposition::ContentStateMismatch => "content_state_mismatch",
        InvalidDisposition::MissingReason => "missing_reason",
        InvalidDisposition::ReasonNotString => "reason_not_string",
        InvalidDisposition::RequestIdNotString => "request_id_not_string",
        InvalidDisposition::ContentRequestIdMismatch => "content_request_id_mismatch",
    }
}

fn warning_str(warning: HistoryWarning) -> &'static str {
    match warning {
        HistoryWarning::DuplicateTerminal => "duplicate_terminal",
        HistoryWarning::OrderedAfterTerminal => "ordered_after_terminal",
    }
}

/// The corpus encodes outcomes as `{kind, state?}` to match the tagged
/// serialization on both sides.
fn assert_outcome(name: &str, got: EffectiveOutcome, want: &Value) {
    let want_kind = want["kind"].as_str().expect("outcome kind");
    let (got_kind, got_state) = match got {
        EffectiveOutcome::Unanswered => ("unanswered", None),
        EffectiveOutcome::Open(s) => ("open", Some(s.as_str())),
        EffectiveOutcome::Settled(s) => ("settled", Some(s.as_str())),
        EffectiveOutcome::Disputed => ("disputed", None),
    };
    assert_eq!(got_kind, want_kind, "{name}: wrong outcome kind");
    assert_eq!(
        got_state,
        want["state"].as_str(),
        "{name}: wrong outcome state"
    );
}

fn request_view<'a>(
    id: &'a str,
    pubkey: &'a str,
    tags: &'a [Vec<String>],
    kind: u16,
) -> EventView<'a> {
    EventView {
        id,
        pubkey,
        kind,
        created_at: 100,
        content: "@agent do the thing",
        tags,
    }
}

/// The corpus names the request-kind set, and each implementation must match
/// it. Without this, both runners defaulted the event kind to their own
/// `REQUEST_KINDS[0]`, so Rust accepting kind 10 while TypeScript accepted
/// kind 9 would pass both suites while the CLI and desktop disagreed about
/// which events are even candidates.
#[test]
fn request_kinds_match_the_corpus() {
    let corpus = corpus();
    let want: Vec<u16> = corpus["requestKinds"]
        .as_array()
        .expect("corpus must pin requestKinds")
        .iter()
        .map(|k| k.as_u64().expect("kind") as u16)
        .collect();
    assert_eq!(
        REQUEST_KINDS.to_vec(),
        want,
        "this implementation's request-kind set disagrees with the corpus"
    );
}

#[test]
fn request_classification_matches_the_corpus() {
    let corpus = corpus();
    let consts = constants(&corpus);
    let request_id = consts["$requestId"].clone();
    let requester = consts["$requester"].clone();

    for case in corpus["requestClassification"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("case name");
        let tags = resolve_tags(&case["tags"], &consts);
        // Cases may pin a kind to exercise the unsupported-kind rule.
        let corpus_kind = corpus["requestKinds"][0].as_u64().expect("kind") as u16;
        let kind = case["kind"].as_u64().map_or(corpus_kind, |k| k as u16);
        let event = request_view(&request_id, &requester, &tags, kind);
        let want = &case["expect"];
        let want_kind = want["kind"].as_str().expect("expect kind");

        match classify_request(&event) {
            RequestClass::NotRequest => {
                assert_eq!(want_kind, "not_request", "{name}: unexpected not_request");
                assert!(!is_marked_request(&event), "{name}: marker disagreement");
            }
            RequestClass::Valid(ob) => {
                assert_eq!(
                    want_kind, "valid",
                    "{name}: expected {want_kind}, got valid"
                );
                let want_target =
                    resolve(want["targetAgent"].as_str().expect("targetAgent"), &consts);
                assert_eq!(ob.target_agent_pubkey, want_target, "{name}: wrong target");
                // Pin the whole obligation, not just the target: a classifier
                // that got the channel or requester wrong would still pass a
                // target-only assertion.
                assert_eq!(ob.request_id, request_id, "{name}: wrong request id");
                assert_eq!(ob.requester_pubkey, requester, "{name}: wrong requester");
                assert_eq!(
                    ob.channel_id,
                    resolve("$channel", &consts),
                    "{name}: wrong channel"
                );
            }
            RequestClass::Invalid(reason) => {
                assert_eq!(
                    want_kind, "invalid",
                    "{name}: expected {want_kind}, got invalid {reason:?}"
                );
                assert_eq!(
                    invalid_reason_str(reason),
                    want["reason"].as_str().expect("reason"),
                    "{name}: wrong invalid reason"
                );
            }
            RequestClass::Unsupported(reason) => {
                assert_eq!(
                    want_kind, "unsupported",
                    "{name}: expected {want_kind}, got unsupported {reason:?}"
                );
                assert_eq!(
                    unsupported_reason_str(reason),
                    want["reason"].as_str().expect("reason"),
                    "{name}: wrong unsupported reason"
                );
            }
        }
    }
}

/// The corpus's first classification case is the canonical valid request every
/// binding and lifecycle case derives its obligation from.
fn canonical_obligation(
    corpus: &Value,
    consts: &HashMap<String, String>,
    request_id: &str,
    requester: &str,
) -> buzz_core::disposition::Obligation {
    let tags = resolve_tags(&corpus["requestClassification"][0]["tags"], consts);
    let request = request_view(request_id, requester, &tags, REQUEST_KINDS[0]);
    match classify_request(&request) {
        RequestClass::Valid(ob) => *ob,
        other => panic!("corpus case 0 must be a valid request, got {other:?}"),
    }
}

#[test]
fn binding_matches_the_corpus() {
    let corpus = corpus();
    let consts = constants(&corpus);
    let request_id = consts["$requestId"].clone();
    let requester = consts["$requester"].clone();
    let obligation = canonical_obligation(&corpus, &consts, &request_id, &requester);

    for case in corpus["binding"].as_array().expect("binding cases") {
        let name = case["name"].as_str().expect("case name");
        let signer = resolve(case["signer"].as_str().expect("signer"), &consts);
        let tags = resolve_tags(&case["tags"], &consts);
        let content = resolve(case["content"].as_str().expect("content"), &consts);
        let disposition = EventView {
            id: "d0",
            pubkey: &signer,
            kind: case["kind"]
                .as_u64()
                .map_or(buzz_core::kind::KIND_AGENT_DISPOSITION as u16, |k| k as u16),
            created_at: 200,
            content: &content,
            tags: &tags,
        };

        match (bind_disposition(&obligation, &disposition), &case["expect"]) {
            (Ok(state), expect) => {
                assert!(
                    expect["bound"].as_bool() == Some(true),
                    "{name}: expected unbound, bound as {state:?}"
                );
                assert_eq!(
                    state.as_str(),
                    expect["state"].as_str().expect("state"),
                    "{name}: wrong state"
                );
            }
            (Err(failure), expect) => {
                assert!(
                    expect["bound"].as_bool() == Some(false),
                    "{name}: expected bound, got {failure:?}"
                );
                assert_eq!(
                    bind_failure_str(failure),
                    expect["reason"].as_str().expect("reason"),
                    "{name}: wrong bind failure"
                );
            }
        }
    }
}

/// Build disposition views for a history of `(id, created_at, state)` triples.
/// Tags and content are owned separately so the views can borrow them.
fn history_events<'a>(
    specs: &'a [(String, i64, String)],
    owned: &'a [(Vec<Vec<String>>, String)],
    agent: &'a str,
) -> Vec<EventView<'a>> {
    specs
        .iter()
        .zip(owned.iter())
        .map(|((id, created_at, _), (tags, content))| EventView {
            id,
            pubkey: agent,
            kind: buzz_core::kind::KIND_AGENT_DISPOSITION as u16,
            created_at: *created_at,
            content,
            tags,
        })
        .collect()
}

fn owned_history(
    specs: &[(String, i64, String)],
    request_id: &str,
    channel: &str,
    requester: &str,
) -> Vec<(Vec<Vec<String>>, String)> {
    specs
        .iter()
        .map(|(_, _, state)| {
            (
                vec![
                    vec!["e".to_string(), request_id.to_string()],
                    vec!["h".to_string(), channel.to_string()],
                    vec!["p".to_string(), requester.to_string()],
                    vec!["disposition".to_string(), state.clone()],
                ],
                format!(r#"{{"disposition":"{state}","reason":"r-{state}"}}"#),
            )
        })
        .collect()
}

#[test]
fn lifecycle_matches_the_corpus() {
    let corpus = corpus();
    let consts = constants(&corpus);
    let request_id = consts["$requestId"].clone();
    let requester = consts["$requester"].clone();
    let agent = consts["$agent"].clone();
    let channel = consts["$channel"].clone();
    let obligation = canonical_obligation(&corpus, &consts, &request_id, &requester);

    for case in corpus["lifecycle"].as_array().expect("lifecycle cases") {
        let name = case["name"].as_str().expect("case name");
        let specs: Vec<(String, i64, String)> = case["events"]
            .as_array()
            .expect("events")
            .iter()
            .map(|e| {
                let row = e.as_array().expect("event triple");
                (
                    row[0].as_str().expect("id").to_string(),
                    row[1].as_i64().expect("created_at"),
                    row[2].as_str().expect("state").to_string(),
                )
            })
            .collect();
        let owned = owned_history(&specs, &request_id, &channel, &requester);
        let events = history_events(&specs, &owned, &agent);

        let derived = derive_obligation(&obligation, &events);
        let expect = &case["expect"];

        assert_outcome(name, derived.outcome, &expect["outcome"]);
        assert_eq!(
            derived.latest_observation.map(DispositionState::as_str),
            expect["latestObservation"].as_str(),
            "{name}: wrong latest observation"
        );
        let want_warnings: Vec<&str> = expect["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .map(|w| w.as_str().expect("warning"))
            .collect();
        let got_warnings: Vec<&str> = derived.warnings.iter().map(|w| warning_str(*w)).collect();
        assert_eq!(got_warnings, want_warnings, "{name}: wrong warnings");
        assert_eq!(
            derived.is_resolved(),
            expect["resolved"].as_bool().expect("resolved"),
            "{name}: wrong resolved"
        );
    }
}

#[test]
fn exhaustive_lifecycle_matches_the_generated_corpus() {
    // Every history up to length 4, with expectations derived from the
    // generator's declarative rules rather than from this implementation.
    let exhaustive = read_corpus("nip-ad-lifecycle-exhaustive.json");
    let corpus = corpus();
    let consts = constants(&corpus);
    let request_id = consts["$requestId"].clone();
    let requester = consts["$requester"].clone();
    let agent = consts["$agent"].clone();
    let channel = consts["$channel"].clone();
    let obligation = canonical_obligation(&corpus, &consts, &request_id, &requester);

    let cases = exhaustive["cases"].as_array().expect("cases");
    assert_eq!(
        cases.len() as u64,
        exhaustive["caseCount"].as_u64().expect("caseCount"),
        "corpus caseCount disagrees with its own case list"
    );

    for case in cases {
        let history: Vec<String> = case["history"]
            .as_array()
            .expect("history")
            .iter()
            .map(|s| s.as_str().expect("state").to_string())
            .collect();
        let name = format!("[{}]", history.join(" -> "));

        // Index order is sort order: increasing created_at, distinct ids.
        let specs: Vec<(String, i64, String)> = history
            .iter()
            .enumerate()
            .map(|(i, state)| (format!("d{i:04}"), 100 + i as i64, state.clone()))
            .collect();
        let owned = owned_history(&specs, &request_id, &channel, &requester);
        let events = history_events(&specs, &owned, &agent);

        let derived = derive_obligation(&obligation, &events);
        let expect = &case["expect"];

        assert_outcome(&name, derived.outcome, &expect["outcome"]);
        assert_eq!(
            derived.latest_observation.map(DispositionState::as_str),
            expect["latestObservation"].as_str(),
            "{name}: wrong latest observation"
        );
        let want_warnings: Vec<&str> = expect["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .map(|w| w.as_str().expect("warning"))
            .collect();
        let got_warnings: Vec<&str> = derived.warnings.iter().map(|w| warning_str(*w)).collect();
        assert_eq!(got_warnings, want_warnings, "{name}: wrong warnings");
        assert_eq!(
            derived.is_resolved(),
            expect["resolved"].as_bool().expect("resolved"),
            "{name}: wrong resolved"
        );
    }
}

#[test]
fn accounting_never_reports_a_malformed_request_as_an_agent_gap() {
    // The corpus's invalid and unsupported cases must land in their own
    // buckets, not in `unanswered` — blaming an agent for a client fault
    // produces a gap that can never be cleared by anyone.
    let corpus = corpus();
    let consts = constants(&corpus);
    let requester = consts["$requester"].clone();

    let cases: Vec<(String, Vec<Vec<String>>, String)> = corpus["requestClassification"]
        .as_array()
        .expect("cases")
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let kind = c["expect"]["kind"].as_str()?;
            if kind != "invalid" && kind != "unsupported" {
                return None;
            }
            // The unsupported-kind case is about the event's kind, not its
            // tags, and is covered by the classification test.
            if c["kind"].as_u64().is_some() {
                return None;
            }
            Some((
                format!("{i:064}"),
                resolve_tags(&c["tags"], &consts),
                kind.to_string(),
            ))
        })
        .collect();

    let requests: Vec<EventView<'_>> = cases
        .iter()
        .map(|(id, tags, _)| EventView {
            id,
            pubkey: &requester,
            kind: REQUEST_KINDS[0],
            created_at: 100,
            content: "marked but unusable",
            tags,
        })
        .collect();

    let acc = account(&requests, &[], Coverage::complete());
    let want_invalid = cases.iter().filter(|(_, _, k)| k == "invalid").count();
    let want_unsupported = cases.iter().filter(|(_, _, k)| k == "unsupported").count();
    assert_eq!(acc.invalid_requests.len(), want_invalid);
    assert_eq!(acc.unsupported_requests.len(), want_unsupported);
    assert!(
        acc.unanswered.is_empty(),
        "malformed requests must never be counted as unanswered obligations"
    );
    assert!(
        !acc.all_resolved(),
        "unanswerable requests block a clean claim"
    );
}
