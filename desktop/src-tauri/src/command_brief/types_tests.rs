use super::types::{
    AdviserContribution, AdviserId, BriefLifecycleRecord, BriefRunState, BriefRunStatus,
    BriefSchedule, BriefSection, CitedFinding, CommandBrief, PublishedCommandBrief,
    ADVISORY_LIMITATION, MAX_ARRAY_ITEMS, MAX_TEXT_BYTES,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const NOW: &str = "2026-07-25T06:00:00Z";

fn finding(text: &str) -> Value {
    json!({ "classification": "OFFICIAL", "text": text, "sourceIds": ["ledger-1"] })
}

fn sections() -> Value {
    json!({
        "today": [finding("A supported specialist finding.")],
        "operations": [],
        "navigation": [],
        "daily_routine": [],
        "reports": [],
        "planning_30_60_90": [],
        "decisions": [],
        "conflicts_and_gaps": [],
        "sources": []
    })
}

fn source() -> Value {
    json!({
        "classification": "OFFICIAL",
        "ledgerId": "ledger-1",
        "sourceId": "source-1",
        "sourceKind": "rag",
        "collection": "engineering-orders",
        "documentId": "document-1",
        "chunkId": "chunk-1",
        "timestamp": NOW,
        "snapshotId": "snapshot-1",
        "quotedLocation": { "quote": "A supported quote.", "location": "section 1" },
        "retrievedAt": NOW,
        "observedAt": NOW
    })
}

fn contribution(adviser: &str) -> Value {
    json!({
        "classification": "OFFICIAL",
        "adviser": adviser,
        "section": match adviser {
            "operations" => "operations",
            "navigation" => "navigation",
            "daily_routine" => "daily_routine",
            "reporting" => "reports",
            "plans" => "planning_30_60_90",
            _ => "today"
        },
        "findings": [finding("A supported specialist finding.")],
        "confidence": 0.85,
        "limitations": ["The source is bounded to the frozen snapshot."],
        "dissent": [],
        "proposedActions": []
    })
}

fn brief_value() -> Value {
    json!({
        "version": 1,
        "classification": "OFFICIAL",
        "generatedAt": NOW,
        "runId": "run-1",
        "scheduleId": "daily-command-brief",
        "snapshotId": "snapshot-1",
        "sections": sections(),
        "degradedSections": [],
        "missingInformation": [],
        "dissent": [],
        "sourceLedger": [source()],
        "sourceFreshness": { "classification": "OFFICIAL", "asOf": NOW, "staleSourceIds": [] },
        "contributions": [
            contribution("operations"),
            contribution("navigation"),
            contribution("daily_routine"),
            contribution("reporting"),
            contribution("plans")
        ],
        "advisoryLimitation": ADVISORY_LIMITATION
    })
}

fn parse(value: Value) -> Result<CommandBrief, String> {
    CommandBrief::try_from(value).map_err(|error| error.to_string())
}

fn parse_contribution(value: Value) -> Result<AdviserContribution, String> {
    AdviserContribution::parse_for_adviser(
        value,
        AdviserId::Operations,
        &BTreeSet::from(["ledger-1".to_string()]),
    )
    .map_err(|error| error.to_string())
}

#[test]
fn serializes_the_exact_camel_case_official_wire_shape() {
    let brief = parse(brief_value()).expect("valid canonical brief");
    let value = serde_json::to_value(&brief).expect("serialize canonical brief");
    assert_eq!(value["classification"], "OFFICIAL");
    assert!(value.get("generatedAt").is_some());
    assert!(value.get("sourceLedger").is_some());
    assert!(value.get("lifecycleAuditEventId").is_none());
    assert_eq!(
        value["sections"]
            .as_object()
            .expect("sections object")
            .len(),
        9
    );
    assert_eq!(
        serde_json::to_value(AdviserId::Navigation).expect("serialize adviser"),
        "navigation"
    );
    assert_eq!(
        serde_json::to_value(BriefSection::Planning306090).expect("serialize section"),
        "planning_30_60_90"
    );
    assert_eq!(
        serde_json::to_value(BriefRunState::CollectingSources).expect("serialize state"),
        "collecting_sources"
    );
}

#[test]
fn rejects_closed_enum_unknowns_and_non_official_classification() {
    for (path, replacement) in [
        ("adviser", "unapproved"),
        ("section", "unapproved"),
        ("sourceKind", "network"),
        ("classification", "PUBLIC"),
    ] {
        let mut value = brief_value();
        match path {
            "adviser" => value["contributions"][0][path] = json!(replacement),
            "section" => value["contributions"][0][path] = json!(replacement),
            "sourceKind" => value["sourceLedger"][0][path] = json!(replacement),
            "classification" => value[path] = json!(replacement),
            _ => unreachable!(),
        }
        assert!(parse(value).is_err(), "{path} must be closed");
    }
}

#[test]
fn accepts_only_the_closed_run_state_vocabulary() {
    for state in [
        "queued",
        "collecting_sources",
        "running_specialists",
        "consolidating",
        "persisting",
        "completed",
        "degraded",
        "cancelled",
        "failed",
    ] {
        assert!(BriefRunStatus::try_from(json!({
            "classification": "OFFICIAL",
            "runId": "run-1",
            "scheduleId": "daily-command-brief",
            "state": state,
            "updatedAt": NOW,
            "degradedSections": [],
            "error": null
        }))
        .is_ok());
    }
    assert!(BriefRunStatus::try_from(json!({
        "classification": "OFFICIAL",
        "runId": "run-1",
        "scheduleId": "daily-command-brief",
        "state": "invented",
        "updatedAt": NOW,
        "degradedSections": [],
        "error": null
    }))
    .is_err());
}

#[test]
fn requires_official_classification_for_schedule_and_lifecycle_wire_records() {
    let schedule = json!({
        "classification": "OFFICIAL",
        "scheduleId": "daily-command-brief",
        "enabled": true,
        "localTime": "06:00",
        "timezone": "Australia/Sydney",
        "catchUpSameDay": true,
        "concurrency": 1
    });
    assert!(BriefSchedule::try_from(schedule.clone()).is_ok());
    let mut missing = schedule.clone();
    missing
        .as_object_mut()
        .expect("schedule")
        .remove("classification");
    assert!(BriefSchedule::try_from(missing).is_err());
    let mut public = schedule;
    public["classification"] = json!("PUBLIC");
    assert!(BriefSchedule::try_from(public).is_err());

    let lifecycle = json!({
        "classification": "OFFICIAL",
        "runId": "run-1",
        "scheduleId": "daily-command-brief",
        "state": "completed",
        "occurredAt": NOW,
        "snapshotId": "snapshot-1",
        "previousLifecycleAuditEventId": null
    });
    assert!(BriefLifecycleRecord::try_from(lifecycle.clone()).is_ok());
    let mut missing = lifecycle;
    missing
        .as_object_mut()
        .expect("lifecycle")
        .remove("classification");
    assert!(BriefLifecycleRecord::try_from(missing).is_err());
}

#[test]
fn rejects_unsafe_nested_classification_and_extra_keys() {
    let mut unsafe_classification = brief_value();
    unsafe_classification["sourceLedger"][0]["classification"] = json!("PUBLIC");
    assert!(parse(unsafe_classification).is_err());

    let mut extra_key = brief_value();
    extra_key["contributions"][1]["findingType"] = json!("order");
    assert!(parse(extra_key).is_err());
}

#[test]
fn requires_official_classification_on_every_nested_wire_object() {
    let mut missing = brief_value();
    missing["sourceLedger"][0]
        .as_object_mut()
        .expect("source record")
        .remove("classification");
    assert!(parse(missing).is_err(), "nested OFFICIAL must be explicit");

    let mut public_source = brief_value();
    public_source["sourceLedger"][0]["classification"] = json!("PUBLIC");
    assert!(parse(public_source).is_err());

    let mut mismatched_action = brief_value();
    mismatched_action["contributions"][0]["proposedActions"] = json!([{
        "classification": "PUBLIC",
        "actionId": "action-1",
        "text": "This must remain a proposal.",
        "approvalState": "pending"
    }]);
    assert!(parse(mismatched_action).is_err());
}

#[test]
fn final_findings_must_exactly_match_validated_specialist_provenance() {
    let mut unsupported = brief_value();
    unsupported["sections"]["today"][0]["text"] = json!("A new claim with a valid citation.");
    assert!(parse(unsupported).is_err());

    let consolidated = brief_value();
    assert!(parse(consolidated).is_ok());
}

#[test]
fn rejects_duplicate_and_dangling_source_references() {
    let mut duplicate_ledger = brief_value();
    duplicate_ledger["sourceLedger"] = json!([source(), source()]);
    assert!(parse(duplicate_ledger).is_err());

    let mut dangling_citation = brief_value();
    dangling_citation["contributions"][0]["findings"][0]["sourceIds"] = json!(["missing"]);
    assert!(parse(dangling_citation).is_err());

    let mut duplicate_citation = brief_value();
    duplicate_citation["contributions"][0]["findings"][0]["sourceIds"] =
        json!(["ledger-1", "ledger-1"]);
    assert!(parse(duplicate_citation).is_err());
}

#[test]
fn rejects_duplicate_or_missing_specialist_contributions_and_mixed_snapshots() {
    let mut duplicate_adviser = brief_value();
    duplicate_adviser["contributions"][1]["adviser"] = json!("operations");
    assert!(parse(duplicate_adviser).is_err());

    let mut missing_specialist = brief_value();
    missing_specialist["contributions"]
        .as_array_mut()
        .expect("array")
        .pop();
    assert!(parse(missing_specialist).is_err());

    let mut mixed_snapshot = brief_value();
    mixed_snapshot["sourceLedger"][0]["snapshotId"] = json!("snapshot-2");
    assert!(parse(mixed_snapshot).is_err());
}

#[test]
fn rejects_invalid_freshness_confidence_and_non_pending_actions() {
    let mut stale_missing = brief_value();
    stale_missing["sourceFreshness"]["staleSourceIds"] = json!(["missing"]);
    assert!(parse(stale_missing).is_err());

    let mut confidence = brief_value();
    confidence["contributions"][0]["confidence"] = json!(1.01);
    assert!(parse(confidence).is_err());

    let mut action = brief_value();
    action["contributions"][0]["proposedActions"] = json!([{
        "actionId": "action-1",
        "text": "This must remain a proposal.",
        "approvalState": "approved"
    }]);
    assert!(parse(action).is_err());
}

#[test]
fn navigation_has_no_order_or_decision_channel_and_the_advisory_limitation_is_required() {
    let mut order = brief_value();
    order["contributions"][1]["orders"] = json!(["Turn port immediately"]);
    assert!(parse(order).is_err());

    let mut decision = brief_value();
    decision["contributions"][1]["decisions"] = json!(["Proceed"]);
    assert!(parse(decision).is_err());

    let mut missing_limitation = brief_value();
    missing_limitation["advisoryLimitation"] = json!("Different wording");
    assert!(parse(missing_limitation).is_err());
}

#[test]
fn rejects_control_characters_and_every_bounded_text_and_array_overflow() {
    for length in [MAX_TEXT_BYTES, MAX_TEXT_BYTES + 1] {
        let mut value = brief_value();
        value["missingInformation"] = json!(["x".repeat(length)]);
        assert_eq!(parse(value).is_ok(), length == MAX_TEXT_BYTES);
    }
    for count in [MAX_ARRAY_ITEMS, MAX_ARRAY_ITEMS + 1] {
        let mut value = brief_value();
        value["missingInformation"] = json!((0..count)
            .map(|index| format!("missing-{index}"))
            .collect::<Vec<_>>());
        assert_eq!(parse(value).is_ok(), count == MAX_ARRAY_ITEMS);
    }
    let mut control = brief_value();
    control["missingInformation"] = json!(["unsafe\u{0000}value"]);
    assert!(parse(control).is_err());
}

#[test]
fn published_wrapper_adds_the_signed_event_id_only_after_validating_the_brief() {
    let published = PublishedCommandBrief::try_from(json!({
        "classification": "OFFICIAL",
        "brief": brief_value(),
        "lifecycleAuditEventId": "abcdef0123456789",
        "publicationState": "queued"
    }))
    .expect("valid post-signing envelope");
    let serialized = serde_json::to_value(published).expect("serialize envelope");
    assert!(serialized["brief"].get("lifecycleAuditEventId").is_none());
    assert_eq!(serialized["publicationState"], "queued");
}

#[test]
fn standalone_contribution_parser_preserves_the_authoritative_contract() {
    let finding = CitedFinding::parse_for_ledger(
        finding("A supported specialist finding."),
        &BTreeSet::from(["ledger-1".to_string()]),
    )
    .expect("standalone finding");
    assert_eq!(finding.source_ids(), &["ledger-1"]);

    let parsed = parse_contribution(contribution("operations")).expect("valid contribution");
    assert_eq!(parsed.adviser(), AdviserId::Operations);
    assert_eq!(parsed.section(), BriefSection::Operations);
    assert_eq!(
        parsed.findings()[0].text(),
        "A supported specialist finding."
    );
    assert_eq!(parsed.findings()[0].source_ids(), &["ledger-1"]);
    assert_eq!(
        parsed.limitations(),
        &["The source is bounded to the frozen snapshot."]
    );
    assert!(parsed.dissent().is_empty());

    let mut missing_classification = contribution("operations");
    missing_classification
        .as_object_mut()
        .expect("contribution")
        .remove("classification");
    assert!(parse_contribution(missing_classification).is_err());

    let mut wrong_adviser = contribution("navigation");
    wrong_adviser["section"] = json!("operations");
    assert!(parse_contribution(wrong_adviser).is_err());

    let mut dangling = contribution("operations");
    dangling["findings"][0]["sourceIds"] = json!(["ledger-missing"]);
    assert!(parse_contribution(dangling).is_err());

    let mut oversized = contribution("operations");
    oversized["findings"][0]["text"] = json!("x".repeat(MAX_TEXT_BYTES + 1));
    assert!(parse_contribution(oversized).is_err());

    let mut approved = contribution("operations");
    approved["proposedActions"] = json!([{
        "classification": "OFFICIAL",
        "actionId": "action-1",
        "text": "This must remain a proposal.",
        "approvalState": "approved"
    }]);
    assert!(parse_contribution(approved).is_err());

    let mut extra = contribution("operations");
    extra["prompt"] = json!("renderer override");
    assert!(parse_contribution(extra).is_err());
}
