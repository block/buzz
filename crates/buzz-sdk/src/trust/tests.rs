//! NIP-FP fixture conformance (`docs/nips/NIP-FP.fixtures.json`).
//!
//! Each case is signed with the fixture keys, served by a simulated origin
//! that answers filters with `Filter::match_event`, and checked against the
//! case's exhaustive `expect.items`.

use std::collections::{BTreeMap, BTreeSet};

use nostr::filter::MatchEventOptions;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::Value;

use super::{
    build_work_fingerprint, transport_bundle, Policy, Report, Request, Response, Selected, Verifier,
};
use crate::nip_oa::compute_auth_tag;

const FIXTURES: &str = include_str!("../../../../docs/nips/NIP-FP.fixtures.json");
const CASE_COUNT: usize = 15;

struct Case {
    name: String,
    report: Report,
    /// Event id hex → fixture ref.
    refs: BTreeMap<String, String>,
    expect: Value,
}

fn fixtures() -> Value {
    serde_json::from_str(FIXTURES).expect("fixtures parse")
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is not a string in {v}"))
}

fn substitute(text: &str, keys: &BTreeMap<String, Keys>, ids: &BTreeMap<String, Event>) -> String {
    let mut out = text.to_owned();
    for (name, k) in keys {
        out = out.replace(&format!("{{pk:{name}}}"), &k.public_key().to_hex());
    }
    for (name, e) in ids {
        out = out.replace(&format!("{{id:{name}}}"), &e.id.to_hex());
    }
    assert!(
        !out.contains("{pk:") && !out.contains("{id:"),
        "unresolved placeholder in {text}"
    );
    out
}

fn auth_tag(owner: &Keys, signer: &Keys, conditions: &str) -> Tag {
    let json = compute_auth_tag(owner, &signer.public_key(), conditions).expect("auth tag");
    let parts: Vec<String> = serde_json::from_str(&json).expect("auth json");
    Tag::parse(parts).expect("auth tag parses")
}

fn mutate(value: &mut Value, op: &str) {
    match op {
        "tamper_sig" => {
            let sig = str_of(value, "sig");
            let flipped = if sig.starts_with('0') { "1" } else { "0" };
            value["sig"] = Value::String(format!("{flipped}{}", &sig[1..]));
        }
        "tamper_content" => {
            let content = str_of(value, "content");
            value["content"] = Value::String(format!("{content} (edited)"));
        }
        other => panic!("unknown mutation {other}"),
    }
}

