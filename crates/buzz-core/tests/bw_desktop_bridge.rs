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

fn fixture_input(case: &Value, corpus: &Value) -> bridge::Input {
    bridge::Input {
        trust: serde_json::from_value(case["trust"].clone()).expect("trust"),
        external: serde_json::from_value(case["external"].clone()).expect("external"),
        now: case["now"].as_u64().expect("now"),
        events: case["input"]
            .as_array()
            .expect("input")
            .iter()
            .map(|label| corpus["events"][label.as_str().expect("label")]["event"].to_string())
            .collect(),
    }
}

/// P5_6A: deterministic Core snapshot export for the desktop's read-only
/// release/run/artifact views. Runs pinned fixture cases through the exact
/// same desktop bridge as the corpus test above and writes the
/// `bridge::snapshot` output, so the desktop tests render Core-authored
/// projections instead of hand-invented ones. Inert unless `BW_SNAPSHOT_EXPORT`
/// names the output path; regenerate with:
/// `BW_SNAPSHOT_EXPORT=desktop/src/features/projects/fixtures/bw-release-snapshots.json cargo test -p buzz-core --test bw_desktop_bridge export_release_view_snapshots`
/// then re-format the export with `pnpm --dir desktop format` (biome) so the
/// committed file stays check-clean.
#[test]
fn export_release_view_snapshots() {
    use sha2::{Digest, Sha256};
    let Ok(path) = std::env::var("BW_SNAPSHOT_EXPORT") else {
        return;
    };
    let bytes = include_bytes!("../../../docs/nips/NIP-BW.fixtures.json");
    assert_eq!(
        hex::encode(Sha256::digest(bytes)),
        "b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a"
    );
    let corpus: Value = serde_json::from_slice(bytes).expect("corpus");
    let case = |name: &str| -> Value {
        corpus["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|case| case["name"] == name)
            .unwrap_or_else(|| panic!("case {name}"))
            .clone()
    };
    let mut snapshots = serde_json::Map::new();
    for name in [
        // No release records at all: the view must render an honest empty state.
        "issue-state-positive",
        // Provider run success without any member verdict: never "completed".
        "build-run-positive",
        // Failed run, retried request, successful retry run.
        "retry-success",
        // Download digest mismatch: artifact refused by external evidence.
        "artifact-digest",
        // Non-durable download: handoff refused, artifact still accepted.
        "ephemeral-download",
        // Late/out-of-order delivery: same view as the in-order history.
        "late-handoff-delivery",
        // Historically resolved issue plus a new artifact starting unreviewed.
        "new-artifact-no-inherited-review",
        // Conflicting member verdicts on one slot.
        "verdict-conflict",
    ] {
        let case = case(name);
        snapshots.insert(
            name.to_owned(),
            bridge::snapshot(&fixture_input(&case, &corpus)),
        );
    }
    // Production shape: the same relay history as build-run-positive with the
    // empty external evidence the live desktop loader supplies
    // (`project_bw.rs::load`), so Core keeps the run/artifact chain pending.
    // The view must show missing evidence as pending, never a faked success.
    let mut production = fixture_input(&case("build-run-positive"), &corpus);
    production.external = buzz_core_pkg::bw::Evidence {
        git_readbacks: vec![],
        git_ancestry: vec![],
        provider_readbacks: vec![],
        downloads: vec![],
        host_authorization: serde_json::json!({"allowed": false}),
    };
    snapshots.insert(
        "build-run-positive-production-shape".to_owned(),
        bridge::snapshot(&production),
    );
    let export = serde_json::json!({
        "format": "bw-release-snapshots-v1",
        "source": "docs/nips/NIP-BW.fixtures.json",
        "fixture_sha256": hex::encode(Sha256::digest(bytes)),
        "generated_by": "BW_SNAPSHOT_EXPORT=desktop/src/features/projects/fixtures/bw-release-snapshots.json cargo test -p buzz-core --test bw_desktop_bridge export_release_view_snapshots",
        "snapshots": Value::Object(snapshots),
    });
    // Relative paths resolve against the repository root, not the crate CWD.
    let path = std::path::PathBuf::from(&path);
    let path = if path.is_relative() {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    } else {
        path
    };
    std::fs::write(path, serde_json::to_string_pretty(&export).expect("export"))
        .expect("write export");
}
