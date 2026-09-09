use buzz_core::bw::{parse_json, Consumer};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
fn corpus() -> Value {
    parse_json(include_bytes!("../../../docs/nips/NIP-BW.fixtures.json")).expect("corpus")
}
fn consumer(d: &Value, case: &str) -> Consumer {
    let c = d["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["name"] == case)
        .expect("case");
    Consumer::new(
        serde_json::from_value(c["trust"].clone()).expect("trust"),
        serde_json::from_value(c["external"].clone()).expect("external"),
        c["now"].as_u64().expect("now"),
    )
}
fn feed(c: &mut Consumer, d: &Value, label: &str) -> buzz_core::bw::Decision {
    c.ingest(&serde_json::to_vec(&d["events"][label]["event"]).expect("wire"))
}
fn signed(mut e: Value, scalar: u8) -> Value {
    let mut secret = [0u8; 32];
    secret[31] = scalar;
    let keys = Keys::parse(&hex::encode(secret)).expect("test key");
    let tags = e["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .map(|t| {
            Tag::parse(
                t.as_array()
                    .expect("tag")
                    .iter()
                    .map(|s| s.as_str().expect("str")),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .expect("tags");
    let event = EventBuilder::new(
        Kind::Custom(e["kind"].as_u64().expect("kind") as u16),
        e["content"].as_str().expect("content"),
    )
    .tags(tags)
    .custom_created_at(Timestamp::from(e["created_at"].as_u64().expect("time")))
    .sign_with_keys(&keys)
    .expect("sign");
    e = serde_json::from_str(&event.as_json()).expect("wire");
    e
}
#[test]
fn historical_acceptance_survives_late_text_fork() {
    let d = corpus();
    let c = &d["cases"][0];
    let mut a = consumer(&d, "complete-partial-reject");
    for l in c["input"].as_array().expect("input") {
        feed(&mut a, &d, l.as_str().expect("label"));
    }
    let fork = feed(&mut a, &d, "text-fork");
    assert_eq!(fork.outcome, "conflict");
    let root = d["events"]["root_a"]["event"]["id"].as_str().expect("root");
    assert_eq!(a.projection()["issues"][root], "resolved");
    let mut b = consumer(&d, "complete-partial-reject");
    for l in c["input"].as_array().expect("input") {
        let label = l.as_str().expect("label");
        feed(&mut b, &d, label);
        if label == "update_a" {
            feed(&mut b, &d, "text-fork");
        }
    }
    assert_eq!(a.projection(), b.projection());
}
#[test]
fn corrected_evidence_recomputes_acceptance_without_a_latch() {
    let d = corpus();
    let c = &d["cases"][0];
    let mut h = consumer(&d, "complete-partial-reject");
    for l in c["input"].as_array().expect("input") {
        feed(&mut h, &d, l.as_str().expect("label"));
    }
    let root = d["events"]["root_a"]["event"]["id"].as_str().expect("root");
    assert_eq!(h.projection()["issues"][root], "resolved");
    let mut external = c["external"].clone();
    external["downloads"] = json!([]);
    h.observe(
        serde_json::from_value(external).expect("external"),
        1800001000,
    );
    assert_ne!(h.projection()["issues"][root], "resolved");
    h.observe(
        serde_json::from_value(c["external"].clone()).expect("external"),
        1800001000,
    );
    assert_eq!(h.projection()["issues"][root], "resolved");
    assert_eq!(
        h.archived_inputs().len(),
        c["input"].as_array().expect("input").len()
    );
}
#[test]
fn unknown_record_duplicate_content_and_noncausal_reference() {
    let d = corpus();
    let mut h = consumer(&d, "issue-state-positive");
    for label in ["repo", "policy", "root_a"] {
        feed(&mut h, &d, label);
    }
    let mut e = d["events"]["enroll_a"]["event"].clone();
    e["tags"][0][1] = json!("unknown");
    let out = h.ingest(&serde_json::to_vec(&signed(e, 2)).expect("wire"));
    assert_eq!(out.stage, "shape");
    assert_eq!(out.code, "unknown-record");
    let mut e = d["events"]["enroll_a"]["event"].clone();
    e["content"] = json!(r#"{"state":"triage","state":"backlog"}"#);
    let out = h.ingest(&serde_json::to_vec(&signed(e, 2)).expect("wire"));
    assert_eq!(out.stage, "shape");
    let mut e = d["events"]["enroll_a"]["event"].clone();
    e["created_at"] = json!(1800000000);
    let out = h.ingest(&serde_json::to_vec(&signed(e, 2)).expect("wire"));
    assert_eq!(out.code, "reference-time");
}
#[test]
fn signature_failure_never_poisons_genuine_id() {
    let d = corpus();
    let case = d["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["name"] == "test-ready-positive")
        .expect("case");
    let mut h = consumer(&d, "test-ready-positive");
    for l in case["input"].as_array().expect("input") {
        feed(&mut h, &d, l.as_str().expect("label"));
    }
    assert_eq!(feed(&mut h, &d, "forged-verdict").stage, "signature");
    assert_eq!(feed(&mut h, &d, "verdict_a").outcome, "accept");
    assert_eq!(feed(&mut h, &d, "forged-verdict").stage, "signature");
    assert_eq!(feed(&mut h, &d, "verdict_a").outcome, "replay");
}
#[test]
fn regular_storage_class_and_repository_id_time_filters() {
    let d = corpus();
    for label in ["policy", "artifact"] {
        let e = nostr::Event::from_json(d["events"][label]["event"].to_string()).expect("event");
        let kind = e.kind.as_u16() as u32;
        assert!(!buzz_core::kind::is_ephemeral(kind));
        assert!(!buzz_core::kind::is_replaceable(kind));
        assert!(!buzz_core::kind::is_parameterized_replaceable(kind));
        let repo = d["cases"][0]["trust"]["repo"].as_str().expect("repo");
        let f:nostr::Filter=serde_json::from_value(json!({"kinds":[kind],"#a":[repo],"ids":[e.id.to_hex()],"until":e.created_at.as_secs(),"limit":1})).expect("filter");
        let stored = buzz_core::StoredEvent::new(e.clone(), None);
        assert!(buzz_core::filter::filters_match(
            std::slice::from_ref(&f),
            &stored
        ));
        let mut bad = serde_json::to_value(&f).expect("filter");
        bad["#a"] = json!(["30617:wrong:repo"]);
        assert!(!buzz_core::filter::filters_match(
            &[serde_json::from_value(bad).expect("filter")],
            &stored
        ));
        let mut before = serde_json::to_value(&f).expect("filter");
        before["until"] = json!(e.created_at.as_secs() - 1);
        assert!(!buzz_core::filter::filters_match(
            &[serde_json::from_value(before).expect("filter")],
            &stored
        ));
    }
}

#[test]
fn historical_acceptance_survives_policy_fork_with_valid_bound_authority() {
    let d = corpus();
    let case = &d["cases"][0];
    let mut h = consumer(&d, "complete-partial-reject");
    for l in case["input"].as_array().expect("inputs") {
        feed(&mut h, &d, l.as_str().expect("label"));
    }
    let fork = feed(&mut h, &d, "policy-fork");
    assert_eq!(fork.outcome, "conflict");
    let root = d["events"]["root_a"]["event"]["id"].as_str().expect("root");
    assert_eq!(h.projection()["issues"][root], "resolved");
}

#[test]
fn every_case_converges_when_dependencies_arrive_in_reverse_order() {
    let d = corpus();
    let mut failures = Vec::new();
    for case in d["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("name");
        let mut forward = consumer(&d, name);
        let mut reverse = consumer(&d, name);
        for l in case["input"].as_array().expect("inputs") {
            feed(&mut forward, &d, l.as_str().expect("label"));
        }
        for l in case["input"].as_array().expect("inputs").iter().rev() {
            feed(&mut reverse, &d, l.as_str().expect("label"));
        }
        if forward.projection() != reverse.projection() {
            failures.push(name);
        }
    }
    assert!(
        failures.is_empty(),
        "delivery-order dependence: {failures:?}"
    );
}

#[test]
fn partial_text_patch_preserves_classification_and_ready_criteria() {
    let d = corpus();
    let mut h = consumer(&d, "issue-state-positive");
    for l in [
        "repo",
        "policy",
        "root_a",
        "enroll_a",
        "update_a",
        "accept_a",
        "backlog_a",
        "assign_a",
    ] {
        feed(&mut h, &d, l);
    }
    let mut update = d["events"]["update_a"]["event"].clone();
    update["created_at"] = json!(1800000006);
    update["tags"]
        .as_array_mut()
        .expect("tags")
        .push(json!(["previous", d["events"]["update_a"]["event"]["id"]]));
    update["content"] = json!(
        json!({"patch":{"description":"Clarify the text; retain classification"}}).to_string()
    );
    let update = signed(update, 1);
    assert_eq!(
        h.ingest(&serde_json::to_vec(&update).expect("wire"))
            .outcome,
        "accept"
    );
    let mut ready = d["events"]["ready_a"]["event"].clone();
    let mut body: Value =
        serde_json::from_str(ready["content"].as_str().expect("body")).expect("json");
    body["update"] = update["id"].clone();
    ready["content"] = json!(body.to_string());
    let ready = signed(ready, 1);
    let out = h.ingest(&serde_json::to_vec(&ready).expect("wire"));
    assert_eq!(out.outcome, "accept", "{out:?}");
    let root = d["events"]["root_a"]["event"]["id"].as_str().expect("root");
    assert_eq!(out.projection["issue_fields"][root]["type"], "bug");
    assert_eq!(out.projection["issue_fields"][root]["platform"], "windows");
}

#[test]
fn shape_errors_use_lexical_code_order() {
    let d = corpus();
    let mut h = consumer(&d, "issue-state-positive");
    let mut event = d["events"]["enroll_a"]["event"].clone();
    let tag = event["tags"][0].clone();
    event["tags"].as_array_mut().expect("tags").push(tag);
    event["content"] = json!("not JSON");
    let out = h.ingest(&serde_json::to_vec(&signed(event, 2)).expect("wire"));
    assert_eq!(out.stage, "shape");
    assert_eq!(out.code, "duplicate-tag");
}

#[test]
fn missing_provider_proof_is_pending_and_host_denial_is_separate() {
    let d = corpus();
    let case = &d["cases"][0];
    let mut h = consumer(&d, "complete-partial-reject");
    let mut evidence = case["external"].clone();
    evidence["provider_readbacks"] = json!([]);
    h.observe(
        serde_json::from_value(evidence).expect("evidence"),
        1800001000,
    );
    for label in case["input"].as_array().expect("input") {
        let label = label.as_str().expect("label");
        let result = feed(&mut h, &d, label);
        if label == "run" {
            assert_eq!(result.outcome, "pending");
            assert_eq!(result.stage, "external");
            assert_eq!(result.code, "provider-readback");
        }
    }
    let root = d["events"]["root_a"]["event"]["id"].as_str().expect("root");
    assert_ne!(h.projection()["issues"][root], "resolved");
    let mut evidence = case["external"].clone();
    evidence["host_authorization"]["allowed"] = json!(false);
    h.observe(
        serde_json::from_value(evidence).expect("evidence"),
        1800001000,
    );
    let p = h.projection();
    assert_eq!(p["issues"][root], "resolved");
    assert_eq!(p["host_authorized"], false);
    assert_eq!(p["dispatch_count"], 0);
}
#[test]
fn multiple_external_errors_use_lexical_code_order() {
    let d = corpus();
    let mut h = consumer(&d, "release-set-positive");
    let case = d["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .find(|c| c["name"] == "release-set-positive")
        .expect("case");
    let mut evidence = case["external"].clone();
    evidence["git_ancestry"][0]["is_ancestor"] = json!(false);
    evidence["git_readbacks"][2]["head"] = json!("9".repeat(40));
    h.observe(
        serde_json::from_value(evidence).expect("evidence"),
        1800001000,
    );
    for l in case["input"].as_array().expect("input") {
        let label = l.as_str().expect("label");
        let out = feed(&mut h, &d, label);
        if label == "set" {
            assert_eq!(out.stage, "external");
            assert_eq!(out.code, "git-ancestry");
        }
    }
}