fn run(fixtures: &Value, case: &Value) -> Case {
    let name = str_of(case, "name").to_owned();
    let keys: BTreeMap<String, Keys> = fixtures["keys"]
        .as_object()
        .expect("keys")
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                Keys::parse(v.as_str().expect("key hex")).expect("key"),
            )
        })
        .collect();
    let key = |name: &str| keys.get(name).unwrap_or_else(|| panic!("no key {name}"));

    let mut signed: BTreeMap<String, Event> = BTreeMap::new();
    let mut order = Vec::new();
    for spec in case["events"].as_array().expect("events") {
        let r = str_of(spec, "ref").to_owned();
        let signer = key(str_of(spec, "signer"));
        let t = &spec["template"];
        let mut tags: Vec<Tag> = t["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .map(|tag| {
                let parts: Vec<String> = tag
                    .as_array()
                    .expect("tag")
                    .iter()
                    .map(|p| substitute(p.as_str().expect("tag part"), &keys, &signed))
                    .collect();
                Tag::parse(parts).expect("tag parses")
            })
            .collect();
        if let Some(auth) = spec.get("auth") {
            tags.push(auth_tag(
                key(str_of(auth, "owner")),
                signer,
                str_of(auth, "conditions"),
            ));
        }
        let kind = u16::try_from(t["kind"].as_u64().expect("kind")).expect("u16 kind");
        let event = EventBuilder::new(
            Kind::from(kind),
            substitute(str_of(t, "content"), &keys, &signed),
        )
        .tags(tags)
        .custom_created_at(Timestamp::from(
            t["created_at"].as_u64().expect("created_at"),
        ))
        .sign_with_keys(signer)
        .expect("sign");
        order.push(r.clone());
        signed.insert(r, event);
    }

    let mut served: BTreeMap<String, Value> = signed
        .iter()
        .map(|(r, e)| (r.clone(), serde_json::to_value(e).expect("event json")))
        .collect();
    for m in case["mutate"].as_array().into_iter().flatten() {
        let value = served.get_mut(str_of(m, "ref")).expect("mutated ref");
        mutate(value, str_of(m, "op"));
    }
    for r in case["served_except"].as_array().into_iter().flatten() {
        served.remove(r.as_str().expect("ref"));
    }
    let served: Vec<Value> = order
        .iter()
        .filter_map(|r| served.get(r).cloned())
        .collect();

    let origin = str_of(fixtures, "origin");
    let fp = &case["fingerprint"];
    let fp_signer = key(str_of(fp, "signer"));
    let selections: Vec<Selected> = fp["select"]
        .as_array()
        .expect("select")
        .iter()
        .map(|r| Selected {
            id: signed[r.as_str().expect("ref")].id,
            origin: origin.to_owned(),
        })
        .collect();
    let mut builder = build_work_fingerprint(&name, &selections).expect("fingerprint");
    if let Some(owner) = fp.get("auth").and_then(Value::as_str) {
        builder = builder.tag(auth_tag(key(owner), fp_signer, ""));
    }
    let fingerprint = builder.sign_with_keys(fp_signer).expect("sign fingerprint");
    let mut bundle = transport_bundle(&fingerprint);
    if let Some(op) = fp.get("mutate").and_then(Value::as_str) {
        mutate(&mut bundle["fingerprint"], op);
    }

    let policy = match case["policy"]["viewer_known"].as_array() {
        Some(known) => Policy::viewer_known(
            known
                .iter()
                .map(|k| key(k.as_str().expect("key name")).public_key()),
        ),
        None => Policy::default(),
    };
    let cutoff = fixtures["cutoff"].as_u64().expect("cutoff");
    let relay_self = key(str_of(fixtures, "relay_self")).public_key();

    let mut verifier = Verifier::new(&bundle, policy, cutoff).expect("verifier");
    loop {
        let queries = verifier.next_queries();
        if queries.is_empty() {
            break;
        }
        for q in queries {
            assert_eq!(q.origin, origin, "{name}: query to an unnamed origin");
            let response = match &q.request {
                Request::RelayInfo => Response::RelayInfo(Some(relay_self)),
                Request::Events(filter) => Response::Events(
                    served
                        .iter()
                        .filter(|v| {
                            serde_json::from_value::<Event>((*v).clone())
                                .is_ok_and(|e| filter.match_event(&e, MatchEventOptions::new()))
                        })
                        .take(filter.limit.unwrap_or(usize::MAX))
                        .cloned()
                        .collect(),
                ),
            };
            verifier.ingest(q.scope, response).expect("ingest");
        }
    }

    let refs = signed
        .iter()
        .map(|(r, e)| (e.id.to_hex(), r.clone()))
        .collect();
    Case {
        name,
        report: verifier.finish(),
        refs,
        expect: case["expect"].clone(),
    }
}

/// One observed item flattened the way `expect.items` lists it.
fn flatten(case: &Case, item: &Value) -> Value {
    let mut flat = item.clone();
    for key in ["authority", "ref"] {
        if let Some(status) = item.get(key).and_then(|v| v.get("status")) {
            flat[key] = status.clone();
        }
    }
    if let Some(target) = item.get("target").and_then(Value::as_str) {
        flat["target"] = Value::String(case.refs.get(target).cloned().unwrap_or_default());
    }
    let id = str_of(item, "id");
    let rungs = serde_json::to_value(&case.report.interpretation.rungs).expect("rungs");
    if let Some(rung) = rungs
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["id"] == id)
    {
        flat["rung"] = rung["rung"].clone();
        if let Some(known) = rung.get("viewer_known") {
            flat["viewer_known"] = known.clone();
        }
    }
    flat
}

