use buzz_core::battle_rhythm::{BattleRhythmEventV1, BattleRhythmSourceV1};

#[test]
fn contracts_deserialize_the_shared_v1_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../desktop/src/features/battle-rhythm/domain/fixtures/contracts-v1.json"
    ))
    .expect("fixture JSON");
    serde_json::from_value::<BattleRhythmSourceV1>(fixture["source"].clone())
        .expect("source contract");
    let event = serde_json::from_value::<BattleRhythmEventV1>(fixture["event"].clone())
        .expect("event contract");
    assert_eq!(
        event.recurrence.expect("recurrence").series_id,
        "sail-routine"
    );
    assert_eq!(
        event.excluded_occurrence_starts,
        ["2026-08-17T08:00:00+10:00"]
    );
}

#[test]
fn event_contract_rejects_unknown_recurrence_fields() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../desktop/src/features/battle-rhythm/domain/fixtures/contracts-v1.json"
    ))
    .expect("fixture JSON");
    let mut event = fixture["event"].clone();
    event["recurrence"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<BattleRhythmEventV1>(event).is_err());
}
