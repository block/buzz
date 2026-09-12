use buzz_core::bw::{parse_json, Consumer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
fn subset(actual: &Value, expected: &Value) -> bool {
    if let Some(o) = expected.as_object() {
        return actual.is_object() && o.iter().all(|(k, v)| subset(&actual[k], v));
    }
    actual == expected
}
#[test]
fn complete_pinned_corpus() {
    let bytes = include_bytes!("../../../docs/nips/NIP-BW.fixtures.json");
    assert_eq!(
        hex::encode(Sha256::digest(bytes)),
        "b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a"
    );
    assert_eq!(
        hex::encode(Sha256::digest(include_bytes!(
            "../../../docs/nips/NIP-BW.md"
        ))),
        "86184a747dda4692c64db65c216f4cdb6dfe69e765755abd1eabba893df55d29"
    );
    let data: Value = parse_json(bytes).expect("pinned strict corpus");
    let shapes: Value =
        parse_json(include_bytes!("../src/bw/schema.json")).expect("shared schemas");
    for key in ["schemas", "types", "arrays"] {
        assert_eq!(shapes[key], data[key]);
    }
    let mut export = Vec::new();
    let mut failures = Vec::new();
    let mut steps = 0;
    for case in data["cases"].as_array().expect("cases") {
        let mut consumer = Consumer::new(
            serde_json::from_value(case["trust"].clone()).expect("trust"),
            serde_json::from_value(case["external"].clone()).expect("evidence"),
            case["now"].as_u64().expect("now"),
        );
        let mut results = Vec::new();
        for (i, label) in case["input"].as_array().expect("inputs").iter().enumerate() {
            let event = &data["events"][label.as_str().expect("label")]["event"];
            let result = consumer.ingest(&serde_json::to_vec(event).expect("wire"));
            let expected = &case["expected"][i];
            let actual = serde_json::to_value(&result).expect("decision");
            if !subset(&actual, expected) {
                failures.push(format!(
                    "{} step {} {}: expected {} actual {}",
                    case["name"], i, label, expected, actual
                ));
            }
            results.push(actual);
            steps += 1;
        }
        export.push(json!({"case":case["name"],"steps":results}));
    }
    if let Ok(path) = std::env::var("BW_RESULT_EXPORT") {
        std::fs::write(Path::new(&path),serde_json::to_vec_pretty(&json!({"format":"nip-bw-results-v1","p1_commit":"529136c39aa42db64af61f811fc33dac73d16dd1","fixture_sha256":"b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a","document_sha256":"86184a747dda4692c64db65c216f4cdb6dfe69e765755abd1eabba893df55d29","cases":export})).expect("export")).expect("write export");
    }
    assert_eq!(steps, 1804);
    assert_eq!(export.len(), 98);
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
#[test]
fn strict_json_rejects_lossy_inputs() {
    for bad in [
        r#"{"a":1,"a":2}"#,
        r#"{"a":{"b":1,"b":2}}"#,
        r#"[1.0]"#,
        r#"[1e0]"#,
        r#"[-0]"#,
        r#"[-1]"#,
        r#"[NaN]"#,
        r#""\ud800""#,
        r#"{} {}"#,
        "\u{feff}{}",
    ] {
        assert!(parse_json(bad.as_bytes()).is_err(), "{bad}");
    }
}