fn check(case: &Case) {
    let name = &case.name;
    let report = serde_json::to_value(&case.report).expect("report json");
    assert_eq!(
        report["selection"]["fingerprint"], case.expect["fingerprint"],
        "{name}: fingerprint"
    );
    assert_eq!(
        report["selection"]["coverage"], "selected",
        "{name}: coverage label"
    );

    let expected = case.expect["items"].as_object().expect("expect.items");
    let mut seen = BTreeSet::new();
    for item in report["observed"]["items"].as_array().expect("items") {
        let id = str_of(item, "id");
        let r = case
            .refs
            .get(id)
            .unwrap_or_else(|| panic!("{name}: reported an event that is not in the case: {item}"));
        assert!(seen.insert(r.clone()), "{name}: {r} reported twice");
        let want = expected
            .get(r)
            .unwrap_or_else(|| panic!("{name}: {r} reported but not expected: {item}"));
        let got = flatten(case, item);
        for (field, value) in want.as_object().expect("expected item") {
            assert_eq!(&got[field.as_str()], value, "{name}: {r}.{field} in {got}");
        }
    }
    let missing: Vec<&String> = expected.keys().filter(|r| !seen.contains(*r)).collect();
    assert!(
        missing.is_empty(),
        "{name}: expected but not reported: {missing:?}"
    );

    let omitted: Vec<String> = case
        .report
        .interpretation
        .omitted_contrary
        .iter()
        .map(|id| case.refs.get(id).cloned().unwrap_or_else(|| id.clone()))
        .collect();
    let want_omitted: Vec<String> = case.expect["omitted_contrary"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| r.as_str().expect("ref").to_owned())
        .collect();
    assert_eq!(omitted, want_omitted, "{name}: omitted_contrary");

    let text = report.to_string();
    for banned in ["\"independent\"", "no dispute", "clean", "\"score\""] {
        assert!(!text.contains(banned), "{name}: report contains {banned}");
    }
    for line in &case.report.interpretation.coverage {
        assert!(
            !line.contains("complete"),
            "{name}: coverage claims completeness: {line}"
        );
    }
}

#[test]
fn fixtures_cover_every_attack() {
    let fixtures = fixtures();
    let cases = fixtures["cases"].as_array().expect("cases");
    assert_eq!(
        cases.len(),
        CASE_COUNT,
        "fixture case count changed; update CASE_COUNT"
    );
    for case in cases {
        check(&run(&fixtures, case));
    }
}

#[test]
fn bundle_carries_label_and_unanswered_queries_stay_unknown() {
    let fixtures = fixtures();
    let keys = Keys::parse(str_of(&fixtures["keys"], "subject")).expect("key");
    let id = nostr::EventId::all_zeros();
    let fp = build_work_fingerprint(
        "x",
        &[Selected {
            id,
            origin: "https://o.example".into(),
        }],
    )
    .expect("fingerprint")
    .sign_with_keys(&keys)
    .expect("sign");
    let mut verifier =
        Verifier::new(&transport_bundle(&fp), Policy::default(), 1).expect("verifier");
    assert!(!verifier.next_queries().is_empty());
    // Abandon every query: the next round records them as failed.
    while !verifier.next_queries().is_empty() {}
    let report = verifier.finish();
    assert_eq!(report.selection.transport, Some(super::BUNDLE_LABEL));
    let item = &report.observed.items[0];
    assert_eq!(item.bucket, super::Bucket::Inaccessible);
    assert!(
        !item.scope.is_empty(),
        "an inaccessible item cites the query that missed it"
    );
    assert!(report.observed.coverage.iter().all(|c| c.error.is_some()));
}
