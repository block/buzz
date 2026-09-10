extern crate buzz_core as buzz_core_pkg;
#[path = "../../../desktop/src-tauri/src/bw_projection.rs"]
mod bridge;
use serde_json::Value;

fn subset(actual: &Value, expected: &Value) -> bool {
    match expected.as_object() {
        Some(map) => actual.is_object() && map.iter().all(|(k, v)| subset(&actual[k], v)),
        None => actual == expected,
    }
}

#[test]
fn all_pinned_steps_execute_through_the_desktop_bridge() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../docs/nips/NIP-BW.fixtures.json"))
            .expect("corpus");
    let mut count = 0;
    for case in corpus["cases"].as_array().expect("cases") {
        let mut input = bridge::Input {
            trust: serde_json::from_value(case["trust"].clone()).expect("trust"),
            external: serde_json::from_value(case["external"].clone()).expect("external"),
            now: case["now"].as_u64().expect("now"),
            events: case["input"]
                .as_array()
                .expect("input")
                .iter()
                .map(|label| corpus["events"][label.as_str().expect("label")]["event"].to_string())
                .collect(),
        };
        let (consumer, decisions) = bridge::replay(&input);
        for (i, decision) in decisions.iter().enumerate() {
            assert!(
                subset(
                    &serde_json::to_value(decision).expect("decision"),
                    &case["expected"][i]
                ),
                "{} step {i}",
                case["name"]
            );
            count += 1;
        }
        let forward = bridge::snapshot(&input);
        assert_eq!(forward["projection"], consumer.projection());
        input.events.reverse();
        assert_eq!(
            forward,
            bridge::snapshot(&input),
            "{} reverse",
            case["name"]
        );
        input.events.extend(input.events.clone());
        assert_eq!(
            forward,
            bridge::snapshot(&input),
            "{} duplicate/reload",
            case["name"]
        );
    }
    assert_eq!(count, 1804);
}
