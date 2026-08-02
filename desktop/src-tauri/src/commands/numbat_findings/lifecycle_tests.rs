use std::{fs::File, io::Write as _};

use super::*;

const TEST_AGENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn finding_json(finding_id: &str) -> String {
    serde_json::json!({
        "schema_version": "0.2.0",
        "record_type": "finding",
        "finding_id": finding_id,
        "rule_id": "chain.secret_read_then_egress",
        "title": "Secret access followed by network egress",
        "severity": "high",
        "detected_at": "2026-07-30T14:40:00Z",
        "source_agent": "codex",
        "session_id": "session-safe-01",
        "cited_event_ids": ["event-one", "event-two"]
    })
    .to_string()
}

fn test_health() -> NumbatGuardianHealth {
    NumbatGuardianHealth {
        state: "configured".into(),
        detail: "test".into(),
    }
}

#[test]
fn retention_reader_preserves_a_record_appended_to_the_previous_generation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("findings.ndjson");
    let padding = format!("{{\"padding\":\"{}\"}}\n", "x".repeat(1024));
    let mut file = File::create(&path).expect("create");
    while file.stream_position().expect("position") <= MAX_LOCAL_RECORD_BYTES {
        file.write_all(padding.as_bytes()).expect("write padding");
    }
    file.sync_all().expect("sync");
    drop(file);

    assert!(enforce_continuous_retention(&path).expect("retain"));
    let late = finding_json("fnd-late-previous");
    {
        let mut previous = OpenOptions::new()
            .append(true)
            .open(previous_findings_path(&path))
            .expect("open previous generation");
        writeln!(previous, "{late}").expect("append late record");
    }

    let findings = read_previous_findings_tail(
        &path,
        Some((
            TEST_AGENT,
            "session-safe-01",
            "channel-safe-01",
            "turn-safe-01",
        )),
        test_health(),
    )
    .expect("read previous generation");
    assert!(findings
        .iter()
        .any(|finding| finding.finding_id == "fnd-late-previous"));
    assert!(!std::fs::read_to_string(&path)
        .expect("read current generation")
        .contains("fnd-late-previous"));
}

#[test]
fn activation_requires_a_valid_record_after_configuration_baseline() {
    let agent = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    record_verification_baseline(agent, 7, 500);
    assert!(!is_post_configuration_finding(agent, 7, 500, true));
    assert!(is_post_configuration_finding(agent, 7, 501, true));
    assert!(!is_post_configuration_finding(agent, 7, 600, false));
    assert!(!is_post_configuration_finding(agent, 8, 100, true));
    assert!(is_post_configuration_finding(agent, 8, 101, true));

    let unseen_agent = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    assert!(!is_post_configuration_finding(unseen_agent, 11, 900, true));
    assert!(!is_post_configuration_finding(unseen_agent, 11, 900, true));
    assert!(is_post_configuration_finding(unseen_agent, 11, 901, true));
}
