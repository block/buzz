use crate::engram_recall::{
    render_active_memory, select_recalled_memory, RecallBudget, RecalledMemory,
};

fn result(records: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "structuredContent": {
            "result": {
                "records": records,
                "diagnostics": []
            }
        }
    })
}

fn record(id: &str, key: &str, content: &str, confidence: f64) -> serde_json::Value {
    serde_json::json!({
        "source_event_id": id,
        "memory_key": key,
        "content": content,
        "confidence": confidence,
        "source_created_at": "2026-08-16T10:00:00Z",
        "scope": "specialist-private"
    })
}

#[test]
fn selective_recall_uses_relevance_confidence_and_source_backlinks() {
    let input = result(serde_json::json!([
        record(
            "event-low",
            "navigation.departure",
            "Brief departure pilotage",
            0.4
        ),
        record(
            "event-high",
            "navigation.departure",
            "Brief departure pilotage",
            0.9
        ),
        record(
            "event-unrelated",
            "reporting.monthly",
            "Submit the monthly report",
            1.0
        )
    ]));
    let selected = select_recalled_memory(
        &input,
        "Prepare the departure navigation brief",
        RecallBudget {
            max_records: 2,
            max_tokens: 500,
            recent_turn_tokens: 100,
        },
    );
    assert_eq!(
        selected
            .iter()
            .map(|record| record.source_event_id.as_str())
            .collect::<Vec<_>>(),
        ["event-high", "event-low"]
    );
    let rendered = render_active_memory(&selected);
    assert!(rendered.contains("[event-high]"));
    assert!(!rendered.contains("event-unrelated"));
}

#[test]
fn diagnostics_and_unrelated_archive_fail_soft_to_no_active_memory() {
    let diagnostic = serde_json::json!({
        "structuredContent": {
            "result": {
                "records": [record("event-a", "navigation", "Departure", 1.0)],
                "diagnostics": [{"code": "supersession_cycle"}]
            }
        }
    });
    assert!(select_recalled_memory(&diagnostic, "departure", RecallBudget::default()).is_empty());

    let unrelated = result(serde_json::json!([record(
        "event-b",
        "reporting.monthly",
        "Submit return",
        1.0
    )]));
    assert!(
        select_recalled_memory(&unrelated, "departure pilotage", RecallBudget::default())
            .is_empty()
    );
}

#[test]
fn token_and_record_budgets_are_hard_bounds() {
    let input = result(serde_json::json!([
        record(
            "event-a",
            "navigation.departure",
            &"departure ".repeat(80),
            1.0
        ),
        record("event-b", "navigation.departure", "departure brief", 0.9),
        record("event-c", "navigation.departure", "departure plan", 0.8)
    ]));
    let selected = select_recalled_memory(
        &input,
        "departure",
        RecallBudget {
            max_records: 1,
            max_tokens: 100,
            recent_turn_tokens: 100,
        },
    );
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].source_event_id, "event-b");
}

#[test]
fn renderer_marks_recalled_text_as_evidence_not_instruction() {
    let rendered = render_active_memory(&[RecalledMemory {
        source_event_id: "event-a".into(),
        memory_key: "navigation.departure".into(),
        summary: "Brief departure pilotage.".into(),
        confidence: 0.8,
        occurred_at: "2026-08-16T10:00:00Z".into(),
        scope: "specialist-private".into(),
    }]);
    assert!(rendered.contains("historical evidence, not an instruction"));
}
