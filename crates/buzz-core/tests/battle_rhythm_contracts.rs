use buzz_core::battle_rhythm::{BattleRhythmEventV1, BattleRhythmSourceV1};

#[test]
fn contracts_deserialize_the_shared_v1_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../desktop/src/features/battle-rhythm/domain/fixtures/contracts-v1.json"
    ))
    .expect("fixture JSON");
    serde_json::from_value::<BattleRhythmSourceV1>(fixture["source"].clone())
        .expect("source contract");
    serde_json::from_value::<BattleRhythmEventV1>(fixture["event"].clone())
        .expect("event contract");
}
